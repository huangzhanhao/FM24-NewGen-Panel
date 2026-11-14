//! 标准二：头像包按 14 个种族/大区分目录组织（African、Asian、Caucasian…）。
//! 抽图逻辑 = 国籍查表（json `Ethnics`）→ 种族码修正（json `CodeRules`）→ 池优先级 → 随机抽图。
//! 规则全部来自 `assets/standard2.json`（内置默认）与 `data/rules/standard2.json`（用户可编辑），
//! 本模块不硬编码任何目录名或码表。
use super::{filter_pools, pick_from_pools_flat, PickRule, PickedImage, Pools, RuleFile, SkipReason};
use crate::json_store;
use crate::rtf_scanner::PlayerRecord;
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
    /// 优先级目录列表：取第一个命中主/第二国籍目录的项
    #[serde(default)]
    pub prefer: Vec<String>,
    /// 都不命中时的兜底目录
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

impl Standard2Config {
    /// 结构校验（配置不变量的守卫），与标准三同口径：`RuleConfig::load` 与保存规则文件共用这一个入口。
    /// 【为什么必须拦】这里写错的目录名不会报错，只会让目标球员被静默跳过（`pick` 找不到该目录），
    /// 日志里只剩下「未抽到图 N 人」的汇总，定位成本极高。
    pub fn validate(&self) -> Result<(), String> {
        if self.folder_list.is_empty() {
            return Err("FolderList 为空：标准二至少需要一个种族目录".into());
        }
        let folders: BTreeSet<&str> = self.folder_list.iter().map(String::as_str).collect();
        if folders.len() != self.folder_list.len() {
            let mut seen = BTreeSet::new();
            let dup = self
                .folder_list
                .iter()
                .find(|d| !seen.insert(d.as_str()))
                .cloned()
                .unwrap_or_default();
            return Err(format!("FolderList 存在重复目录：{dup:?}"));
        }
        for (nat, dir) in &self.ethnics {
            if !folders.contains(dir.as_str()) {
                return Err(format!("Ethnics[{nat}] = {dir:?} 不在 FolderList 内"));
            }
        }
        for (code, rule) in &self.code_rules {
            let targets = rule.prefer.iter().chain(std::iter::once(&rule.fallback));
            for dir in targets {
                if !folders.contains(dir.as_str()) {
                    return Err(format!(
                        "CodeRules[{code}] 指向的目录 {dir:?} 不在 FolderList 内"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// 本标准的可编辑规则文件登记（命令层查表用）。
pub fn rule_file() -> RuleFile {
    RuleFile {
        name: "standard2",
        path: |data_dir: &Path| rule_path(data_dir),
        save: save_file,
    }
}

/// 用户可编辑规则文件路径：<data_dir>/rules/standard2.json。
fn rule_path(data_dir: &Path) -> PathBuf {
    data_dir.join("rules").join("standard2.json")
}

/// 解析 + 结构校验 + 原子写回：非法内容不写盘，错误信息直接回显给规则编辑对话框。
fn save_file(path: &Path, content: &str) -> Result<(), String> {
    let cfg: Standard2Config =
        serde_json::from_str(content).map_err(|e| format!("JSON 格式错误（标准二）：{e}"))?;
    cfg.validate()?;
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
            rule.prefer
                .iter()
                .find(|n| hit(n))
                .cloned()
                .unwrap_or_else(|| rule.fallback.clone())
        }
        None => e1.to_string(),
    }
}

/// 标准二抽图规则：池 = FolderList 过滤后的一级种族/国籍目录。
/// 池用有序 `Vec`（见 `templates::Pools`）：随机下标直取 O(1)、禁重复 O(1) 摘除。
pub struct Standard2Rule {
    pools: Pools,
    cfg: Standard2Config, // 构造时只拿自己这份配置，之后不再看其他标准
    /// 构造时的池内图数：区分「池本来就空」与「禁重复下被摘干」两种 None 成因。
    initial_total: usize,
    /// 最近一次 None 的成因（只在 pick 返回 None 后被读，初始值无意义）。
    last_skip: SkipReason,
    /// 目标目录不可用、改从全池回落取图的次数（stats 键 "target_dir_fallback"）。
    fallback_picks: usize,
}

impl Standard2Rule {
    /// 按配置的 FolderList 过滤出抽图池（清单外目录与空目录都不进池）。
    pub fn new(faces_map: &BTreeMap<String, BTreeSet<String>>, cfg: &Standard2Config) -> Self {
        let pools = filter_pools(faces_map, &cfg.folder_list);
        let initial_total = pools.values().map(|s| s.len()).sum();
        Self {
            pools,
            cfg: cfg.clone(),
            initial_total,
            last_skip: SkipReason::EmptyPack,
            fallback_picks: 0,
        }
    }

    /// 池优先级：国籍同名目录 > 种族目录。任一目录在池中不可用时由 `pick` 统一回落。
    fn image_pool_dirs(&self, ethnicity: &str, nationality: &str) -> Vec<String> {
        let mut pools = Vec::new();
        let nat_in = self.pools.get(nationality).map(|s| !s.is_empty()).unwrap_or(false);
        let eth_in = self.pools.get(ethnicity).map(|s| !s.is_empty()).unwrap_or(false);
        if nat_in { 
            pools.push(nationality.to_string()); 
        }
        if eth_in { 
            pools.push(ethnicity.to_string()); 
        }
        pools
    }

    /// 依序取第一个非空池随机选图；返回 (实际取图的目录, 词干)，禁重复时从池中移除。
    /// 【为什么连目录一起返回】候选不止一个目录，from 必须指向图真正所在的那个，
    /// 写死成别的目录会让 config.xml 指向不存在的路径。
    fn pick_image_from_pools(
        &mut self,
        pools: &[String],
        duplicates: bool,
    ) -> Option<(String, String)> {
        for dir in pools {
            let Some(pool) = self.pools.get_mut(dir) else {
                continue;
            };
            if pool.is_empty() {
                continue;
            }
            let idx = rand::rng().random_range(0..pool.len());
            // 下标直取 O(1)；禁重复就地 swap_remove 摘除。
            let stem = if duplicates {
                pool[idx].clone()
            } else {
                pool.swap_remove(idx)
            };
            return Some((dir.clone(), stem));
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
        // 14 种族目录只描述球员的种族/国籍映射；职员没有这层语义，必须显式跳过，
        // 不能拿职员的国籍去套球员目录（否则职员会被错误地分配种族脸）。
        if player.kind.is_staff() {
            self.last_skip = SkipReason::StaffNotApplicable;
            return None;
        }
        // 双国籍查表(json Ethnics) → 种族码修正(json CodeRules) → 目标目录选图 → 全池兜底。
        let e2 = if player.sec_nat.is_empty() {
            None
        } else {
            self.cfg.ethnics.get(&player.sec_nat).map(|s| s.as_str())
        };
        let Some(e1) = self.cfg.ethnics.get(&player.primary_nat) else {
            self.last_skip = SkipReason::NoNationRule;
            return None;
        };
        let ethnicity = apply_code_rule(&player.ethnicity_code, e1, e2, &self.cfg.code_rules);
        let dirs = self.image_pool_dirs(&ethnicity, &player.primary_nat);
        // 目标目录不可用（包里没建、或禁重复下已被抽空）时回落到全部目录，
        // 与标准三「候选 → 归属者 → 全池」同口径：包里还有图就给出映射，而不是静默跳过。
        // 回落范围仍在 FolderList 之内 —— 本标准的池只有清单内目录，包外目录进不来。
        let Some((dir, stem)) = self.pick_image_from_pools(&dirs, duplicates) else {
            let img = pick_from_pools_flat(&mut self.pools, duplicates);
            match &img {
                Some(_) => self.fallback_picks += 1,
                None => {
                    self.last_skip = if self.initial_total == 0 {
                        SkipReason::EmptyPack
                    } else {
                        SkipReason::PoolExhausted
                    };
                }
            }
            return img;
        };
        Some(PickedImage { dir, stem })
    }

    fn last_skip_reason(&self) -> SkipReason {
        self.last_skip
    }

    fn stats(&self) -> Vec<(&'static str, usize)> {
        vec![("target_dir_fallback", self.fallback_picks)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::RuleConfig;

    fn player(uid: &str, nat: &str, code: &str) -> PlayerRecord {
        PlayerRecord {
            uid: uid.into(),
            primary_nat: nat.into(),
            sec_nat: String::new(),
            name: "T".into(),
            hair_length: "1".into(),
            hair_color: "1".into(),
            ethnicity_code: code.into(),
            ..Default::default()
        }
    }

    fn sset(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn apply_code_rule_matches_legacy_correct_ethnic() {
        // 内置 CodeRules（数据驱动）逐分支覆盖 0–10 码的修正语义。
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
    fn standard2_maps_by_ethnic_and_reports_skip_reasons() {
        // 主题：标准二的映射与跳过成因。池过滤 / 命中 / 国籍无映射 / 目标目录缺失回落 /
        // 同名目录优先 / 空包 / 禁重复抽干，一条链走完。
        let faces_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("SpanMed".into(), sset(&["s1", "s2"])),
            ("Caucasian".into(), sset(&["c1"])),
            ("Extra".into(), sset(&["x1"])),   // json 未配置 → 不进池
            ("Empty".into(), BTreeSet::new()), // 空目录 → 不进池
        ]);
        // ① 池构造：FolderList 过滤 —— 多余目录与空目录都不进抽图池。
        let mut rule = Standard2Rule::new(&faces_map, &RuleConfig::builtin().standard2);
        assert!(!rule.pools.contains_key("Extra"));
        assert!(!rule.pools.contains_key("Empty"));
        // ② ESP 经内置 Ethnics → SpanMed，种族码 1 → SpanMed，随机取池内图。
        let img = rule.pick(&player("r-1", "ESP", "1"), true).unwrap();
        assert_eq!(img.dir, "SpanMed");
        assert!(["s1", "s2"].contains(&img.stem.as_str()));
        // ③ 未配置国籍（内置表查不到）→ 球员跳过，成因 = 国籍无映射。
        assert!(rule.pick(&player("r-2", "ZZZ", "1"), true).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::NoNationRule);
        // ④ 目标目录缺失（African 未建）→ 回落到池内其余目录，且对上层可见。
        let img = rule.pick(&player("r-3", "NGA", "3"), true).unwrap();
        assert!(
            ["SpanMed", "Caucasian"].contains(&img.dir.as_str()),
            "回落抽到了池外目录 {img:?}"
        );
        assert_eq!(rule.stats(), vec![("target_dir_fallback", 1)]);
        // ⑤ 包里还有与国籍同名的目录并写进 FolderList → 取图优先走它，from 必须记它。
        let mut cfg_same = RuleConfig::builtin().standard2;
        cfg_same.folder_list.push("ESP".into());
        let faces_same = BTreeMap::from([
            ("ESP".into(), sset(&["n1", "n2"])),
            ("SpanMed".into(), sset(&["s1"])),
        ]);
        let mut rule = Standard2Rule::new(&faces_same, &cfg_same);
        let img = rule.pick(&player("r-1", "ESP", "1"), true).unwrap();
        assert_eq!(img.dir, "ESP");
        assert!(["n1", "n2"].contains(&img.stem.as_str()));
        // ⑥ 池本来就空（清单内目录都没建/全空）→ EmptyPack。
        let empty: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut rule = Standard2Rule::new(&empty, &RuleConfig::builtin().standard2);
        assert!(rule.pick(&player("r-1", "ESP", "1"), true).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::EmptyPack);
        // ⑦ 池有图但禁重复摘干：SpanMed 仅一张，第二次目标目录已空、全池也空 → PoolExhausted。
        let only_one: BTreeMap<String, BTreeSet<String>> =
            BTreeMap::from([("SpanMed".into(), sset(&["s1"]))]);
        let mut rule = Standard2Rule::new(&only_one, &RuleConfig::builtin().standard2);
        assert!(rule.pick(&player("r-2", "ESP", "1"), false).is_some());
        assert!(rule.pick(&player("r-3", "ESP", "1"), false).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::PoolExhausted);
        // ⑧ 标准二只作用于球员：职员即便国籍/种族码齐全也必须跳过，且不能消耗图片池。
        let mut staff = player("s-1", "ESP", "1");
        staff.kind = crate::rtf_scanner::PersonKind::StaffMale;
        let mut rule = Standard2Rule::new(&faces_map, &RuleConfig::builtin().standard2);
        assert!(rule.pick(&staff, false).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::StaffNotApplicable);
        assert!(rule.pick(&player("r-1", "ESP", "1"), false).is_some());
    }

    #[test]
    fn standard2_config_validation_and_rule_file() {
        // 主题：标准二配置的校验与规则文件读写 —— 校验失败一律不写盘。
        // 出厂配置自身必须通过校验（否则每次替换都会在 load 处炸掉）。
        RuleConfig::builtin().standard2.validate().unwrap();
        // Ethnics 指向清单外目录 → 报错指名字段与值。
        let mut cfg = RuleConfig::builtin().standard2;
        cfg.ethnics.insert("XXX".into(), "Ghost".into());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("Ethnics[XXX]") && err.contains("Ghost"), "{err}");
        // CodeRules 的 prefer 与 fallback 都要在清单内。
        let mut cfg = RuleConfig::builtin().standard2;
        cfg.code_rules.insert(
            "3".into(),
            CodeRule { prefer: vec!["Ghost".into()], fallback: "African".into() },
        );
        assert!(cfg.validate().unwrap_err().contains("CodeRules[3]"));
        cfg.code_rules.insert(
            "3".into(),
            CodeRule { prefer: vec![], fallback: "Ghost".into() },
        );
        assert!(cfg.validate().unwrap_err().contains("CodeRules[3]"));
        // 空清单与重复目录。
        cfg.folder_list = Vec::new();
        assert!(cfg.validate().unwrap_err().contains("FolderList 为空"));
        cfg.folder_list = vec!["African".into(), "African".into()];
        assert!(cfg.validate().unwrap_err().contains("重复目录"));
        // save_file 拦下同一类错误：校验失败不写盘。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("rules/standard2.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let bad = "{\"FolderList\":[\"African\"],\"Ethnics\":{\"XXX\":\"Spain\"}}";
        assert!(save_file(&p, bad).is_err());
        assert!(!p.exists(), "校验失败不得写盘");
        // 登记表项：name 与路径遵循 standardN.json 主轴。
        // save 的「拒绝坏 json / 不写盘 / 合法内容回读」由 standard3_rule_file_registration 覆盖 ——
        // 两个标准共用同一张函数指针表，重复测一遍只是复制同一段机制。
        let rf = rule_file();
        assert_eq!(rf.name, "standard2");
        let data = tempfile::tempdir().unwrap();
        assert_eq!((rf.path)(data.path()).file_name().unwrap(), "standard2.json");
    }
}
