//! `.fm` 存档解析器（FM24）：26 B 头 → 尾块 TOC 整帧解压 → 版本闸门 → `game_db` 整帧解压，
//! 再按 game_db-only 的启发式流水线还原「球员 + 职员」混合名单。
//!
//! 【数据来源】只读存档自带的 game_db、三段名池、国籍名次表与俱乐部名表；不读任何 RTF 导出、
//! 不按 uid 回填显示层字段。取不到的列一律留空并保留行，绝不猜填或丢行。
//!
//! 【版本边界】所有随版本漂移的常量集中在 [`FmLayout`]；流水线本身按 FM24 的偏移编写。
//! 换版本要先在 [`version_layout`] 增加孔位并核对整条流水线，不能只换字典。

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::assets::{FM_NATION_IDS_24, FM_NATION_RANKS_24};
use crate::logger::LogSink;
use crate::nationality::NationalityTable;
use crate::rtf_scanner::{
    person_kind, DetailedInfo, PlayerRecord, RosterCounts, RosterSummary, RtfFormat,
    NEWGAN_UID_PREFIX,
};

// ===================== 常量 ======================================================================

/// 肤色原始字节 0xFF 表示「按种族取默认档」，渲染表来自参考实现。
const ETH_SKIN: [u8; 11] = [5, 7, 12, 18, 14, 13, 16, 2, 15, 16, 8];
/// 主绑定的距离窗口；四档补配另有各自窗口。
const BIND_WINDOW: usize = 1024;
/// uid / selfUid 的合法上界（0 < v < 0x7866C101）。**不设 uid > 999999 的下界**：那会把职员
/// 召回率从 95.856% 压到 84.081%，替换目标集整体错配。
const UID_CEIL: u32 = 0x7866_C101;
/// 名池 id 的「无此字段」哨兵。
const SENT: u32 = 0xFFFF_FFFF;
/// 球员记录体 cid 的「无俱乐部」哨兵（u24 全 1）。
const CID_NONE: u32 = 0x00FF_FFFF;
/// 节绝对偏移 = 目录里的 A + 26。
const SECTION_BASE: u64 = 26;
/// 名池签名首字。
const POOL_SIGNATURE: u32 = 46421;
/// 俱乐部名记录扫描上限（参考实现同值）；俱乐部表集中在 game_db 前段。
const CLUB_SCAN_CAP: usize = 45_000_000;
/// 队伍 id 上界（本档最大 113,838）。
const MAX_TEAM: u32 = 200_000;
/// 批量追加结果、上报进度与检查取消的间隔。
const BATCH: usize = 4096;

/// §9 错误契约的 8 个阶段名。不建枚举类型：错误文案要逐字稳定，枚举只会多一层无意义的转换。
const STAGE_PRECHECK: &str = "前置校验";
const STAGE_HEADER: &str = "容器头";
const STAGE_TOC: &str = "TOC";
const STAGE_SECTION: &str = "目标节解压";
const STAGE_VERSION: &str = "版本判定";
const STAGE_ENUM: &str = "人员枚举";
const STAGE_FIELDS: &str = "字段还原";
const STAGE_CACHE: &str = "写缓存";
const STAGE_NAMES: [&str; 8] = [
    STAGE_PRECHECK, STAGE_HEADER, STAGE_TOC, STAGE_SECTION, STAGE_VERSION, STAGE_ENUM,
    STAGE_FIELDS, STAGE_CACHE,
];

/// 版本判定完成前的版本槽位文案。
const VERSION_UNKNOWN: &str = "未知（版本判定未完成）";
/// 版本串的 +build 形态正则（第 1 组为 major）。
const VERSION_BUILD_RE: &str = r"([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,4})\+([0-9]{1,10})";
/// 无 +build 段的版本串正则（第 1 组为 major）。
const VERSION_PLAIN_RE: &str = r"([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,4})";
/// 俱乐部名的合法字符类；Unicode 字母/数字 + 常见标点，长度 1..90。
const NAME_CHARS_RE: &str = r"^[\p{L}\p{N} ·&()（）./,'‘’“”:；;!！?？*_*%#@=\-]{1,90}$";

// ===================== 小工具 ====================================================================

#[inline]
fn u16_le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

#[inline]
fn u24_le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], 0])
}

#[inline]
fn u32_le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

#[inline]
fn u64_le(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([
        b[i], b[i + 1], b[i + 2], b[i + 3], b[i + 4], b[i + 5], b[i + 6], b[i + 7],
    ])
}

/// `base` 加带符号偏移；越界返回 None，供记录体/槽位这类可漂移的取数使用。
#[inline]
fn at(base: usize, off: isize) -> Option<usize> {
    (base as isize).checked_add(off).and_then(|v| usize::try_from(v).ok())
}

/// 把「期望 / 实际」写进诊断账目，并生成唯一的对外错误文案。
fn fail(diag: &mut Diag, path: &Path, version: &str, stage: &str, detail: impl AsRef<str>) -> String {
    let detail = detail.as_ref();
    diag.stage = stage.to_string();
    diag.reason = detail.to_string();
    fm_err(path, version, stage, detail)
}

/// §9 三要素模板。`path.file_name()` 只给基名，完整路径只进日志。
fn fm_err(path: &Path, version: &str, stage: &str, detail: &str) -> String {
    // 阶段名收敛到契约常量表：既守住 8 个常量都是活跃引用，也避免调用点手写近似文案。
    let canonical = STAGE_NAMES
        .iter()
        .copied()
        .find(|s| *s == stage)
        .unwrap_or(stage);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("(无文件名)");
    format!("FM 存档失败：文件「{name}」｜版本：{version}｜阶段：{canonical}｜{detail}")
}

/// 版本槽位：判定前为「未知」，判定后写实测 major。
fn version_label(major: Option<u8>) -> String {
    match major {
        None => VERSION_UNKNOWN.to_string(),
        Some(24) => "FM24 (major=24)".to_string(),
        Some(m) => format!("FM{m} (major={m})"),
    }
}

/// Error 日志的完整上下文：完整路径、大小、P、n1/n2、sum(compLen)、节长度、版本串、major、
/// 阶段、原因、耗时、person 跳过数。放在一处生成，避免散落的 `format!` 漏项。
#[derive(Default)]
struct Diag {
    file_size: u64,
    p: u64,
    to: u64,
    n1: u32,
    n2: u32,
    sum_expect: u64,
    sum_actual: u64,
    comp_len: u64,
    decomp_len: u64,
    actual_len: u64,
    version_raw: String,
    major: String,
    stage: String,
    reason: String,
    person_skipped: usize,
}

impl Diag {
    fn render(&self, path: &Path, elapsed_ms: u128) -> String {
        format!(
            "完整路径={}｜大小={} B｜P={}｜to={}｜TOC n1={}/n2={}｜sum(compLen) 期望={} B/实际={} B｜\
             compLen={} B｜decompLen={} B｜实解长度={} B｜版本原始串={}｜major={}｜阶段={}｜原因={}｜\
             耗时={} ms｜person 跳过数={}",
            path.display(),
            self.file_size,
            self.p,
            self.to,
            self.n1,
            self.n2,
            self.sum_expect,
            self.sum_actual,
            self.comp_len,
            self.decomp_len,
            self.actual_len,
            self.version_raw,
            self.major,
            self.stage,
            self.reason,
            elapsed_ms,
            self.person_skipped,
        )
    }
}

/// 所有失败走这里：写一条带完整上下文的 Error 日志，再把三要素文案原样交回调用方。
fn log_failure(logger: &impl LogSink, path: &Path, diag: &Diag, started: Instant, err: String) -> String {
    logger.error("fm", diag.render(path, started.elapsed().as_millis()));
    logger.error("fm", err.clone());
    err
}

// ===================== 版本孔位 =================================================================

/// 名池签名与条目约束。换版本时这些是最先漂移的一组常量。
#[derive(Clone, Copy)]
struct NamePoolLayout {
    signature: u32,
    c1_ratio: u32,
    c3_ratio: u32,
    max_name_len: u32,
    first_key: u32,
}

/// 记录体的取数偏移；负值是记录头之前的字段。全部集中在孔位，避免散落在流水线里。
#[derive(Clone, Copy)]
struct RecordOffsets {
    kind: isize,
    pindex: isize,
    self_uid: isize,
    j: isize,
    pos0: isize,
    pos1: isize,
    attr1: isize,
    ca: isize,
    pa: isize,
    st_height: isize,
    j_stride: isize,
    cid: isize,
    pl_height: isize,
    pl_ca: isize,
    pl_pa: isize,
}

#[derive(Clone, Copy)]
struct FmLayout {
    name_pool: NamePoolLayout,
    record_offsets: RecordOffsets,
    dict: &'static str,
    db_section: &'static str,
}

static FM24_LAYOUT: FmLayout = FmLayout {
    name_pool: NamePoolLayout {
        signature: POOL_SIGNATURE,
        c1_ratio: 4,
        c3_ratio: 8,
        max_name_len: 200,
        first_key: 0,
    },
    record_offsets: RecordOffsets {
        kind: -7,
        pindex: -19,
        self_uid: -15,
        j: -6,
        pos0: 23,
        pos1: 38,
        attr1: 92,
        ca: 2,
        pa: 4,
        st_height: 97,
        j_stride: 3,
        cid: 15,
        pl_height: 122,
        pl_ca: 0,
        pl_pa: 2,
    },
    dict: FM_NATION_RANKS_24,
    db_section: "game_db",
};

/// 单点版本闸门：只放行 FM24，其余立即拒绝且不得触碰 game_db。
fn version_layout(major: u8) -> Result<FmLayout, String> {
    match major {
        24 => Ok(FM24_LAYOUT),
        other => Err(format!(
            "不支持的 FM 版本 major={other}，当前仅支持 24；未解压 game_db"
        )),
    }
}

// ===================== 容器头 / TOC =============================================================

#[derive(Clone)]
struct TocEntry {
    name: String,
    a: u64,
    comp_len: u64,
    decomp_len: u64,
}

struct Toc {
    entries: Vec<TocEntry>,
    n1: u32,
    n2: u32,
    sum_comp: u64,
    p: u64,
    to: u64,
}

impl Toc {
    fn find(&self, name: &str) -> Option<&TocEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

/// 26 B 头 + 尾块 TOC 整帧解压。TOC 记录严格按声明条数循环，绝不「解析失败即停」。
fn read_header_and_toc(path: &Path, version: &str, diag: &mut Diag) -> Result<Toc, String> {
    let mut f = File::open(path).map_err(|e| {
        fail(
            diag,
            path,
            version,
            STAGE_HEADER,
            format!("期望可读取存档文件，实际打开失败：{e}"),
        )
    })?;

    let mut h = [0u8; 26];
    f.read_exact(&mut h).map_err(|e| {
        fail(
            diag,
            path,
            version,
            STAGE_HEADER,
            format!("容器头期望 26 B，实际读取失败：{e}"),
        )
    })?;
    if h[0] != 2 || h[1] != 1 || &h[2..6] != b"fmf." || h[6] != 8 {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_HEADER,
            format!(
                "容器头期望 h[0]=2,h[1]=1,magic=fmf.,h[6]=8，实际 h[0]={},h[1]={},magic={:?},h[6]={}",
                h[0],
                h[1],
                &h[2..6],
                h[6]
            ),
        ));
    }
    let p = u64_le(&h, 9);
    let to = 9u64.saturating_add(p);
    diag.p = p;
    diag.to = to;

    if to + 9 > diag.file_size {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_HEADER,
            format!(
                "尾块起点期望 <= 文件大小，实际 to+9={} > 文件大小={} B",
                to.saturating_add(9),
                diag.file_size
            ),
        ));
    }

    f.seek(SeekFrom::Start(to)).map_err(|e| {
        fail(
            diag,
            path,
            version,
            STAGE_TOC,
            format!("期望定位到尾块偏移 {to}，实际 seek 失败：{e}"),
        )
    })?;
    let mut tail = Vec::new();
    f.read_to_end(&mut tail).map_err(|e| {
        fail(
            diag,
            path,
            version,
            STAGE_TOC,
            format!("期望读取尾块，实际失败：{e}"),
        )
    })?;
    if tail.len() < 9 {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_TOC,
            format!("尾块头期望 9 B，实际 {} B", tail.len()),
        ));
    }
    let toc_bytes = zstd::stream::decode_all(&tail[9..]).map_err(|e| {
        fail(
            diag,
            path,
            version,
            STAGE_TOC,
            format!("TOC 整帧期望可 zstd 解压，实际解压失败：{e}"),
        )
    })?;
    if toc_bytes.len() < 64 {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_TOC,
            format!("TOC 解压后期望 >=64 B，实际 {} B", toc_bytes.len()),
        ));
    }

    // 目录第一块：p = 4 + u32@0，随后 n1 条记录。
    let mut pos = 4usize.saturating_add(u32_le(&toc_bytes, 0) as usize);
    if pos + 4 > toc_bytes.len() {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_TOC,
            format!(
                "TOC 期望在偏移 {pos} 处读到 n1，实际目录仅 {} B",
                toc_bytes.len()
            ),
        ));
    }
    let n1 = u32_le(&toc_bytes, pos);
    pos += 4;
    let (mut entries, first_end) = read_toc_block(&toc_bytes, pos, n1, path, version, diag, "第一块")?;
    pos = first_end;

    // 第二块头：u32@p == 1 时按 gl / n2 再读一块。
    let mut n2 = 0u32;
    if pos + 4 <= toc_bytes.len() && u32_le(&toc_bytes, pos) == 1 {
        if pos + 8 > toc_bytes.len() {
            return Err(fail(diag, path, version, STAGE_TOC, "第二块头期望 8 B，实际目录已截断"));
        }
        let gl = u32_le(&toc_bytes, pos + 4) as usize;
        let n2_at = pos + 8 + gl;
        if n2_at + 4 > toc_bytes.len() {
            return Err(fail(diag, path, version, STAGE_TOC, "第二块 n2 期望 4 B，实际目录已截断"));
        }
        n2 = u32_le(&toc_bytes, n2_at);
        pos = n2_at + 4;
        let (second, _) = read_toc_block(&toc_bytes, pos, n2, path, version, diag, "第二块")?;
        entries.extend(second);
    }
    diag.n1 = n1;
    diag.n2 = n2;

    let sum: u64 = entries.iter().map(|e| e.comp_len).sum();
    diag.sum_expect = to.saturating_sub(26);
    diag.sum_actual = sum;
    if sum.saturating_add(26) != to {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_TOC,
            format!(
                "节密铺校验期望 sum(compLen)+26 == 9+P = {to} B，实际 sum(compLen)={sum} B、+26={} B",
                sum.saturating_add(26)
            ),
        ));
    }
    Ok(Toc { entries, n1, n2, sum_comp: sum, p, to })
}

