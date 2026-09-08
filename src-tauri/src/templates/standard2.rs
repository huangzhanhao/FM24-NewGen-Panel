//! 【新手导读】标准二：头像包按 14 个种族/大区分目录组织（African、Asian、Caucasian…）。
//! 抽图逻辑 = 国籍查表（json `Ethnics`）→ 种族码修正（json `CodeRules`）→ 池优先级 → 随机抽图。
//! 规则全部来自 `assets/standard2.json`（内置默认）与 `data/rules/standard2.json`（用户可编辑），
//! 本模块不硬编码任何目录名或码表。
use super::{filter_pools, PickedImage, PickRule, RuleFile};
use crate::json_store;
use crate::rtf_parser::PlayerRecord;
use crate::xml_parser::record_from;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// FM 种族码修正规则（码 → 候选目录优先级 + 兜底目录）。
/// 语义：prefer 列表中取第一个命中主/第二国籍种族目录的项；都不命中用 fallback；
/// 固定值码（如码 4 恒为 MESA）表达为 prefer 与 fallback 同值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeRule {
    #[serde(default)]
    pub prefer: Vec<String>,
    pub fallback: String,
}

/// 标准二规则配置（14 种族目录）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Standard2Config {
    #[serde(default)]
    pub folder_list: Vec<String>,                    // "FolderList"：目录清单（判定 + 池过滤基准）
    #[serde(default)]
    pub ethnics: BTreeMap<String, String>,           // "Ethnics"：国籍三字码 → 种族目录
    #[serde(default)]
    pub code_rules: BTreeMap<String, CodeRule>,      // "CodeRules"：FM 种族码修正
}

/// 本标准的可编辑规则文件登记（命令层查表用）。
pub fn rule_file() -> RuleFile {
    RuleFile { name: "standard2", path: |data_dir: &Path| rule_path(data_dir), save: save_file }
}

fn rule_path(data_dir: &Path) -> PathBuf { data_dir.join("rules").join("standard2.json") }

/// 解析 + 原子写回（json_store 自动 pretty 格式化，相当于顺带规范化用户手改的格式）。
fn save_file(path: &Path, content: &str) -> Result<(), String> {
    let cfg: Standard2Config =
        serde_json::from_str(content).map_err(|e| format!("JSON 格式错误（标准二）：{e}"))?;
    json_store::save(path, &cfg).map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

/// 种族码修正：prefer 列表中取第一个命中主/第二国籍种族目录的项，
/// 都不命中用 fallback；未配置的码沿用主国籍目录。
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

/// 标准二抽图规则：池 = FolderList 过滤后的一级种族/国籍目录。
pub struct Standard2Rule {
    pools: BTreeMap<String, BTreeSet<String>>,
    cfg: Standard2Config, // 构造时只拿自己这份配置，之后不再看其他标准
}

impl Standard2Rule {
    pub fn new(faces_map: &BTreeMap<String, BTreeSet<String>>, cfg: &Standard2Config) -> Self {
        Self { pools: filter_pools(faces_map, &cfg.folder_list), cfg: cfg.clone() }
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

    /// 依序取第一个非空池随机选图；禁重复时从池中移除。
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

impl PickRule for Standard2Rule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        // 按全路径精确匹配：只剔除「本目录/词干」确实被占用的图，不跨目录误伤同名图。
        for (dir, pool) in self.pools.iter_mut() {
            pool.retain(|s| !used.contains(&record_from(dir, s)));
        }
    }

    fn pick(&mut self, player: &PlayerRecord, duplicates: bool) -> Option<PickedImage> {
        // 双国籍查表(json Ethnics) → 种族码修正(json CodeRules) → 校验目录在池内 → 选图。
        let e2 = if player.sec_nat.is_empty() {
            None
        } else {
            self.cfg.ethnics.get(&player.sec_nat).map(|s| s.as_str())
        };
        let e1 = self.cfg.ethnics.get(&player.primary_nat)?;
        let ethnicity = apply_code_rule(&player.ethnicity_code, e1, e2, &self.cfg.code_rules);
        if !self.pools.contains_key(&ethnicity) {
            return None; // 头像包缺该种族目录
        }
        let dirs = self.image_pool_dirs(&ethnicity, &player.primary_nat);
        let stem = self.pick_image_from_pools(&dirs, duplicates)?;
        Some(PickedImage { dir: ethnicity, stem })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::RuleConfig;

    fn player(uid: &str, nat: &str, code: &str) -> PlayerRecord {
        PlayerRecord {
            uid: uid.into(), primary_nat: nat.into(), sec_nat: String::new(),
            name: "T".into(), hair_length: "1".into(), hair_color: "1".into(),
            ethnicity_code: code.into(), detailed: None,
        }
    }

    fn sset(v: &[&str]) -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn apply_code_rule_matches_legacy_correct_ethnic() {
        // 内置 CodeRules（数据驱动）必须与旧 correct_ethnic 硬编码逐分支等价。
        let rules = RuleConfig::builtin().standard2.code_rules;
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
        // 未配置的码：沿用主国籍目录。
        assert_eq!(apply_code_rule("99", "SpanMed", None, &rules), "SpanMed");
    }

    #[test]
    fn standard2_uses_config_mapping_and_filters_pools() {
        // 池构造：FolderList 过滤——多余目录（Extra）与空目录不进抽图池。
        let faces_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("SpanMed".into(), sset(&["s1", "s2"])),
            ("Caucasian".into(), sset(&["c1"])),
            ("Extra".into(), sset(&["x1"])),   // json 未配置 → 不进池
            ("Empty".into(), BTreeSet::new()), // 空目录 → 不进池
        ]);
        let mut rule = Standard2Rule::new(&faces_map, &RuleConfig::builtin().standard2);
        assert!(!rule.pools.contains_key("Extra"));
        assert!(!rule.pools.contains_key("Empty"));
        // ESP 经内置 Ethnics → SpanMed，种族码 1 → SpanMed，随机取池内图。
        let img = rule.pick(&player("r-1", "ESP", "1"), true).unwrap();
        assert_eq!(img.dir, "SpanMed");
        assert!(["s1", "s2"].contains(&img.stem.as_str()));
        // 未配置国籍（内置表查不到）→ 球员跳过。
        assert!(rule.pick(&player("r-2", "ZZZ", "1"), true).is_none());
        // 池中无该种族目录（如 African 未建）→ 跳过。
        assert!(rule.pick(&player("r-3", "NGA", "3"), true).is_none());
    }

    #[test]
    fn standard2_rule_file_registration() {
        // 登记表项：name 与路径遵循 standardN.json 主轴；save 拒绝非法 json 且不写盘。
        let rf = rule_file();
        assert_eq!(rf.name, "standard2");
        let data = tempfile::tempdir().unwrap();
        assert_eq!((rf.path)(data.path()).file_name().unwrap(), "standard2.json");
        let p = (rf.path)(data.path());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap(); // 真实流程里 rules/ 由 ensure_user_files 先建好
        assert!((rf.save)(&p, "{ broken").is_err());
        assert!(!p.exists(), "解析失败不得写盘");
        (rf.save)(&p, "{\"FolderList\":[\"African\"],\"Ethnics\":{\"XXX\":\"African\"}}").unwrap();
        let back: Standard2Config = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(back.folder_list, vec!["African".to_string()]);
    }
}
