//! 头像包「标准」的抽图模型总入口：**一个标准 = 一个自包含模块**。
//! 每个标准模块（standardN.rs）里放着它自己的三样东西：规则 json 的结构体、抽图算法、单元测试。
//! 标准之间互不 import —— 因为每套头像包的目录组织方式不同，抽图逻辑天然不同，
//! 强行共享只会让改一处影响另一处。本文件只放「跨标准无语义」的东西：
//!   1. `PickRule` trait 与 `PickedImage`：所有标准都遵守的接口与返回值；
//!   2. `for_layout`：唯一分发入口（按头像包扫描出的布局挑一套规则）；
//!   3. `RuleConfig`：各标准规则的聚合与加载（内置默认 + 用户可编辑 json 双层来源）；
//!   4. `filter_pools` / `pick_from_pools_flat`：两个纯机械工具，不含任何标准语义；
//!   5. `validate_registry`：标准之间的登记冲突守卫。
//!
//! 头像包布局判定在 `facepack_scanner::classify`（数据驱动，按各标准的 FolderList 推断），
//! 本模块不重复实现判定 —— 判定与分发各只有一处真源。
//!
//! 【扩展点】新增标准四/五/六：①`assets/standardN.json` ②`templates/standardN.rs`
//! （自带 schema + validate + 规则 + 单测，只 import 本文件的 trait 与两个机械工具）
//! ③`assets.rs` 加 `STANDARD{N}_RAW` ④本文件登记（RuleConfig 字段 / builtin / load /
//! ensure_user_files / folder_lists / rule_files / for_layout 分支 / IMPLEMENTED_STANDARDS）
//! ⑤前端 `api.ts` + `RuleEditor.svelte` + `text.ts` 各加一项。判定层零改动。
pub(crate) mod standard1;
pub(crate) mod standard2;
pub(crate) mod standard3;
pub(crate) mod unknown;

use crate::facepack_scanner::{PackLayout, PackScan};
use crate::json_store::{self, LoadError};
use crate::rtf_scanner::PlayerRecord;
use rand::Rng;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 当前已实现抽图规则的标准号；replace_flow 据此对清单外的标准拦截提示。
/// 标准一（平铺）与空包 / 无法识别的退化布局不经过这个拦截：它们不走 `PackLayout::Standard`。
pub const IMPLEMENTED_STANDARDS: [u8; 2] = [2, 3];

/// 一次抽图的结果：图片相对头像包根的目录路径（"" = 根，多级 "子目录/孙目录"）+ 文件词干。
/// 写 config.xml 时 dir + stem 经 record_from 拼成 from（统一相对路径口径）。
#[derive(Debug)]
pub struct PickedImage { pub dir: String, pub stem: String }

/// 抽图规则统一接口：每个标准一套实现。
/// 【为什么 pick 不带 config 参数】每套规则在构造时只拿自己那份配置，之后不再看别的标准；
/// 这样"某标准的抽图逻辑"完全封闭在它自己的文件里，读一个文件就能读完整套行为。
pub trait PickRule {
    /// Preserve+禁重复：把已有映射占用的图片从池中剔除。
    /// used 为「from 全路径」键集合（record_from 形式：种族目录非空 → "目录/词干"，平铺 → "词干"）。
    fn exclude_used(&mut self, used: &BTreeSet<String>);
    /// 为一名球员抽图；返回 None 表示该球员无法映射（跳过）。
    fn pick(&mut self, player: &PlayerRecord, duplicates: bool) -> Option<PickedImage>;
}

/// 全部标准规则的集合。字段名与标准号一一对应（standardN）。
#[derive(Debug, Clone)]
pub struct RuleConfig {
    pub standard2: standard2::Standard2Config,
    pub standard3: standard3::Standard3Config,
}

/// 一个可编辑规则文件的登记表项：命令层只查这张表，不再按标准号写 match。
/// 【为什么用函数指针而不是泛型/闭包】每项对应一个不同的 Config 类型，函数指针能把
/// 「解析 + 校验 + 写回」整套动作擦除到同一张表里，模板增加时 commands.rs 零改动。
pub struct RuleFile {
    /// 前端传的规则文件名（`"standardN"`）。
    pub name: &'static str,
    /// 该规则在用户数据目录下的路径。
    pub path: fn(&Path) -> PathBuf,
    /// 解析 + 结构校验 + 原子写回；失败时错误信息含文件路径（不静默回退）。
    pub save: fn(&Path, &str) -> Result<(), String>,
}

