//! 【新手导读】球员 → 头像映射（自旧版 core/FaceMapper.py 移植，含 0-10 种族码修正规则）。
//! 这是整个应用的「大脑」：根据每个球员的国籍 + 种族码，算出该用头像包里哪个种族目录，
//! 再从目录里选一张图片，最终得到一条「球员 UID → 图片」的映射。
use crate::facepack::{self, scan_pack, PackLayout, PackScan}; // 标准识别/扫描的唯一来源（facepack.rs）
use crate::templates::{self, PickRule, RuleConfig};           // 抽图模型（一个标准一套）+ 规则配置聚合
use crate::rtf_parser::PlayerRecord;
use crate::xml_parser::{self, MappingEntry};
use std::collections::{BTreeSet, HashMap}; // BTreeSet = 有序集合，保证行为可复现
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

/// 头像映射器：持有扫描判定的布局与对应标准的抽图模型，负责按模式编排并组装映射条目。
pub struct FaceMapper {
    pub img_dir: String,
    /// 布局判定结果（facepack::classify 按 templates 的 FolderList 判定）；replace_flow 据此拦截未实现标准。
    pub layout: PackLayout,
    /// 抽图模型（标准一平铺 / 标准二种族匹配 / 标准三细分目录检索 / 退化布局递归全池），池的所有权在模型内。
    rule: Box<dyn PickRule>,
}

impl FaceMapper {
    /// 扫描头像包并建立索引。扫描交给 facepack::scan_pack，标准判定与模型选择交给 templates。
    pub fn new(img_dir: &str, config: &RuleConfig) -> Result<Self, String> {
        let scan = scan_pack(img_dir)?;
        Self::from_scan(img_dir, &scan, config)
    }