/// 按声明条数读一组 TOC 记录；任一条不合法即整份失败，但循环次数恒为 n。
fn read_toc_block(
    toc: &[u8],
    start: usize,
    n: u32,
    path: &Path,
    version: &str,
    diag: &mut Diag,
    label: &str,
) -> Result<(Vec<TocEntry>, usize), String> {
    let mut out = Vec::with_capacity(n.min(4096) as usize);
    let mut p = start;
    for i in 0..n {
        // rec(o)：{u32 nl(1..250)}{name}{u32 extLen==4}{4B ext}{u64 A}{u64 compLen}{u64 decompLen}{i64 ts1}{i64 ts2}
        if p + 8 > toc.len() {
            return Err(fail(
                diag,
                path,
                version,
                STAGE_TOC,
                format!("{label} 第 {i}/{n} 条期望至少 8 B 记录头，实际目录仅 {} B", toc.len()),
            ));
        }
        let nl = u32_le(toc, p) as usize;
        if !(1..=250).contains(&nl) || p + 4 + nl + 8 > toc.len() {
            return Err(fail(
                diag,
                path,
                version,
                STAGE_TOC,
                format!("{label} 第 {i}/{n} 条期望名长 1..250，实际 nl={nl}"),
            ));
        }
        let name_bytes = &toc[p + 4..p + 4 + nl];
        if !name_bytes.iter().all(|b| (0x20..=0x7e).contains(b)) {
            return Err(fail(
                diag,
                path,
                version,
                STAGE_TOC,
                format!("{label} 第 {i}/{n} 条节名期望 ASCII 0x20..0x7e，实际含控制字符"),
            ));
        }
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        let mut q = p + 4 + nl;
        let ext_len = u32_le(toc, q);
        if ext_len != 4 {
            return Err(fail(
                diag,
                path,
                version,
                STAGE_TOC,
                format!("{label} 第 {i}/{n} 条期望 extLen=4，实际 {ext_len}"),
            ));
        }
        q += 8; // 跳过 extLen 与 4 B ext，落在 A
        if q + 40 > toc.len() {
            return Err(fail(
                diag,
                path,
                version,
                STAGE_TOC,
                format!("{label} 第 {i}/{n} 条期望 40 B 字段区，实际目录仅 {} B", toc.len()),
            ));
        }
        out.push(TocEntry {
            name,
            a: u64_le(toc, q),
            comp_len: u64_le(toc, q + 8),
            decomp_len: u64_le(toc, q + 16),
        });
        p = q + 40;
    }
    Ok((out, p))
}

// ===================== 版本判定 ================================================================

/// 长度前缀字符串扫描：版本串在 game_info / save_game_summary 里都是 {u32 len}{utf8} 形态。
fn length_prefixed_strings(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        let len = u32_le(bytes, i) as usize;
        if (1..=64).contains(&len) && i + 4 + len <= bytes.len() {
            if let Ok(s) = std::str::from_utf8(&bytes[i + 4..i + 4 + len]) {
                out.push(s.to_string());
                i += 4 + len;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// 解版本节 → 取 major。搜索对象按 `game_info` / `save_game_summary` 双路兜底：真档里
/// `save_game_summary` 只有无 build 段的 `24.4.2`，`game_info` 才是 `24.3.0+0`。
fn read_version(
    path: &Path,
    toc: &Toc,
    version: &str,
    diag: &mut Diag,
) -> Result<(u8, String), String> {
    let mut summary: Vec<String> = Vec::new();
    let mut info: Vec<String> = Vec::new();
    if let Some(e) = toc.find("save_game_summary") {
        summary = length_prefixed_strings(&decompress_section(path, e, version, diag)?);
    }
    if let Some(e) = toc.find("game_info") {
        info = length_prefixed_strings(&decompress_section(path, e, version, diag)?);
    }
    if summary.is_empty() && info.is_empty() {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_SECTION,
            "期望 TOC 含 save_game_summary 或 game_info 版本节，实际两者都缺或为空",
        ));
    }

    // +build 优先：save_game_summary → game_info；再退到无 build 形态。
    let build_re = regex::Regex::new(VERSION_BUILD_RE).expect("版本正则为字面量常量");
    let plain_re = regex::Regex::new(VERSION_PLAIN_RE).expect("版本正则为字面量常量");
    let sources: [(&[String], &str); 2] = [(&summary, "save_game_summary"), (&info, "game_info")];
    let found = sources
        .iter()
        .find_map(|(cands, _)| find_major_in(cands, &build_re))
        .or_else(|| {
            sources
                .iter()
                .find_map(|(cands, _)| find_major_in(cands, &plain_re))
        });

    let (major, raw) = match found {
        Some(v) => v,
        None => {
            return Err(fail(
                diag,
                path,
                version,
                STAGE_VERSION,
                format!(
                    "期望在版本节找到 x.y.z[+build] 串，实际 save_game_summary {} 项、game_info {} 项候选均无匹配",
                    summary.len(),
                    info.len()
                ),
            ))
        }
    };
    diag.version_raw = raw.clone();
    diag.major = major.to_string();
    Ok((major, raw))
}

fn find_major_in(candidates: &[String], re: &regex::Regex) -> Option<(u8, String)> {
    for s in candidates {
        if let Some(cap) = re.captures(s) {
            if let Ok(m) = cap[1].parse::<u8>() {
                return Some((m, cap[0].to_string()));
            }
        }
    }
    None
}

// ===================== 节解压 ===================================================================

/// 整帧一次解压到预分配缓冲；written 必须等于 decompLen，且帧后不得再吐出字节。
/// 截断帧在 zstd 层可能只返回 0 字节而不抛错，故用「实解长度 + 额外读一字节」两重校验兜住。
fn decompress_section(
    path: &Path,
    e: &TocEntry,
    version: &str,
    diag: &mut Diag,
) -> Result<Vec<u8>, String> {
    let abs = e.a.saturating_add(SECTION_BASE);
    diag.comp_len = e.comp_len;
    diag.decomp_len = e.decomp_len;

    let mut f = File::open(path).map_err(|io| {
        fail(
            diag,
            path,
            version,
            STAGE_SECTION,
            format!("节 {} 期望可打开存档，实际打开失败：{io}", e.name),
        )
    })?;
    f.seek(SeekFrom::Start(abs)).map_err(|io| {
        fail(
            diag,
            path,
            version,
            STAGE_SECTION,
            format!("节 {} 期望定位到绝对偏移 {abs}，实际 seek 失败：{io}", e.name),
        )
    })?;
    let mut dec = zstd::stream::read::Decoder::new(f.take(e.comp_len)).map_err(|io| {
        fail(
            diag,
            path,
            version,
            STAGE_SECTION,
            format!("节 {} 期望可创建 zstd 解码器，实际失败：{io}", e.name),
        )
    })?;

    let want = usize::try_from(e.decomp_len).map_err(|_| {
        fail(
            diag,
            path,
            version,
            STAGE_SECTION,
            format!("节 {} 声明 {} B 超出本机可分配范围", e.name, e.decomp_len),
        )
    })?;
    let mut out = vec![0u8; want];
    let mut written = 0usize;
    while written < want {
        match dec.read(&mut out[written..]) {
            Ok(0) => break,
            Ok(n) => written += n,
            Err(io) => {
                diag.actual_len = written as u64;
                return Err(fail(
                    diag,
                    path,
                    version,
                    STAGE_SECTION,
                    format!(
                        "节 {} 声明 {} B，实际解压 {} B 后解码失败：{io}",
                        e.name, e.decomp_len, written
                    ),
                ));
            }
        }
    }
    diag.actual_len = written as u64;
    if written != want {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_SECTION,
            format!(
                "节 {} 声明 {} B，实际解压 {} B（压缩帧不完整）",
                e.name, e.decomp_len, written
            ),
        ));
    }
    let mut extra = [0u8; 1];
    match dec.read(&mut extra) {
        Ok(0) => {}
        Ok(_) => {
            return Err(fail(
                diag,
                path,
                version,
                STAGE_SECTION,
                format!("节 {} 声明 {} B，实际解压输出超出声明长度", e.name, e.decomp_len),
            ))
        }
        Err(io) => {
            return Err(fail(
                diag,
                path,
                version,
                STAGE_SECTION,
                format!("节 {} 帧尾校验失败：{io}", e.name),
            ))
        }
    }
    Ok(out)
}

// ===================== 游戏日 ===================================================================

fn is_leap(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// 日序 → (月, 日)。必须按各自年份的闰年换算，不能拿两位月日的朴素日序直接比。
fn md_of(doy: u32, year: u16) -> (u32, u32) {
    let months: [u32; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut d = doy;
    let mut m = 0usize;
    while m < 12 && d > months[m] {
        d -= months[m];
        m += 1;
    }
    ((m + 1) as u32, d)
}

/// 游戏日的月日分数（month*100+day），用于与生日比较决定年龄是否已减一。
fn md_score(doy: u32, year: u16) -> u32 {
    let (m, d) = md_of(doy, year);
    m * 100 + d
}

/// 年龄：两侧日序各自按所在年份的闰平换算成月日再比较。跨闰年时朴素日序比较
/// （`gd < bd`）会在 2/29 前后算错约 0.3%，所以不能用日序直接比。
fn age_of(game_year: u16, game_day: u16, birth_year: u16, birth_day: u16) -> i32 {
    let gmd = md_score(game_day as u32, game_year);
    let bmd = md_score(birth_day as u32, birth_year);
    game_year as i32 - birth_year as i32 - i32::from(gmd < bmd)
}

fn game_date(db: &[u8], path: &Path, version: &str, diag: &mut Diag) -> Result<(u16, u16), String> {
    if db.len() < 40 {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_FIELDS,
            format!("game_db 期望 >=40 B 以读取游戏日，实际 {} B", db.len()),
        ));
    }
    let gy = u16_le(db, 38);
    let gd = u16_le(db, 36) & 0x1FF;
    if !(1990..=2100).contains(&gy) || !(1..=366).contains(&gd) {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_FIELDS,
            format!(
                "游戏日期望年 1990..=2100 / 日 1..=366，实际 年 {gy} / 日 {gd}"
            ),
        ));
    }
    Ok((gy, gd))
}

// ===================== 国籍索引 =================================================================

/// rank → 三字码。查不到映射留 `""`（空串），保留行，不报错不丢行。
fn build_nation_index(
    nat: &NationalityTable,
    db: &[u8],
    layout: &FmLayout,
    logger: &impl LogSink,
) -> Result<HashMap<u8, String>, String> {
    let ranks: Vec<[u64; 2]> =
        serde_json::from_str(layout.dict).map_err(|e| format!("国籍名次表解析失败：{e}"))?;
    let ids: Vec<Vec<String>> =
        serde_json::from_str(FM_NATION_IDS_24).map_err(|e| format!("国籍 id 表解析失败：{e}"))?;

    let id2row: HashMap<u64, &Vec<String>> = ids
        .iter()
        .filter_map(|r| r.first().and_then(|s| s.parse::<u64>().ok()).map(|id| (id, r)))
        .collect();
    let known: HashSet<u64> = id2row.keys().copied().collect();
    let mut rank2id: HashMap<u64, u64> = ranks.iter().map(|p| (p[0], p[1])).collect();

    // 名次表里 id=0 的空位在档内以 {u32 rank}{u32 nationId}{u32 nationId} 双写补齐；
    // 只认双写可避开大量撞值的小 id。
    for &[rank, nation_id] in &ranks {
        if nation_id != 0 {
            continue;
        }
        let needle = (rank as u32).to_le_bytes();
        let mut p = 0usize;
        while p + 12 <= db.len() {
            if db[p..p + 4] == needle {
                let cand = u32_le(db, p + 4) as u64;
                if known.contains(&cand) && u32_le(db, p + 8) as u64 == cand {
                    rank2id.insert(rank, cand);
                    break;
                }
            }
            p += 1;
        }
    }

    let mut out: HashMap<u8, String> = HashMap::with_capacity(rank2id.len());
    let mut fallback = 0usize;
    for (&rank, &nation_id) in &rank2id {
        let code = match id2row.get(&nation_id) {
            None => String::new(),
            Some(row) => {
                let raw = row.get(1).cloned().unwrap_or_default();
                if nat.is_code(&raw) {
                    raw
                } else {
                    let en = row.get(2).map(String::as_str).unwrap_or("");
                    let zh = row.get(3).map(String::as_str).unwrap_or("");
                    nat.resolve(en)
                        .or_else(|| nat.resolve(zh))
                        .map(|(_, c)| c)
                        .unwrap_or_default()
                }
            }
        };
        if code.is_empty() {
            fallback += 1;
        }
        if let Ok(key) = u8::try_from(rank) {
            out.insert(key, code);
        }
    }
    logger.debug(
        "fm",
        format!("国籍索引：{} 个 rank，{fallback} 个回落空码", out.len()),
    );
    Ok(out)
}

// ===================== 名池 ======================================================================

struct Pool {
    /// 每条记录 {u32 key}{u32 len}{utf8} 的起始偏移；下标即池内 id。
    offs: Vec<usize>,
    n: usize,
    head: usize,
    end: usize,
    broken: i64,
}

fn ptext<'a>(db: &'a [u8], pool: &Pool, i: usize) -> &'a str {
    let Some(&p) = pool.offs.get(i) else { return "" };
    if p + 8 > db.len() {
        return "";
    }
    let len = u32_le(db, p + 4) as usize;
    if p + 8 + len > db.len() {
        return "";
    }
    std::str::from_utf8(&db[p + 8..p + 8 + len]).unwrap_or("")
}