impl RuleConfig {
    /// 内置出厂默认（编译期内嵌 assets/standardN.json）。
    pub fn builtin() -> Self {
        RuleConfig {
            standard2: serde_json::from_str(crate::assets::STANDARD2_RAW)
                .expect("内置 standard2.json 格式错误"),
            standard3: serde_json::from_str(crate::assets::STANDARD3_RAW)
                .expect("内置 standard3.json 格式错误"),
        }
    }

    /// 所有参与目录判定的标准 (标准号, FolderList)，按标准号升序登记。
    /// 【这张表的顺序就是判定的唯一真源】`facepack_scanner::classify` 两轮都是先命中先赢，
    /// 因此任意两份清单不能互相包含（由 `validate_registry` 守卫）。
    pub fn folder_lists(&self) -> Vec<(u8, &Vec<String>)> {
        vec![(2, &self.standard2.folder_list), (3, &self.standard3.folder_list)]
    }

    /// 加载生效规则：用户文件存在则用之，缺失用内置默认，格式错误报错（含文件路径）。
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let mut cfg = Self::builtin();
        let dir = data_dir.join("rules");
        let s2 = dir.join("standard2.json");
        match json_store::load::<standard2::Standard2Config>(&s2) {
            Ok(user) => cfg.standard2 = user,
            Err(LoadError::Missing) => {} // 无用户文件 → 内置默认
            Err(LoadError::Corrupt(e)) => {
                return Err(format!("标准二规则文件无法解析：{}（{e}）", s2.display()))
            }
        }
        let s3 = dir.join("standard3.json");
        match json_store::load::<standard3::Standard3Config>(&s3) {
            Ok(user) => cfg.standard3 = user,
            Err(LoadError::Missing) => {}
            Err(LoadError::Corrupt(e)) => {
                return Err(format!("标准三规则文件无法解析：{}（{e}）", s3.display()))
            }
        }
        // 结构校验：用户改坏规则 json 时在这里拦下（不静默回退内置默认），错误信息含文件路径。
        cfg.standard3.validate().map_err(|e| format!("标准三规则无效：{}（{e}）", s3.display()))?;
        validate_registry(&cfg)?;
        Ok(cfg)
    }
}

/// 标准之间的登记冲突守卫：任意两份 FolderList 不得互相包含（A ⊆ B 或 B ⊆ A）。
/// 一旦包含，`classify` 的超集轮/子集轮就会先命中先赢，出现「62 目录的包被判成标准三、
/// 其实该是标准五」这类静默错判。空清单跳过（空集是任何集合的子集，属退化情形）。
pub fn validate_registry(cfg: &RuleConfig) -> Result<(), String> {
    let lists = cfg.folder_lists();
    for i in 0..lists.len() {
        for j in (i + 1)..lists.len() {
            let (na, fa) = lists[i];
            let (nb, fb) = lists[j];
            if fa.is_empty() || fb.is_empty() { continue; }
            let a: BTreeSet<&str> = fa.iter().map(String::as_str).collect();
            let b: BTreeSet<&str> = fb.iter().map(String::as_str).collect();
            if a.is_subset(&b) || b.is_subset(&a) {
                let shared: Vec<&str> = a.intersection(&b).copied().collect();
                let head: Vec<&str> = shared.iter().copied().take(5).collect();
                return Err(format!(
                    "标准{na} 与 标准{nb} 的目录清单互相包含（重合 {} 项，如 {}），\
                     头像包判定会先命中先赢；请调整其中一份清单，让两者互不包含",
                    shared.len(), head.join(", ")
                ));
            }
        }
    }
    Ok(())
}

/// 可编辑规则文件登记表（命令层唯一查询入口）。新增标准在此加一项即可。
pub fn rule_files() -> Vec<RuleFile> {
    vec![standard2::rule_file(), standard3::rule_file()]
}

