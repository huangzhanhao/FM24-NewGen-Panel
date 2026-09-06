//! 【新手导读】球员 → 头像映射（自旧版 core/FaceMapper.py 移植，含 0-10 种族码修正规则）。
//! 这是整个应用的「大脑」：根据每个球员的国籍 + 种族码，算出该用头像包里哪个种族目录，
//! 再从目录里选一张图片，最终得到一条「球员 UID → 图片」的映射。
use crate::facepack::{scan_pack, PackLayout, PackScan}; // 标准识别/扫描的唯一来源（facepack.rs）
use crate::rtf_parser::PlayerRecord;
use crate::xml_parser::{self, MappingEntry};
use rand::Rng;                          // 随机数库（从图片池里随机选一张）
use std::collections::{BTreeMap, BTreeSet, HashMap}; // BTree* = 有序集合，保证行为可复现
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering}; // 原子布尔：跨线程读取「是否已取消」

/// 替换模式（对应前端三个模式按钮）。
/// enum 是「枚举」：值只能是这几个之一，比用字符串更安全。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Preserve,   // 保留：已换过的不动，只给新球员补图
    Overwrite,  // 覆写：名单内球员全部重抽，名单外保留
    Generate,   // 生成：无视现有 config.xml，从零生成全部映射
}

impl Mode {
    /// 把前端传来的字符串解析为 Mode 枚举（未知字符串返回错误）。
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "Preserve" => Ok(Mode::Preserve),
            "Overwrite" => Ok(Mode::Overwrite),
            "Generate" => Ok(Mode::Generate),
            _ => Err(format!("未知模式: {s}")),
        }
    }
}

/// 头像映射器。持有一个头像包目录的「子目录 → 图片名集合」索引。
pub struct FaceMapper {
    pub img_dir: String,
    /// 目录名 → 图片名集合（无扩展名）；BTreeMap/BTreeSet 保证行为可复现。
    /// 【为何用有序集合】随机选图时需要按下标取元素，且有序保证同样输入产生同样结果（便于测试）。
    pub faces_map: BTreeMap<String, BTreeSet<String>>,
    /// 布局判定结果（facepack::classify），平铺分支据此早返回。
    pub layout: PackLayout,
    /// 标准一平铺池：包根全部图片名（去扩展名）。仅 layout == Flat 时使用。
    pub flat_pool: BTreeSet<String>,
}

impl FaceMapper {
    /// 扫描头像包并建立索引（对等 v1 __init__）。扫描与分类统一交给 facepack::scan_pack。
    pub fn new(img_dir: &str) -> Result<Self, String> {
        Self::from_scan(img_dir, scan_pack(img_dir)?)
    }

    /// 用现成的扫描结果构造（「扫描头像」命令的缓存复用入口）。
    pub fn from_scan(img_dir: &str, scan: PackScan) -> Result<Self, String> {
        Ok(Self {
            img_dir: img_dir.to_string(),
            faces_map: scan.faces_map,
            flat_pool: scan.root_images,
            layout: scan.layout,
        })
    }

