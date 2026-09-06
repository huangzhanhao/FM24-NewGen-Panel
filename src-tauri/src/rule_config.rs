//! 【新手导读】模板规则配置：每个头像包模板一份 json（template2.json / template3.json）。
//! 双层来源：编译期内嵌默认（assets/templateN.json，出厂规则，随版本更新）+
//! 用户可编辑文件（便携目录 data/rules/templateN.json，改完下次 Replace 即生效）。
//! 首次启动把内置默认导出为用户文件（已存在不覆盖）；加载时用户文件缺失→内置默认兜底，
//! 格式错误→报错中止（错误含文件路径，不静默回退，避免用户改坏后用错规则而不自知）。
use crate::assets;
use crate::json_store::{self, LoadError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// 模板二的 FM 种族码修正规则（码 → 候选目录优先级 + 兜底目录）。
/// 语义：prefer 列表中取第一个命中主/第二国籍种族目录的项；都不命中用 fallback；
/// 固定值码（如码 4 恒为 MESA）表达为 prefer 与 fallback 同值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeRule {
    #[serde(default)]
    pub prefer: Vec<String>,
    pub fallback: String,
}

/// 模板二（14 种族目录）规则配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Template2Config {
    #[serde(default)]
    pub folder_list: Vec<String>,                    // "FolderList"：目录清单（判定 + 池过滤基准）
    #[serde(default)]
    pub ethnics: BTreeMap<String, String>,           // "Ethnics"：国籍三字码 → 种族目录
    #[serde(default)]
    pub code_rules: BTreeMap<String, CodeRule>,      // "CodeRules"：FM 种族码修正
}

/// 模板三的 Mappings 两种形态（serde 自动识别，json 里写哪种都接受）：
/// 【Flat】国籍三字码 → 细分目录（简洁，模板三内置默认的形态）；
///         抽图检索时按 DefaultCodes 的码候选展开为三层检索表。
/// 【Detailed】国籍 → 种族码 → 候选目录列表（完整形态，同国籍不同码可指向不同目录池）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MappingsSpec {
    Detailed(BTreeMap<String, BTreeMap<String, Vec<String>>>),
    Flat(BTreeMap<String, String>),
}

impl Default for MappingsSpec {
    fn default() -> Self { MappingsSpec::Flat(BTreeMap::new()) }
}

impl MappingsSpec {
    /// 展开为三层检索表：Flat 形态按 DefaultCodes 的码候选展开（无码候选的国籍跳过，走兜底）。
    pub fn resolve(
        &self,
        default_codes: &BTreeMap<String, Vec<u8>>,
    ) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
        match self {
            MappingsSpec::Detailed(d) => d.clone(),
            MappingsSpec::Flat(flat) => {
                let mut out: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
                for (nat, dir) in flat {
                    let Some(codes) = default_codes.get(nat) else { continue };
                    let entry = out.entry(nat.clone()).or_default();
                    for c in codes {
                        entry.insert(c.to_string(), vec![dir.clone()]);
                    }
                }
                out
            }
        }
    }
}

/// 模板三（细分目录）规则配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Template3Config {
    #[serde(default)]
    pub folder_list: Vec<String>,                    // "FolderList"
    #[serde(default)]
    pub default_codes: BTreeMap<String, Vec<u8>>,    // "DefaultCodes"：国籍 → FM 种族码候选列表
    #[serde(default)]
    pub mappings: MappingsSpec,                      // "Mappings"：国籍 → 细分目录（Flat）或三层检索表（Detailed）
    #[serde(default)]
    pub fallback: Fallback,                          // "Fallback"：检索未命中时的兜底方式
}

/// 模板三检索未命中（国籍/种族码未配置、目录不在包内、池为空）时的兜底方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fallback {
    /// 从包内递归全池随机抽图（映射表收集期间保证可用）。
    #[default]
    Random,
    /// 跳过该球员（不生成映射）。
    Skip,
}

/// 全部模板规则的集合。
/// 【扩展点】新增模板四/五/六：①加 TemplateNConfig 字段 ②folder_lists() 登记一行
/// ③实现对应 PickRule 并登记 IMPLEMENTED_TEMPLATES。判定层（facepack::classify）零改动。
#[derive(Debug, Clone)]
pub struct RuleConfig {
    pub template2: Template2Config,
    pub template3: Template3Config,
}

impl RuleConfig {
    /// 内置出厂默认（编译期内嵌 assets/templateN.json）。
    pub fn builtin() -> Self {
        RuleConfig {
            template2: serde_json::from_str(assets::TEMPLATE2_RAW)
                .expect("内置 template2.json 格式错误"),
            template3: serde_json::from_str(assets::TEMPLATE3_RAW)
                .expect("内置 template3.json 格式错误"),
        }
    }

    /// 所有参与目录判定的模板 (标准号, FolderList)。新增模板时在此登记一行。
    pub fn folder_lists(&self) -> Vec<(u8, &Vec<String>)> {
        vec![(2, &self.template2.folder_list), (3, &self.template3.folder_list)]
    }