/// 三段连续名池。段首随档漂移，必须靠签名扫描定位，绝不硬编码偏移。
fn read_pools(
    db: &[u8],
    layout: &FmLayout,
    path: &Path,
    version: &str,
    diag: &mut Diag,
    logger: &impl LogSink,
) -> Result<[Pool; 3], String> {
    let np = layout.name_pool;
    let mut start: Option<usize> = None;
    let mut p = 0usize;
    while p + 32 <= db.len() {
        if u32_le(db, p) == np.signature
            && u32_le(db, p + 4) == 0
            && u32_le(db, p + 20) == 0
            && u32_le(db, p + 28) == np.first_key
        {
            let c1 = u32_le(db, p + 8) as u64;
            let c2 = u32_le(db, p + 12) as u64;
            let c3 = u32_le(db, p + 16) as u64;
            if c2 != 0
                && c1 == np.c1_ratio as u64 * c2
                && c3 == np.c3_ratio as u64 * c2
            {
                start = Some(p + 24);
                break;
            }
        }
        p += 1;
    }
    let Some(mut c0) = start else {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_ENUM,
            format!(
                "名池期望找到签名 {{{} 0 c1 c2 c3 0}} 且 c1={}*c2、c3={}*c2、key0={}，实际全缓冲扫描无命中",
                np.signature, np.c1_ratio, np.c3_ratio, np.first_key
            ),
        ));
    };

    let mut pools: Vec<Pool> = Vec::with_capacity(3);
    for _ in 0..3 {
        if c0 + 4 > db.len() {
            break;
        }
        let n = u32_le(db, c0) as usize;
        // 非法计数不放大成巨额分配；当作断裂处理。
        if n > 20_000_000 {
            break;
        }
        let mut offs = Vec::with_capacity(n.min(1_000_000));
        let mut q = c0 + 4;
        let mut broken = -1i64;
        for i in 0..n {
            if q + 8 > db.len() {
                broken = i as i64;
                break;
            }
            let key = u32_le(db, q);
            let len = u32_le(db, q + 4);
            if key != i as u32 || len > np.max_name_len {
                broken = i as i64;
                break;
            }
            let next = q + 8 + len as usize;
            if next > db.len() {
                broken = i as i64;
                break;
            }
            offs.push(q);
            q = next;
        }
        let eff = offs.len();
        pools.push(Pool {
            head: c0 + 4,
            offs,
            n: eff,
            end: q,
            broken,
        });
        c0 = q;
    }
    if pools.len() != 3 {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_ENUM,
            format!("名池期望 3 段，实际解析到 {} 段", pools.len()),
        ));
    }
    for (i, pool) in pools.iter().enumerate() {
        logger.debug(
            "fm",
            format!(
                "名池[{i}] head={} 条数={} end={} 断裂={}",
                pool.head, pool.n, pool.end, pool.broken
            ),
        );
    }
    let mut it = pools.into_iter();
    Ok([it.next().unwrap(), it.next().unwrap(), it.next().unwrap()])
}

// ===================== 人员块 ==================================================================

struct Block {
    r: usize,
    a: u32,
    b: u32,
    c: u32,
    day: u16,
    year: u16,
    rank: u8,
}

/// 粗筛 + 名池反查定位人员块。`cur = R-19-L` 的两种写法都必须保留：L=0 与 L=2..80
/// 对应档内两种不同的记录头排布，只认一种会整族漏人。
fn read_blocks(db: &[u8], pools: &[Pool; 3], game_year: u16) -> Vec<Block> {
    let size = db.len();
    let mut blocks = Vec::new();
    let mut r = 24usize;
    while r + 40 < size {
        if db[r + 5] < 11 && db[r + 6] < 21 && db[r + 7] < 11 {
            let day = (db[r] as u16 | ((db[r + 1] as u16) << 8)) & 0x1FF;
            let year = db[r + 2] as u16 | ((db[r + 3] as u16) << 8);
            if (1..=366).contains(&day) && (1900..=game_year).contains(&year) {
                let mut cur: Option<usize> = None;
                if u32_le(db, r - 4) == 0 {
                    cur = Some(r - 19);
                } else {
                    for k in 2..=80usize {
                        let c = match (r - 19).checked_sub(k) {
                            Some(v) => v,
                            None => break,
                        };
                        if c < 4 {
                            break;
                        }
                        if c + 19 <= size && u32_le(db, c + 15) == k as u32 {
                            cur = Some(c);
                            break;
                        }
                    }
                }
                if let Some(cur) = cur {
                    if cur + 15 <= size
                        && db[cur + 4] == 0
                        && db[cur + 9] == 0
                        && db[cur + 14] == 0
                    {
                        let a = u32_le(db, cur);
                        let b = u32_le(db, cur + 5);
                        let c = u32_le(db, cur + 10);
                        let a_ok = a == SENT || ((a as u64) < pools[0].n as u64 && db[cur + 3] == 0);
                        let b_ok = b == SENT || ((b as u64) < pools[1].n as u64 && db[cur + 8] == 0);
                        // 第三字段有四种写法：哨兵、第三池 id、C==A、落在姓池范围。
                        let c_ok = c == SENT
                            || (c as u64) < pools[2].n as u64
                            || c == a
                            || (c as u64) < pools[1].n as u64;
                        if a_ok && b_ok && c_ok {
                            let mut g = true;
                            for k in 17..25usize {
                                let v = db[r + k];
                                if !(1..=20).contains(&v) {
                                    g = false;
                                    break;
                                }
                            }
                            if g {
                                blocks.push(Block {
                                    r,
                                    a,
                                    b,
                                    c,
                                    day,
                                    year,
                                    rank: db[r + 9],
                                });
                            }
                        }
                    }
                }
            }
        }
        r += 1;
    }
    blocks
}

// ===================== 记录体 ==================================================================

#[derive(Clone, Copy)]
struct PlayerRec {
    b: usize,
    self_uid: u32,
}

#[derive(Clone, Copy)]
struct StaffRec {
    p: usize,
    self_uid: u32,
    ca: u16,
    pa: u16,
    hraw: u8,
}

struct StaffCounts {
    player: usize,
    staff: usize,
    kind3: usize,
}

#[inline]
fn ob(db: &[u8], base: usize, off: isize) -> u8 {
    at(base, off).and_then(|i| db.get(i).copied()).unwrap_or(0)
}

#[inline]
fn ou16(db: &[u8], base: usize, off: isize) -> u16 {
    at(base, off)
        .and_then(|i| (i + 2 <= db.len()).then(|| u16_le(db, i)))
        .unwrap_or(0)
}

#[inline]
fn ou32(db: &[u8], base: usize, off: isize) -> u32 {
    at(base, off)
        .and_then(|i| (i + 4 <= db.len()).then(|| u32_le(db, i)))
        .unwrap_or(0)
}

#[inline]
fn ou24(db: &[u8], base: usize, off: isize) -> u32 {
    at(base, off)
        .and_then(|i| (i + 3 <= db.len()).then(|| u24_le(db, i)))
        .unwrap_or(0)
}