    /// 三模式映射总入口；返回 None 表示被取消（对等旧版 generate_mapping）。
    /// cancel 是跨线程的「取消标志」，处理大名单时逐项检查，用户点取消能及时停下。
    pub fn generate_mapping(
        &mut self,
        rtf_data: &[PlayerRecord],
        mode: Mode,
        duplicates: bool,      // 是否允许图片复用
        cancel: &AtomicBool,
        eth_cfg: &BTreeMap<String, String>, // 国籍→种族目录表
    ) -> Option<Vec<MappingEntry>> {
        // 开始前先看一眼是否已取消（Ordering::Relaxed 是最宽松的内存顺序，这里只需原子性）。
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        // Preserve/Overwrite 需要现有 config.xml 作为基础；Generate 从零开始，用空表。
        let xml_data = match mode {
            Mode::Preserve | Mode::Overwrite => {
                xml_parser::parse_xml(&Path::new(&self.img_dir).join("config.xml"))
                    .unwrap_or_default() // 解析失败（如没文件）就用空字典
            }
            Mode::Generate => HashMap::new(),
        };
        // Preserve + 禁重复：从各池剔除 XML 已用图（避免新球员抽到已被占用的图）。
        if !duplicates && mode == Mode::Preserve {
            let used: BTreeSet<String> = xml_data.values().map(|e| e.image.clone()).collect();
            for pool in self.faces_map.values_mut() {
                // difference 取「在池中但不在 used 中」的部分，即剔除已用图。
                *pool = pool.difference(&used).cloned().collect();
            }
            // 平铺池同理（标准一：旧 config.xml 的图名同样可能占用根池）。
            if self.layout.is_flat() {
                self.flat_pool = self.flat_pool.difference(&used).cloned().collect();
            }
        }
        // 按模式分发到具体处理函数。
        match mode {
            Mode::Preserve => self.process_preserve(rtf_data, &xml_data, duplicates, cancel, eth_cfg),
            Mode::Overwrite => self.process_overwrite(rtf_data, &xml_data, duplicates, cancel, eth_cfg),
            Mode::Generate => self.process_generate(rtf_data, duplicates, cancel, eth_cfg),
        }
    }

