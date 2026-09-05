//! 球员 → 头像映射（自 core/FaceMapper.py 移植，含 0-10 种族码修正规则）
use crate::rtf_parser::PlayerRecord;
use crate::xml_parser::{self, MappingEntry};
use rand::Rng;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Preserve,
    Overwrite,
    Generate,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "Preserve" => Ok(Mode::Preserve),
            "Overwrite" => Ok(Mode::Overwrite),
            "Generate" => Ok(Mode::Generate),
            _ => Err(format!("未知模式: {s}")),
        }
    }
}

pub struct FaceMapper {
    pub img_dir: String,
    /// 目录名 → 图片名集合（无扩展名）；BTreeMap/BTreeSet 保证行为可复现
    pub faces_map: BTreeMap<String, BTreeSet<String>>,
}

impl FaceMapper {
    /// 扫描头像包一级子目录及图片（对等 v1 __init__：split('.')[0] 取首段）
    pub fn new(img_dir: &str) -> Result<Self, String> {
        let root = Path::new(img_dir);
        let mut faces_map = BTreeMap::new();
        for entry in std::fs::read_dir(root).map_err(|e| format!("读取头像包目录失败: {e}"))? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.path().is_dir() {
                continue;
            }
            let mut imgs = BTreeSet::new();
            if let Ok(files) = std::fs::read_dir(entry.path()) {
                for f in files.flatten() {
                    if f.path().is_file() {
                        let name = f.file_name().to_string_lossy().to_string();
                        let stem = name.split('.').next().unwrap_or("").to_string();
                        if !stem.is_empty() {
                            imgs.insert(stem);
                        }
                    }
                }
            }
            faces_map.insert(entry.file_name().to_string_lossy().to_string(), imgs);
        }
        Ok(Self { img_dir: img_dir.to_string(), faces_map })
    }

    /// 三模式映射总入口；返回 None 表示被取消（对等 generate_mapping）
    pub fn generate_mapping(
        &mut self,
        rtf_data: &[PlayerRecord],
        mode: Mode,
        duplicates: bool,
        cancel: &AtomicBool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<Vec<MappingEntry>> {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let xml_data = match mode {
            Mode::Preserve | Mode::Overwrite => {
                xml_parser::parse_xml(&Path::new(&self.img_dir).join("config.xml"))
                    .unwrap_or_default()
            }
            Mode::Generate => HashMap::new(),
        };
        // Preserve + 禁重复：从各池剔除 XML 已用图
        if !duplicates && mode == Mode::Preserve {
            let used: BTreeSet<String> = xml_data.values().map(|e| e.image.clone()).collect();
            for pool in self.faces_map.values_mut() {
                *pool = pool.difference(&used).cloned().collect();
            }
        }
        match mode {
            Mode::Preserve => self.process_preserve(rtf_data, &xml_data, duplicates, cancel, eth_cfg),
            Mode::Overwrite => self.process_overwrite(rtf_data, &xml_data, duplicates, cancel, eth_cfg),
            Mode::Generate => self.process_generate(rtf_data, duplicates, cancel, eth_cfg),
        }
    }

    fn process_preserve(
        &mut self,
        rtf_data: &[PlayerRecord],
        xml_data: &HashMap<String, MappingEntry>,
        duplicates: bool,
        cancel: &AtomicBool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<Vec<MappingEntry>> {
        let mut mapping = Vec::new();
        // 跳过 XML 已存在的 UID，只给新球员建映射
        for p in rtf_data.iter().filter(|p| !xml_data.contains_key(&p.uid)) {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(m) = self.build_player_mapping(p, duplicates, eth_cfg) {
                mapping.push(m);
            }
        }
        // 原样保留 XML 中所有旧映射
        mapping.extend(xml_data.values().cloned());
        Some(mapping)
    }

    fn process_overwrite(
        &mut self,
        rtf_data: &[PlayerRecord],
        xml_data: &HashMap<String, MappingEntry>,
        duplicates: bool,
        cancel: &AtomicBool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<Vec<MappingEntry>> {
        let mut rest = xml_data.clone();
        let mut mapping = Vec::new();
        for p in rtf_data {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(m) = self.build_player_mapping(p, duplicates, eth_cfg) {
                mapping.push(m);
                rest.remove(&p.uid); // 名单内球员全部重抽
            }
        }
        mapping.extend(rest.into_values()); // 名单外保留
        Some(mapping)
    }

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

    /// 单球员映射：双国籍查表 → 种族码修正 → 校验在包内 → 选图
    fn build_player_mapping(
        &mut self,
        p: &PlayerRecord,
        duplicates: bool,
        eth_cfg: &BTreeMap<String, String>,
    ) -> Option<MappingEntry> {
        let e2 = if p.sec_nat.is_empty() {
            None
        } else {
            eth_cfg.get(&p.sec_nat).map(|s| s.as_str())
        };
        let e1 = eth_cfg.get(&p.primary_nat)?;
        let ethnicity = Self::correct_ethnic(&p.ethnicity_code, e1, e2);
        if !self.faces_map.contains_key(&ethnicity) {
            return None; // 头像包缺该种族目录（前置校验已保证存在）
        }
        let pools = self.image_pool_dirs(&ethnicity, &p.primary_nat);
        let image = self.pick_image_from_pools(&pools, duplicates)?;
        Some(MappingEntry { uid: p.uid.clone(), ethnicity, image })
    }

    /// 池优先级：国籍同名目录 > 种族目录 > 两者都不在包内时用全部目录
    fn image_pool_dirs(&self, ethnicity: &str, nationality: &str) -> Vec<String> {
        let mut pools = Vec::new();
        let nat_in = self.faces_map.get(nationality).map(|s| !s.is_empty()).unwrap_or(false);
        let eth_in = self.faces_map.get(ethnicity).map(|s| !s.is_empty()).unwrap_or(false);
        if nat_in {
            pools.push(nationality.to_string());
        }
        if eth_in {
            pools.push(ethnicity.to_string());
        }
        if !self.faces_map.contains_key(nationality) && !self.faces_map.contains_key(ethnicity) {
            pools.extend(self.faces_map.keys().cloned());
        }
        pools
    }

    /// 依序取第一个非空池随机选图；禁重复时从池中移除（对等 pick_image_from_pools）
    pub fn pick_image_from_pools(&mut self, pools: &[String], duplicates: bool) -> Option<String> {
        for dir in pools {
            let Some(pool) = self.faces_map.get_mut(dir) else { continue };
            if pool.is_empty() {
                continue;
            }
            let mut rng = rand::rng();
            let idx = rng.random_range(0..pool.len());
            if let Some(choice) = pool.iter().nth(idx).cloned() {
                if !duplicates {
                    pool.remove(&choice);
                }
                return Some(choice);
            }
        }
        None
    }

    /// FM 种族码（0-10）在两个候选种族间修正归类（对等 correct_ethnic，逐分支一致）
    pub fn correct_ethnic(code: &str, e1: &str, e2: Option<&str>) -> String {
        let hit = |name: &str| e1 == name || e2 == Some(name);
        match code {
            "0" => {
                if hit("Scandinavian") { "Scandinavian".into() }
                else if hit("Caucasian") { "Caucasian".into() }
                else { "Central European".into() }
            }
            "1" => ["Caucasian", "EECA", "Italmed", "SAMed", "SpanMed", "YugoGreek", "South American"]
                .iter()
                .find(|n| hit(n))
                .map(|n| n.to_string())
                .unwrap_or_else(|| "Central European".into()),
            "2" => if hit("MESA") { "MESA".into() } else { "MENA".into() },
            "3" | "9" => if hit("South American") { "South American".into() } else { "African".into() },
            "6" => if hit("SAMed") { "SAMed".into() } else { "African".into() },
            "8" => if hit("Seasian") { "Seasian".into() } else { "African".into() },
            "4" => "MESA".into(),
            "5" => "Seasian".into(),
            "7" => "South American".into(),
            "10" => {
                if hit("MESA") { "MESA".into() }
                else if hit("Seasian") { "Seasian".into() }
                else if hit("South American") { "South American".into() }
                else { "Asian".into() }
            }
            _ => e1.to_string(), // 解析层已保证 0-10，防御性兜底
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
}
