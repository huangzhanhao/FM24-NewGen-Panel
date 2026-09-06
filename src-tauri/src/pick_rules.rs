//! 【新手导读】抽图模板规则（按头像包模板独立成套，规则数据来自 rule_config 的 json）。
//! 头像包扫描判定（facepack::classify）决定使用哪套规则：
//!   模板一 Flat        → FlatRule      ：包根单一池，完全随机抽图（忽略国籍/种族码）；
//!   模板二 Standard{2} → EthnicRule    ：国籍查表(json) → 种族码修正(json) → 池优先级 → 随机抽图；
//!   模板三 Standard{3} → Standard3Rule ：Mappings[国籍][种族码] → 目录池随机；未命中按 Fallback 兜底；
//!   Empty / Unknown    → RecursiveRule ：包内所有子/孙目录（含根）图片归入一个大池，全随机。
//! 每套规则实现统一接口 PickRule（pick 抽图 + exclude_used 禁重复剔除）。
//! 【扩展点】新增模板四/五/六：rule_config 登记文件夹清单 + 在 for_layout 加分支 +
//! 实现对应 PickRule + 更新 IMPLEMENTED_TEMPLATES；判定层零改动。
use crate::facepack::{PackLayout, PackScan};
use crate::rule_config::{CodeRule, Fallback, RuleConfig, Template2Config, Template3Config};
use crate::rtf_parser::PlayerRecord;
use crate::xml_parser::record_from;
use rand::Rng;
use std::collections::{BTreeMap, BTreeSet};

/// 当前已实现抽图规则的模板号；replace_flow 据此对清单外的模板拦截提示。
pub const IMPLEMENTED_TEMPLATES: [u8; 2] = [2, 3];

/// 一次抽图的结果：图片相对头像包根的目录路径（"" = 根，多级 "子目录/孙目录"）+ 文件词干。
/// 写 config.xml 时 dir + stem 经 record_from 拼成 from（统一相对路径口径）。
pub struct PickedImage { pub dir: String, pub stem: String }

/// 抽图模板规则统一接口：每种头像包模板一套实现。
pub trait PickRule {
    /// Preserve+禁重复：把已有映射占用的图片从池中剔除。
    /// used 为「from 全路径」键集合（record_from 形式：种族目录非空 → "目录/词干"，平铺 → "词干"）。
    fn exclude_used(&mut self, used: &BTreeSet<String>);
    /// 为一名球员抽图；返回 None 表示该球员无法映射（跳过）。
    /// config 为全部模板规则（各规则只读自己模板的部分）。
    fn pick(
        &mut self,
        player: &PlayerRecord,
        duplicates: bool,
        config: &RuleConfig,
    ) -> Option<PickedImage>;
}

/// 按"目录 → 图片集合"的池构造规则时通用：只保留 FolderList 内、且非空的目录。
/// 【语义】json 模板未配置的多余子目录不进入抽图池。
fn filter_pools(
    faces_map: &BTreeMap<String, BTreeSet<String>>,
    folder_list: &[String],
) -> BTreeMap<String, BTreeSet<String>> {
    folder_list
        .iter()
        .filter_map(|d| faces_map.get(d).filter(|s| !s.is_empty()).map(|s| (d.clone(), s.clone())))
        .collect()
}

/// 从"目录 → 图片集合"的多级池中全局随机取一张（展平计数按下标取，有序集合保证可复现）。
/// 禁重复时把选中的图从池中移除。RecursiveRule 与 Standard3Rule 的兜底共用。
fn pick_from_pools_flat(
    pools: &mut BTreeMap<String, BTreeSet<String>>,
    duplicates: bool,
) -> Option<PickedImage> {
    let total: usize = pools.values().map(|s| s.len()).sum();
    if total == 0 { return None; }
    let idx = rand::rng().random_range(0..total);
    let mut passed = 0usize;
    let mut chosen: Option<(String, String)> = None;
    for (dir, set) in pools.iter() {
        if idx < passed + set.len() {
            chosen = Some((dir.clone(), set.iter().nth(idx - passed).cloned().unwrap_or_default()));
            break;
        }
        passed += set.len();
    }
    let (dir, stem) = chosen?;
    if !duplicates {
        if let Some(set) = pools.get_mut(&dir) { set.remove(&stem); }
    }
    Some(PickedImage { dir, stem })
}