    /// 用现成的扫描结果构造（「扫描头像」命令的缓存复用入口）。
    /// 【借用而非所有权】内部只读 scan（classify 与 for_layout 都吃 &PackScan），
    /// 大包索引（数万图片名）一份克隆就是几十 MB 白复制，不强制调用方克隆。
    pub fn from_scan(img_dir: &str, scan: &PackScan, config: &RuleConfig) -> Result<Self, String> {
        // classify 输入剔除垃圾目录（__MACOSX 等不参与判定）。
        let subdirs: BTreeSet<String> = scan.faces_map.keys()
            .filter(|n| !facepack::is_junk_dir(n))
            .cloned()
            .collect();
        let layout = facepack::classify(&subdirs, !scan.root_images.is_empty(), &config.folder_lists());
        Ok(Self {
            img_dir: img_dir.to_string(),
            layout,
            rule: templates::for_layout(layout, scan, config),
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
        // Preserve + 禁重复：把现有 config.xml 已占用的图片从池中剔除（避免新球员抽到已被占用的图）。
        // used 用 from 全路径做键（种族目录非空 → "种族/图名"，平铺 → "图名"），由规则内部精确比对。
        if !duplicates && mode == Mode::Preserve {
            let used: BTreeSet<String> = xml_data.values()
                .map(|e| xml_parser::record_from(&e.ethnicity, &e.image))
                .collect();
            self.rule.exclude_used(&used);
        }
        // 按模式分发到具体处理函数。
        match mode {
            Mode::Preserve => self.process_preserve(rtf_data, &xml_data, duplicates, cancel),
            Mode::Overwrite => self.process_overwrite(rtf_data, &xml_data, duplicates, cancel),
            Mode::Generate => self.process_generate(rtf_data, duplicates, cancel),
        }
    }

    /// Preserve：跳过 XML 已存在的 UID，只给新球员建映射，旧映射原样保留。
    fn process_preserve(
        &mut self,
        rtf_data: &[PlayerRecord],
        xml_data: &HashMap<String, MappingEntry>,
        duplicates: bool,
        cancel: &AtomicBool,
    ) -> Option<Vec<MappingEntry>> {
        let mut mapping = Vec::new();
        // filter 只留下 XML 中还没有的 UID（即新球员）。
        for p in rtf_data.iter().filter(|p| !xml_data.contains_key(&p.uid)) {
            if cancel.load(Ordering::Relaxed) {
                return None; // 逐项检查取消，及时中断
            }
            if let Some(m) = self.build_player_mapping(p, duplicates) {
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
    ) -> Option<Vec<MappingEntry>> {
        let mut rest = xml_data.clone(); // 先把旧映射全部拷贝一份，后续逐个剔除名单内的
        let mut mapping = Vec::new();
        for p in rtf_data {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(m) = self.build_player_mapping(p, duplicates) {
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
    ) -> Option<Vec<MappingEntry>> {
        let mut mapping = Vec::new();
        for p in rtf_data {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(m) = self.build_player_mapping(p, duplicates) {
                mapping.push(m);
            }
        }
        Some(mapping)
    }

    /// 单球员映射：按扫描出的标准规则抽图（规则选择见 templates::for_layout）。
    /// 返回 None 表示该球员无法映射（跳过）。
    fn build_player_mapping(
        &mut self,
        p: &PlayerRecord,
        duplicates: bool,
    ) -> Option<MappingEntry> {
        // PickedImage.dir = 图片相对头像包根的目录（"" = 根，可多级 "a/b"），原样写入 ethnicity 字段；
        // config.xml 的 from 由 write_xml 经 record_from 拼成统一相对路径。
        let img = self.rule.pick(p, duplicates)?;
        Some(MappingEntry { uid: p.uid.clone(), ethnicity: img.dir, image: img.stem })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::RuleConfig;

    /// 测试用规则配置：内置出厂默认。
    fn config() -> RuleConfig {
        RuleConfig::builtin()
    }

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
    fn generate_mode_maps_into_ethnic_dir() {
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        let mut mapper = FaceMapper::new(&img, &config()).unwrap();
        let data = vec![
            player("r-2001", "ESP", "1"),  // ESP → SpanMed
            player("r-2002", "USA", "0"),  // USA → Caucasian
        ];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, true, &cancel)
            .unwrap();
        assert_eq!(mapping.len(), 2);
        assert_eq!(mapping[0].ethnicity, "SpanMed");
        assert_eq!(mapping[1].ethnicity, "Caucasian");
        // 每条映射的图都能在对应种族目录下定位到真实文件。
        for m in &mapping {
            assert!(xml_parser::resolve_image_file(Path::new(&img), &m.ethnicity, &m.image).is_some());
        }
    }

    #[test]
    fn no_duplicates_exhausts_pool() {
        let dir = tempfile::tempdir().unwrap();
        // {ESP, SpanMed} 混合目录包：ESP 不在标准二 FolderList → 判 Unknown → 递归全池规则。
        // 禁重复 + 6 张图、3 名球员：断言 3 条互不重复的映射（无重复语义与标准无关）。
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
        let mut mapper = FaceMapper::new(&img, &config()).unwrap();
        let data: Vec<PlayerRecord> = (0..3).map(|i| player(&format!("r-30{i}"), "ESP", "1")).collect();
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, false, &cancel)
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
        let mut mapper = FaceMapper::new(&img, &config()).unwrap();
        let data = vec![player("r-2001", "ESP", "1"), player("r-2002", "ESP", "1")];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Preserve, true, &cancel)
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
        let mut mapper = FaceMapper::new(&img, &config()).unwrap();
        let cancel = AtomicBool::new(true); // 已取消
        let data = vec![player("r-2001", "ESP", "1")];
        assert!(mapper.generate_mapping(&data, Mode::Generate, true, &cancel).is_none());
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
        // facepack() 恰为 14 个合法种族目录 → 按内置 FolderList 判标准二且走种族匹配规则。
        let dir = tempfile::tempdir().unwrap();
        let img = facepack(&dir);
        let mut mapper = FaceMapper::new(&img, &config()).unwrap();
        assert_eq!(mapper.layout, PackLayout::Standard { standard: 2, complete: true });
        let data = vec![player("r-2001", "ESP", "1"), player("r-2002", "USA", "0")];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, true, &cancel)
            .unwrap();
        assert_eq!(mapping.len(), 2);
        assert_eq!(mapping[0].ethnicity, "SpanMed"); // 按 correct_ethnic 进种族目录
        assert_eq!(mapping[1].ethnicity, "Caucasian");
        for m in &mapping {
            assert!(xml_parser::resolve_image_file(Path::new(&img), &m.ethnicity, &m.image).is_some());
        }
    }

    #[test]
    fn flat_generate_maps_everyone_randomly() {
        let dir = tempfile::tempdir().unwrap();
        let img = flatpack(&dir);
        let mut mapper = FaceMapper::new(&img, &config()).unwrap();
        assert_eq!(mapper.layout, PackLayout::Flat);
        // ZZZ 不在标准映射：平铺必须忽略国籍/种族码，未知国籍球员同样被映射。
        let data = vec![player("r-3001", "ZZZ", "1"), player("r-3002", "ESP", "9")];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, true, &cancel)
            .unwrap();
        assert_eq!(mapping.len(), 2);
        for m in &mapping {
            assert_eq!(m.ethnicity, "");
            // 每条映射的图名都能在包根命中真实文件（file_stem 贯通写入/解析/定位）。
            assert!(xml_parser::resolve_image_file(Path::new(&img), "", &m.image).is_some());
        }
    }

    #[test]
    fn flat_no_duplicates_exhausts_pool() {
        let dir = tempfile::tempdir().unwrap();
        let img = flatpack(&dir); // 池共 6 张
        let mut mapper = FaceMapper::new(&img, &config()).unwrap();
        let data: Vec<PlayerRecord> = (0..8).map(|i| player(&format!("r-40{i}"), "ESP", "1")).collect();
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, false, &cancel)
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
        let mut mapper = FaceMapper::new(&img, &config()).unwrap();
        let data = vec![player("r-3001", "ESP", "1"), player("r-3002", "ESP", "1")];
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Preserve, true, &cancel)
            .unwrap();
        // r-3001 保留旧平铺映射；r-3002 新建且种族为空。
        let e = mapping.iter().find(|m| m.uid == "r-3001").unwrap();
        assert_eq!((e.ethnicity.as_str(), e.image.as_str()), ("", "face0"));
        assert!(mapping.iter().any(|m| m.uid == "r-3002" && m.ethnicity.is_empty()));
    }

