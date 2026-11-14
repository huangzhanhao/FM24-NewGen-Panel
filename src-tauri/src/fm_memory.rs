//! 从运行中的 FM 进程读取名单。
//! 【复用关系】产出与 .fm 存档扫描相同的 `PlayerRecord` / `RosterSummary`，直接复用名单缓存与替换流水线。
//! 【分层结构】进程与模块 → 构建识别 → 数据库根 → 人员表 → 字段解码。
//! 【关键约束】只读内存，绝不写入；只支持 Windows 与 FM24 已验证构建，认不出构建时明确报错。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::assets::FM_NATION_IDS_24;
use crate::fm_parser::{hair_display, skin_display, valid_code};
use crate::logger::LogSink;
use crate::nationality::NationalityTable;
use crate::rtf_scanner::{
    DetailedInfo, PersonKind, PlayerRecord, RosterSummary, RtfFormat, NEWGAN_UID_PREFIX,
};

// ---------------------------------------------------------------------------------------------
// FM24 已验证构建。字段偏移在所有 FM24 构建中一致，只有哈希与混合球员 vtable 不同。
// ---------------------------------------------------------------------------------------------

struct Layout {
    display: &'static str,
    sha256: &'static str,
    hybrid_vtable: u64,
}

/// Steam 24.4.2。
const FM24_STEAM: Layout =
    Layout { display: "Football Manager 2024 (Steam 24.4.2)", sha256: "E1059EEE82FA7832188831521A3FA633EC3260DD98D03DA48B16661147E3AB48", hybrid_vtable: 0x5C7E798 };
/// Epic 24.4.2。
const FM24_EPIC: Layout =
    Layout { display: "Football Manager 2024 (Epic 24.4.2)", sha256: "33E8A2E48C4986F2A15E11C4C93ACA1199029D66B19BD765403EB22DEA265378", hybrid_vtable: 0x5C7E798 };
/// Steam 24.4.0。
const FM24_240: Layout =
    Layout { display: "Football Manager 2024 (Steam 24.4.0)", sha256: "1473E5C3A778CB255A30C7A43D8346E27B031B47C42E93533097E049C110D58E", hybrid_vtable: 0x5C7E778 };
/// Steam 24.4.1 Beta。
const FM24_241: Layout =
    Layout { display: "Football Manager 2024 (Steam 24.4.1)", sha256: "ED5D0F9C3D06194FA5D19E5A2C14EF8F9C6AB8A9FF7E77220594E8A600A75F5F", hybrid_vtable: 0x5C7E798 };

const FM24_LAYOUTS: &[Layout] = &[FM24_STEAM, FM24_EPIC, FM24_240, FM24_241];

// FM24 共享偏移（来源：FMODD GameLayout 的 FM24_LAYOUT）。
const ACTUAL_PLAYER_VTABLE: u64 = 0x5A4E958;
const STAFF_PERSON_VTABLE: u64 = 0x5A4C358;
const PLAYER_PERSON_OFFSET: u64 = 0x278;
const HYBRID_PERSON_OFFSET: u64 = 0x368;
const PLAYER_CA_OFFSET: u64 = 0x200;
const PLAYER_WEIGHT_OFFSET: u64 = 0x14C;
const PERSON_NATIONALITY_OFFSET: u64 = 0x70;
const PERSON_RELATIONSHIPS_OFFSET: u64 = 0x80;
const PERSON_DOB_OFFSET: u64 = 0x44;
const PERSON_FULL_NAME_OFFSET: u64 = 0x48;
const PERSON_FIRST_NAME_OFFSET: u64 = 0x58;
const PERSON_LAST_NAME_OFFSET: u64 = 0x60;
const PERSON_COMMON_NAME_OFFSET: u64 = 0x68;
const PERSON_APPEARANCE_OFFSET: u64 = 0x90;
const PERSON_ETHNICITY_OFFSET: u64 = 0x91;
const PERSON_HAIR_COLOR_OFFSET: u64 = 0x92;
const PERSON_HAIR_LENGTH_OFFSET: u64 = 0x93;
const PERSON_SKIN_OFFSET: u64 = 0x94;
const ENTITY_UID_OFFSET: u64 = 0x0C;
/// 内存对象里与存档槽位 `slot-12` 同源的 4 字节前缀：新生标志、性别位、属性标记。
const ENTITY_FLAGS_OFFSET: u64 = 24;