/// 按扫描出的布局选择对应模板规则（唯一分发入口）。
pub fn for_layout(layout: PackLayout, scan: &PackScan, config: &RuleConfig) -> Box<dyn PickRule> {
    match layout {
        // 模板一：包根单一池。
        PackLayout::Flat => Box::new(FlatRule { pool: scan.root_images.clone() }),
        PackLayout::Standard { template, .. } => match template {
            2 => Box::new(EthnicRule::new(&scan.faces_map, &config.template2)),
            3 => Box::new(Standard3Rule::new(&scan.faces_map, &scan.recursive_pools, &config.template3)),
            // 未实现模板号已被 replace_flow 拦截；防御性返回永不命中的空规则。
            _ => Box::new(FlatRule { pool: BTreeSet::new() }),
        },
        // 空包 / 无法识别：递归全池全随机（Empty 池为空 → 全部球员跳过）。
        PackLayout::Empty | PackLayout::Unknown =>
            Box::new(RecursiveRule { pools: scan.recursive_pools.clone() }),
    }
}

// ---------------- 模板一：平铺全随机 ----------------

/// 包根全部图片为一个单一池，完全随机抽图。
pub struct FlatRule { pool: BTreeSet<String> }

impl PickRule for FlatRule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        // 平铺条目的 from 全路径就是纯词干，直接比对。
        self.pool.retain(|s| !used.contains(s));
    }

    fn pick(
        &mut self,
        _player: &PlayerRecord,
        duplicates: bool,
        _config: &RuleConfig,
    ) -> Option<PickedImage> {
        if self.pool.is_empty() { return None; }
        let idx = rand::rng().random_range(0..self.pool.len());
        let stem = self.pool.iter().nth(idx).cloned()?;
        if !duplicates {
            self.pool.remove(&stem); // 禁重复：选过的图不再出现
        }
        Some(PickedImage { dir: String::new(), stem })
    }
}

// ---------------- 模板二：种族匹配（数据驱动：Ethnics 映射 + CodeRules 码修正） ----------------

/// 种族码修正（数据驱动）：prefer 列表中取第一个命中主/第二国籍种族目录的项，
/// 都不命中用 fallback；未配置的码沿用主国籍目录（与旧硬编码 `_ => e1` 分支一致）。
pub fn apply_code_rule(
    code: &str,
    e1: &str,
    e2: Option<&str>,
    rules: &BTreeMap<String, CodeRule>,
) -> String {
    match rules.get(code) {
        Some(rule) => {
            let hit = |name: &str| e1 == name || e2 == Some(name);
            rule.prefer.iter().find(|n| hit(n)).cloned().unwrap_or_else(|| rule.fallback.clone())
        }
        None => e1.to_string(),
    }
}

/// 模板二规则：池 = FolderList 过滤后的一级种族/国籍目录。
pub struct EthnicRule { pools: BTreeMap<String, BTreeSet<String>> }

impl EthnicRule {
    pub fn new(faces_map: &BTreeMap<String, BTreeSet<String>>, cfg: &Template2Config) -> Self {
        Self { pools: filter_pools(faces_map, &cfg.folder_list) }
    }

    /// 池优先级：国籍同名目录 > 种族目录 > 两者都不在池内时用全部目录。
    fn image_pool_dirs(&self, ethnicity: &str, nationality: &str) -> Vec<String> {
        let mut pools = Vec::new();
        let nat_in = self.pools.get(nationality).map(|s| !s.is_empty()).unwrap_or(false);
        let eth_in = self.pools.get(ethnicity).map(|s| !s.is_empty()).unwrap_or(false);
        if nat_in { pools.push(nationality.to_string()); }
        if eth_in { pools.push(ethnicity.to_string()); }
        if !self.pools.contains_key(nationality) && !self.pools.contains_key(ethnicity) {
            pools.extend(self.pools.keys().cloned());
        }
        pools
    }