    #[test]
    fn unknown_layout_uses_recursive_pool() {
        // 无法识别的包（含孙目录）→ 递归全池全随机：子/孙目录图片都会被抽到，
        // 映射的 ethnicity 为相对目录路径（可多级），from/resolve 全程用相对路径口径。
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().to_string_lossy().to_string();
        std::fs::create_dir_all(dir.path().join("weirdA/sub")).unwrap();
        std::fs::write(dir.path().join("weirdA/i.png"), b"png").unwrap();
        std::fs::write(dir.path().join("weirdA/sub/j.png"), b"png").unwrap();
        std::fs::write(dir.path().join("root.png"), b"png").unwrap();
        let mut mapper = FaceMapper::new(&img, &config()).unwrap();
        assert_eq!(mapper.layout, PackLayout::Unknown);
        // 禁重复 + 池恰好 3 张、球员 3 名 → 必然抽完全部图片（确定性断言，不依赖随机）。
        let data: Vec<PlayerRecord> =
            (0..3).map(|i| player(&format!("r-50{i}"), "ZZZ", "1")).collect();
        let cancel = AtomicBool::new(false);
        let mapping = mapper
            .generate_mapping(&data, Mode::Generate, false, &cancel)
            .unwrap();
        assert_eq!(mapping.len(), 3);
        let froms: BTreeSet<String> = mapping
            .iter()
            .map(|m| xml_parser::record_from(&m.ethnicity, &m.image))
            .collect();
        // 根、子、孙三个层级的图片全部进入抽图结果。
        assert_eq!(froms, BTreeSet::from([
            "root".to_string(), "weirdA/i".to_string(), "weirdA/sub/j".to_string()
        ]));
        // 相对路径口径：resolve 用同一 ethnicity 字符串即可命中真实文件。
        for m in &mapping {
            assert!(xml_parser::resolve_image_file(Path::new(&img), &m.ethnicity, &m.image).is_some());
        }
    }
}