/// 数据库根签名：`lea rdi,[rcx+disp32]; mov r9,[rip+disp32]; lea rcx,[rip+disp32]`。
/// 通配项是三条指令各自的 rel32 操作数，命中后用 +10 处的 rel32 求根地址。
const DATABASE_ROOT_PATTERN: [Option<u8>; 17] = [
    Some(0x48), Some(0x8D), Some(0xB9), None, None, None, None,
    Some(0x4C), Some(0x8B), Some(0x0D), None, None, None, None,
    Some(0x48), Some(0x8D), Some(0x0D),
];
const DATABASE_ROOT_REL32_OFFSET: usize = 10;
const DATABASE_ROOT_INSTRUCTION_SIZE: u64 = 14;

/// 人员表槽位与表对象里的向量偏移（FM24）。
const PERSON_TABLE_SLOT: u64 = 0x68;
const TABLE_VECTOR_OFFSET: u64 = 0x80;
const ENTITY_POINTER_SIZE: u64 = 8;

const MAX_PERSONS: usize = 2_000_000;
const CHUNK_OVERLAP: usize = DATABASE_ROOT_PATTERN.len() - 1;
const MODULE_SCAN_CHUNK: usize = 4 * 1024 * 1024;

/// 一次内存扫描的完整结果。
pub struct MemoryScan {
    pub records: Vec<PlayerRecord>,
    pub summary: RosterSummary,
}

// ---------------------------------------------------------------------------------------------
// 对外入口
// ---------------------------------------------------------------------------------------------

/// 扫描正在运行的 FM24，返回与 .fm 路线同构的名单。
pub fn scan_running_game(
    nat: &NationalityTable,
    logger: &impl LogSink,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(usize, usize),
) -> Result<MemoryScan, String> {
    let game = GameProcess::attach()?;
    let layout = game.identify_layout()?;
    logger.info("mem", format!("已连接游戏进程：{}", layout.display));

    let base = game.module_base;
    let root = game.database_root()?;
    let persons = game.person_table(root)?;
    let total = persons.len();
    logger.info("mem", format!("人员表：{total} 个对象，开始读取字段"));

    let id2code = nation_id_map(nat)?;
    let game_date = game
        .read_u32(base + layout_game_date_rva(&layout))
        .ok_or_else(|| "游戏日期读取失败".to_string())?;

    let mut records = Vec::with_capacity(total / 2);
    for (i, &p) in persons.iter().enumerate() {
        if i % 4096 == 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err("已取消游戏内存读取".into());
            }
            report(i, total);
        }
        if let Some(rec) = game.read_person(p, &layout, &id2code, game_date) {
            records.push(rec);
        }
    }
    report(total, total);

    let summary = RosterSummary {
        total: records.len(),
        newgan: records.iter().filter(|r| r.uid.starts_with(NEWGAN_UID_PREFIX)).count(),
        detailed: true,
        format: "FM 游戏内存名单（球员+职员）",
        format_code: RtfFormat::ExtPlayer,
        language: "English".to_string(),
    };
    // 与 .fm 路线的「球员/职员」账目对齐：两路线的总数差异必须能落到具体一侧，否则无从归因。
    let staff = records.iter().filter(|r| r.kind.is_staff()).count();
    logger.info(
        "mem",
        format!(
            "游戏内存读取完成：{} 条（球员 {} / 职员 {}，随机人 {}），构建 {}",
            summary.total,
            summary.total - staff,
            staff,
            summary.newgan,
            layout.display
        ),
    );
    Ok(MemoryScan { records, summary })
}