/// 首次运行导出用户可编辑的规则文件（已存在不覆盖，避免冲掉用户修改）。
pub fn ensure_user_files(data_dir: &Path) -> Result<(), String> {
    let dir = data_dir.join("rules");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建规则目录失败: {e}"))?;
    let cfg = RuleConfig::builtin();
    let s2 = dir.join("standard2.json");
    let s3 = dir.join("standard3.json");
    if !s2.is_file() {
        json_store::save(&s2, &cfg.standard2)?;
    }
    if !s3.is_file() {
        json_store::save(&s3, &cfg.standard3)?;
    }
    Ok(())
}

/// 按扫描出的布局选择对应标准规则（唯一分发入口）。
/// 【一律借用、由各规则自己转池】大包索引（28 万图片名）整份 clone 一次就是几十 MB 的白复制，
/// 所以这里只传 `&PackScan` 的引用，各规则构造时按需把要用的那部分转成自己的池。
pub fn for_layout(layout: PackLayout, scan: &PackScan, config: &RuleConfig) -> Box<dyn PickRule> {
    match layout {
        // 标准一：包根单一池，完全随机。
        PackLayout::Flat => Box::new(standard1::Standard1Rule::new(&scan.root_images)),
        PackLayout::Standard { standard, .. } => match standard {
            2 => Box::new(standard2::Standard2Rule::new(&scan.faces_map, &config.standard2)),
            3 => Box::new(standard3::Standard3Rule::new(
                &scan.faces_map, &scan.recursive_pools, &config.standard3)),
            // 未实现标准号已被 replace_flow 拦截；防御性返回永不命中的空规则。
            _ => Box::new(standard1::Standard1Rule::new(&BTreeSet::new())),
        },
        // 空包 / 无法识别：递归全池全随机（Empty 池为空 → 全部球员跳过）。
        PackLayout::Empty | PackLayout::Unknown =>
            Box::new(unknown::UnknownRule::new(&scan.recursive_pools)),
    }
}

// ---------------- 两个跨标准无语义的机械工具 ----------------

/// 图片池：目录路径 → 该目录下的图片词干列表（"" = 包根，多级用 "/" 分隔）。
///
/// 【为什么是 Vec 而不是 BTreeSet】抽图要按**随机下标**取图，BTreeSet 只能 `iter().nth(idx)`，
/// 那是 O(idx) 线性游走（标准三大池每球员平均要走两千多步，实测是抽图成本的大头）；Vec 下标直取
/// O(1)，禁重复时 `swap_remove` 也是 O(1)，且不必再按值在线性表里找位置。
///
/// 【为什么仍然有序】所有池都经 `sorted_unique` 构造（排序 + 去重）：发图顺序可复现，
/// 也把 `read_dir` 的顺序差异（NTFS≈名字序 / ext4·APFS≈哈希序）挡在池外面。
pub type Pools = BTreeMap<String, Vec<String>>;