fn read_records(
    db: &[u8],
    ro: &RecordOffsets,
) -> (
    HashMap<u32, PlayerRec>,
    HashMap<u32, Vec<StaffRec>>,
    StaffCounts,
) {
    let size = db.len();
    let mut rec: HashMap<u32, PlayerRec> = HashMap::new();
    let mut staff: HashMap<u32, Vec<StaffRec>> = HashMap::new();
    let mut counts = StaffCounts { player: 0, staff: 0, kind3: 0 };

    let mut b = 20usize;
    while b + 130 < size {
        let kind = ob(db, b, ro.kind);
        if kind == 2 || kind == 3 {
            let self_u = ou32(db, b, ro.self_uid);
            if self_u > 0 && self_u < UID_CEIL {
                let pi = ou32(db, b, ro.pindex);
                if pi != 0 {
                    let mut ok = true;
                    for k in ro.pos0..ro.pos1 {
                        let v = ob(db, b, k);
                        if !(1..=20).contains(&v) {
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        for k in ro.pos1..ro.attr1 {
                            let v = ob(db, b, k);
                            if !(1..=100).contains(&v) {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        counts.player += 1;
                        rec.entry(pi).or_insert(PlayerRec { b, self_uid: self_u });
                    }
                }
            }
        }
        b += 1;
    }

    let mut p = 20usize;
    while p + 200 < size {
        let k = ob(db, p, ro.kind);
        if k == 1 || k == 3 {
            let pi = ou32(db, p, ro.pindex);
            if pi != 0 {
                let self_u = ou32(db, p, ro.self_uid);
                if self_u > 0 && self_u < UID_CEIL {
                    let j = ob(db, p, ro.j) as usize;
                    if j <= 20 {
                        let stride = (ro.j_stride as usize) * j;
                        let ca = ou16(db, p, ro.ca + stride as isize);
                        let pa = ou16(db, p, ro.pa + stride as isize);
                        if (1..=200).contains(&ca) && (1..=200).contains(&pa) {
                            let hraw = ob(db, p, ro.st_height + stride as isize);
                            let h_ok = hraw == 0
                                || (50..=115).contains(&hraw)
                                || (145..=220).contains(&hraw);
                            if h_ok {
                                // kind==3 是与球员记录体共用 seq 空间的未解族，只计数不入库。
                                if k == 3 {
                                    counts.kind3 += 1;
                                } else {
                                    counts.staff += 1;
                                    staff.entry(pi).or_default().push(StaffRec {
                                        p,
                                        self_uid: self_u,
                                        ca,
                                        pa,
                                        hraw,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        p += 1;
    }
    // 职员与球员共用 pindex 时以球员为准。
    for pi in rec.keys() {
        staff.remove(pi);
    }
    (rec, staff, counts)
}

// ===================== 槽位与绑定 ==============================================================

#[derive(Clone, Copy)]
struct Slot {
    i: usize,
    uid: u32,
    seq: u32,
    kind: u8,
    dual: bool,
    gender_bit: bool,
    newgen: bool,
}

#[inline]
fn kind_ok(v: u8) -> bool {
    (1..=9).contains(&v) || v == 16 || v == 18
}

#[inline]
fn near_uid(self_uid: u32, uid: u32) -> bool {
    self_uid == uid
        || self_uid == uid.wrapping_sub(1)
        || self_uid == uid.wrapping_sub(2)
}

fn pick_staff<'a>(
    staff: &'a HashMap<u32, Vec<StaffRec>>,
    pindex: u32,
    slot: usize,
) -> Option<&'a StaffRec> {
    let arr = staff.get(&pindex)?;
    let mut best: Option<&'a StaffRec> = None;
    let mut bd = usize::MAX;
    for r in arr {
        let d = r.p.abs_diff(slot);
        if d < bd {
            bd = d;
            best = Some(r);
        }
    }
    best
}

fn self_ok(
    s: &Slot,
    rec: &HashMap<u32, PlayerRec>,
    staff: &HashMap<u32, Vec<StaffRec>>,
) -> bool {
    let seq = s.seq.wrapping_sub(1);
    rec.get(&seq)
        .map(|r| near_uid(r.self_uid, s.uid))
        .unwrap_or(false)
        || pick_staff(staff, seq, s.i)
            .map(|r| near_uid(r.self_uid, s.uid))
            .unwrap_or(false)
}

fn read_slots(
    db: &[u8],
    rec: &HashMap<u32, PlayerRec>,
    staff: &HashMap<u32, Vec<StaffRec>>,
) -> Vec<Slot> {
    let size = db.len();
    let mut slots = Vec::new();
    let mut i = 8192usize;
    while i + 13 < size {
        if kind_ok(db[i + 8]) && db[i - 1] == 0 {
            let uid = u32_le(db, i);
            if uid > 0 && uid < UID_CEIL {
                let seq = u32_le(db, i - 4);
                if seq != 0 {
                    let dual = u32_le(db, i + 4) == uid;
                    let ok = dual || {
                        let near = |v: Option<u32>| v.map(|x| near_uid(x, uid)).unwrap_or(false);
                        let r = rec.get(&seq.wrapping_sub(1)).map(|r| r.self_uid);
                        let st = pick_staff(staff, seq.wrapping_sub(1), i).map(|r| r.self_uid);
                        near(r) || near(st)
                    };
                    if ok {
                        slots.push(Slot {
                            i,
                            uid,
                            seq,
                            kind: db[i + 8],
                            dual,
                            gender_bit: (db[i - 11] & 0x10) != 0,
                            newgen: db[i - 12] >= 8,
                        });
                    }
                }
            }
        }
        i += 1;
    }
    slots
}

/// uid 在全缓冲宽口径下的出现次数（让位判据用）。与 `uid_seq` 的去重口径不同，
/// 不能拿去重后的槽位统计来替代。
fn uid_occurrences(db: &[u8]) -> HashMap<u32, usize> {
    let mut occ: HashMap<u32, usize> = HashMap::new();
    let size = db.len();
    let mut i = 8192usize;
    while i + 13 < size {
        if kind_ok(db[i + 8]) && db[i - 1] == 0 {
            let u = u32_le(db, i);
            if u > 0 && u < UID_CEIL && u32_le(db, i - 4) != 0 {
                *occ.entry(u).or_insert(0) += 1;
            }
        }
        i += 1;
    }
    occ
}

fn uid_sequence_map(slots: &[Slot]) -> HashMap<u32, i64> {
    let mut m: HashMap<u32, i64> = HashMap::new();
    for s in slots {
        match m.get(&s.uid) {
            None => {
                m.insert(s.uid, s.seq as i64);
            }
            Some(&prev) => {
                if prev != s.seq as i64 {
                    m.insert(s.uid, -1);
                }
            }
        }
    }
    m
}

fn strong_of(s: &Slot, uid_seq: &HashMap<u32, i64>) -> bool {
    uid_seq.get(&s.uid) == Some(&(s.seq as i64))
}

/// 块 R 已按升序产生，二分找最后一个 R <= i 的块。
fn near_block(r_arr: &[usize], i: usize) -> Option<usize> {
    let mut lo = 0usize;
    let mut hi = r_arr.len();
    while lo < hi {
        let m = (lo + hi) / 2;
        if r_arr[m] <= i {
            lo = m + 1;
        } else {
            hi = m;
        }
    }
    // r_arr 元素可能全大于 i（首个块已在 i 之后），此时没有前驱块；
    // 用 checked_sub 让 lo == 0 直接返回 None，避免 usize 下溢。
    lo.checked_sub(1)
}

fn dist_of(r_arr: &[usize], i: usize) -> (Option<usize>, i64) {
    match near_block(r_arr, i) {
        Some(bi) => (Some(bi), i as i64 - r_arr[bi] as i64),
        None => (None, -1),
    }
}

fn score_of(s: &Slot, r_arr: &[usize]) -> i64 {
    let (_, d) = dist_of(r_arr, s.i);
    if (20..=BIND_WINDOW as i64).contains(&d) {
        d
    } else {
        i64::MAX
    }
}

struct PatchStat {
    cands: [usize; 4],
    patched: usize,
    displaced: usize,
}

fn can_displace(
    cur: &Slot,
    d: i64,
    rule: u8,
    uid_seq: &HashMap<u32, i64>,
    uid_occ: &HashMap<u32, usize>,
    rec: &HashMap<u32, PlayerRec>,
    staff: &HashMap<u32, Vec<StaffRec>>,
) -> bool {
    match rule {
        // 跨 seq 假人占位：近距离 + uid 并非唯一对应该 seq + 占用者无自证。
        0 => d < 500
            && uid_seq.get(&cur.uid).copied() != Some(cur.seq as i64)
            && !self_ok(cur, rec, staff),
        1 => !self_ok(cur, rec, staff),
        2 => uid_occ.get(&cur.uid).copied().unwrap_or(0) >= 100,
        _ => false,
    }
}

/// 两轮主绑定（有记录体先绑）+ 四档补配。`emitted` / `used_slot` / `used_seq` 在三档之间
/// 持续推进，所以候选集必须每档重新筛，不能一次算好复用。
fn bind_all(
    slots: &[Slot],
    r_arr: &[usize],
    rec: &HashMap<u32, PlayerRec>,
    staff: &HashMap<u32, Vec<StaffRec>>,
    uid_seq: &HashMap<u32, i64>,
    uid_occ: &HashMap<u32, usize>,
) -> (Vec<Option<Slot>>, PatchStat) {
    // 按 seq 去重：强候选优先、其次双写、其余；池内按距离取最近的一个。
    type Group = (
        Vec<(Slot, i64)>,
        Vec<(Slot, i64)>,
        Vec<(Slot, i64)>,
    );
    let mut order: Vec<u32> = Vec::new();
    let mut groups: HashMap<u32, Group> = HashMap::new();
    for s in slots {
        if !groups.contains_key(&s.seq) {
            order.push(s.seq);
            groups.insert(s.seq, Group::default());
        }
        let item = (*s, score_of(s, r_arr));
        let g = groups.get_mut(&s.seq).unwrap();
        if strong_of(s, uid_seq) {
            g.0.push(item);
        } else if s.dual {
            g.1.push(item);
        } else {
            g.2.push(item);
        }
    }
    let mut slots_u: Vec<Slot> = Vec::with_capacity(order.len());
    for seq in order {
        let (a, b, c) = groups.remove(&seq).unwrap();
        let mut pool = if !a.is_empty() { a } else if !b.is_empty() { b } else { c };
        pool.sort_by_key(|(_, sc)| *sc);
        if let Some((s, _)) = pool.first() {
            slots_u.push(*s);
        }
    }

    let staff_pindex: HashSet<u32> = staff.keys().copied().collect();
    let has_rec = |s: &Slot| {
        let seq = s.seq.wrapping_sub(1);
        rec.contains_key(&seq) || staff_pindex.contains(&seq)
    };
    let mut used_slot: HashSet<usize> = HashSet::new();
    let mut used_seq: HashSet<u32> = HashSet::new();
    let mut binding: Vec<Option<Slot>> = vec![None; r_arr.len()];

    for want_rec in [true, false] {
        let mut bi_order: Vec<usize> = Vec::new();
        let mut cand: HashMap<usize, Vec<(Slot, i64)>> = HashMap::new();
        for s in slots_u.iter().filter(|s| has_rec(s) == want_rec) {
            if used_seq.contains(&s.seq) {
                continue;
            }
            let Some(bi) = near_block(r_arr, s.i) else { continue };
            if binding[bi].is_some() {
                continue;
            }
            let d = s.i as i64 - r_arr[bi] as i64;
            if !(20..=BIND_WINDOW as i64).contains(&d) {
                continue;
            }
            if !cand.contains_key(&bi) {
                bi_order.push(bi);
            }
            cand.entry(bi).or_default().push((*s, d));
        }
        let mut pairs: Vec<(usize, Vec<(Slot, i64)>)> = bi_order
            .into_iter()
            .map(|bi| (bi, cand.remove(&bi).unwrap()))
            .collect();
        for (_, arr) in &mut pairs {
            arr.sort_by_key(|(_, d)| *d);
        }
        pairs.sort_by_key(|(_, arr)| arr[0].1);
        for (bi, arr) in pairs {
            for (s, _) in arr {
                if used_slot.contains(&s.i) || used_seq.contains(&s.seq) {
                    continue;
                }
                used_slot.insert(s.i);
                used_seq.insert(s.seq);
                binding[bi] = Some(s);
                break;
            }
        }
    }

    let mut emitted: HashSet<u32> = binding
        .iter()
        .flatten()
        .map(|s| s.uid)
        .collect();
    let mut patch = PatchStat { cands: [0; 4], patched: 0, displaced: 0 };
    // (窗口, 让位规则, 是否要求 selfOk)。四档都要求 strong + 双写 + 未发出 + 未占槽。
    let tiers: [(i64, u8, bool); 4] = [
        (1500, 0, true),  // selfOk
        (1500, 0, false), // freeOrCross
        (500, 1, true),   // reclaim
        (1500, 2, false), // highFreq
    ];
    for (ti, &(window, rule, require_self_ok)) in tiers.iter().enumerate() {
        let mut cands: Vec<(Slot, usize, i64)> = Vec::new();
        for s in slots {
            if !strong_of(s, uid_seq) || !s.dual {
                continue;
            }
            if emitted.contains(&s.uid) || used_slot.contains(&s.i) {
                continue;
            }
            if require_self_ok && !self_ok(s, rec, staff) {
                continue;
            }
            let (bi, d) = dist_of(r_arr, s.i);
            let Some(bi) = bi else { continue };
            if !(20..=window).contains(&d) {
                continue;
            }
            cands.push((*s, bi, d));
        }
        cands.sort_by_key(|(_, _, d)| *d);
        patch.cands[ti] = cands.len();
        for (s, bi, d) in cands {
            if emitted.contains(&s.uid) {
                continue;
            }
            match binding[bi] {
                None => {
                    binding[bi] = Some(s);
                    emitted.insert(s.uid);
                    used_slot.insert(s.i);
                    used_seq.insert(s.seq);
                    patch.patched += 1;
                }
                Some(cur) => {
                    if can_displace(&cur, d, rule, uid_seq, uid_occ, rec, staff) {
                        emitted.remove(&cur.uid);
                        used_seq.remove(&cur.seq);
                        binding[bi] = Some(s);
                        emitted.insert(s.uid);
                        used_slot.insert(s.i);
                        used_seq.insert(s.seq);
                        patch.patched += 1;
                        patch.displaced += 1;
                    }
                }
            }
        }
    }
    (binding, patch)
}

// ===================== 俱乐部名 =================================================================

struct ClubIndex {
    cid_club: HashMap<u32, String>,
    seq_club: HashMap<u32, String>,
}

fn name_at<'a>(
    db: &'a [u8],
    q: usize,
    len: u32,
    name_re: &regex::Regex,
    letter_re: &regex::Regex,
) -> Option<&'a str> {
    let l = len as usize;
    if !(2..=90).contains(&l) || q + l >= db.len() {
        return None;
    }
    let b0 = db[q];
    if b0 < 0x20 || b0 == 0x7f {
        return None;
    }
    let t = std::str::from_utf8(&db[q..q + l]).ok()?;
    if !letter_re.is_match(t) || !name_re.is_match(t) {
        return None;
    }
    Some(t)
}

/// cid → 短名（球员）与 seq → 短名（职员）。两套证据都要收：记录头能给出真 idx，
/// 配对扫描补回记录头 flag 不恒定时漏掉的俱乐部；只认一套会让邻居队伍块落进错误区间。
fn read_club_names(
    db: &[u8],
    slots: &[Slot],
    rec: &HashMap<u32, PlayerRec>,
    staff: &HashMap<u32, Vec<StaffRec>>,
    logger: &impl LogSink,
) -> ClubIndex {
    let name_re = regex::Regex::new(NAME_CHARS_RE).expect("俱乐部名字符类为字面量常量");
    let letter_re = regex::Regex::new(r"\p{L}").expect("Unicode 字母类为字面量常量");
    let cap = CLUB_SCAN_CAP.min(db.len());
    let mut club_name: HashMap<usize, String> = HashMap::new();
    let mut idx_of: HashMap<usize, u32> = HashMap::new();

    // 证据一：严格记录头 {idx}{uid}{uid}{u8}{nat}{SENT}{nat}{based}{city}{flag}{u16 0}{len}{long}{len}{short}。
    let mut p = 64usize;
    while p + 64 < cap && p + 64 < db.len() {
        if u32_le(db, p + 17) == SENT {
            let uid = u32_le(db, p + 4);
            if uid != 0 && u32_le(db, p + 8) == uid {
                let nat = u32_le(db, p + 21);
                let based = u32_le(db, p + 25);
                let city = u32_le(db, p + 29);
                if nat > 0
                    && nat < 400
                    && based > 0
                    && based < 400
                    && city > 0
                    && city < 400_000
                    && u32_le(db, p + 33) < 1024
                {
                    let l1 = u32_le(db, p + 39);
                    if name_at(db, p + 43, l1, &name_re, &letter_re).is_some() {
                        let q2 = p + 43 + l1 as usize;
                        if q2 + 4 <= db.len() {
                            let l2 = u32_le(db, q2);
                            if let Some(short) = name_at(db, q2 + 4, l2, &name_re, &letter_re) {
                                club_name.insert(p + 43, short.to_string());
                                idx_of.insert(p + 43, u32_le(db, p));
                                p = q2 + 4 + l2 as usize;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        p += 1;
    }

    // 证据二：任意相邻的 {u32 len}{utf8}{u32 len}{utf8} 配对，补回记录头漏掉的锚点。
    let mut q = 8usize;
    while q < cap && q + 4 <= db.len() {
        let l1 = u32_le(db, q - 4);
        let mut jump: Option<usize> = None;
        if name_at(db, q, l1, &name_re, &letter_re).is_some() {
            let l2_at = q + l1 as usize;
            if l2_at + 4 <= db.len() {
                let l2 = u32_le(db, l2_at);
                if let Some(short) = name_at(db, l2_at + 4, l2, &name_re, &letter_re) {
                    club_name.entry(q).or_insert_with(|| short.to_string());
                    jump = Some(l2_at + 4 + l2 as usize);
                }
            }
        }
        q = jump.unwrap_or(q + 1);
    }

    let mut club_list: Vec<(usize, String, i64)> = club_name
        .into_iter()
        .map(|(q, short)| (q, short, -1i64))
        .collect();
    club_list.sort_by_key(|(q, _, _)| *q);

    // 配对扫描补进来的锚点读不到记录头 idx，按「左邻已知 idx + 位置差」推；名字顺序与
    // idx 同序，用数组下标替代真 idx 会把漏抓的俱乐部整段压掉、让距离判据虚高。
    let mut last_idx = -1i64;
    let mut last_pos = -1i64;
    for i in 0..club_list.len() {
        if let Some(&real) = idx_of.get(&club_list[i].0) {
            club_list[i].2 = real as i64;
            last_idx = real as i64;
            last_pos = i as i64;
        } else {
            club_list[i].2 = if last_pos >= 0 {
                last_idx + (i as i64 - last_pos)
            } else {
                i as i64
            };
        }
    }

    // 队伍块：{3×00}{u8 n}{n×u32 teamId}，落在 [本条名字, 下一条名字) 区间内。
    let mut cands: Vec<Vec<(Vec<u32>, usize)>> = vec![Vec::new(); club_list.len()];
    for i in 0..club_list.len() {
        let from = club_list[i].0;
        let to = if i + 1 < club_list.len() {
            club_list[i + 1].0
        } else {
            (from + 8000).min(db.len())
        };
        let mut pp = from + 3;
        while pp + 33 <= to && pp + 33 <= db.len() {
            let n = db[pp] as usize;
            if (1..=12).contains(&n)
                && db[pp - 1] == 0
                && db[pp - 2] == 0
                && db[pp - 3] == 0
            {
                let mut ids = Vec::with_capacity(n);
                let mut seen: HashSet<u32> = HashSet::new();
                let mut ok = true;
                for k in 0..n {
                    let pos = pp + 1 + 4 * k;
                    if pos + 4 > db.len() {
                        ok = false;
                        break;
                    }
                    let v = u32_le(db, pos);
                    if v < 1 || v > MAX_TEAM || !seen.insert(v) {
                        ok = false;
                        break;
                    }
                    ids.push(v);
                }
                if ok {
                    cands[i].push((ids, pp + 1));
                }
            }
            pp += 1;
        }
    }

    // 先取窗口里 id 最多的块；k 命中时用真实字段偏移覆盖；再按首队 id 单调先验
    // ±200 邻域中位数 3 轮收敛，把挤进区间的同形干扰块换掉。
    let mut pick: Vec<Option<Vec<u32>>> = vec![None; club_list.len()];
    for i in 0..cands.len() {
        if cands[i].is_empty() {
            continue;
        }
        let mut best = &cands[i][0];
        for c in &cands[i][1..] {
            if c.0.len() > best.0.len() {
                best = c;
            }
        }
        pick[i] = Some(best.0.clone());
    }
    for i in 0..club_list.len() {
        let from = club_list[i].0;
        if from < 8 || from + 4 > db.len() {
            continue;
        }
        let l1 = u32_le(db, from - 4);
        if !(2..=90).contains(&l1) {
            continue;
        }
        let l1u = l1 as usize;
        if from + l1u + 4 > db.len() {
            continue;
        }
        let se = from + l1u + 4 + u32_le(db, from + l1u) as usize;
        if se + 56 >= db.len() {
            continue;
        }
        let target = se + 67 + 12 * db[se + 56] as usize;
        if let Some(hit) = cands[i].iter().find(|c| c.1 == target) {
            pick[i] = Some(hit.0.clone());
        }
    }
    for _ in 0..3 {
        let dev: Vec<Option<f64>> = (0..pick.len())
            .map(|i| {
                pick[i]
                    .as_ref()
                    .map(|ids| ids[0] as f64 - club_list[i].2 as f64)
            })
            .collect();
        for i in 0..cands.len() {
            if cands[i].is_empty() {
                continue;
            }
            let lo = i.saturating_sub(200);
            let hi = (i + 201).min(dev.len());
            let mut seg: Vec<f64> = (lo..hi).filter_map(|k| dev[k]).collect();
            if seg.len() < 5 {
                continue;
            }
            seg.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let want = club_list[i].2 as f64 + seg[seg.len() >> 1];
            let mut best: Option<&Vec<u32>> = None;
            let mut best_dev = f64::INFINITY;
            for c in &cands[i] {
                let x = (c.0[0] as f64 - want).abs();
                if x < best_dev {
                    best_dev = x;
                    best = Some(&c.0);
                }
            }
            if let Some(b) = best {
                if best_dev <= 40.0 {
                    pick[i] = Some(b.clone());
                }
            }
        }
    }

    let mut cid_club: HashMap<u32, String> = HashMap::new();
    for i in 0..pick.len() {
        if let Some(ids) = &pick[i] {
            for &c in ids {
                cid_club.entry(c).or_insert_with(|| club_list[i].1.clone());
            }
        }
    }
    // 孤立块补一轮：只补未被任何俱乐部认领的 id，改不动已有映射。
    for i in 0..cands.len() {
        if cands[i].is_empty() {
            continue;
        }
        let mut best = &cands[i][0];
        for c in &cands[i][1..] {
            if c.0.len() > best.0.len() {
                best = c;
            }
        }
        for &c in &best.0 {
            cid_club.entry(c).or_insert_with(|| club_list[i].1.clone());
        }
    }

    // 职员 → club：club 记录末尾「≥5 个连续 seq」段。长列表本身足以排除偶然，
    // 短列表必须贴记录末尾，否则会和缓冲里其它表的 seq 数组混淆。
    let staff_seq: HashSet<u32> = slots
        .iter()
        .filter(|s| {
            let seq = s.seq.wrapping_sub(1);
            staff.contains_key(&seq) && !rec.contains_key(&seq)
        })
        .map(|s| s.seq)
        .collect();
    let mut seq_club: HashMap<u32, String> = HashMap::new();
    for i in 0..club_list.len() {
        let from = club_list[i].0;
        let to = if i + 1 < club_list.len() {
            club_list[i + 1].0
        } else {
            (from + 8000).min(db.len())
        };
        let tail = 300usize;
        let minrun = 5usize;
        let mut segs: Vec<Vec<u32>> = Vec::new();
        let mut pp = from.max(to.saturating_sub(4000));
        while pp + 4 <= to && pp + 4 <= db.len() {
            let mut run: Vec<u32> = Vec::new();
            let mut qq = pp;
            while qq + 4 <= to && qq + 4 <= db.len() && run.len() <= 400 {
                let v = u32_le(db, qq);
                if staff_seq.contains(&v) {
                    run.push(v);
                    qq += 4;
                    continue;
                }
                if qq + 5 <= to && qq + 5 <= db.len() {
                    let v2 = u32_le(db, qq + 1);
                    if staff_seq.contains(&v2) {
                        run.push(v2);
                        qq += 5;
                        continue;
                    }
                }
                break;
            }
            let dist_ok = to.saturating_sub(qq) <= tail || run.len() >= 20;
            if run.len() >= minrun && dist_ok {
                segs.push(run.clone());
            }
            if !run.is_empty() {
                pp = qq.saturating_sub(1);
            }
            pp += 1;
        }
        for seg in segs {
            for v in seg {
                seq_club.entry(v).or_insert_with(|| club_list[i].1.clone());
            }
        }
    }

    logger.debug(
        "fm",
        format!(
            "俱乐部名：{} 个名字、{} 个 cid 映射、{} 个职员 seq 映射",
            club_list.len(),
            cid_club.len(),
            seq_club.len()
        ),
    );
    ClubIndex { cid_club, seq_club }
}

// ===================== 关系区 ==================================================================

#[derive(Clone, Copy)]
struct Rel {
    ref_id: u32,
    kind: u8,
    flag: u8,
    q: u8,
    b13: u8,
}

fn relations(db: &[u8], r: usize) -> Vec<Rel> {
    let mut out = Vec::new();
    for k in 0..40 {
        let e = r + 35 + 16 * k;
        if e + 16 > db.len() || db[e + 15] != 0xFF {
            break;
        }
        out.push(Rel {
            ref_id: u32_le(db, e),
            kind: db[e + 10],
            flag: db[e + 11],
            q: db[e + 12],
            b13: db[e + 13],
        });
    }
    out
}

/// 关系区里第一条 kind=8/flag=9 的条目即第二国籍。q 只是来源标记，不按它排序；
/// ref 与主名次相同也不排除，存档里确实存在同国籍双条目。flag=0x0E 自然被排除。
fn second_nation(rel: &[Rel]) -> Option<Rel> {
    rel.iter().copied().find(|e| e.kind == 8 && e.flag == 0x09)
}

// ===================== 字段组装 ================================================================

fn valid_code(value: u8, min: u32, max: u32) -> String {
    let v = value as u32;
    if (min..=max).contains(&v) {
        v.to_string()
    } else {
        String::new()
    }
}

fn skin_display(raw: u8, eth: u8) -> u8 {
    if raw == 0xFF {
        // ETH_SKIN 渲染后的值恒在 0..19；直接写 255 会让绝大多数肤色查不到图。
        ETH_SKIN.get(eth as usize).copied().unwrap_or(0)
    } else {
        raw
    }
}

fn hair_display(raw: u8, status: u8, eth: u8) -> u8 {
    if status != 0xFF || !matches!(raw, 6 | 17 | 18) {
        return raw;
    }
    if eth == 0 {
        return 3;
    }
    if eth == 1 || eth == 7 {
        return 5;
    }
    raw
}

fn height_of(hraw: u8) -> u32 {
    if hraw == 0 {
        182
    } else if hraw <= 115 {
        hraw as u32 + 100
    } else {
        hraw as u32
    }
}

fn nat_code(rank2code: &HashMap<u8, String>, rank: u32) -> String {
    u8::try_from(rank)
        .ok()
        .and_then(|k| rank2code.get(&k))
        .cloned()
        .unwrap_or_default()
}

struct AsmCtx<'a> {
    db: &'a [u8],
    pools: &'a [Pool; 3],
    rank2code: &'a HashMap<u8, String>,
    clubs: &'a ClubIndex,
    ro: &'a RecordOffsets,
    gy: u16,
    gd: u16,
}

/// §5 字段口径逐列落地。L8（avatar/based/preferred_job/rca/extra_fields）恒空：
/// 这些列会拼进提示词，塞占位符等于污染整批生图。
fn assemble(
    ctx: &AsmCtx<'_>,
    blk: &Block,
    s: &Slot,
    rec: Option<&PlayerRec>,
    staff: Option<&StaffRec>,
) -> PlayerRecord {
    let db = ctx.db;
    let r = blk.r;
    let rel = relations(db, r);
    let eth = db[r + 5];
    let status = db[r + 4];

    let uid = if s.newgen {
        format!("{NEWGAN_UID_PREFIX}{}", s.uid)
    } else {
        s.uid.to_string()
    };

    // 归化：原籍条目 + 新国籍条目 + status==7 三条件同时成立才覆盖主名次。
    let origin = rel
        .iter()
        .any(|e| e.kind == 8 && e.flag == 70 && e.q == 1 && e.ref_id == blk.rank as u32);
    let naturalized = rel
        .iter()
        .find(|e| e.kind == 8 && e.flag == 9 && (e.b13 == 82 || e.b13 == 88));
    let effective_rank = if status == 7 && origin {
        naturalized.map(|e| e.ref_id).unwrap_or(blk.rank as u32)
    } else {
        blk.rank as u32
    };
    let primary_nat = nat_code(ctx.rank2code, effective_rank);
    let sec_nat = second_nation(&rel)
        .map(|e| nat_code(ctx.rank2code, e.ref_id))
        .unwrap_or_default();

    let given = if blk.a == SENT {
        ""
    } else {
        ptext(db, &ctx.pools[0], blk.a as usize)
    };
    let surname = if blk.b == SENT {
        ""
    } else {
        ptext(db, &ctx.pools[1], blk.b as usize)
    };
    let common = if blk.c == SENT || blk.c as u64 >= ctx.pools[2].n as u64 {
        ""
    } else {
        ptext(db, &ctx.pools[2], blk.c as usize)
    };
    let name = if !given.is_empty() && !surname.is_empty() {
        format!("{given} {surname}")
    } else {
        [given, surname, common]
            .into_iter()
            .find(|x| !x.is_empty())
            .unwrap_or("")
            .to_string()
    };

    let gender = if s.gender_bit { "Female" } else { "Male" };
    let fmt = if rec.is_some() {
        RtfFormat::ExtPlayer
    } else {
        RtfFormat::ExtStaff
    };
    let kind = person_kind(fmt, gender);

    let age = age_of(ctx.gy, ctx.gd, blk.year, blk.day);

    let (height, club, ca, pa) = if let Some(pr) = rec {
        let b = pr.b;
        let height = ou16(db, b, ctx.ro.pl_height);
        let cid = ou24(db, b, ctx.ro.cid);
        let club = if cid == CID_NONE {
            String::new()
        } else {
            ctx.clubs.cid_club.get(&cid).cloned().unwrap_or_default()
        };
        (
            format!("{height} cm"),
            club,
            ou16(db, b, ctx.ro.pl_ca).to_string(),
            ou16(db, b, ctx.ro.pl_pa).to_string(),
        )
    } else if let Some(st) = staff {
        (
            format!("{} cm", height_of(st.hraw)),
            ctx.clubs.seq_club.get(&s.seq).cloned().unwrap_or_default(),
            st.ca.to_string(),
            st.pa.to_string(),
        )
    } else {
        (String::new(), String::new(), String::new(), String::new())
    };

    let detailed = DetailedInfo {
        skin_code: valid_code(skin_display(db[r + 8], eth), 0, 19),
        avatar: String::new(),
        is_newgan: if s.newgen { "Yes" } else { "No" }.to_string(),
        gender: gender.to_string(),
        age: age.to_string(),
        height,
        club,
        based: String::new(),
        preferred_job: String::new(),
        ca,
        pa,
        rca: String::new(),
        extra_fields: Vec::new(),
    };

    PlayerRecord {
        uid,
        primary_nat,
        sec_nat,
        name,
        hair_length: valid_code(db[r + 7], 0, 5),
        hair_color: valid_code(hair_display(db[r + 6], status, eth), 1, 19),
        ethnicity_code: valid_code(eth, 0, 10),
        kind,
        detailed: Some(detailed),
    }
}

// ===================== 分批产出 ================================================================

/// 一批人员块组装后的账目。`cancelled_at` 为 Some 时只用于让入口生成取消错误，绝不返回部分名单。
struct EmitReport {
    data: Vec<PlayerRecord>,
    bound: usize,
    dropped: usize,
    filtered: usize,
    empty_nat: usize,
    skin_ff: usize,
    cancelled_at: Option<usize>,
}

/// 按绑定结果逐块组装：Q10 跳过无记录体的 person 行、按「仅随机人」过滤、每 `BATCH` 块
/// 批量 `extend` 并上报进度/检查取消。抽成独立函数是为了让跳过与分布口径可在纯内存夹具上验证，
/// 不必每次拉起几百 MB 的真档。
fn emit_records(
    ctx: &AsmCtx<'_>,
    blocks: &[Block],
    binding: &[Option<Slot>],
    rec: &HashMap<u32, PlayerRec>,
    staff: &HashMap<u32, Vec<StaffRec>>,
    keep_only_newgen: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize),
) -> EmitReport {
    let total = blocks.len();
    let mut data: Vec<PlayerRecord> = Vec::with_capacity(total);
    let mut chunk: Vec<PlayerRecord> = Vec::with_capacity(BATCH);
    let mut bound = 0usize;
    let mut dropped = 0usize;
    let mut filtered = 0usize;
    let mut empty_nat = 0usize;
    let mut skin_ff = 0usize;
    let mut cancelled_at: Option<usize> = None;

    for bi in 0..total {
        let Some(s) = binding[bi] else { continue };
        bound += 1;
        let seq = s.seq.wrapping_sub(1);
        let rec_found = rec.get(&seq);
        let staff_found = pick_staff(staff, seq, s.i);
        // Q10：无球员且无职员记录体的 person 行不产出，否则默认 PlayerMale 会
        // 系统性污染约 6% 的提示词身份段。
        if rec_found.is_none() && staff_found.is_none() {
            dropped += 1;
            continue;
        }
        let record = assemble(ctx, &blocks[bi], &s, rec_found, staff_found);
        if ctx.db[blocks[bi].r + 8] == 0xFF {
            skin_ff += 1;
        }
        if record.primary_nat.is_empty() {
            empty_nat += 1;
        }
        if keep_only_newgen && !record.uid.starts_with(NEWGAN_UID_PREFIX) {
            filtered += 1;
        } else {
            chunk.push(record);
        }
        if chunk.len() >= BATCH {
            data.extend(chunk.drain(..));
        }
        if (bi + 1) % BATCH == 0 {
            progress(bi + 1, total);
            if cancel.load(Ordering::Relaxed) {
                cancelled_at = Some(bi + 1);
                break;
            }
        }
    }
    data.extend(chunk.drain(..));
    progress(total, total);
    EmitReport { data, bound, dropped, filtered, empty_nat, skin_ff, cancelled_at }
}

/// Q10 跳过数与空国籍行数汇总为两条 WARN；数字由调用方账目直接给出，便于断言条数一致。
fn emit_warnings(logger: &impl LogSink, dropped: usize, bound: usize, empty_nat: usize) {
    if dropped > 0 {
        let pct = if bound == 0 {
            0.0
        } else {
            100.0 * dropped as f64 / bound as f64
        };
        logger.warn(
            "fm",
            format!(
                "person 行（无球员/职员记录体）跳过 {dropped} 条，占绑定人员块 {bound} 的 {pct:.2}%；不产出以免污染提示词"
            ),
        );
    }
    if empty_nat > 0 {
        logger.warn(
            "fm",
            format!("空国籍行 {empty_nat} 条：保留行并留空国籍列，不猜填不丢行"),
        );
    }
}

// ===================== 前置校验 ================================================================

fn precheck(path: &Path, version: &str, diag: &mut Diag) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("fm") {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_PRECHECK,
            format!("扩展名期望 .fm，实际 {ext:?}"),
        ));
    }
    let meta = fs::metadata(path).map_err(|e| {
        fail(
            diag,
            path,
            version,
            STAGE_PRECHECK,
            format!("期望存档存在且可读，实际 metadata 失败：{e}"),
        )
    })?;
    diag.file_size = meta.len();
    if !meta.is_file() || meta.len() < 35 {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_PRECHECK,
            format!(
                "期望普通文件且 >=35 B（26 B 头 + 9 B 尾块头），实际 is_file={} 大小={} B",
                meta.is_file(),
                meta.len()
            ),
        ));
    }
    let mut h = [0u8; 26];
    let mut f = File::open(path).map_err(|e| {
        fail(
            diag,
            path,
            version,
            STAGE_PRECHECK,
            format!("期望可打开存档读取容器头，实际失败：{e}"),
        )
    })?;
    f.read_exact(&mut h).map_err(|e| {
        fail(
            diag,
            path,
            version,
            STAGE_PRECHECK,
            format!("容器头期望 26 B，实际读取失败：{e}"),
        )
    })?;
    if h[0] != 2 || h[1] != 1 || &h[2..6] != b"fmf." || h[6] != 8 {
        return Err(fail(
            diag,
            path,
            version,
            STAGE_PRECHECK,
            format!(
                "容器头期望 h[0]=2,h[1]=1,magic=fmf.,h[6]=8，实际 h[0]={},h[1]={},magic={:?},h[6]={}",
                h[0], h[1], &h[2..6], h[6]
            ),
        ));
    }
    Ok(())
}

// ===================== 公开入口 ================================================================

/// 解析 `.fm` 存档为「球员 + 职员」混合全量名单。
///
/// 顺序固定：前置校验 → 容器头/TOC → 版本判定与闸门 → `game_db` 解压 → 游戏日 →
/// 流水线 → 分批产出。所有失败都发生在返回之前，绝不吐出部分结果；本函数不碰 `AppState`、
/// 不写缓存，扫描阶段账目与替换阶段账目互相独立。
pub(crate) fn parse_fm_save(
    path: &Path,
    nat: &NationalityTable,
    keep_only_newgen: bool,
    logger: &impl LogSink,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(Vec<PlayerRecord>, RosterSummary, RosterCounts), String> {
    let started = Instant::now();
    let path_buf: PathBuf = path.to_path_buf();
    let mut diag = Diag::default();
    diag.file_size = fs::metadata(&path_buf).map(|m| m.len()).unwrap_or(0);
    let mut version = VERSION_UNKNOWN.to_string();

    macro_rules! step {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(err) => {
                    return Err(log_failure(logger, &path_buf, &diag, started, err));
                }
            }
        };
    }

    step!(precheck(&path_buf, &version, &mut diag));
    let toc = step!(read_header_and_toc(&path_buf, &version, &mut diag));
    // 版本闸门必须在解压 game_db 之前：非 24 立即整份 Err，绝不触碰几百 MB。
    let (major, _version_raw) = step!(read_version(&path_buf, &toc, &version, &mut diag));
    version = version_label(Some(major));
    let layout = match version_layout(major) {
        Ok(l) => l,
        Err(detail) => {
            let err = fail(&mut diag, &path_buf, &version, STAGE_VERSION, detail);
            return Err(log_failure(logger, &path_buf, &diag, started, err));
        }
    };
    let db_entry = match toc.find(layout.db_section) {
        Some(e) => e.clone(),
        None => {
            let err = fail(
                &mut diag,
                &path_buf,
                &version,
                STAGE_SECTION,
                format!(
                    "期望 TOC 含 {} 节，实际 {} 个条目中未找到",
                    layout.db_section,
                    toc.entries.len()
                ),
            );
            return Err(log_failure(logger, &path_buf, &diag, started, err));
        }
    };
    let db = step!(decompress_section(&path_buf, &db_entry, &version, &mut diag));
    let (gy, gd) = step!(game_date(&db, &path_buf, &version, &mut diag));
    let rank2code = match build_nation_index(nat, &db, &layout, logger) {
        Ok(m) => m,
        Err(e) => {
            let err = fail(&mut diag, &path_buf, &version, STAGE_FIELDS, e);
            return Err(log_failure(logger, &path_buf, &diag, started, err));
        }
    };
    let pools = step!(read_pools(&db, &layout, &path_buf, &version, &mut diag, logger));

    let blocks = read_blocks(&db, &pools, gy);
    let r_arr: Vec<usize> = blocks.iter().map(|b| b.r).collect();
    let (rec, staff, counts) = read_records(&db, &layout.record_offsets);
    let slots = read_slots(&db, &rec, &staff);
    let uid_occ = uid_occurrences(&db);
    let uid_seq = uid_sequence_map(&slots);
    let (binding, patch) = bind_all(&slots, &r_arr, &rec, &staff, &uid_seq, &uid_occ);
    let clubs = read_club_names(&db, &slots, &rec, &staff, logger);

    let mut kind_hist = [0usize; 19];
    for s in &slots {
        if (s.kind as usize) < kind_hist.len() {
            kind_hist[s.kind as usize] += 1;
        }
    }

    let total = blocks.len();
    let asm = AsmCtx {
        db: &db,
        pools: &pools,
        rank2code: &rank2code,
        clubs: &clubs,
        ro: &layout.record_offsets,
        gy,
        gd,
    };
    let report = emit_records(
        &asm,
        &blocks,
        &binding,
        &rec,
        &staff,
        keep_only_newgen,
        cancel,
        progress,
    );
    if let Some(at) = report.cancelled_at {
        let err = fail(
            &mut diag,
            &path_buf,
            &version,
            STAGE_CACHE,
            format!("名单解析已取消（停在第 {at} / {total} 块），未写入缓存"),
        );
        return Err(log_failure(logger, &path_buf, &diag, started, err));
    }
    let EmitReport { data, bound, dropped, filtered, empty_nat, skin_ff, .. } = report;
    drop(db);

    diag.person_skipped = dropped;
    emit_warnings(logger, dropped, bound, empty_nat);

    logger.debug(
        "fm",
        format!(
            "扫描完成：P={} to={} n1/n2={}/{} sum(compLen)={} B｜gameDate={gy}/{gd}｜\
             blocks={} slots={} kind={:?}｜playerRecs={} staffRecs={} staffKind3={}｜\
             绑定={bound} 补配={} 让位={} 四档候选={:?}｜产出={} person跳过={dropped} \
             空国籍={empty_nat} 仅随机人过滤={filtered}｜cid映射={} seq映射={}｜0xFF肤色={skin_ff}",
            toc.p,
            toc.to,
            toc.n1,
            toc.n2,
            toc.sum_comp,
            blocks.len(),
            slots.len(),
            kind_hist,
            counts.player,
            counts.staff,
            counts.kind3,
            patch.patched,
            patch.displaced,
            patch.cands,
            data.len(),
            clubs.cid_club.len(),
            clubs.seq_club.len(),
        ),
    );

    let summary = RosterSummary {
        total: data.len(),
        newgan: data
            .iter()
            .filter(|p| p.uid.starts_with(NEWGAN_UID_PREFIX))
            .count(),
        detailed: true,
        format: "FM 存档名单（球员+职员）",
        format_code: RtfFormat::ExtPlayer,
        language: "English".to_string(),
    };
    let counts_out = RosterCounts {
        rows: bound,
        kept: data.len(),
        filtered: 0,
        dropped,
        stray: 0,
    };
    Ok((data, summary, counts_out))
}

// ===================== 编译期活跃根 ============================================================

/// 解析入口由命令层调用；本模块单独编译时没有上游调用者，函数指针常量把整条解析链
/// 计入已使用，避免尚未接线的实现被判为 dead_code。
const _: fn(
    &Path,
    &NationalityTable,
    &crate::logger::Logger,
    &AtomicBool,
    &mut dyn FnMut(usize, usize),
) = keep_parse_chain_live;

fn keep_parse_chain_live(
    path: &Path,
    nat: &NationalityTable,
    logger: &crate::logger::Logger,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize),
) {
    let _ = parse_fm_save(path, nat, false, logger, cancel, progress);
    let _ = FM_NATION_RANKS_24.len() + FM_NATION_IDS_24.len();
}

// ===================== 单元测试 =================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::{ChannelLogger, LogLine};
    use crate::nationality::{NationalityDoc, NationalityMapping};
    use crate::rtf_scanner::PersonKind;

    /// 一节合成存档内容。`decomp_len` 允许谎报声明长度，用来构造「压缩帧不完整」负样本。
    struct Section {
        name: &'static str,
        raw: Vec<u8>,
        decomp_len: Option<u64>,
    }

    /// 版本节：长度前缀字符串形态的 `major.3.0+0`，与档内 game_info / save_game_summary 一致。
    fn version_section(major: u8) -> Vec<u8> {
        let s = format!("{major}.3.0+0");
        let mut v = Vec::with_capacity(4 + s.len());
        v.extend_from_slice(&(s.len() as u32).to_le_bytes());
        v.extend_from_slice(s.as_bytes());
        v
    }

    /// 造一份能被 `read_header_and_toc` 完整解析的最小 .fm：26 B 头 + 逐节 zstd 帧 + 9 B 尾块头
    /// + zstd(TOC)。TOC 第一块逐条记录，末尾留一个 !=1 的标记跳过第二块；`sum(compLen)+26 == 9+P`
    /// 由布局天然满足，不依赖任何真实存档。
    fn build_container(sections: &[Section]) -> Vec<u8> {
        let comps: Vec<Vec<u8>> = sections
            .iter()
            .map(|s| zstd::stream::encode_all(&s.raw[..], 0).unwrap())
            .collect();
        let sum: u64 = comps.iter().map(|c| c.len() as u64).sum();
        let to = 26u64 + sum;
        let p = to - 9;

        let mut toc: Vec<u8> = Vec::new();
        toc.extend_from_slice(&0u32.to_le_bytes()); // u32@0 = 0 → 第一块起点 p = 4
        toc.extend_from_slice(&(sections.len() as u32).to_le_bytes()); // n1 @ 4
        let mut a = 0u64;
        for (i, s) in sections.iter().enumerate() {
            let nm = s.name.as_bytes();
            toc.extend_from_slice(&(nm.len() as u32).to_le_bytes());
            toc.extend_from_slice(nm);
            toc.extend_from_slice(&4u32.to_le_bytes()); // extLen
            toc.extend_from_slice(&[0u8; 4]); // ext
            toc.extend_from_slice(&a.to_le_bytes()); // A
            toc.extend_from_slice(&(comps[i].len() as u64).to_le_bytes()); // compLen
            toc.extend_from_slice(&s.decomp_len.unwrap_or(s.raw.len() as u64).to_le_bytes());
            toc.extend_from_slice(&0i64.to_le_bytes()); // ts1
            toc.extend_from_slice(&0i64.to_le_bytes()); // ts2
            a += comps[i].len() as u64;
        }
        toc.extend_from_slice(&0u32.to_le_bytes()); // 第二块头 != 1 → 跳过

        let mut bytes = vec![0u8; 26];
        bytes[0] = 2;
        bytes[1] = 1;
        bytes[2..6].copy_from_slice(b"fmf.");
        bytes[6] = 8;
        bytes[9..17].copy_from_slice(&p.to_le_bytes());
        for c in &comps {
            bytes.extend_from_slice(c);
        }
        bytes.extend_from_slice(&[0u8; 9]); // 尾块 9 B 头
        bytes.extend_from_slice(&zstd::stream::encode_all(&toc[..], 0).unwrap());
        bytes
    }

    fn temp_fm(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn run_parse(path: &Path) -> (Result<(Vec<PlayerRecord>, RosterSummary, RosterCounts), String>, Vec<LogLine>) {
        let nat = NationalityTable::builtin();
        let (logger, rx) = ChannelLogger::channel();
        let cancel = AtomicBool::new(false);
        let mut progress = |_: usize, _: usize| {};
        let res = parse_fm_save(path, &nat, false, &logger, &cancel, &mut progress);
        drop(logger);
        let logs: Vec<LogLine> = rx.try_iter().collect();
        (res, logs)
    }

    // ---- 纯内存组装夹具：让 Q10 / 分布 / 恒空字段在 CI 上可复现，不依赖真档 ---------------------------

    fn empty_pools() -> [Pool; 3] {
        [
            Pool { offs: Vec::new(), n: 0, head: 0, end: 0, broken: -1 },
            Pool { offs: Vec::new(), n: 0, head: 0, end: 0, broken: -1 },
            Pool { offs: Vec::new(), n: 0, head: 0, end: 0, broken: -1 },
        ]
    }

    struct Harness {
        db: Vec<u8>,
        pools: [Pool; 3],
        rank2code: HashMap<u8, String>,
        clubs: ClubIndex,
        ro: RecordOffsets,
        gy: u16,
        gd: u16,
        blocks: Vec<Block>,
        binding: Vec<Option<Slot>>,
        rec: HashMap<u32, PlayerRec>,
        staff: HashMap<u32, Vec<StaffRec>>,
    }

    impl Harness {
        fn ctx(&self) -> AsmCtx<'_> {
            AsmCtx {
                db: &self.db,
                pools: &self.pools,
                rank2code: &self.rank2code,
                clubs: &self.clubs,
                ro: &self.ro,
                gy: self.gy,
                gd: self.gd,
            }
        }

        fn run(&self, keep_only_newgen: bool, cancel: &AtomicBool) -> EmitReport {
            let ctx = self.ctx();
            let mut progress = |_: usize, _: usize| {};
            emit_records(
                &ctx,
                &self.blocks,
                &self.binding,
                &self.rec,
                &self.staff,
                keep_only_newgen,
                cancel,
                &mut progress,
            )
        }
    }

    /// n 个已绑定人员块：默认全球员、40% 肤色 0xFF、1/3 随机人、1/4 女性；名池取哨兵、
    /// 关系区首字节非 0xFF 使得归化/第二国籍走空路径，字段只由 db[r+4..+9] 与 rank 决定。
    fn harness(n: usize) -> Harness {
        let mut db = vec![0u8; n * 8 + 2048];
        let mut blocks = Vec::with_capacity(n);
        let mut binding = Vec::with_capacity(n);
        let mut rec = HashMap::new();
        let rank2code: HashMap<u8, String> = (0u8..=255).map(|r| (r, "USA".to_string())).collect();
        for i in 0..n {
            let r = i * 8;
            db[r + 4] = 0; // status：不触发归化 / 发色调色板
            db[r + 5] = (i % 11) as u8; // eth
            db[r + 6] = (i % 6 + 1) as u8; // 发色原值
            db[r + 7] = (i % 6) as u8; // 发长
            db[r + 8] = if i % 5 < 2 { 0xFF } else { (i % 20 + 1) as u8 };
            db[r + 9] = 0;
            blocks.push(Block {
                r,
                a: SENT,
                b: SENT,
                c: SENT,
                day: 100,
                year: 1990,
                rank: (i % 250) as u8,
            });
            let uid = (i + 1) as u32;
            binding.push(Some(Slot {
                i: r,
                uid,
                seq: (i + 1) as u32,
                kind: 2,
                dual: true,
                gender_bit: i % 4 == 0,
                newgen: i % 3 == 0,
            }));
            rec.insert(i as u32, PlayerRec { b: r, self_uid: uid });
        }
        Harness {
            db,
            pools: empty_pools(),
            rank2code,
            clubs: ClubIndex { cid_club: HashMap::new(), seq_club: HashMap::new() },
            ro: FM24_LAYOUT.record_offsets,
            gy: 2032,
            gd: 200,
            blocks,
            binding,
            rec,
            staff: HashMap::new(),
        }
    }

    // ---- 1. 版本孔位 -------------------------------------------------------------------------------

    #[test]
    fn version_layout_rejects_non_24_without_decompress() {
        assert!(version_layout(24).is_ok());
        for m in [0u8, 23, 25, 26, 30, 255] {
            let e = version_layout(m).err().expect("非 24 版本必须拒绝");
            assert!(e.contains("不支持的 FM 版本"), "major={m}: {e}");
            assert!(e.contains("未解压 game_db"), "major={m}: {e}");
        }

        // 整档层面：版本节报 major=26，闸门必须在解压 game_db 之前整份拒绝。
        let dir = tempfile::tempdir().unwrap();
        let path = temp_fm(
            &dir,
            "gate26.fm",
            &build_container(&[
                Section { name: "save_game_summary", raw: version_section(26), decomp_len: None },
                Section { name: "game_db", raw: vec![0u8; 16], decomp_len: Some(999) },
            ]),
        );
        let nat = NationalityTable::builtin();
        let (logger, rx) = ChannelLogger::channel();
        let cancel = AtomicBool::new(false);
        let mut progress = |_: usize, _: usize| {};
        let err = parse_fm_save(&path, &nat, false, &logger, &cancel, &mut progress).unwrap_err();
        drop(logger);
        assert!(err.contains("阶段：版本判定"), "{err}");
        assert!(err.contains("版本：FM26 (major=26)"), "{err}");
        assert!(err.contains("未解压 game_db"), "{err}");
        let logs: Vec<LogLine> = rx.try_iter().collect();
        assert!(
            !logs.iter().any(|l| l.message.contains("节 game_db")),
            "版本闸门之后才允许出现 game_db 解压记录"
        );
    }

    // ---- 2. 截断 TOC -------------------------------------------------------------------------------

    #[test]
    fn truncated_toc_returns_stage_toc() {
        // 尾块 TOC 是一个合法但只解出 10 B 的 zstd 帧 → `toc_bytes.len() < 64` → 阶段 TOC。
        let mut bytes = vec![0u8; 26];
        bytes[0] = 2;
        bytes[1] = 1;
        bytes[2..6].copy_from_slice(b"fmf.");
        bytes[6] = 8;
        bytes[9..17].copy_from_slice(&17u64.to_le_bytes()); // P=17 → to=26
        bytes.extend_from_slice(&[0u8; 9]); // 尾块头
        bytes.extend_from_slice(&zstd::stream::encode_all(&[0u8; 10][..], 0).unwrap());

        let dir = tempfile::tempdir().unwrap();
        let path = temp_fm(&dir, "trunc_toc.fm", &bytes);
        let (res, _logs) = run_parse(&path);
        let err = res.unwrap_err();
        assert!(err.contains("阶段：TOC"), "{err}");
        assert!(err.contains(format!("版本：{VERSION_UNKNOWN}").as_str()), "{err}");
        assert!(err.contains("文件「trunc_toc.fm」"), "{err}");
        assert!(
            !err.contains(dir.path().to_string_lossy().as_ref()),
            "三要素文案只给基名，不得带完整路径：{err}"
        );
    }

    // ---- 3. 截断 game_db ---------------------------------------------------------------------------

    #[test]
    fn truncated_game_db_returns_stage_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_fm(
            &dir,
            "trunc_db.fm",
            &build_container(&[
                Section { name: "save_game_summary", raw: version_section(24), decomp_len: None },
                Section { name: "game_db", raw: vec![0u8; 12], decomp_len: Some(100) },
            ]),
        );
        let (res, _logs) = run_parse(&path);
        let err = res.unwrap_err();
        assert!(err.contains("阶段：目标节解压"), "{err}");
        assert!(err.contains("版本：FM24 (major=24)"), "{err}");
        assert!(err.contains("声明 100 B"), "{err}");
        assert!(err.contains("实际解压 12 B"), "{err}");
        assert!(err.contains("压缩帧不完整"), "{err}");
        assert!(err.contains("文件「trunc_db.fm」"), "{err}");
        assert!(!err.contains(dir.path().to_string_lossy().as_ref()), "{err}");
    }

    // ---- 4. 前置校验阶段 ---------------------------------------------------------------------------

    #[test]
    fn front_check_stages() {
        let dir = tempfile::tempdir().unwrap();
        let bad_ext = temp_fm(&dir, "plain.txt", &[0u8; 40]);
        let missing = dir.path().join("not_there.fm");
        let mut bad_header = vec![0u8; 40];
        bad_header[0] = 9;
        let bad_header = temp_fm(&dir, "bad_header.fm", &bad_header);

        for path in [&bad_ext, &missing, &bad_header] {
            let (res, _logs) = run_parse(path);
            let err = res.unwrap_err();
            assert!(err.contains("阶段：前置校验"), "{err}");
            assert!(!err.contains("zstd decompress failed"), "{err}");
            assert!(!err.contains("decompress failed"), "{err}");
        }
    }

    // ---- 5. Error 日志完整上下文 -----------------------------------------------------------------------

    #[test]
    fn error_log_full_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_fm(
            &dir,
            "ctx.fm",
            &build_container(&[
                Section { name: "save_game_summary", raw: version_section(24), decomp_len: None },
                Section { name: "game_db", raw: vec![0u8; 12], decomp_len: Some(100) },
            ]),
        );
        let nat = NationalityTable::builtin();
        let (logger, rx) = ChannelLogger::channel();
        let cancel = AtomicBool::new(false);
        let mut progress = |_: usize, _: usize| {};
        let res = parse_fm_save(&path, &nat, false, &logger, &cancel, &mut progress);
        drop(logger);
        let err = res.unwrap_err();
        assert!(err.contains("阶段：目标节解压"), "{err}");
        let logs: Vec<LogLine> = rx.try_iter().collect();
        let errors: Vec<&LogLine> = logs.iter().filter(|l| l.level == "ERROR").collect();
        assert!(errors.len() >= 2, "应各有一条完整上下文与一条三要素文案：{errors:?}");

        let full = errors
            .iter()
            .find(|l| l.message.contains("完整路径="))
            .expect("缺少完整上下文 Error 日志");
        for key in [
            "完整路径=",
            "大小=",
            "P=",
            "to=",
            "TOC n1=",
            "n2=",
            "sum(compLen) 期望=",
            "实际=",
            "compLen=",
            "decompLen=",
            "实解长度=",
            "版本原始串=",
            "major=",
            "阶段=",
            "原因=",
            "耗时=",
            "person 跳过数=",
        ] {
            assert!(full.message.contains(key), "完整上下文缺少 {key}：{}", full.message);
        }
        assert!(full.message.contains("版本原始串=24.3.0+0"), "{}", full.message);
        assert!(full.message.contains("major=24"), "{}", full.message);
        assert!(full.message.contains("阶段=目标节解压"), "{}", full.message);
        assert!(full.message.contains(path.to_string_lossy().as_ref()), "{}", full.message);
        assert!(
            errors.iter().any(|l| l.message.contains("FM 存档失败：文件「") && l.message.contains("阶段：目标节解压")),
            "三要素文案应作为独立 Error 行落盘"
        );
    }

    // ---- 6. 肤色渲染 -------------------------------------------------------------------------------

    #[test]
    fn skin_code_rendered_within_range() {
        for eth in 0u8..11 {
            let v = skin_display(0xFF, eth);
            assert_eq!(v, ETH_SKIN[eth as usize], "eth={eth}");
            assert!((0..=19).contains(&v), "eth={eth} 渲染出 {v} 越界");
            let code = valid_code(v, 0, 19);
            assert_eq!(code, v.to_string(), "eth={eth}");
            assert_ne!(code, "255", "eth={eth} 不得写 255");
        }
        // 全 raw × eth 口径：任何输入都不得产出 255，解析出的码必须 <=19。
        for raw in 0u8..=255 {
            for eth in 0u8..=11 {
                let code = valid_code(skin_display(raw, eth), 0, 19);
                assert_ne!(code, "255", "raw={raw} eth={eth}");
                if let Ok(n) = code.parse::<u32>() {
                    assert!(n <= 19, "raw={raw} eth={eth} 产出 {n}");
                }
            }
        }
        for raw in [0u8, 1, 5, 19, 20, 254] {
            assert_eq!(skin_display(raw, 3), raw, "非 0xFF 原值应原样透传");
        }
        assert_eq!(valid_code(skin_display(20, 3), 0, 19), "", "越 0..19 收敛为空");
        assert_eq!(valid_code(0xFF, 0, 19), "", "裸 255 必须收敛为空");
        assert_eq!(skin_display(0xFF, 200), 0, "eth 越表回落 0");
        assert_eq!(valid_code(skin_display(0xFF, 200), 0, 19), "0");
    }

    // ---- 7. 发色调色板全分支 -----------------------------------------------------------------------

    #[test]
    fn hair_color_palette_predicate() {
        // 独立复述 fmexport.mjs:355-363 的谓词，逐值比对全部分支。
        let reference = |raw: u8, status: u8, eth: u8| -> u8 {
            if status != 0xFF || !matches!(raw, 6 | 17 | 18) {
                return raw;
            }
            if eth == 0 {
                return 3;
            }
            if eth == 1 || eth == 7 {
                return 5;
            }
            raw
        };
        for raw in 0u8..=255 {
            for status in [0u8, 1, 0xFE, 0xFF] {
                for eth in 0u8..=11 {
                    let got = hair_display(raw, status, eth);
                    assert_eq!(got, reference(raw, status, eth), "raw={raw} status={status} eth={eth}");
                    if status == 0xFF && matches!(raw, 6 | 17 | 18) {
                        assert!((1..=19).contains(&got), "调色板产出 {got} 越 1..19");
                        assert!(!valid_code(got, 1, 19).is_empty(), "调色板产出 {got} 应合法");
                    }
                }
            }
        }
        assert_eq!(hair_display(6, 0xFF, 0), 3);
        assert_eq!(hair_display(17, 0xFF, 0), 3);
        assert_eq!(hair_display(18, 0xFF, 0), 3);
        assert_eq!(hair_display(6, 0xFF, 1), 5);
        assert_eq!(hair_display(18, 0xFF, 7), 5);
        assert_eq!(hair_display(6, 0xFF, 2), 6);
        assert_eq!(hair_display(17, 0xFF, 10), 17);
        assert_eq!(hair_display(6, 0xFE, 0), 6);
        assert_eq!(hair_display(5, 0xFF, 0), 5);
    }

    // ---- 8. 闰年年龄换算 ---------------------------------------------------------------------------

    #[test]
    fn age_leap_year_conversion() {
        assert!(is_leap(2000));
        assert!(is_leap(2024));
        assert!(!is_leap(1900));
        assert!(!is_leap(2100));
        assert!(!is_leap(2023));
        assert_eq!(md_of(1, 2024), (1, 1));
        assert_eq!(md_of(59, 2023), (2, 28));
        assert_eq!(md_of(60, 2023), (3, 1));
        assert_eq!(md_of(59, 2024), (2, 28));
        assert_eq!(md_of(60, 2024), (2, 29));
        assert_eq!(md_of(61, 2024), (3, 1));
        assert_eq!(md_of(365, 2023), (12, 31));
        assert_eq!(md_of(366, 2024), (12, 31));

        // 残差集：BY 闰年、GY 平年、BD = GD+1。平年第 GD 天与闰年第 BD 天是同一月日，
        // 生日已到 → 年龄不减一；朴素 GD<BD 一律减一，故必然漏判。
        let leap_birth = [1992u16, 2000, 2004, 2016, 2020];
        let plain_game = [2021u16, 2023, 2100];
        let mut residual = 0usize;
        let mut residual_hit = 0usize;
        let mut naive_hit = 0usize;
        for &by in &leap_birth {
            for &gy in &plain_game {
                if gy <= by {
                    continue;
                }
                for gd in 60u16..=365 {
                    let bd = gd + 1;
                    let expected = gy as i32 - by as i32;
                    if age_of(gy, gd, by, bd) == expected {
                        residual_hit += 1;
                    }
                    if gy as i32 - by as i32 - i32::from(gd < bd) == expected {
                        naive_hit += 1;
                    }
                    residual += 1;
                }
            }
        }
        assert!(residual > 0);
        assert_eq!(residual_hit, residual, "闰年换算在 day=GD+1 残差集上应 100% 命中");
        assert!(naive_hit < residual, "朴素式对照命中率必须 <100%");

        // 同年型对照：都平年时 GD+1 生日未到，两种口径一致减一。
        for gd in 1u16..=365 {
            let got = age_of(2023, gd, 1999, gd + 1);
            assert_eq!(got, 2023 - 1999 - 1, "gd={gd}");
            assert_eq!(got, 2023i32 - 1999 - i32::from(gd < gd + 1));
        }

        // 全网格：朴素式整体命中率 <100% 但接近 100%（差异集中在 2/29 前后）。
        let mut total = 0usize;
        let mut agree = 0usize;
        for &gy in &[2023u16, 2024] {
            for &by in &[1999u16, 2000] {
                for gd in 1u16..=365 {
                    for bd in 1u16..=366 {
                        total += 1;
                        let got = age_of(gy, gd, by, bd);
                        let naive = gy as i32 - by as i32 - i32::from(gd < bd);
                        if got == naive {
                            agree += 1;
                        }
                    }
                }
            }
        }
        assert!(agree < total, "朴素式整体命中率必须 <100%");
        assert!(agree as f64 / total as f64 > 0.99, "差异应集中在闰日边界");
    }

    // ---- 9. L8 恒空 --------------------------------------------------------------------------------

    #[test]
    fn l8_empty_fields() {
        let mut h = harness(4);
        for i in 2..4usize {
            h.rec.remove(&(i as u32));
            h.staff.insert(
                i as u32,
                vec![StaffRec { p: h.blocks[i].r, self_uid: (i + 1) as u32, ca: 88, pa: 99, hraw: 0 }],
            );
        }
        let report = h.run(false, &AtomicBool::new(false));
        assert_eq!(report.data.len(), 4);
        assert_eq!(report.bound, 4);
        assert_eq!(report.dropped, 0);
        for p in &report.data {
            let d = p.detailed.as_ref().expect("detailed 恒 Some");
            assert_eq!(d.avatar, "");
            assert_eq!(d.based, "");
            assert_eq!(d.preferred_job, "");
            assert_eq!(d.rca, "");
            assert!(d.extra_fields.is_empty());
            // 恒空字段不能顺手抹掉同结构里的有效列。
            assert!(!d.height.is_empty(), "身高列不应为空");
            assert!(!d.ca.is_empty() && !d.pa.is_empty(), "CA/PA 列不应为空");
            assert!(!d.age.is_empty() && !d.gender.is_empty(), "年龄/性别列不应为空");
            assert!((0..=19).contains(&d.skin_code.parse::<u32>().unwrap_or(0)), "skin_code={}", d.skin_code);
        }
        // 球员 / 职员各占一半，四条记录的 kind 覆盖两类性别口径。
        assert!(report.data.iter().any(|p| p.kind.is_staff()));
        assert!(report.data.iter().any(|p| !p.kind.is_staff()));
    }

    // ---- 10. Q10 跳过 + WARN --------------------------------------------------------------------------

    #[test]
    fn person_rows_dropped_and_warned() {
        let mut h = harness(64);
        let mut expected_dropped = 0usize;
        for i in 0..64usize {
            if i % 7 == 0 {
                h.rec.remove(&(i as u32)); // 无球员、无职员记录体 → person 行
                expected_dropped += 1;
            }
        }
        let report = h.run(false, &AtomicBool::new(false));
        assert_eq!(report.bound, 64);
        assert_eq!(report.dropped, expected_dropped);
        assert_eq!(report.data.len(), report.bound - report.dropped, "记录数 == 绑定数 − dropped");

        // WARN 汇总一条，报出的条数与 dropped 一致，且含占比。
        let (logger, rx) = ChannelLogger::channel();
        emit_warnings(&logger, report.dropped, report.bound, report.empty_nat);
        drop(logger);
        let warns: Vec<LogLine> = rx.try_iter().filter(|l| l.level == "WARNING").collect();
        let person: Vec<&LogLine> = warns.iter().filter(|l| l.message.contains("person 行")).collect();
        assert_eq!(person.len(), 1, "person 跳过应汇总为一条 WARN");
        assert!(person[0].message.contains(&format!("{} 条", report.dropped)), "{}", person[0].message);
        assert!(person[0].message.contains('%'), "WARN 应含占比：{}", person[0].message);

        // 取消：命中第一个 BATCH 检查点即停，绝不返回完整结果。
        let h2 = harness(4100);
        let cancelled = AtomicBool::new(true);
        let r2 = h2.run(false, &cancelled);
        assert_eq!(r2.cancelled_at, Some(BATCH));
        assert!(r2.bound < 4100, "取消后不应继续组装余下人员块");
    }

    // ---- 11. 国籍索引回落空串 --------------------------------------------------------------------------

    #[test]
    fn nation_index_falls_back_to_empty_string() {
        // 迷你国籍表：码全集只有 DZA，且英文名 Algeria → DZA，可同时覆盖
        // resolve(en) 命中与「查不到回落空串」两条路径。
        let doc = NationalityDoc {
            languages: vec!["English".to_string()],
            codes: Vec::new(),
            mappings: vec![NationalityMapping {
                code: "DZA".to_string(),
                language: "English".to_string(),
                name: "Algeria".to_string(),
            }],
        };
        let nat = NationalityTable::from_doc(doc).unwrap();
        let db = vec![0u8; 64];
        let (logger, _rx) = ChannelLogger::channel();
        let idx = build_nation_index(&nat, &db, &FM24_LAYOUT, &logger).unwrap();

        // 名次表 238 对 rank 全部保留：查不到映射留空，不报错不丢行。
        assert_eq!(idx.len(), 238, "rank 表 0..=251 有 14 个缺号，应剩 238 个键");
        let empties = idx.values().filter(|c| c.is_empty()).count();
        assert!(empties >= 200, "大部分异码应回落空串，实际 {empties}");
        assert!(empties < 238, "至少要有 resolve 命中的异码，实际 {empties} 个空码");
        assert_eq!(nat_code(&idx, 0), "DZA", "rank 0 的 ALG 应经英文名归一到 DZA");
        assert_eq!(nat_code(&idx, 122), "", "名次表缺号 122 应回落空串");
        assert_eq!(nat_code(&idx, 250), "", "名次表缺号 250 应回落空串");
        assert_eq!(nat_code(&idx, 999), "", "表外 rank 应回落空串");

        // 行数不减：空国籍行照样产出 record，绝不因国籍查不到丢行。
        let mut h = harness(8);
        h.rank2code = idx;
        let report = h.run(false, &AtomicBool::new(false));
        assert_eq!(report.bound, 8);
        assert_eq!(report.dropped, 0);
        assert_eq!(report.data.len(), 8, "空国籍不得丢行");
        assert!(report.data.iter().any(|p| p.primary_nat.is_empty()), "应存在空国籍行");
        assert!(report.empty_nat >= 1, "空国籍行应单列计数");
    }

    // ---- 12. 分布比例区间 -----------------------------------------------------------------------------

    #[test]
    fn distribution_ratios() {
        const N: usize = 5000;
        let mut h = harness(N);
        // 造出四值 kind：偶数块球员、奇数块职员；i%4<2 为女性。
        for i in 0..N {
            if let Some(s) = h.binding[i].as_mut() {
                s.gender_bit = i % 4 < 2;
            }
            if i % 2 == 1 {
                h.rec.remove(&(i as u32));
                h.staff.insert(
                    i as u32,
                    vec![StaffRec {
                        p: h.blocks[i].r,
                        self_uid: (i + 1) as u32,
                        ca: 100,
                        pa: 120,
                        hraw: 0,
                    }],
                );
            }
            h.blocks[i].rank = 1; // rank 1 在表里 → 非空国籍
        }
        // 只留 1 条表外 rank，使空国籍占比 <0.05%。
        h.blocks[0].rank = 200;
        h.rank2code.remove(&200);

        let report = h.run(false, &AtomicBool::new(false));
        assert_eq!(report.bound, N);
        assert_eq!(report.dropped, 0);
        assert_eq!(report.data.len(), N);
        let total = report.data.len() as f64;

        // 新生占比：1/3 附近，落在宽区间内（真档两端差异极大，这里只守住口径不被清零/全占）。
        let newgan = report
            .data
            .iter()
            .filter(|p| p.uid.starts_with(NEWGAN_UID_PREFIX))
            .count();
        let newgan_ratio = newgan as f64 / total;
        assert!((0.05..=0.60).contains(&newgan_ratio), "新生占比 {newgan_ratio:.4} 越界");

        // 0xFF 肤色占比：§14 红线区间 16.12%..=74.63%；夹具按 2/5 构造。
        let ff_ratio = report.skin_ff as f64 / report.bound as f64;
        assert!((0.1612..=0.7463).contains(&ff_ratio), "0xFF 肤色占比 {ff_ratio:.4} 越界");

        // 空国籍 <0.05%：只构造 1 条表外 rank。
        let empty_nat = report.data.iter().filter(|p| p.primary_nat.is_empty()).count();
        assert!(empty_nat as f64 / total < 0.0005, "空国籍占比 {} 越界", empty_nat as f64 / total);
        assert_eq!(report.empty_nat, empty_nat, "空国籍应与账目一致");

        // kind 四值占比区间：构造为各 25%，断言都落在 0.15..=0.35 且总和 == 总数。
        let mut hist = [0usize; 4];
        for p in &report.data {
            let k = match p.kind {
                PersonKind::PlayerMale => 0,
                PersonKind::PlayerFemale => 1,
                PersonKind::StaffMale => 2,
                PersonKind::StaffFemale => 3,
            };
            hist[k] += 1;
        }
        assert_eq!(hist.iter().sum::<usize>(), report.data.len(), "kind 四值覆盖全量");
        for (i, &c) in hist.iter().enumerate() {
            let ratio = c as f64 / total;
            assert!((0.15..=0.35).contains(&ratio), "kind[{i}] 占比 {ratio:.4} 越界");
        }

        // 全局不变量：skin_code 不越界、不写 255、detailed 恒 Some、L8 恒空。
        let mut out_of_range = 0usize;
        let mut has_255 = 0usize;
        let mut detailed_some = 0usize;
        for p in &report.data {
            let d = p.detailed.as_ref();
            if d.is_some() {
                detailed_some += 1;
            }
            let d = d.unwrap();
            if let Ok(v) = d.skin_code.parse::<u32>() {
                if !(0..=19).contains(&v) {
                    out_of_range += 1;
                }
                if v == 255 {
                    has_255 += 1;
                }
            }
            assert_eq!(d.avatar, "");
            assert_eq!(d.based, "");
            assert_eq!(d.preferred_job, "");
            assert_eq!(d.rca, "");
            assert!(d.extra_fields.is_empty());
        }
        assert_eq!(out_of_range, 0, "skin_code 越 0..19 的条数");
        assert_eq!(has_255, 0, "skin_code 写成 255 的条数");
        assert_eq!(detailed_some, report.data.len(), "detailed 必须恒 Some");

        // 「仅随机人」筛选：keep_only_newgen=true 时只留 r- 前缀，filtered 与差量一致。
        let filtered = h.run(true, &AtomicBool::new(false));
        assert_eq!(filtered.data.len(), newgan);
        assert_eq!(filtered.filtered, N - newgan);
        assert!(filtered.data.iter().all(|p| p.uid.starts_with(NEWGAN_UID_PREFIX)));
    }

    // ---- 13. 真档判卷（T05 逐列 harness 在此骨架上扩展） -----------------------------------------------

    /// 环境变量 `FM_SAVE_DIR` 解门控；`-- --ignored` 才运行。缺档打印后跳过，不留悬挂失败。
    /// 只判产出条数 ±0.5% 与全局不变量：逐列 rate / 汉字名比例依赖外部 verify.json 判卷资产，
    /// 不属于单测能自洽判定的范围。
    fn real_save_dir() -> Option<PathBuf> {
        std::env::var_os("FM_SAVE_DIR").map(PathBuf::from)
    }

    struct JudgeCase {
        file: &'static str,
        expect: usize,
    }

    fn judge_one(dir: &Path, case: &JudgeCase) -> bool {
        let path = dir.join(case.file);
        if !path.is_file() {
            eprintln!("[real_saves_judging] 缺档跳过：{}", path.display());
            return false;
        }
        let nat = NationalityTable::builtin();
        let (logger, rx) = ChannelLogger::channel();
        let cancel = AtomicBool::new(false);
        let mut progress = |_: usize, _: usize| {};
        let started = Instant::now();
        let res = parse_fm_save(&path, &nat, false, &logger, &cancel, &mut progress);
        let elapsed_ms = started.elapsed().as_millis();
        drop(logger);
        let logs: Vec<LogLine> = rx.try_iter().collect();
        let err = match res {
            Ok(v) => v,
            Err(e) => {
                let tail: Vec<&str> = logs.iter().map(|l| l.message.as_str()).collect();
                panic!("真档解析失败：{e}\n阶段日志：{tail:#?}");
            }
        };
        let (data, summary, counts) = err;

        // 产出条数 ±0.5%：期望值来自 summary.json 的 role.player + role.staff；
        // role.person 按 Q10 跳过、计入 dropped，不计入产出。
        let lo = (case.expect as f64 * 0.995) as usize;
        let hi = (case.expect as f64 * 1.005) as usize;
        assert!(
            (lo..=hi).contains(&data.len()),
            "{} 产出 {} 条，期望 {} ±0.5% [{lo}, {hi}]",
            case.file,
            data.len(),
            case.expect
        );
        assert_eq!(summary.total, data.len(), "摘要 total 应与产出一致");
        assert!(summary.detailed, "摘要 detailed 应为 true");
        assert_eq!(summary.format, "FM 存档名单（球员+职员）");
        assert_eq!(summary.format_code, RtfFormat::ExtPlayer);
        assert_eq!(summary.language, "English");
        assert_eq!(summary.newgan, data.iter().filter(|p| p.uid.starts_with(NEWGAN_UID_PREFIX)).count());
        assert_eq!(counts.kept, data.len(), "扫描阶段 kept == 产出条数");
        assert_eq!(counts.rows, counts.kept + counts.dropped, "rows == kept + dropped");
        assert_eq!(counts.filtered, 0, "扫描阶段 filtered 恒 0");
        assert_eq!(counts.stray, 0, "扫描阶段 stray 恒 0");
        assert!(counts.dropped > 0, "真档应存在 Q10 person 跳过");

        let mut skin_oob = 0usize;
        let mut skin_255 = 0usize;
        let mut detailed_some = 0usize;
        let mut kinds = [0usize; 4];
        for p in &data {
            if let Some(d) = p.detailed.as_ref() {
                detailed_some += 1;
                if let Ok(v) = d.skin_code.parse::<u32>() {
                    if !(0..=19).contains(&v) {
                        skin_oob += 1;
                    }
                    if v == 255 {
                        skin_255 += 1;
                    }
                }
                assert_eq!(d.avatar, "");
                assert_eq!(d.based, "");
                assert_eq!(d.preferred_job, "");
                assert_eq!(d.rca, "");
                assert!(d.extra_fields.is_empty());
            }
            let k = match p.kind {
                PersonKind::PlayerMale => 0,
                PersonKind::PlayerFemale => 1,
                PersonKind::StaffMale => 2,
                PersonKind::StaffFemale => 3,
            };
            kinds[k] += 1;
        }
        assert_eq!(skin_oob, 0, "skin_code 越 0..19 的条数");
        assert_eq!(skin_255, 0, "skin_code 写成 255 的条数");
        assert_eq!(detailed_some, data.len(), "detailed 必须恒 Some");
        assert!(kinds[0] > 0 && kinds[2] > 0, "球员与职员两族都应出现");
        assert_eq!(kinds.iter().sum::<usize>(), data.len(), "kind 四值覆盖全量");

        eprintln!(
            "[real_saves_judging] {}｜产出={} 条｜dropped={}｜耗时={} ms｜skin 越界={}｜detailed Some={}｜kind={:?}",
            case.file,
            data.len(),
            counts.dropped,
            elapsed_ms,
            skin_oob,
            detailed_some,
            kinds,
        );
        true
    }

    #[test]
    #[ignore]
    fn real_saves_judging() {
        let Some(dir) = real_save_dir() else {
            eprintln!("[real_saves_judging] 未设置 FM_SAVE_DIR，跳过");
            return;
        };
        let cases = [
            JudgeCase { file: "ZhanHao Huang-肇庆天峰东湖20261025.fm", expect: 219_014 },
            JudgeCase { file: "ZhanHao Huang-利物浦.fm", expect: 297_262 },
        ];
        let mut ran = 0usize;
        for case in &cases {
            if judge_one(&dir, case) {
                ran += 1;
            }
        }
        assert!(ran > 0, "FM_SAVE_DIR={} 下没有任何可判真档", dir.display());
    }
}