fn layout_game_date_rva(layout: &Layout) -> u64 {
    // 四份 FM24 构建的日期全局同址（24.4.1 的布局注释也确认日期与 24.4.2 一致）。
    let _ = layout;
    0x631D5BC
}

fn nation_id_map(nat: &NationalityTable) -> Result<HashMap<u32, String>, String> {
    let rows: Vec<Vec<String>> =
        serde_json::from_str(FM_NATION_IDS_24).map_err(|e| format!("国籍 id 表解析失败：{e}"))?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let Some(id) = row.first().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let raw = row.get(1).cloned().unwrap_or_default();
        let code = if nat.is_code(&raw) {
            raw
        } else {
            nat.resolve(row.get(2).map(String::as_str).unwrap_or(""))
                .or_else(|| nat.resolve(row.get(3).map(String::as_str).unwrap_or("")))
                .map(|(_, c)| c)
                .unwrap_or_default()
        };
        out.insert(id, code);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// 进程读取
// ---------------------------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    pub const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    pub const PROCESS_VM_READ: u32 = 0x0010;
    pub const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    pub const TH32CS_SNAPMODULE: u32 = 0x0000_0008;

    #[repr(C)]
    pub struct ProcessEntry32W {
        pub dw_size: u32,
        pub cnt_usage: u32,
        pub th32_process_id: u32,
        pub th32_default_heap_id: usize,
        pub th32_module_id: u32,
        pub cnt_threads: u32,
        pub th32_parent_process_id: u32,
        pub pc_pri_class_base: i32,
        pub dw_flags: u32,
        pub sz_exe_file: [u16; 260],
    }

    #[repr(C)]
    pub struct ModuleEntry32W {
        pub dw_size: u32,
        pub th32_module_id: u32,
        pub th32_process_id: u32,
        pub global_usage: u32,
        pub process_usage: u32,
        pub mod_base_addr: *mut u8,
        pub mod_base_size: u32,
        pub h_module: *mut c_void,
        pub sz_module: [u16; 256],
        pub sz_exe_path: [u16; 260],
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        pub fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        pub fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry32W) -> i32;
        pub fn Module32FirstW(snapshot: *mut c_void, entry: *mut ModuleEntry32W) -> i32;
        pub fn Module32NextW(snapshot: *mut c_void, entry: *mut ModuleEntry32W) -> i32;
        pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        pub fn ReadProcessMemory(
            process: *mut c_void,
            address: *const c_void,
            buffer: *mut c_void,
            size: usize,
            read: *mut usize,
        ) -> i32;
        pub fn CloseHandle(handle: *mut c_void) -> i32;
        pub fn GetLastError() -> u32;
    }
}

struct GameProcess {
    handle: *mut std::ffi::c_void,
    module_base: u64,
    module_size: u64,
    exe_path: String,
}

impl Drop for GameProcess {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            win::CloseHandle(self.handle);
        }
    }
}

impl GameProcess {
    #[cfg(not(windows))]
    fn attach() -> Result<Self, String> {
        Err("游戏内存读取仅支持 Windows".into())
    }

    #[cfg(windows)]
    fn attach() -> Result<Self, String> {
        unsafe {
            let pid = find_fm_process().ok_or_else(|| "未检测到正在运行的 FM24，请先进入游戏并载入存档".to_string())?;
            let handle = win::OpenProcess(win::PROCESS_QUERY_INFORMATION | win::PROCESS_VM_READ, 0, pid);
            if handle.is_null() {
                return Err(format!(
                    "无法读取游戏进程（错误 {}），请尝试以管理员身份运行本工具",
                    win::GetLastError()
                ));
            }
            let (module_base, module_size, exe_path) = module_info(pid, handle)
                .ok_or_else(|| "无法定位游戏模块 fm.exe".to_string())?;
            Ok(Self { handle, module_base, module_size, exe_path })
        }
    }