/// 把图片词干收集成池：排序 + 去重。
///
/// 【去重不是多余的】上游扫描结果是 `BTreeSet`（天生按词干去重），但同目录下 `face1.png` 与
/// `face1.jpg` 的词干相同——一旦上游换成非去重来源，Vec 会同时装下两项：禁重复时抽走第一项
/// `swap_remove` 掉、第二项还在，两人就会拿到同一张图（from 相同）。显式去重把这条兜住，
/// 三个平台的这个场景（同目录同词干不同扩展名）都是合法的。
///
/// 【排序是白送的】上游 BTreeSet 本就升序，Rust 的排序对已序输入接近线性，成本可忽略。
pub fn sorted_unique(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = items.into_iter().collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// 按"目录 → 图片集合"的池构造规则时通用：只保留 FolderList 内、且非空的目录。
/// 【语义】json 未配置的多余子目录不进入抽图池。
fn filter_pools(faces_map: &BTreeMap<String, BTreeSet<String>>, folder_list: &[String]) -> Pools {
    folder_list
        .iter()
        .filter_map(|d| {
            faces_map
                .get(d)
                .filter(|s| !s.is_empty())
                .map(|s| (d.clone(), sorted_unique(s.iter().cloned())))
        })
        .collect()
}

/// 从"目录 → 图片集合"的多级池中全局随机取一张（展平计数按下标取，有序池保证可复现）。
/// 禁重复时把选中的图从池中移除（swap_remove：O(1)，不需要按值查找）。标准三的兜底与 Unknown 布局共用。
fn pick_from_pools_flat(pools: &mut Pools, duplicates: bool) -> Option<PickedImage> {
    let total: usize = pools.values().map(|s| s.len()).sum();
    if total == 0 { return None; }
    let idx = rand::rng().random_range(0..total);
    let mut passed = 0usize;
    let mut chosen: Option<(String, String)> = None;
    for (dir, pool) in pools.iter_mut() {
        if idx < passed + pool.len() {
            let i = idx - passed;
            // 下标直取 O(1)；禁重复就地 swap_remove 摘除。
            let stem = if duplicates { pool[i].clone() } else { pool.swap_remove(i) };
            chosen = Some((dir.clone(), stem));
            break;
        }
        passed += pool.len();
    }
    let (dir, stem) = chosen?;
    Some(PickedImage { dir, stem })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实头像包冒烟测试（`#[ignore]`，CI 不跑、未设环境变量时自动跳过）。用法：
    /// `NEWGAN_REAL_PACK_DIR=<包根> cargo test --manifest-path src-tauri/Cargo.toml \
    ///  real_pack_smoke -- --ignored --nocapture`
    ///
    /// 全链路验证 scan → classify → 标准三抽图：233 国 × 11 码全量扫一遍，
    /// 硬断言只有两条——每人都能抽到图、抽中的图在包里真实存在（config.xml 的 from
    /// 不带扩展名，逐个试图片扩展名）；直中/回退占比与空清单目录只打印不拦，
    /// 便于对任何社区包做体检（部分目录缺失时回退是正确行为而非错误）。
    #[test]
    #[ignore]
    fn real_pack_smoke_scan_classify_pick() {
        use crate::xml_parser::record_from;
        use super::standard3::Candidates;
        let Some(pack) = std::env::var_os("NEWGAN_REAL_PACK_DIR") else {
            eprintln!("未设 NEWGAN_REAL_PACK_DIR，跳过真实包冒烟测试");
            return;
        };
        let pack = pack.to_string_lossy().to_string();
        // 规则来源与真实运行同源（开发数据目录的 rules/）；用户文件损坏不中断冒烟，退回内置默认。
        let data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().join("data");
        let config = RuleConfig::load(&data_dir).unwrap_or_else(|e| {
            eprintln!("用户规则不可用（{e}），改用内置默认");
            RuleConfig::builtin()
        });
        let t = std::time::Instant::now();
        let scan = crate::facepack_scanner::scan_pack(&pack).unwrap();
        let total_imgs: usize = scan.recursive_pools.values().map(|s| s.len()).sum();
        eprintln!("扫描：{} 个一级目录，递归池 {total_imgs} 张图，耗时 {:?}",
                  scan.faces_map.len(), t.elapsed());
        let refs: Vec<_> = config.folder_lists().iter().map(|(t, f)| (*t, *f)).collect();
        let subdirs: BTreeSet<String> = scan.faces_map.keys()
            .filter(|n| !crate::facepack_scanner::is_junk_dir(n))
            .cloned().collect();
        let layout = crate::facepack_scanner::classify(&subdirs, !scan.root_images.is_empty(), &refs);
        eprintln!("判定：{layout:?}");
        assert!(matches!(layout, PackLayout::Standard { standard: 3, .. }),
                "该包未被判为标准三：{layout:?}");
        // 清单目录在包内的健康度：缺失 / 空目录列出来（对应「一键创建缺失目录」的语义）。
        let unhealthy: Vec<&String> = config.standard3.folder_list.iter()
            .filter(|d| scan.faces_map.get(*d).is_none_or(|s| s.is_empty()))
            .collect();
        if !unhealthy.is_empty() {
            eprintln!("清单内缺失或为空的目录 {} 个：{unhealthy:?}", unhealthy.len());
        }
        // 第一轮：允许复用，全量 (国籍 × 码) 扫一遍，统计直中候选 vs 走回退。
        let mut rule = for_layout(layout, &scan, &config);
        let cfg = &config.standard3;
        let (mut direct, mut fallback) = (0usize, 0usize);
        let mut missing: Vec<String> = Vec::new();
        let mut by_dir: BTreeMap<String, usize> = BTreeMap::new();
        for nat in cfg.ethnics.keys() {
            for code in 0..=10u8 {
                let res = cfg.resolve(nat, &code.to_string());
                let img = rule.pick(&PlayerRecord {
                    uid: format!("{nat}/{code}"), primary_nat: nat.into(), sec_nat: String::new(),
                    name: "T".into(), hair_length: "1".into(), hair_color: "1".into(),
                    ethnicity_code: code.to_string(), detailed: None,
                }, true).unwrap_or_else(|| panic!("{nat}/{code} 没抽到图"));
                let in_cands = match res.candidates.as_ref() {
                    Some(Candidates::Single(d)) => *d == img.dir,
                    Some(Candidates::Uniform(v)) => v.iter().any(|d| *d == img.dir),
                    Some(Candidates::Weighted { dirs, .. }) => dirs.iter().any(|d| *d == img.dir),
                    None => false,
                };
                if in_cands { direct += 1 } else { fallback += 1 }
                *by_dir.entry(img.dir.clone()).or_insert(0) += 1;
                let base = std::path::Path::new(&pack).join(&img.dir).join(&img.stem);
                if !["png", "jpg", "jpeg"].iter().any(|e|
                    std::path::Path::new(&format!("{}.{e}", base.display())).is_file())
                {
                    missing.push(format!("{}/{}", img.dir, img.stem));
                }
            }
        }
        assert!(missing.is_empty(),
                "{} 条映射指向包里不存在的文件，如 {:?}", missing.len(), &missing[..missing.len().min(5)]);
        let mut ranked: Vec<_> = by_dir.iter().collect();
        ranked.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        eprintln!("全量 2563 组：直中候选 {direct}，走回退 {fallback}；目录命中数 Top8：{:?}",
                  &ranked[..ranked.len().min(8)]);
        // 第二轮：禁重复再扫一遍，验证两份索引的同步摘除在真实规模下也无重复。
        let mut rule = for_layout(layout, &scan, &config);
        let mut issued = BTreeSet::new();
        for (i, nat) in cfg.ethnics.keys().enumerate() {
            for code in 0..=10u8 {
                let img = rule.pick(&PlayerRecord {
                    uid: format!("nd-{nat}/{code}"), primary_nat: nat.into(), sec_nat: String::new(),
                    name: "T".into(), hair_length: "1".into(), hair_color: "1".into(),
                    ethnicity_code: code.to_string(), detailed: None,
                }, false).unwrap_or_else(|| panic!("禁重复第 {i} 组 {nat}/{code} 没抽到图"));
                assert!(issued.insert(record_from(&img.dir, &img.stem)),
                        "禁重复抽到重复图 {img:?}");
            }
        }
        eprintln!("禁重复 {} 组：from 全部唯一", issued.len());
    }

    #[test]
    fn builtin_has_expected_defaults() {
        let cfg = RuleConfig::builtin();
        // 标准二：14 目录 + 国籍映射 + 全部 0-10 码修正规则。
        assert_eq!(cfg.standard2.folder_list.len(), 14);
        assert!(cfg.standard2.folder_list.contains(&"YugoGreek".to_string()));
        assert_eq!(cfg.standard2.ethnics["AFG"], "MESA");
        for code in ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"] {
            assert!(cfg.standard2.code_rules.contains_key(code), "缺码 {code}");
        }
        // 标准三：62 目录清单 + Ethnics（233 国）+ 规则块（国家/族群/全局共 43 个）+ Version 4 + Random 兜底。
        assert_eq!(cfg.standard3.folder_list.len(), 62);
        assert!(cfg.standard3.folder_list.contains(&"SouthConeSA".to_string()));
        assert!(cfg.standard3.folder_list.contains(&"WestSlavic".to_string()));
        assert_eq!(cfg.standard3.ethnics.len(), 233);
        assert_eq!(cfg.standard3.ethnics["ESP"], "Spain");
        assert_eq!(cfg.standard3.ethnics["COL"], "Mestizo");
        assert_eq!(cfg.standard3.default_codes["ARG"], vec![1, 7]);
        assert_eq!(cfg.standard3.default_codes["BRA"], vec![1, 3, 9]);
        assert_eq!(cfg.standard3.version, 4);
        let rules = &cfg.standard3.ethnicity_mapping_rules;
        assert_eq!(rules.len(), 43);
        // 三类块各抽查一条：国家块（BRA）、族群块（Mestizo）、全局块（_DEFAULT_）。
        assert!(rules.contains_key("BRA") && rules.contains_key("Mestizo") && rules.contains_key("_DEFAULT_"));
        assert_eq!(rules["_DEFAULT_"]["9"],
                   standard3::MappingRule::Directly("MixedRace".into()));
        assert_eq!(rules["_DEFAULT_"]["0"], standard3::MappingRule::Default);
        assert_eq!(rules["Mestizo"]["7"],
                   standard3::MappingRule::Directly("IndigenousSA".into()));
        assert!(matches!(rules["DOM"]["1"], standard3::MappingRule::Weighted { .. }));
        assert_eq!(cfg.standard3.fallback, standard3::Fallback::Random);
        // 内置默认必须通过自身的结构校验。
        cfg.standard3.validate().unwrap();
    }

    #[test]
    fn ensure_then_load_roundtrip_and_corrupt_reports_path() {
        let data = tempfile::tempdir().unwrap();
        // 首次运行：导出两个用户文件；load 结果与内置一致。
        ensure_user_files(data.path()).unwrap();
        assert!(data.path().join("rules/standard2.json").is_file());
        assert!(data.path().join("rules/standard3.json").is_file());
        let cfg = RuleConfig::load(data.path()).unwrap();
        assert_eq!(cfg.standard2.ethnics["ARG"], "SAMed");
        assert_eq!(cfg.standard3.folder_list.len(), 62);
        // 二次 ensure：已存在不覆盖（写个标记内容验证不被冲掉）。
        let s2 = data.path().join("rules/standard2.json");
        std::fs::write(&s2, "{\"FolderList\":[\"African\"],\"Ethnics\":{\"XXL\":\"African\"}}").unwrap();
        ensure_user_files(data.path()).unwrap();
        let cfg = RuleConfig::load(data.path()).unwrap();
        assert_eq!(cfg.standard2.folder_list, vec!["African".to_string()]);
        assert_eq!(cfg.standard2.ethnics["XXL"], "African");
        // 改坏 json：load 报错且信息含文件路径（不静默回退）。
        std::fs::write(&s2, "{ broken").unwrap();
        let err = RuleConfig::load(data.path()).unwrap_err();
        assert!(err.contains("standard2.json"), "错误应含文件路径: {err}");
        // 用户文件删除：回退内置默认。
        std::fs::remove_file(&s2).unwrap();
        let cfg = RuleConfig::load(data.path()).unwrap();
        assert_eq!(cfg.standard2.folder_list.len(), 14);
        // 标准三：json 格式合法但语义不成立（Version 缺失、无 _DEFAULT_ 块）→ 报错且含文件路径。
        let s3 = data.path().join("rules/standard3.json");
        std::fs::write(&s3, "{\"FolderList\":[\"China\"],\"Ethnics\":{\"CHN\":\"China\"}}").unwrap();
        let err = RuleConfig::load(data.path()).unwrap_err();
        assert!(err.contains("standard3.json"), "错误应含文件路径: {err}");
        assert!(err.contains("Version"), "应说明版本不符: {err}");
        std::fs::remove_file(&s3).unwrap();
        assert_eq!(RuleConfig::load(data.path()).unwrap().standard3.folder_list.len(), 62);
    }

    #[test]
    fn folder_lists_registers_all_standards() {
        let cfg = RuleConfig::builtin();
        let lists = cfg.folder_lists();
        assert_eq!(lists.iter().map(|(t, _)| *t).collect::<Vec<_>>(), vec![2, 3]);
        assert!(lists.iter().all(|(_, f)| !f.is_empty()));
    }

    #[test]
    fn validate_registry_rejects_nested_folder_lists() {
        // 现状两份清单互不包含 → 通过。
        validate_registry(&RuleConfig::builtin()).unwrap();
        // 人为构造包含关系（标准二清单 ⊂ 标准三清单）→ 报错并指名两个标准号。
        let mut cfg = RuleConfig::builtin();
        cfg.standard2.folder_list = vec!["China".into(), "Japan".into()];
        let err = validate_registry(&cfg).unwrap_err();
        assert!(err.contains("标准2") && err.contains("标准3"), "应指名冲突标准: {err}");
        assert!(err.contains("China"), "应给出重合目录名: {err}");
        // 空清单不参与判定（否则空集恒为子集，误报）。
        let mut cfg = RuleConfig::builtin();
        cfg.standard2.folder_list = Vec::new();
        validate_registry(&cfg).unwrap();
    }
}