    /// 依序取第一个非空池随机选图；禁重复时从池中移除（对等旧版 pick_image_from_pools）。
    fn pick_image_from_pools(&mut self, pools: &[String], duplicates: bool) -> Option<String> {
        for dir in pools {
            let Some(pool) = self.pools.get_mut(dir) else { continue };
            if pool.is_empty() { continue; }
            let idx = rand::rng().random_range(0..pool.len());
            if let Some(choice) = pool.iter().nth(idx).cloned() {
                if !duplicates { pool.remove(&choice); }
                return Some(choice);
            }
        }
        None
    }
}

impl PickRule for EthnicRule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        // 按全路径精确匹配：只剔除「本目录/词干」确实被占用的图，不跨目录误伤同名图。
        for (dir, pool) in self.pools.iter_mut() {
            pool.retain(|s| !used.contains(&record_from(dir, s)));
        }
    }

    fn pick(
        &mut self,
        player: &PlayerRecord,
        duplicates: bool,
        config: &RuleConfig,
    ) -> Option<PickedImage> {
        // 双国籍查表(json Ethnics) → 种族码修正(json CodeRules) → 校验目录在池内 → 选图。
        let t2 = &config.template2;
        let e2 = if player.sec_nat.is_empty() {
            None
        } else {
            t2.ethnics.get(&player.sec_nat).map(|s| s.as_str())
        };
        let e1 = t2.ethnics.get(&player.primary_nat)?;
        let ethnicity = apply_code_rule(&player.ethnicity_code, e1, e2, &t2.code_rules);
        if !self.pools.contains_key(&ethnicity) {
            return None; // 头像包缺该种族目录
        }
        let dirs = self.image_pool_dirs(&ethnicity, &player.primary_nat);
        let stem = self.pick_image_from_pools(&dirs, duplicates)?;
        Some(PickedImage { dir: ethnicity, stem })
    }
}

// ---------------- 模板三：Mappings[国籍][种族码] → 目录池 ----------------

/// 模板三规则：按 json 的 Mappings 精确检索目录池；未命中按 Fallback 兜底。
pub struct Standard3Rule {
    pools: BTreeMap<String, BTreeSet<String>>,           // FolderList 过滤后的一级目录池
    recursive_pools: BTreeMap<String, BTreeSet<String>>, // 兜底 random 用的递归全池
    fallback: Fallback,
}

impl Standard3Rule {
    pub fn new(
        faces_map: &BTreeMap<String, BTreeSet<String>>,
        recursive_pools: &BTreeMap<String, BTreeSet<String>>,
        cfg: &Template3Config,
    ) -> Self {
        Self {
            pools: filter_pools(faces_map, &cfg.folder_list),
            recursive_pools: recursive_pools.clone(),
            fallback: cfg.fallback,
        }
    }
}