    /// Preserve：跳过 XML 已存在的 UID，只给新球员建映射，旧映射原样保留。
    fn process_preserve(
        &mut self,
        rtf_data: &[PlayerRecord],
        xml_data: &HashMap<String, MappingEntry>,
        duplicates: bool,
        cancel: &AtomicBool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<Vec<MappingEntry>> {
        let mut mapping = Vec::new();
        // filter 只留下 XML 中还没有的 UID（即新球员）。
        for p in rtf_data.iter().filter(|p| !xml_data.contains_key(&p.uid)) {
            if cancel.load(Ordering::Relaxed) {
                return None; // 逐项检查取消，及时中断
            }
            if let Some(m) = self.build_player_mapping(p, duplicates, eth_cfg) {
                mapping.push(m);
            }
        }
        // 原样保留 XML 中所有旧映射。
        mapping.extend(xml_data.values().cloned());
        Some(mapping)
    }

    /// Overwrite：名单内球员全部重抽，名单外的旧映射保留。
    fn process_overwrite(
        &mut self,
        rtf_data: &[PlayerRecord],
        xml_data: &HashMap<String, MappingEntry>,
        duplicates: bool,
        cancel: &AtomicBool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<Vec<MappingEntry>> {
        let mut rest = xml_data.clone(); // 先把旧映射全部拷贝一份，后续逐个剔除名单内的
        let mut mapping = Vec::new();
        for p in rtf_data {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(m) = self.build_player_mapping(p, duplicates, eth_cfg) {
                mapping.push(m);
                rest.remove(&p.uid); // 名单内球员全部重抽（从 rest 中移除旧的）
            }
        }
        mapping.extend(rest.into_values()); // 剩下的（名单外）保留
        Some(mapping)
    }

    /// Generate：从零为名单里每个球员生成映射，不管现有 config.xml。
    fn process_generate(
        &mut self,
        rtf_data: &[PlayerRecord],
        duplicates: bool,
        cancel: &AtomicBool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<Vec<MappingEntry>> {
        let mut mapping = Vec::new();
        for p in rtf_data {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(m) = self.build_player_mapping(p, duplicates, eth_cfg) {
                mapping.push(m);
            }
        }
        Some(mapping)
    }

    /// 单球员映射：双国籍查表 → 种族码修正 → 校验在包内 → 选图。
    /// 【流程】这是把一个球员变成一条映射的核心步骤，返回 None 表示这个球员无法映射（跳过）。
    fn build_player_mapping(
        &mut self,
        p: &PlayerRecord,
        duplicates: bool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<MappingEntry> {
        // 标准一平铺：忽略 RTF 国籍/种族码，从单一根池完全随机抽图。
        // （早返回放在最前，Standard2/Empty/Unknown 一律走下方原有 faces_map 通用路径。）
        if self.layout.is_flat() {
            let image = self.pick_from_flat(duplicates)?;
            return Some(MappingEntry { uid: p.uid.clone(), ethnicity: String::new(), image });
        }
        // e2 = 第二国籍对应的种族目录（没有第二国籍则为 None）。
        let e2 = if p.sec_nat.is_empty() {
            None
        } else {
            eth_cfg.get(&p.sec_nat).map(|s| s.as_str())
        };
        // e1 = 主国籍对应的种族目录；? 表示主国籍查不到就放弃这个球员。
        let e1 = eth_cfg.get(&p.primary_nat)?;
        // 根据种族码 + 两个候选种族目录，修正出最终种族目录。
        let ethnicity = Self::correct_ethnic(&p.ethnicity_code, e1, e2);
        if !self.faces_map.contains_key(&ethnicity) {
            return None; // 头像包缺该种族目录（前置校验已保证存在）
        }
        // 确定从哪些目录（图片池）里选图，然后选一张。
        let pools = self.image_pool_dirs(&ethnicity, &p.primary_nat);
        let image = self.pick_image_from_pools(&pools, duplicates)?;
        Some(MappingEntry { uid: p.uid.clone(), ethnicity, image })
    }

    /// 池优先级：国籍同名目录 > 种族目录 > 两者都不在包内时用全部目录。
    /// 【含义】有些头像包会按国籍建目录（如 ESP），优先用；否则用种族目录；都没有则退而用全部。
    fn image_pool_dirs(&self, ethnicity: &str, nationality: &str) -> Vec<String> {
        let mut pools = Vec::new();
        // nat_in / eth_in 判断对应目录是否存在且非空。
        let nat_in = self.faces_map.get(nationality).map(|s| !s.is_empty()).unwrap_or(false);
        let eth_in = self.faces_map.get(ethnicity).map(|s| !s.is_empty()).unwrap_or(false);
        if nat_in {
            pools.push(nationality.to_string()); // 国籍目录优先
        }
        if eth_in {
            pools.push(ethnicity.to_string());   // 其次种族目录
        }
        // 若国籍和种族目录都不在包内，则用包里所有目录兑底（总比什么都不选强）。
        if !self.faces_map.contains_key(nationality) && !self.faces_map.contains_key(ethnicity) {
            pools.extend(self.faces_map.keys().cloned());
        }
        pools
    }

    /// 标准一平铺选图：从根池随机取一张；禁重复时移除所选（与 pick_image_from_pools 同款可复现取法）。
    fn pick_from_flat(&mut self, duplicates: bool) -> Option<String> {
        if self.flat_pool.is_empty() {
            return None;
        }
        let idx = rand::rng().random_range(0..self.flat_pool.len());
        let choice = self.flat_pool.iter().nth(idx).cloned()?;
        if !duplicates {
            self.flat_pool.remove(&choice); // 禁重复：选过的图不再出现
        }
        Some(choice)
    }

    /// 依序取第一个非空池随机选图；禁重复时从池中移除（对等旧版 pick_image_from_pools）。
    /// &mut self：因为禁重复时需要修改池（移除已选图）。
    pub fn pick_image_from_pools(&mut self, pools: &[String], duplicates: bool) -> Option<String> {
        for dir in pools {
            // get_mut 拿到可修改的池；let ... else 是「拿不到就 continue」的简洁写法。
            let Some(pool) = self.faces_map.get_mut(dir) else { continue };
            if pool.is_empty() {
                continue; // 空池跳过，试下一个
            }
            // 生成随机下标，从有序集合里取第 idx 个元素（BTreeSet 不支持直接下标，用 nth）。
            let mut rng = rand::rng();
            let idx = rng.random_range(0..pool.len());
            if let Some(choice) = pool.iter().nth(idx).cloned() {
                if !duplicates {
                    pool.remove(&choice); // 禁重复：选过的图从池里删掉，下次不会再选到
                }
                return Some(choice);
            }
        }
        None // 所有池都空，无图可选
    }

    /// FM 种族码（0-10）在两个候选种族间修正归类（对等旧版 correct_ethnic，逐分支一致）。
    /// 【为何需要】FM 的种族码比较粗糙，需要结合国籍把它修正到更准确的种族目录。
    /// e1=主国籍种族，e2=第二国籍种族（可选）。hit 是个小闭包，判断某个种族名是否命中两个候选之一。
    pub fn correct_ethnic(code: &str, e1: &str, e2: Option<&str>) -> String {
        let hit = |name: &str| e1 == name || e2 == Some(name);
        match code {
            "0" => {
                // 优先 Scandinavian，其次 Caucasian，都不命中则兑底 Central European。
                if hit("Scandinavian") { "Scandinavian".into() }
                else if hit("Caucasian") { "Caucasian".into() }
                else { "Central European".into() }
            }
            "1" => ["Caucasian", "EECA", "Italmed", "SAMed", "SpanMed", "YugoGreek", "South American"]
                .iter()
                .find(|n| hit(n))            // 找第一个命中的候选
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Central European".into()), // 都不命中则兑底
            "2" => if hit("MESA") { "MESA".into() } else { "MENA".into() },
            "3" | "9" => if hit("South American") { "South American".into() } else { "African".into() },
            "6" => if hit("SAMed") { "SAMed".into() } else { "African".into() },
            "8" => if hit("Seasian") { "Seasian".into() } else { "African".into() },
            "4" => "MESA".into(),            // 固定值，不看国籍
            "5" => "Seasian".into(),
            "7" => "South American".into(),
            "10" => {
                // 链式兑底：MESA → Seasian → South American → Asian。
                if hit("MESA") { "MESA".into() }
                else if hit("Seasian") { "Seasian".into() }
                else if hit("South American") { "South American".into() }
                else { "Asian".into() }
            }
            _ => e1.to_string(), // 解析层已保证 0-10，防御性兑底
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::ETH_CFG;

    fn facepack(dir: &tempfile::TempDir) -> String {
        for eth in ["SpanMed", "Caucasian", "Central European", "MENA", "MESA", "Seasian",
                    "South American", "Asian", "African", "SAMed", "EECA", "Italmed",
                    "YugoGreek", "Scandinavian"] {
            let d = dir.path().join(eth);
            std::fs::create_dir_all(&d).unwrap();
            for i in 0..3 {
                std::fs::write(d.join(format!("e{i}.png")), b"png").unwrap();
            }
        }
        dir.path().to_string_lossy().to_string()
    }

    fn player(uid: &str, nat: &str, code: &str) -> PlayerRecord {
        PlayerRecord {
            uid: uid.into(), primary_nat: nat.into(), sec_nat: String::new(),
            name: "T".into(), hair_length: "1".into(), hair_color: "1".into(),
            ethnicity_code: code.into(), detailed: None,
        }
    }

    #[test]
    fn correct_ethnic_all_codes() {
        // 0：优先 Scandinavian > Caucasian > 兜底
        assert_eq!(FaceMapper::correct_ethnic("0", "African", Some("Scandinavian")), "Scandinavian");
        assert_eq!(FaceMapper::correct_ethnic("0", "African", Some("Caucasian")), "Caucasian");
        assert_eq!(FaceMapper::correct_ethnic("0", "African", None), "Central European");
        // 1：命中列表则用，否则兜底
        assert_eq!(FaceMapper::correct_ethnic("1", "EECA", None), "EECA");
        assert_eq!(FaceMapper::correct_ethnic("1", "MENA", None), "Central European");
        // 2：MESA 命中否则 MENA
        assert_eq!(FaceMapper::correct_ethnic("2", "MESA", None), "MESA");
        assert_eq!(FaceMapper::correct_ethnic("2", "MENA", None), "MENA");
        // 3/6/8/9：仅对应码命中，否则 African
        assert_eq!(FaceMapper::correct_ethnic("6", "SAMed", None), "SAMed");
        assert_eq!(FaceMapper::correct_ethnic("8", "Seasian", None), "Seasian");
        assert_eq!(FaceMapper::correct_ethnic("3", "South American", None), "South American");
        assert_eq!(FaceMapper::correct_ethnic("9", "South American", None), "South American");
        assert_eq!(FaceMapper::correct_ethnic("3", "SAMed", None), "African");
        // 固定值
        assert_eq!(FaceMapper::correct_ethnic("4", "African", None), "MESA");
        assert_eq!(FaceMapper::correct_ethnic("5", "African", None), "Seasian");
        assert_eq!(FaceMapper::correct_ethnic("7", "African", None), "South American");
        // 10：链式兜底到 Asian
        assert_eq!(FaceMapper::correct_ethnic("10", "MESA", None), "MESA");
        assert_eq!(FaceMapper::correct_ethnic("10", "African", Some("Seasian")), "Seasian");
        assert_eq!(FaceMapper::correct_ethnic("10", "Asian", None), "Asian");
    }

    #[test]
    fn generate_mode_maps_into_ethnic_dir() {
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        let mut mapper = FaceMapper::new(&img).unwrap();
        let data = vec![
            player("r-2001", "ESP", "1"),  // ESP → SpanMed
            player("r-2002", "USA", "0"),  // USA → Caucasian
        ];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, true, &cancel, &ETH_CFG)
            .unwrap();
        assert_eq!(mapping.len(), 2);
        assert_eq!(mapping[0].ethnicity, "SpanMed");
        assert_eq!(mapping[1].ethnicity, "Caucasian");
        for m in &mapping {
            let pool = &mapper.faces_map[&m.ethnicity];
            assert!(pool.contains(&m.image));
        }
    }

    #[test]
    fn no_duplicates_exhausts_pool() {
        let dir = tempfile::tempdir().unwrap();
        // 国籍目录恰好 3 张图（国籍目录优先于种族目录）。
        // 种族目录（ESP 经 eth_cfg 归类为 SpanMed）必须存在：
        // v1 行为是种族目录不在包内时整条跳过该球员（对等 _build_player_mapping）
        let nat_dir = dir.path().join("ESP");
        std::fs::create_dir_all(&nat_dir).unwrap();
        for i in 0..3 {
            std::fs::write(nat_dir.join(format!("e{i}.png")), b"png").unwrap();
        }
        let eth_dir = dir.path().join("SpanMed");
        std::fs::create_dir_all(&eth_dir).unwrap();
        for i in 0..3 {
            std::fs::write(eth_dir.join(format!("s{i}.png")), b"png").unwrap();
        }
        let img = dir.path().to_string_lossy().to_string();
        let mut mapper = FaceMapper::new(&img).unwrap();
        let data: Vec<PlayerRecord> = (0..3).map(|i| player(&format!("r-30{i}"), "ESP", "1")).collect();
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, false, &cancel, &ETH_CFG)
            .unwrap();
        assert_eq!(mapping.len(), 3);
        let images: BTreeSet<_> = mapping.iter().map(|m| m.image.clone()).collect();
        assert_eq!(images.len(), 3); // 无重复
    }

    #[test]
    fn preserve_keeps_xml_and_skips_existing_uids() {
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        // 现有 config.xml 已有 r-2001
        crate::xml_parser::write_xml(
            &[MappingEntry { uid: "r-2001".into(), ethnicity: "African".into(), image: "old".into() }],
            Path::new(&img), false,
        ).unwrap();
        let mut mapper = FaceMapper::new(&img).unwrap();
        let data = vec![player("r-2001", "ESP", "1"), player("r-2002", "ESP", "1")];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Preserve, true, &cancel, &ETH_CFG)
            .unwrap();
        // r-2001 保留旧映射，r-2002 新建
        let e = mapping.iter().find(|m| m.uid == "r-2001").unwrap();
        assert_eq!((e.ethnicity.as_str(), e.image.as_str()), ("African", "old"));
        assert!(mapping.iter().any(|m| m.uid == "r-2002"));
    }

    #[test]
    fn cancel_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        let mut mapper = FaceMapper::new(&img).unwrap();
        let cancel = AtomicBool::new(true); // 已取消
        let data = vec![player("r-2001", "ESP", "1")];
        assert!(mapper.generate_mapping(&data, Mode::Generate, true, &cancel, &ETH_CFG).is_none());
    }

    /// 标准一平铺包 fixture：根目录 5 张普通图 + 1 张点号文件名（锁定 file_stem 词干规则）。
    fn flatpack(dir: &tempfile::TempDir) -> String {
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("face{i}.png")), b"png").unwrap();
        }
        std::fs::write(dir.path().join("x.1.png"), b"png").unwrap();
        dir.path().to_string_lossy().to_string()
    }

    #[test]
    fn standard2_pack_classified_and_maps_by_ethnic() {
        // facepack() 恰为 14 个合法种族目录 → 必须判 Standard2 且走原有种族匹配路径。
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        let mut mapper = FaceMapper::new(&img).unwrap();
        assert_eq!(mapper.layout, PackLayout::Standard2 { complete: true });
        let data = vec![player("r-2001", "ESP", "1"), player("r-2002", "USA", "0")];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, true, &cancel, &ETH_CFG)
            .unwrap();
        assert_eq!(mapping.len(), 2);
        assert_eq!(mapping[0].ethnicity, "SpanMed"); // 按 correct_ethnic 进种族目录
        assert_eq!(mapping[1].ethnicity, "Caucasian");
        for m in &mapping {
            assert!(mapper.faces_map[&m.ethnicity].contains(&m.image));
        }
    }

    #[test]
    fn flat_generate_maps_everyone_randomly() {
        let dir = tempfile::tempdir().unwrap();
        let img = flatpack(&dir);
        let mut mapper = FaceMapper::new(&img).unwrap();
        assert_eq!(mapper.layout, PackLayout::Flat);
        // ZZZ 不在 ETH_CFG：平铺必须忽略国籍/种族码，未知国籍球员同样被映射。
        let data = vec![player("r-3001", "ZZZ", "1"), player("r-3002", "ESP", "9")];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, true, &cancel, &ETH_CFG)
            .unwrap();
        assert_eq!(mapping.len(), 2);
        for m in &mapping {
            assert_eq!(m.ethnicity, "");
            assert!(mapper.flat_pool.contains(&m.image)); // 允许复用时池不变
            // 每条映射的图名都能在包根命中真实文件（file_stem 贯通写入/解析/定位）。
            assert!(xml_parser::resolve_image_file(Path::new(&img), "", &m.image).is_some());
        }
    }

    #[test]
    fn flat_no_duplicates_exhausts_pool() {
        let dir = tempfile::tempdir().unwrap();
        let img = flatpack(&dir); // 池共 6 张
        let mut mapper = FaceMapper::new(&img).unwrap();
        let data: Vec<PlayerRecord> = (0..8).map(|i| player(&format!("r-40{i}"), "ESP", "1")).collect();
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, false, &cancel, &ETH_CFG)
            .unwrap();
        assert_eq!(mapping.len(), 6); // 池耗尽后其余球员被跳过
        let images: BTreeSet<_> = mapping.iter().map(|m| m.image.clone()).collect();
        assert_eq!(images.len(), 6); // 全不重复
    }

    #[test]
    fn flat_preserve_keeps_existing_flat_entries() {
        let dir = tempfile::tempdir().unwrap();
        let img = flatpack(&dir);
        // 现有 config.xml 已有平铺条目 from="face0"（解析回 ("", "face0")）。
        crate::xml_parser::write_xml(
            &[MappingEntry { uid: "r-3001".into(), ethnicity: "".into(), image: "face0".into() }],
            Path::new(&img), false,
        ).unwrap();
        let mut mapper = FaceMapper::new(&img).unwrap();
        let data = vec![player("r-3001", "ESP", "1"), player("r-3002", "ESP", "1")];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Preserve, true, &cancel, &ETH_CFG)
            .unwrap();
        // r-3001 保留旧平铺映射；r-3002 新建且种族为空。
        let e = mapping.iter().find(|m| m.uid == "r-3001").unwrap();
        assert_eq!((e.ethnicity.as_str(), e.image.as_str()), ("", "face0"));
        assert!(mapping.iter().any(|m| m.uid == "r-3002" && m.ethnicity.is_empty()));
    }
}