    fn identify_layout(&self) -> Result<&'static Layout, String> {
        let hash = file_sha256(&self.exe_path)?;
        FM24_LAYOUTS
            .iter()
            .find(|l| l.sha256.eq_ignore_ascii_case(&hash))
            .ok_or_else(|| format!("当前游戏构建暂不支持（SHA256 {hash}），已知构建：FM24 Steam/Epic 24.4.0–24.4.2"))
    }

    fn read(&self, address: u64, len: usize) -> Option<Vec<u8>> {
        if !is_user_pointer(address) || len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len];
        let mut read = 0usize;
        #[cfg(windows)]
        let ok = unsafe {
            win::ReadProcessMemory(
                self.handle,
                address as *const std::ffi::c_void,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                len,
                &mut read,
            )
        };
        #[cfg(not(windows))]
        let ok = 0;
        (ok != 0 && read == len).then_some(buf)
    }

    fn read_u32(&self, address: u64) -> Option<u32> {
        self.read(address, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_u64(&self, address: u64) -> Option<u64> {
        self.read(address, 8).map(|b| u64::from_le_bytes(b[..8].try_into().unwrap()))
    }

    /// 在全模块范围内按通配签名定位数据库根，再把 rel32 解成绝对地址。
    fn database_root(&self) -> Result<u64, String> {
        let end = self.module_base + self.module_size;
        let mut cursor = self.module_base;
        let mut tail: Vec<u8> = Vec::new();
        while cursor < end {
            let take = ((end - cursor) as usize).min(MODULE_SCAN_CHUNK);
            let Some(chunk) = self.read(cursor, take) else {
                cursor += take as u64;
                tail.clear();
                continue;
            };
            let mut window = Vec::with_capacity(tail.len() + chunk.len());
            window.extend_from_slice(&tail);
            window.extend_from_slice(&chunk);
            if let Some(off) = find_pattern(&window, &DATABASE_ROOT_PATTERN) {
                let hit = cursor - tail.len() as u64 + off as u64;
                let header = self
                    .read(hit, DATABASE_ROOT_INSTRUCTION_SIZE as usize)
                    .ok_or_else(|| "数据库根签名的指令读取失败".to_string())?;
                let rel = i32::from_le_bytes(
                    header[DATABASE_ROOT_REL32_OFFSET..DATABASE_ROOT_REL32_OFFSET + 4].try_into().unwrap(),
                ) as i64;
                let root = (hit as i64 + DATABASE_ROOT_INSTRUCTION_SIZE as i64 + rel) as u64;
                return is_user_pointer(root)
                    .then_some(root)
                    .ok_or_else(|| "数据库根地址不合法".to_string());
            }
            tail = window[window.len().saturating_sub(CHUNK_OVERLAP)..].to_vec();
            cursor += take as u64;
        }
        Err("未找到数据库根签名，游戏可能尚未载入存档".into())
    }

    /// 读取人员表中的对象指针数组。向量头在表对象 +0x80，格式为 begin/end/cap。
    fn person_table(&self, root: u64) -> Result<Vec<u64>, String> {
        let table = self
            .read_u64(root + PERSON_TABLE_SLOT)
            .ok_or_else(|| "人员表对象读取失败".to_string())?;
        let vector = self
            .read_u64(table + TABLE_VECTOR_OFFSET)
            .ok_or_else(|| "人员表向量读取失败".to_string())?;
        let header = self.read(vector, 24).ok_or_else(|| "人员表向量头读取失败".to_string())?;
        let begin = u64::from_le_bytes(header[0..8].try_into().unwrap());
        let end = u64::from_le_bytes(header[8..16].try_into().unwrap());
        if !is_user_pointer(begin) || end <= begin || (end - begin) % ENTITY_POINTER_SIZE != 0 {
            return Err("人员表向量范围不合法".into());
        }
        let count = ((end - begin) / ENTITY_POINTER_SIZE) as usize;
        if count == 0 || count > MAX_PERSONS {
            return Err(format!("人员表数量异常：{count}"));
        }
        let mut out = Vec::with_capacity(count);
        let mut offset = 0usize;
        while offset < count {
            let take = (count - offset).min(8192);
            let Some(chunk) = self.read(begin + (offset as u64 * ENTITY_POINTER_SIZE), take * 8) else {
                return Err("人员表指针数组读取失败".into());
            };
            for item in chunk.chunks_exact(8) {
                out.push(u64::from_le_bytes(item.try_into().unwrap()));
            }
            offset += take;
        }
        Ok(out)
    }

    /// 解码单个人员对象；非球员/职员（经纪人、记者等）返回 None。
    fn read_person(
        &self,
        person: u64,
        layout: &Layout,
        id2code: &HashMap<u32, String>,
        game_date: u32,
    ) -> Option<PlayerRecord> {
        if !is_user_pointer(person) {
            return None;
        }
        let block = self.read(person, 0x100)?;
        let base = self.module_base;
        let person_vtable = le_u64(&block, 0);
        let staff = person_vtable == base + STAFF_PERSON_VTABLE;
        let outer = if staff {
            0
        } else {
            let hybrid = person.checked_sub(HYBRID_PERSON_OFFSET)?;
            let actual = person.checked_sub(PLAYER_PERSON_OFFSET)?;
            // 两个地址各自探测：对象若是一段提交内存的首个对象，person-0x368 会落在未提交页而读失败，
            // 此时不能直接放弃，必须继续试 person-0x278——否则纯球员会被整条丢掉。
            let is_hybrid = self
                .read_u64(hybrid)
                .map(|v| v == base + layout.hybrid_vtable)
                .unwrap_or(false);
            let is_actual = self
                .read_u64(actual)
                .map(|v| v == base + ACTUAL_PLAYER_VTABLE)
                .unwrap_or(false);
            if is_hybrid {
                hybrid
            } else if is_actual {
                actual
            } else {
                return None;
            }
        };

        let uid = le_u32(&block, ENTITY_UID_OFFSET as usize);
        if uid == 0 {
            return None;
        }
        let newgen = block[ENTITY_FLAGS_OFFSET as usize] >= 8;
        let female = block[ENTITY_FLAGS_OFFSET as usize + 1] & 0x10 != 0;
        let eth = block[PERSON_ETHNICITY_OFFSET as usize];
        let status = block[PERSON_APPEARANCE_OFFSET as usize];
        let hair_raw = block[PERSON_HAIR_COLOR_OFFSET as usize];
        let hair_len = block[PERSON_HAIR_LENGTH_OFFSET as usize];
        let skin_raw = block[PERSON_SKIN_OFFSET as usize];

        let (ca, pa, height) = if staff {
            let skills = self.read(person.checked_sub(0x24)?, 6)?;
            (le_u16(&skills, 2), le_u16(&skills, 4), 0)
        } else {
            let ca_pa = self.read(outer + PLAYER_CA_OFFSET, 4)?;
            let body = self.read(outer + PLAYER_WEIGHT_OFFSET, 4)?;
            (le_u16(&ca_pa, 0), le_u16(&ca_pa, 2), le_u16(&body, 2))
        };

        let name = self.read_display_name(&block);
        let primary_nat = self.read_nation_code(&block, PERSON_NATIONALITY_OFFSET, id2code);
        let sec_nat = self.read_second_nation(&block, id2code);
        let age = age_from(game_date, le_u32(&block, PERSON_DOB_OFFSET as usize));

        let kind = match (staff, female) {
            (false, false) => PersonKind::PlayerMale,
            (false, true) => PersonKind::PlayerFemale,
            (true, false) => PersonKind::StaffMale,
            (true, true) => PersonKind::StaffFemale,
        };
        let detailed = DetailedInfo {
            skin_code: skin_display(skin_raw, eth).to_string(),
            avatar: String::new(),
            is_newgan: if newgen { "是".into() } else { "否".into() },
            gender: if female { "女".into() } else { "男".into() },
            age: (age > 0).then(|| age.to_string()).unwrap_or_default(),
            height: (height > 0).then(|| height.to_string()).unwrap_or_default(),
            club: String::new(),
            based: String::new(),
            preferred_job: String::new(),
            ca: ca.to_string(),
            pa: pa.to_string(),
            rca: String::new(),
            extra_fields: Vec::new(),
        };
        Some(PlayerRecord {
            uid: if newgen { format!("{NEWGAN_UID_PREFIX}{uid}") } else { uid.to_string() },
            primary_nat,
            sec_nat,
            name,
            hair_length: valid_code(hair_len, 0, 5),
            hair_color: valid_code(hair_display(hair_raw, status, eth), 1, 19),
            ethnicity_code: valid_code(eth, 0, 10),
            kind,
            detailed: Some(detailed),
        })
    }

    fn read_display_name(&self, block: &[u8]) -> String {
        let full = self.string_at(le_u64(block, PERSON_FULL_NAME_OFFSET as usize));
        if !full.is_empty() {
            return full;
        }
        let first = self.nested_string(le_u64(block, PERSON_FIRST_NAME_OFFSET as usize));
        let last = self.nested_string(le_u64(block, PERSON_LAST_NAME_OFFSET as usize));
        match (first.is_empty(), last.is_empty()) {
            (false, false) => format!("{first} {last}"),
            (false, true) => first,
            (true, false) => last,
            (true, true) => self.nested_string(le_u64(block, PERSON_COMMON_NAME_OFFSET as usize)),
        }
    }

    fn nested_string(&self, link: u64) -> String {
        if !is_user_pointer(link) {
            return String::new();
        }
        self.read_u64(link).map(|p| self.string_at(p)).unwrap_or_default()
    }

    /// FM 字符串：`{u32 len}{utf8}`。长度先过界校验，坏指针不 panic 也不读出海量垃圾。
    fn string_at(&self, pointer: u64) -> String {
        if !is_user_pointer(pointer) {
            return String::new();
        }
        let Some(len_bytes) = self.read(pointer, 4) else { return String::new() };
        let len = u32::from_le_bytes(len_bytes[..4].try_into().unwrap()) as usize;
        if len == 0 || len > 512 {
            return String::new();
        }
        self.read(pointer + 4, len)
            .map(|b| String::from_utf8_lossy(&b).trim().to_string())
            .unwrap_or_default()
    }

    fn read_nation_code(&self, block: &[u8], offset: u64, id2code: &HashMap<u32, String>) -> String {
        // block+offset 放的就是国籍对象地址，uid 直接读该对象 +0x0C；再解引用一次会读到 vtable。
        let object = le_u64(block, offset as usize);
        if !is_user_pointer(object) {
            return String::new();
        }
        match self.read_u32(object + ENTITY_UID_OFFSET) {
            Some(id) => id2code.get(&id).cloned().unwrap_or_default(),
            None => String::new(),
        }
    }

    /// 关系区第一条 `object_type=8 / relation_type=9` 即第二国籍。
    fn read_second_nation(&self, block: &[u8], id2code: &HashMap<u32, String>) -> String {
        let Some(vector) = self.read_u64(le_u64(block, PERSON_RELATIONSHIPS_OFFSET as usize)) else {
            return String::new();
        };
        let (Some(begin), Some(end)) = (self.read_u64(vector), self.read_u64(vector + 8)) else {
            return String::new();
        };
        if !is_user_pointer(begin) || end <= begin || (end - begin) % 16 != 0 || (end - begin) / 16 > 4096 {
            return String::new();
        }
        let count = ((end - begin) / 16) as usize;
        let Some(data) = self.read(begin, count * 16) else { return String::new() };
        for rec in data.chunks_exact(16) {
            if rec[0x0A] != 8 || rec[0x0B] != 9 {
                continue;
            }
            let target = u64::from_le_bytes(rec[..8].try_into().unwrap());
            if let Some(id) = self.read_u32(target + ENTITY_UID_OFFSET) {
                return id2code.get(&id).cloned().unwrap_or_default();
            }
        }
        String::new()
    }
}

// ---------------------------------------------------------------------------------------------
// 纯函数辅助
// ---------------------------------------------------------------------------------------------

fn le_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn le_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn is_user_pointer(value: u64) -> bool {
    (0x1_0000..0x0000_7FFF_FFFF_FFFF).contains(&value)
}

fn find_pattern(haystack: &[u8], pattern: &[Option<u8>]) -> Option<usize> {
    if pattern.is_empty() || haystack.len() < pattern.len() {
        return None;
    }
    haystack
        .windows(pattern.len())
        .position(|window| window.iter().zip(pattern).all(|(b, p)| p.is_none_or(|v| *b == v)))
}

/// 年龄以游戏内日期与出生“日序”比较：同一年里生日还没到就减一岁。
fn age_from(game_date: u32, birth: u32) -> u32 {
    let game_year = game_date >> 16;
    let game_day = game_date & 0x1FF;
    let birth_year = birth >> 16;
    let birth_day = birth & 0x1FF;
    if game_year < 1900 || birth_year < 1900 || game_year < birth_year {
        return 0;
    }
    let mut age = game_year - birth_year;
    if game_day < birth_day {
        age = age.saturating_sub(1);
    }
    age
}

// ---------------------------------------------------------------------------------------------
// Windows 进程/模块枚举
// ---------------------------------------------------------------------------------------------

#[cfg(windows)]
fn find_fm_process() -> Option<u32> {
    unsafe {
        let snapshot = win::CreateToolhelp32Snapshot(win::TH32CS_SNAPPROCESS, 0);
        if snapshot as isize == -1 {
            return None;
        }
        let mut entry: win::ProcessEntry32W = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<win::ProcessEntry32W>() as u32;
        let mut found = None;
        if win::Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let name = utf16z(&entry.sz_exe_file).to_ascii_lowercase();
                if name == "fm.exe" {
                    found = Some(entry.th32_process_id);
                    break;
                }
                if win::Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        win::CloseHandle(snapshot);
        found
    }
}

#[cfg(windows)]
fn module_info(pid: u32, process: *mut std::ffi::c_void) -> Option<(u64, u64, String)> {
    let _ = process;
    unsafe {
        let snapshot = win::CreateToolhelp32Snapshot(win::TH32CS_SNAPMODULE, pid);
        if snapshot as isize == -1 {
            return None;
        }
        let mut entry: win::ModuleEntry32W = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<win::ModuleEntry32W>() as u32;
        let mut found = None;
        if win::Module32FirstW(snapshot, &mut entry) != 0 {
            loop {
                if utf16z(&entry.sz_module).eq_ignore_ascii_case("fm.exe") {
                    found = Some((
                        entry.mod_base_addr as u64,
                        entry.mod_base_size as u64,
                        utf16z(&entry.sz_exe_path),
                    ));
                    break;
                }
                if win::Module32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        win::CloseHandle(snapshot);
        found
    }
}

#[cfg(windows)]
fn utf16z(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// 流式 SHA256：EXE 约 489 MB，不能整文件读进内存。
fn file_sha256(path: &str) -> Result<String, String> {
    use ring::digest;
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|e| format!("无法读取游戏可执行文件：{e}"))?;
    let mut ctx = digest::Context::new(&digest::SHA256);
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("读取游戏可执行文件失败：{e}"))?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    Ok(ctx.finish().as_ref().iter().map(|b| format!("{b:02X}")).collect())
}