impl PickRule for Standard3Rule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        for (dir, pool) in self.pools.iter_mut() {
            pool.retain(|s| !used.contains(&record_from(dir, s)));
        }
        for (dir, pool) in self.recursive_pools.iter_mut() {
            pool.retain(|s| !used.contains(&record_from(dir, s)));
        }
    }

    fn pick(
        &mut self,
        player: &PlayerRecord,
        duplicates: bool,
        config: &RuleConfig,
    ) -> Option<PickedImage> {
        let t3 = &config.template3;
        // 精确检索：Mappings[国籍三字码][球员 RTF 种族码] → 候选目录列表。
        if let Some(dirs) = t3.mappings
            .get(&player.primary_nat)
            .and_then(|m| m.get(&player.ethnicity_code))
        {
            // 候选目录须在包内（FolderList 过滤后）且非空，随机取一个再从池中随机取图。
            let candidates: Vec<&String> = dirs.iter()
                .filter(|d| self.pools.get(*d).map_or(false, |s| !s.is_empty()))
                .collect();
            if !candidates.is_empty() {
                let i1 = rand::rng().random_range(0..candidates.len());
                let dir = candidates[i1].clone();
                let pool = self.pools.get_mut(&dir).expect("上方已校验存在");
                let i2 = rand::rng().random_range(0..pool.len());
                let stem = pool.iter().nth(i2).cloned()?;
                if !duplicates { pool.remove(&stem); }
                return Some(PickedImage { dir, stem });
            }
        }
        // 兜底：映射未收集/未命中 → random=递归全池随机；skip=跳过该球员。
        match self.fallback {
            Fallback::Random => pick_from_pools_flat(&mut self.recursive_pools, duplicates),
            Fallback::Skip => None,
        }
    }
}

// ---------------- Empty / Unknown：递归全池全随机 ----------------

/// 包内所有子/孙目录（含根）的全部图片归入一个大池，完全随机抽图。
/// 选中的 dir 原样写入映射（可空、可多级），config.xml 的 from 即相对路径。
pub struct RecursiveRule { pools: BTreeMap<String, BTreeSet<String>> }