    /// 加载生效规则：用户文件存在则用之，缺失用内置默认，格式错误报错（含文件路径）。
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let mut cfg = Self::builtin();
        let dir = data_dir.join("rules");
        let t2 = dir.join("template2.json");
        match json_store::load::<Template2Config>(&t2) {
            Ok(user) => cfg.template2 = user,
            Err(LoadError::Missing) => {} // 无用户文件 → 内置默认
            Err(LoadError::Corrupt(e)) => {
                return Err(format!("模板二规则文件无法解析：{}（{e}）", t2.display()))
            }
        }
        let t3 = dir.join("template3.json");
        match json_store::load::<Template3Config>(&t3) {
            Ok(user) => cfg.template3 = user,
            Err(LoadError::Missing) => {}
            Err(LoadError::Corrupt(e)) => {
                return Err(format!("模板三规则文件无法解析：{}（{e}）", t3.display()))
            }
        }
        Ok(cfg)
    }
}

/// 首次运行导出用户可编辑的规则文件（已存在不覆盖，避免冲掉用户修改）。
pub fn ensure_user_files(data_dir: &Path) -> Result<(), String> {
    let dir = data_dir.join("rules");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建规则目录失败: {e}"))?;
    let cfg = RuleConfig::builtin();
    let t2 = dir.join("template2.json");
    if !t2.is_file() {
        json_store::save(&t2, &cfg.template2)?;
    }
    let t3 = dir.join("template3.json");
    if !t3.is_file() {
        json_store::save(&t3, &cfg.template3)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_has_expected_defaults() {
        let cfg = RuleConfig::builtin();
        // 模板二：14 目录 + 国籍映射 + 全部 0-10 码修正规则。
        assert_eq!(cfg.template2.folder_list.len(), 14);
        assert!(cfg.template2.folder_list.contains(&"YugoGreek".to_string()));
        assert_eq!(cfg.template2.ethnics["AFG"], "MESA");
        for code in ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"] {
            assert!(cfg.template2.code_rules.contains_key(code), "缺码 {code}");
        }
        // 模板三：50 目录清单 + DefaultCodes + Mappings（Flat：国籍→细分目录，240 条）+ Random 兜底。
        assert_eq!(cfg.template3.folder_list.len(), 50);
        assert!(!cfg.template3.folder_list.contains(&"SouthConeSA".to_string())); // 50 清单不含旧 65 表的 SouthConeSA
        assert!(cfg.template3.folder_list.contains(&"WestSlavic".to_string()));
        assert_eq!(cfg.template3.default_codes["ARG"], vec![1, 7]);
        assert_eq!(cfg.template3.default_codes["BRA"], vec![1, 3, 9]);
        match &cfg.template3.mappings {
            MappingsSpec::Flat(m) => {
                assert_eq!(m.len(), 240);
                assert_eq!(m["CHN"], "China");
                assert_eq!(m["ESP"], "Spain");
                // 展开校验：DefaultCodes 的码候选 × 目录 → 三层检索表。
                let resolved = cfg.template3.mappings.resolve(&cfg.template3.default_codes);
                assert_eq!(resolved["ARG"]["1"], vec!["Mestizo".to_string()]);
                assert_eq!(resolved["CHN"]["10"], vec!["China".to_string()]);
            }
            _ => panic!("内置 Mappings 应为 Flat 形态"),
        }
        assert_eq!(cfg.template3.fallback, Fallback::Random);
    }

    #[test]
    fn ensure_then_load_roundtrip_and_corrupt_reports_path() {
        let data = tempfile::tempdir().unwrap();
        // 首次运行：导出两个用户文件；load 结果与内置一致。
        ensure_user_files(data.path()).unwrap();
        assert!(data.path().join("rules/template2.json").is_file());
        assert!(data.path().join("rules/template3.json").is_file());
        let cfg = RuleConfig::load(data.path()).unwrap();
        assert_eq!(cfg.template2.ethnics["ARG"], "SAMed");
        assert_eq!(cfg.template3.folder_list.len(), 50);
        // 二次 ensure：已存在不覆盖（写个标记内容验证不被冲掉）。
        let t2 = data.path().join("rules/template2.json");
        std::fs::write(&t2, "{\"FolderList\":[\"African\"],\"Ethnics\":{\"XXL\":\"African\"}}").unwrap();
        ensure_user_files(data.path()).unwrap();
        let cfg = RuleConfig::load(data.path()).unwrap();
        assert_eq!(cfg.template2.folder_list, vec!["African".to_string()]);
        assert_eq!(cfg.template2.ethnics["XXL"], "African");
        // 改坏 json：load 报错且信息含文件路径（不静默回退）。
        std::fs::write(&t2, "{ broken").unwrap();
        let err = RuleConfig::load(data.path()).unwrap_err();
        assert!(err.contains("template2.json"), "错误应含文件路径: {err}");
        // 用户文件删除：回退内置默认。
        std::fs::remove_file(&t2).unwrap();
        let cfg = RuleConfig::load(data.path()).unwrap();
        assert_eq!(cfg.template2.folder_list.len(), 14);
    }

    #[test]
    fn folder_lists_registers_all_templates() {
        let cfg = RuleConfig::builtin();
        let lists = cfg.folder_lists();
        assert_eq!(lists.iter().map(|(t, _)| *t).collect::<Vec<_>>(), vec![2, 3]);
        assert!(lists.iter().all(|(_, f)| !f.is_empty()));
    }
}