impl PickRule for RecursiveRule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        for (dir, pool) in self.pools.iter_mut() {
            pool.retain(|s| !used.contains(&record_from(dir, s)));
        }
    }

    fn pick(
        &mut self,
        _player: &PlayerRecord,
        duplicates: bool,
        _config: &RuleConfig,
    ) -> Option<PickedImage> {
        pick_from_pools_flat(&mut self.pools, duplicates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtf_parser::PlayerRecord;

    fn player(uid: &str, nat: &str, code: &str) -> PlayerRecord {
        PlayerRecord {
            uid: uid.into(), primary_nat: nat.into(), sec_nat: String::new(),
            name: "T".into(), hair_length: "1".into(), hair_color: "1".into(),
            ethnicity_code: code.into(), detailed: None,
        }
    }

    fn sset(v: &[&str]) -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() }

    fn bmap(v: &[(&str, &str)]) -> BTreeMap<String, String> {
        v.iter().map(|(k, val)| (k.to_string(), val.to_string())).collect()
    }

    #[test]
    fn flat_rule_picks_from_root_and_excludes_used() {
        let mut rule = FlatRule { pool: sset(&["a", "b", "c"]) };
        let config = RuleConfig::builtin();
        // 全随机：多次抽取都应落在池内，且 dir 为空串。
        for _ in 0..10 {
            let img = rule.pick(&player("r-1", "ZZZ", "1"), true, &config).unwrap();
            assert!(img.dir.is_empty());
            assert!(["a", "b", "c"].contains(&img.stem.as_str()));
        }
        // 禁重复剔除：b 被占用后不再被抽到。
        rule.exclude_used(&sset(&["b"]));
        for _ in 0..10 {
            let img = rule.pick(&player("r-1", "ZZZ", "1"), true, &config).unwrap();
            assert_ne!(img.stem, "b");
        }
    }

    #[test]
    fn recursive_rule_picks_everything_and_excludes_by_full_path() {
        // 池：根 {r1}、A {x1, x2}、A/sub {d1} —— 共 4 张。
        let mut rule = RecursiveRule { pools: BTreeMap::from([
            (String::new(), sset(&["r1"])),
            ("A".into(), sset(&["x1", "x2"])),
            ("A/sub".into(), sset(&["d1"])),
        ]) };
        let config = RuleConfig::builtin();
        // 禁重复抽满 4 次（池恰 4 张）→ 确定性断言：根、子、孙三个层级的图片全部可达。
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for _ in 0..4 {
            let img = rule.pick(&player("r-1", "ZZZ", "1"), false, &config).unwrap();
            seen.insert(record_from(&img.dir, &img.stem));
        }
        assert_eq!(seen, sset(&["r1", "A/x1", "A/x2", "A/sub/d1"]));
        assert!(rule.pick(&player("r-1", "ZZZ", "1"), false, &config).is_none()); // 池耗尽
        // 禁重复剔除按全路径：A/x1 与孙目录 d1 被占用后不再出现，其余仍在（重置池后验证）。
        let mut rule = RecursiveRule { pools: BTreeMap::from([
            (String::new(), sset(&["r1"])),
            ("A".into(), sset(&["x1", "x2"])),
            ("A/sub".into(), sset(&["d1"])),
        ]) };
        rule.exclude_used(&sset(&["A/x1", "A/sub/d1"]));
        assert_eq!(rule.pools["A"], sset(&["x2"]));
        assert!(rule.pools["A/sub"].is_empty());
        assert_eq!(rule.pools.get("").unwrap(), &sset(&["r1"]));
    }

    #[test]
    fn apply_code_rule_matches_legacy_correct_ethnic() {
        // 内置 CodeRules（数据驱动）必须与旧 correct_ethnic 硬编码逐分支等价。
        let rules = RuleConfig::builtin().template2.code_rules;
        // 0：优先 Scandinavian > Caucasian > 兜底
        assert_eq!(apply_code_rule("0", "African", Some("Scandinavian"), &rules), "Scandinavian");
        assert_eq!(apply_code_rule("0", "African", Some("Caucasian"), &rules), "Caucasian");
        assert_eq!(apply_code_rule("0", "African", None, &rules), "Central European");
        // 1：命中列表则用，否则兜底
        assert_eq!(apply_code_rule("1", "EECA", None, &rules), "EECA");
        assert_eq!(apply_code_rule("1", "MENA", None, &rules), "Central European");
        // 2：MESA 命中否则 MENA
        assert_eq!(apply_code_rule("2", "MESA", None, &rules), "MESA");
        assert_eq!(apply_code_rule("2", "MENA", None, &rules), "MENA");
        // 3/6/8/9：仅对应码命中，否则 African
        assert_eq!(apply_code_rule("6", "SAMed", None, &rules), "SAMed");
        assert_eq!(apply_code_rule("8", "Seasian", None, &rules), "Seasian");
        assert_eq!(apply_code_rule("3", "South American", None, &rules), "South American");
        assert_eq!(apply_code_rule("9", "South American", None, &rules), "South American");
        assert_eq!(apply_code_rule("3", "SAMed", None, &rules), "African");
        // 固定值（prefer 与 fallback 同值，无论 e1/e2）
        assert_eq!(apply_code_rule("4", "African", None, &rules), "MESA");
        assert_eq!(apply_code_rule("5", "African", None, &rules), "Seasian");
        assert_eq!(apply_code_rule("7", "African", None, &rules), "South American");
        // 10：链式兜底到 Asian
        assert_eq!(apply_code_rule("10", "MESA", None, &rules), "MESA");
        assert_eq!(apply_code_rule("10", "African", Some("Seasian"), &rules), "Seasian");
        assert_eq!(apply_code_rule("10", "Asian", None, &rules), "Asian");
        // 未配置的码：沿用主国籍目录（旧 `_ => e1` 分支）。
        assert_eq!(apply_code_rule("99", "SpanMed", None, &rules), "SpanMed");
    }

    #[test]
    fn ethnic_rule_uses_config_mapping_and_filters_pools() {
        // 池构造：FolderList 过滤——多余目录（Extra）与空目录不进抽图池。
        let faces_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("SpanMed".into(), sset(&["s1", "s2"])),
            ("Caucasian".into(), sset(&["c1"])),
            ("Extra".into(), sset(&["x1"])),   // json 未配置 → 不进池
            ("Empty".into(), BTreeSet::new()), // 空目录 → 不进池
        ]);
        let mut rule = EthnicRule::new(&faces_map, &RuleConfig::builtin().template2);
        assert!(!rule.pools.contains_key("Extra"));
        assert!(!rule.pools.contains_key("Empty"));
        // ESP 经内置 Ethnics → SpanMed，种族码 1 → SpanMed，随机取池内图。
        let config = RuleConfig::builtin();
        let img = rule.pick(&player("r-1", "ESP", "1"), true, &config).unwrap();
        assert_eq!(img.dir, "SpanMed");
        assert!(["s1", "s2"].contains(&img.stem.as_str()));
        // 未配置国籍（内置表查不到）→ 球员跳过。
        assert!(rule.pick(&player("r-2", "ZZZ", "1"), true, &config).is_none());
        // 池中无该种族目录（如 African 未建）→ 跳过。
        assert!(rule.pick(&player("r-3", "NGA", "3"), true, &config).is_none());
    }

    #[test]
    fn standard3_rule_mapping_hit_miss_random_and_skip() {
        let cfg = RuleConfig::builtin();
        let faces_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("Spain".into(), sset(&["sp1", "sp2"])),
            ("Anglosphere".into(), sset(&["an1"])),
        ]);
        // 自定义 Mappings：ESP 码 1 → Spain；AUS 码 0 → 缺失目录 NorthernSA（不在包内）。
        let mut t3 = cfg.template3.clone();
        t3.mappings = BTreeMap::from([
            ("ESP".to_string(), bmap_dirs(&[("1", &["Spain"])])),
            ("AUS".to_string(), bmap_dirs(&[("0", &["NorthernSA"])])),
            ("XXX".to_string(), bmap_dirs(&[("3", &["Spain"])])),
        ]);
        let mut config = cfg.clone();
        config.template3 = t3.clone();
        // 命中：ESP 码 1 → Spain 池随机。
        let mut rule = Standard3Rule::new(&faces_map, &scan_recursive(&faces_map), &config.template3);
        let img = rule.pick(&player("r-1", "ESP", "1"), true, &config).unwrap();
        assert_eq!(img.dir, "Spain");
        assert!(["sp1", "sp2"].contains(&img.stem.as_str()));
        // 目录不在包内（NorthernSA 缺失）→ 兜底 random：从递归全池随机取（仍可抽到图）。
        let img = rule.pick(&player("r-2", "AUS", "0"), true, &config).unwrap();
        assert!(!img.stem.is_empty());
        // 未配置国籍 → 兜底 random。
        let img = rule.pick(&player("r-3", "ZZZ", "5"), true, &config).unwrap();
        assert!(!img.stem.is_empty());
        // 兜底 skip：未命中直接跳过。
        let mut t3s = t3.clone();
        t3s.fallback = Fallback::Skip;
        let mut config_skip = config.clone();
        config_skip.template3 = t3s;
        let mut rule = Standard3Rule::new(&faces_map, &scan_recursive(&faces_map), &config_skip.template3);
        assert!(rule.pick(&player("r-4", "ZZZ", "5"), true, &config_skip).is_none());
        // XXX 码 3 配置的是 Spain（与码无关性验证）：命中 Spain 池。
        let mut rule = Standard3Rule::new(&faces_map, &scan_recursive(&faces_map), &config.template3);
        let img = rule.pick(&player("r-5", "XXX", "3"), true, &config).unwrap();
        assert_eq!(img.dir, "Spain");
    }

    /// 测试辅助：把 (码 → 目录列表) 字面量转成 Mappings 内层结构。
    fn bmap_dirs(v: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        v.iter().map(|(k, dirs)| (k.to_string(), dirs.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    /// 测试辅助：由一级目录图集构造递归池（根 + 各目录）。
    fn scan_recursive(
        faces_map: &BTreeMap<String, BTreeSet<String>>,
    ) -> BTreeMap<String, BTreeSet<String>> {
        let mut pools = faces_map.clone();
        pools.entry(String::new()).or_default();
        pools
    }
}
