//! 标准三：头像包按细分目录组织（国籍级 + 族群级，如 Spain、China、Mestizo…）。
//! 抽图逻辑 = **规则块模型**：先按「国家 → 族群 → 全局默认」三级选出规则块，
//! 再取该块里球员种族码对应的那条规则，把规则解析成候选目录，最后从目录池里随机抽图。
//! 规则全部来自 `assets/standard3.json`（内置默认）与 `data/rules/standard3.json`（用户可编辑）。
//!
//! 规则块的键有三类：国籍三字码（国家级块）、目录名（族群/群组块）、`_DEFAULT_`（全局块）。
//! 每条规则有四种形态：`"default"`（归属者自己的目录）、`directly`（单一目录）、
//! `random`（等概率多选一）、`randomWithWeight`（按权重多选一）。
use super::{
    pick_from_dirs, refill_into, sorted_unique, PickRule, PickedImage, Pools, RuleFile, SkipReason,
};
use crate::json_store;
use crate::rtf_scanner::{PersonKind, PlayerRecord};
use crate::xml_parser::record_from;
use rand::Rng;
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 全局默认块的键名：任何国籍与族群块都未命中时的最后一级。
const DEFAULT_BLOCK: &str = "_DEFAULT_";

/// 标准三规则配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Standard3Config {
    #[serde(default)]
    pub folder_list: Vec<String>,                 // "FolderList"：图像池目录清单（判定 + 池过滤基准；含 `WF/x`、`Staff/r/male` 多级条目）
    #[serde(default)]
    pub ethnics: BTreeMap<String, String>,        // "Ethnics"：国籍三字码 → 本国基础池目录
    // ↓ 三张「目录类别映射表」：把男球员族裔目录换成本类目录。数据由 NG 四套出厂配置
    //   （base_config{,_female,_staff,_staff_female}.json）逐国对齐后生成，可选键、前端不可见。
    #[serde(default)]
    pub folder_mapping_female: BTreeMap<String, Vec<String>>, // 男族裔目录 → 女球员目录（WF/<ethnic>）
    #[serde(default)]
    pub folder_mapping_staff: BTreeMap<String, Vec<String>>,  // 男族裔目录 → 职员区域（Staff/<region>）
    #[serde(default)]
    pub folder_mapping_staff_female: BTreeMap<String, String>, // 男职员区域 → 女职员区域（仅 4 条覆盖）
    #[serde(default)]
    pub default_codes: BTreeMap<String, Vec<u8>>, // "DefaultCodes"：国籍 → 种族码候选（透传，抽图不使用，见 resolve 文档）
    #[serde(default)]
    pub ethnicity_mapping_rules: BTreeMap<String, BTreeMap<String, MappingRule>>, // "EthnicityMappingRules"：块键 → 种族码 → 规则
    #[serde(default)]
    pub version: u8,                              // "Version"：配置格式版本，必须为 4
    #[serde(default)]
    pub fallback: Fallback,                       // "Fallback"：目录不可用时的第二级兜底
}

/// 规则的四种形态（与规则编辑器里的四种文案一一对应）。
#[derive(Debug, Clone, PartialEq)]
pub enum MappingRule {
    /// 使用归属者自己的目录（国家块/族群块/全局块下都等于 `Ethnics[国籍]`）。
    Default,
    /// 直接映射到单一目录。
    Directly(String),
    /// 等概率随机映射到列表中的一个目录。
    Random(Vec<String>),
    /// 按权重随机映射（权重为整数百分比，但和不保证等于 100，一律按 Σ 归一）。
    Weighted {
        dirs: Vec<String>,
        weights: Vec<serde_json::Number>,
    },
}

impl Serialize for MappingRule {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // 逐形态写回原样结构，保证「读出 → 写回」不改变用户文件的语义与数值写法。
        #[derive(Serialize)]
        struct Direct<'a> {
            #[serde(rename = "typeOfMapping")]
            kind: &'a str,
            directory: &'a str,
        }
        #[derive(Serialize)]
        struct Multi<'a> {
            #[serde(rename = "typeOfMapping")]
            kind: &'a str,
            directory: &'a [String],
        }
        #[derive(Serialize)]
        struct WeightedDir<'a> {
            ethnicities: &'a [String],
            weights: &'a [serde_json::Number],
        }
        #[derive(Serialize)]
        struct Weighted<'a> {
            #[serde(rename = "typeOfMapping")]
            kind: &'a str,
            directory: WeightedDir<'a>,
        }
        match self {
            MappingRule::Default => s.serialize_str("default"),
            MappingRule::Directly(d) => Direct { kind: "directly", directory: d }.serialize(s),
            MappingRule::Random(v) => Multi { kind: "random", directory: v }.serialize(s),
            MappingRule::Weighted { dirs, weights } => Weighted {
                kind: "randomWithWeight",
                directory: WeightedDir { ethnicities: dirs, weights },
            }.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for MappingRule {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // 【为什么手写而不用 serde untagged】这份 json 就是用户在规则编辑对话框里直接改的对象，
        // untagged 失败时只报 "data did not match any variant"，用户无从下手；
        // 手写解析能说出「实际看到的 typeOfMapping 是什么、缺了哪个字段」，
        // 再配合 serde_json 自带的 line/column 定位，改坏的地方一眼可见。
        let v = Value::deserialize(d)?;
        match v {
            Value::String(s) if s == "default" => Ok(MappingRule::Default),
            Value::String(s) => Err(de::Error::custom(format!(
                "规则值应为 \"default\" 或带 typeOfMapping 的对象，实际是字符串 {s:?}"))),
            Value::Object(mut m) => {
                let kind = m.remove("typeOfMapping")
                    .and_then(|x| x.as_str().map(str::to_string))
                    .ok_or_else(|| de::Error::custom("规则对象缺少 typeOfMapping 字段"))?;
                let dir = m.remove("directory").ok_or_else(|| de::Error::custom(
                    format!("typeOfMapping={kind:?} 的规则缺少 directory 字段")))?;
                match kind.as_str() {
                    "directly" => dir.as_str().map(|x| MappingRule::Directly(x.to_string()))
                        .ok_or_else(|| de::Error::custom("directly 的 directory 应为单个目录名（字符串）")),
                    "random" => {
                        let list = dir.as_array().ok_or_else(|| de::Error::custom(
                            "random 的 directory 应为目录名数组"))?;
                        let mut out = Vec::with_capacity(list.len());
                        for item in list {
                            out.push(item.as_str().ok_or_else(|| de::Error::custom(
                                "random 的 directory 数组里存在非字符串项"))?.to_string());
                        }
                        Ok(MappingRule::Random(out))
                    }
                    "randomWithWeight" => {
                        let obj = dir.as_object().ok_or_else(|| de::Error::custom(
                            "randomWithWeight 的 directory 应为 {ethnicities, weights} 对象"))?;
                        let dirs = obj.get("ethnicities").and_then(Value::as_array)
                            .ok_or_else(|| de::Error::custom("randomWithWeight 缺少 directory.ethnicities 数组"))?;
                        let weights = obj.get("weights").and_then(Value::as_array)
                            .ok_or_else(|| de::Error::custom("randomWithWeight 缺少 directory.weights 数组"))?;
                        let mut ds = Vec::with_capacity(dirs.len());
                        for item in dirs {
                            ds.push(item.as_str().ok_or_else(|| de::Error::custom(
                                "ethnicities 数组里存在非字符串项"))?.to_string());
                        }
                        let mut ws = Vec::with_capacity(weights.len());
                        for item in weights {
                            ws.push(item.as_number().ok_or_else(|| de::Error::custom(
                                "weights 数组里存在非数值项"))?.clone());
                        }
                        Ok(MappingRule::Weighted { dirs: ds, weights: ws })
                    }
                    other => Err(de::Error::custom(format!(
                        "未知 typeOfMapping={other:?}（应为 directly / random / randomWithWeight，或整条规则写成 \"default\"）"))),
                }
            }
            _ => Err(de::Error::custom("规则值应为 \"default\" 字符串或带 typeOfMapping 的对象")),
        }
    }
}

/// 目录不可用（规则解析出的目录不在包内、或包内该目录为空）时的第二级兜底。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fallback {
    /// 从包内递归全池随机抽图（保证只要包里有图就能出映射）。
    #[default]
    Random,
    /// 跳过该球员（不生成映射）。
    Skip,
}

/// 解析后的候选目录形态。
#[derive(Debug, Clone, PartialEq)]
pub enum Candidates {
    Single(String),
    Uniform(Vec<String>),
    Weighted { dirs: Vec<String>, weights: Vec<f64> },
}

/// 一次 (国籍, 种族码) 的解析结果。
/// 两个字段都可能为 None：国籍不在 `Ethnics` 里、或选中的块没有该码 —— 都交给兜底链处理。
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    pub candidates: Option<Candidates>,
    /// 归属者自己的目录（`Ethnics[国籍]`）：既是 `"default"` 规则的解析结果，也是第一级回退目标。
    pub owner_folder: Option<String>,
}

impl Standard3Config {
    /// 三级链条的键序：① 国籍块 ② 族群块 ③ 全局默认块。
    /// 【为什么②不需要判断"是否属于某族群"】族群块的键就是目录名，而国籍块的键是三字码，
    /// 两者不可能相撞，所以直接拿 `Ethnics[国籍]` 去查表即可：命中就是族群块，没命中自然落到③。
    /// 【返回整条链而不是首个存在的块】缺码要能穿透，必须让调用方逐级试码；
    /// `_DEFAULT_` 块缺失由 `validate()` 拦下，故③恒存在。
    /// 【非数字码】如 NG 的 `_country_specific_default_`：查找时不消费（见 `skipped_codes`）。
    fn rules_chain(&self, nat: &str) -> Vec<&BTreeMap<String, MappingRule>> {
        let mut chain: Vec<&BTreeMap<String, MappingRule>> = Vec::new();
        if let Some(b) = self.ethnicity_mapping_rules.get(nat) {
            chain.push(b);
        }
        if let Some(base) = self.ethnics.get(nat) {
            if let Some(b) = self.ethnicity_mapping_rules.get(base) {
                chain.push(b);
            }
        }
        if let Some(b) = self.ethnicity_mapping_rules.get(DEFAULT_BLOCK) {
            chain.push(b);
        }
        chain
    }

    /// 解析 (国籍, 种族码) → 候选目录 + 归属者目录。
    /// 【`DefaultCodes` 不参与】抽图用的种族码来自 RTF 名单里球员自己的 `ethnicity_code`；
    /// `DefaultCodes` 是"该国 regen 可能落到哪些码"的清单，只在配置校验里用来对照，不做路由。
    pub fn resolve(&self, nat: &str, code: &str) -> Resolution {
        let owner_folder = self.ethnics.get(nat).cloned();
        // 三级查找按「块 × 码」一起匹配：**块存在但缺该码 = 未命中，继续往下一级**。
        // 【为什么不能先选定块再取码】NG 的真实规则表 33/50 个块缺码（`Rules[788]` 缺 9），
        // 先定块会让这些行停在第一级、退回归属者目录，而对手语义是穿透到 `_DEFAULT_["9"]`。
        let candidates = self.rules_chain(nat).into_iter().find_map(|block| {
            block
                .get(code)
                .and_then(|rule| to_candidates(rule, owner_folder.as_deref()))
        });
        Resolution { candidates, owner_folder }
    }

    /// 纯函数：按权重取候选下标（r ∈ [0,1)）。Σ 归一在函数内做，
    /// 因此权重和是 99 还是 100 都不影响分布（本仓库内置 json 里两种和都存在：99×4、100×7）。
    /// 【"99 或 100"只对内置 json 成立】NG 真实配置里 6 条加权规则的权重和是
    /// 1 / 3 / 8 / 9 / 10 五种（如 `Rules[373] = {Mestizo:9, NorthernSA:1}` 和为 10）。
    /// Σ 归一这条结论不受影响——它本来就不要求权重和等于任何定值。
    fn weighted_index(weights: &[f64], r: f64) -> usize {
        let sum: f64 = weights.iter().sum();
        if sum <= 0.0 || weights.is_empty() {
            return 0;
        }
        let mut rest = (r * sum).clamp(0.0, sum);
        for (i, w) in weights.iter().enumerate() {
            if rest < *w {
                return i;
            }
            rest -= *w;
        }
        weights.len() - 1 // 浮点累加误差兜底：落到最后一项
    }

    /// 规则表里那些**不是 `"0"`–`"10"` 的种族码**（NG 会写 `_country_specific_default_`）。
    /// 它们不参与三级查找；单列出来是为了让 `validate()` 的 WARN 可测试。
    pub(crate) fn skipped_codes(&self) -> Vec<(&String, &String)> {
        self.ethnicity_mapping_rules
            .iter()
            .flat_map(|(key, block)| block.keys().map(move |code| (key, code)))
            .filter(|(_, code)| !code.parse::<u8>().is_ok_and(|c| c <= 10))
            .collect()
    }

    /// 把「男球员族裔目录」换成指定类别的目标目录列表；映射表里没有 ⇒ 该类别无对应目录。
    /// 多值（如 `PacificIslanders` → `WF/Melanesia` / `WF/Polynesia` / `WF/African`）保留全部，
    /// 由 `Candidates::Uniform` 等概率选一 —— 与 NG 的 `random` 动作同构。
    /// 【女职员不跨性别取图】目标 `Staff/<region>/female` 缺失或为空时不回落男目录，
    /// 该球员宁可走兜底，也不拿异性头像充数。
    fn family_dirs(&self, male_dir: Option<&str>, family: PersonKind) -> Vec<String> {
        let Some(base) = male_dir else {
            return Vec::new();
        };
        let regions = match family {
            PersonKind::PlayerFemale => {
                return self.folder_mapping_female.get(base).cloned().unwrap_or_default()
            }
            // 男球员类别由三级规则链自己决定，不走映射表。
            PersonKind::PlayerMale => return Vec::new(),
            _ => match self.folder_mapping_staff.get(base) {
                Some(v) => v,
                None => return Vec::new(),
            },
        };
        let female = family == PersonKind::StaffFemale;
        regions
            .iter()
            .map(|r| {
                // 女职员区 ⊂ 男职员区（实测 14 ⊂ 18），少数区域要换区，其余沿用男区。
                let region = if female {
                    self.folder_mapping_staff_female.get(r).cloned().unwrap_or_else(|| r.clone())
                } else {
                    r.clone()
                };
                format!("{region}/{}", if female { "female" } else { "male" })
            })
            .collect()
    }

    /// 结构校验（配置不变量的守卫）。`RuleConfig::load` 与保存规则文件共用这一个入口。
    /// 【分工】解析错误由 `MappingRule::deserialize` 报（带 line/column），
    /// 这里只报"格式对但语义不成立"的问题，且错误信息一律指名块键与种族码。
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 4 {
            return Err(format!("仅支持 Version 4 的规则文件（当前 Version {}）", self.version));
        }
        if self.folder_list.is_empty() {
            return Err("FolderList 为空：标准三至少需要一个图像池目录".into());
        }
        let folders: BTreeSet<&str> = self.folder_list.iter().map(String::as_str).collect();
        if folders.len() != self.folder_list.len() {
            let mut seen = BTreeSet::new();
            let dup = self.folder_list.iter().find(|d| !seen.insert(d.as_str())).cloned().unwrap_or_default();
            return Err(format!("FolderList 存在重复目录：{dup:?}"));
        }
        for (nat, dir) in &self.ethnics {
            if !folders.contains(dir.as_str()) {
                return Err(format!("Ethnics[{nat}] = {dir:?} 不在 FolderList 内"));
            }
        }
        for nat in self.default_codes.keys() {
            if !self.ethnics.contains_key(nat) {
                return Err(format!("DefaultCodes[{nat}] 在 Ethnics 里没有对应国籍"));
            }
        }
        if !self.ethnicity_mapping_rules.contains_key(DEFAULT_BLOCK) {
            return Err(format!("缺少 {DEFAULT_BLOCK} 全局默认块：没有它的国籍将无法解析"));
        }
        // 目录类别映射表的值必须落在 FolderList 内：映射出去的目录若不在白名单里，
        // `usable()` 会恒为假、路由静默退化成兜底 —— 这种错配在配置期就能查出来。
        //
        // FolderMappingFemale 另需一道目录类别形态校验：`family_dirs` 对女球员原样返回映射值、
        // 不按目录形态过滤，所以目标指到男球员目录时照样能抽图 —— 男族裔目录也在 FolderList 内，
        // 上一条查不出来。后果是女球员拿到男脸却仍计入 wf_family，日志报「进了女球员类别」而实际没有。
        // 职员映射不需要这道关：下面把 `/male`、`/female` 拼进路径再查，后缀已被强制。
        for (src, dsts) in &self.folder_mapping_female {
            for d in dsts {
                let where_ = format!("FolderMappingFemale[{src}]");
                check_folder(&folders, d, &where_)?;
                if !d.starts_with("WF/") {
                    return Err(format!("{where_} = {d:?} 必须指向 WF/ 目录"));
                }
            }
        }
        for (src, regions) in &self.folder_mapping_staff {
            for r in regions {
                check_folder(&folders, &format!("{r}/male"), &format!("FolderMappingStaff[{src}]"))?;
            }
        }
        for (src, dst) in &self.folder_mapping_staff_female {
            let where_ = format!("FolderMappingStaffFemale[{src}]");
            let male_ok = self.folder_mapping_staff.values().flatten().any(|r| r == src);
            let female_ok = self.folder_mapping_staff.values().flatten().any(|r| r == dst);
            if !male_ok || !female_ok {
                return Err(format!("{where_} = {dst:?}：`{src}` 或 `{dst}` 不是 FolderMappingStaff 里出现过的职员区域"));
            }
        }
        // 非数字种族码（最典型的是 NG 的 `_country_specific_default_`）：跳过 + 告警，不报错。
        // 【为什么不报错】① 它挂在 UI 侧文案（`ruleActionUseDefault` / `ruleOptionUseCountryDefault`）下，
        // 不在 `reasoningApplyRule*` 那四种动作里（§8.4）；② 扰动⑥「先取它再穿透」在 123,447 行新产物上
        // 池覆盖 97.90%，反而**低于**同口径基线 99.69% —— 若引擎真会先取它，覆盖率应上升而非下降。
        // ⇒ 只告警、查找顺序不变；这些键取不到值，等价于"该码在这块里未命中"。
        for (key, code) in self.skipped_codes() {
            log::warn!(target: "facemap",
                "规则块 {key:?} 含非数字种族码 {code:?}，已跳过（不参与三级查找）");
        }
        for (key, block) in &self.ethnicity_mapping_rules {
            // 块键合法域：国籍三字码 / FolderList 里的目录名（族群块）/ _DEFAULT_。
            if key != DEFAULT_BLOCK && !self.ethnics.contains_key(key) && !folders.contains(key.as_str()) {
                return Err(format!("未知规则块键 {key:?}：既不是 Ethnics 里的国籍码，也不是 FolderList 里的目录名"));
            }
            // 块内缺某个种族码是**合法**的：缺码 = 未命中，继续往下一级（NG 出厂配置 50 块里 33 块缺码）。
            for (code, rule) in block {
                let where_ = format!("规则块 {key:?} 码 {code}");
                match rule {
                    // "default" 解析成 Ethnics[国籍]，上面已校验过其值在清单内。
                    MappingRule::Default => {}
                    MappingRule::Directly(d) => check_folder(&folders, d, &where_)?,
                    MappingRule::Random(v) => {
                        if v.is_empty() {
                            return Err(format!("{where_}：random 的目录列表为空"));
                        }
                        for d in v {
                            check_folder(&folders, d, &where_)?;
                        }
                    }
                    MappingRule::Weighted { dirs, weights } => {
                        if dirs.len() != weights.len() {
                            return Err(format!("{where_}：ethnicities 有 {} 项、weights 有 {} 项，两者必须等长",dirs.len(), weights.len()));
                        }
                        if dirs.is_empty() {
                            return Err(format!("{where_}：加权目录列表为空"));
                        }
                        for d in dirs {
                            check_folder(&folders, d, &where_)?;
                        }
                        let nums: Vec<f64> = weights.iter().filter_map(|n| n.as_f64()).collect();
                        if nums.len() != weights.len() {
                            return Err(format!("{where_}：weights 含无法当作数值读取的项"));
                        }
                        if nums.iter().any(|w| *w <= 0.0) {
                            return Err(format!("{where_}：weights 必须全部大于 0"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// 校验单个目录名在 FolderList 内。
fn check_folder(folders: &BTreeSet<&str>, dir: &str, where_: &str) -> Result<(), String> {
    if folders.contains(dir) {
        Ok(())
    } else {
        Err(format!("{where_}：目录 {dir:?} 不在 FolderList 内"))
    }
}

/// Preserve 剔除：按 from 全路径精确匹配清掉已占用项（不跨目录误伤同名图）。
/// 剩牌与已发牌两份都走这里 —— Deck 重洗会把已发牌洗回来，漏一份就是重复发图。
fn retain_unused(pools: &mut BTreeMap<String, Vec<String>>, used: &BTreeSet<String>) {
    for (dir, pool) in pools.iter_mut() {
        pool.retain(|s| !used.contains(&record_from(dir, s)));
    }
}

/// 目录类别只由 FolderList 条目的形态决定（无 `/` 名 / `WF/` / `Staff/…/male|female`）。
/// 返回记录共用的 `PersonKind`，避免兜底池和人员路由采用不同的类别集合。
fn family_of_dir(dir: &str) -> PersonKind {
    if dir.starts_with("WF/") {
        PersonKind::PlayerFemale
    } else if dir.starts_with("Staff/") {
        if dir.ends_with("/female") {
            PersonKind::StaffFemale
        } else {
            PersonKind::StaffMale
        }
    } else {
        PersonKind::PlayerMale
    }
}

/// 把目录列表折成候选形态：0 → 无候选；1 → Single；多个 → Uniform（等概率选一）。
fn shape_of(mut dirs: Vec<String>) -> Option<Candidates> {
    match dirs.len() {
        0 => None,
        1 => dirs.pop().map(Candidates::Single),
        _ => Some(Candidates::Uniform(dirs)),
    }
}

/// 把一条规则解析成候选目录形态；`owner` 是归属者目录（`"default"` 用它）。
fn to_candidates(rule: &MappingRule, owner: Option<&str>) -> Option<Candidates> {
    match rule {
        MappingRule::Default => owner.map(|f| Candidates::Single(f.to_string())),
        MappingRule::Directly(d) => Some(Candidates::Single(d.clone())),
        MappingRule::Random(v) if !v.is_empty() => Some(Candidates::Uniform(v.clone())),
        MappingRule::Weighted { dirs, weights } if !dirs.is_empty() => Some(Candidates::Weighted {
            dirs: dirs.clone(),
            weights: weights.iter().filter_map(|n| n.as_f64()).collect(),
        }),
        _ => None,
    }
}

/// 本标准的可编辑规则文件登记（命令层查表用）。
pub fn rule_file() -> RuleFile {
    RuleFile {
        name: "standard3",
        path: |data_dir: &Path| rule_path(data_dir),
        save: save_file,
    }
}

/// 用户可编辑规则文件路径：<data_dir>/rules/standard3.json。
fn rule_path(data_dir: &Path) -> PathBuf {
    data_dir.join("rules").join("standard3.json")
}

/// 解析 + 结构校验 + 原子写回：非法内容不写盘，错误信息直接回显给规则编辑对话框。
fn save_file(path: &Path, content: &str) -> Result<(), String> {
    let cfg: Standard3Config =
        serde_json::from_str(content).map_err(|e| format!("JSON 格式错误（标准三）：{e}"))?;
    cfg.validate()?;
    json_store::save(path, &cfg).map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

/// 标准三抽图规则：规则块模型。
///
/// 【为什么只有一份池】历史上这里有两份索引：`pools`（清单内目录）与 `recursive_pools`（递归全池），
/// 于是「禁重复摘图」必须两边都摘（drop_image），漏一处同一张图就会被发出两次；反过来兜底抽中的图
/// 也得回头从主池摘。现在只有**一份全池**，规则与兜底都从它取图 —— 摘一次即两边同时生效，
/// 同步问题在结构上就不存在了；顺带省掉构造时的整份克隆（28 万词条，约几十 MB）。
pub struct Standard3Rule {
    /// 唯一的图片存储：目录路径 → 该目录**剩余未发**的牌（建池时洗好，含清单内、清单外与根 ""）。
    pools: Pools,
    /// 与 `pools` 互补：目录 → 本轮**已发出**的牌。
    /// 【为什么要两份】Deck 一轮发完要原地重洗，洗的素材就是这里；摘一张只需动一处
    /// （`pools` 弹出 + `dealt` 入），于是 28 万词条的整份克隆全省掉了。
    /// 不变量：`pools[d]` ∪ `dealt[d]` 恒等于建池时 `d` 的全集，且两集合不相交。
    dealt: BTreeMap<String, Vec<String>>,
    /// 清单内（FolderList）、且构造时非空的目录 —— 规则候选目录只有这些算「可用」。
    /// 只存目录名不存图，所以包外目录（Scandinavian 等）天然进不了规则池。
    /// 多级条目（`WF/x`、`Staff/r/male`）由递归池判空，与一级目录名同口径。
    rule_dirs: BTreeSet<String>,
    /// `rule_dirs` 按类别预分：兜底只在本类的清单内目录里抽。
    /// 【为什么必须分类】全量 `rule_dirs` 含 `WF/*` 与 `Staff/*`，男球员的兜底会抽到
    /// 女足/女职员头像；NG 实测 172,494 条映射里 0 条越过本类（`WF` 一张都没用到）。
    fallback_dirs: BTreeMap<PersonKind, BTreeSet<String>>,
    cfg: Standard3Config, // 构造时只拿自己这份配置
    fallback: Fallback,
    /// 最近一次 None 的成因（只在 pick 返回 None 后被读，初始值无意义）。
    last_skip: SkipReason,
    /// 候选目录不可用、改从本国基础池取图的次数（stats 键 "owner_fallback"）。
    owner_fallbacks: usize,
    /// 候选与本国目录都不可用、经同类清单内目录兜底取图的次数（stats 键 "pool_fallback"）。
    pool_fallbacks: usize,
    /// 成功出图、且类别为**女球员（WF）**的记录数（stats 键 "wf_family"）。
    /// 【只统计成功出图】失败/跳过的人没进过任何目录，计入会把「多少人真的用上 WF 包」污染成
    /// 「多少人被判定为女球员」。男球员类别是默认类别、不计数（否则每批汇总都多一行噪音）。
    wf_hits: usize,
    /// 成功出图、且类别为**职员（Staff，男女合并）**的记录数（stats 键 "staff_family"）。
    /// 口径与 `wf_hits` 一致：只算成功出图，失败/跳过不计。
    staff_hits: usize,
}

impl Standard3Rule {
    /// 构造规则：以 `recursive_pools`（递归全池，含包外目录与根）为**唯一存储**，
    /// 按 `cfg.folder_list` 过滤出「规则可用目录」的集合；`cfg.fallback` 决定第二级兜底策略。
    /// 【为什么查递归池而不是一级目录名】FolderList 里有 `WF/x` 与 `Staff/r/male`
    /// 这类**多级**条目，一级目录名里根本没有它们；递归池的键就是完整相对路径，
    /// 与一级目录名同口径（一级目录在递归池里的键就是它自己）。
    pub fn new(
        recursive_pools: &BTreeMap<String, BTreeSet<String>>,
        cfg: &Standard3Config,
    ) -> Self {
        let mut pools: Pools = recursive_pools
            .iter()
            .map(|(dir, stems)| (dir.clone(), sorted_unique(stems.iter().cloned())))
        .collect();
        // 根键兜底：扫描侧恒会写入 "" 键（无根图时为空集），这里补一次是为了不依赖该前提。
        pools.entry(String::new()).or_default();
        // 建池即洗牌：一轮内按此顺序发，取牌走 `pop()`（等价伪代码里的 `cursor++`）。
        for pool in pools.values_mut() {
            rand::seq::SliceRandom::shuffle(&mut pool[..], &mut rand::rng());
        }
        let rule_dirs: BTreeSet<String> = cfg
            .folder_list
            .iter()
            .filter(|d| recursive_pools.get(*d).is_some_and(|s| !s.is_empty()))
            .cloned()
            .collect();
        // 兜底池按类别预分，键与人员记录共享同一个枚举，避免两侧分类次序漂移。
        let mut fallback_dirs: BTreeMap<PersonKind, BTreeSet<String>> = BTreeMap::new();
        for d in &rule_dirs {
            fallback_dirs.entry(family_of_dir(d)).or_default().insert(d.clone());
        }
        Self {
            pools,
            dealt: BTreeMap::new(),
            rule_dirs,
            fallback_dirs,
            fallback: cfg.fallback,
            cfg: cfg.clone(),
            last_skip: SkipReason::EmptyPack,
            owner_fallbacks: 0,
            pool_fallbacks: 0,
            wf_hits: 0,
            staff_hits: 0,
        }
    }

    /// 目录是否「可用」：在清单内，且**还有剩余牌 或 有已发牌可洗回**。
    /// 【为什么剩余牌空了仍算可用】Deck 一轮发完会把已发的牌洗回再发，所以「暂时没剩余」
    /// 不等于「给不出图」—— 判成不可用会让本轮后面的人白白丢脸。
    fn usable(&self, d: &str) -> bool {
        self.rule_dirs.contains(d)
            && (self.pools.get(d).is_some_and(|p| !p.is_empty())
                || self.dealt.get(d).is_some_and(|p| !p.is_empty()))
    }

    /// 按候选形态挑一个「包内存在且非空」的目录；加权形态会先裁掉不可用项再归一。
    fn choose_dir(&self, candidates: &Candidates) -> Option<String> {
        let usable = |d: &str| self.usable(d);
        match candidates {
            Candidates::Single(d) => usable(d).then(|| d.clone()),
            Candidates::Uniform(list) => {
                let avail: Vec<&String> = list.iter().filter(|d| usable(d)).collect();
                if avail.is_empty() {
                    return None;
                }
                let i = rand::rng().random_range(0..avail.len());
                Some(avail[i].clone())
            }
            Candidates::Weighted { dirs, weights } => {
                let mut ds: Vec<String> = Vec::new();
                let mut ws: Vec<f64> = Vec::new();
                for (d, w) in dirs.iter().zip(weights.iter()) {
                    if usable(d) {
                        ds.push(d.clone());
                        ws.push(*w);
                    }
                }
                if ds.is_empty() {
                    return None;
                }
                let r: f64 = rand::rng().random_range(0.0..1.0);
                Some(ds.swap_remove(Standard3Config::weighted_index(&ws, r)))
            }
        }
    }

    /// 从指定目录池取一张图；目录不可用返回 None。
    ///
    /// 【两种取样模型，按 `duplicates` 分】
    ///   - `duplicates == true`：有放回独立均匀，池内容不变（保持既有行为）。
    ///   - `duplicates == false`：**Deck** —— 建池时洗好，从尾部 `pop()` 顺序发；
    ///     发完把 `dealt` 洗回再发。于是同池内每张图的使用次数最多差 1，
    ///     且**永不因"池抽干"丢人**（NG 实测 172,494 行产物：94 个池无一例外）。
    /// 【为什么不引入 cursor 字段】`pop()` 从尾部取，剩牌堆本身就是游标；
    /// 多存一个下标就多一份要与池长度同步的状态。
    fn take_from(&mut self, dir: &str, duplicates: bool) -> Option<PickedImage> {
        // 清单外目录不得被规则取图（可用性判定与 choose_dir 同一处，owner_folder 路径也走这里）。
        if !self.usable(dir) {
            return None;
        }
        if duplicates {
            let pool = self.pools.get(dir)?;
            if pool.is_empty() {
                return None; // Preserve 剔除可能把目录清空，此时无图可发
            }
            let idx = rand::rng().random_range(0..pool.len());
            return Some(PickedImage { dir: dir.to_string(), stem: pool[idx].clone() });
        }
        let pool = self.pools.get_mut(dir)?;
        if pool.is_empty() {
            refill_into(pool, self.dealt.remove(dir).unwrap_or_default());
        }
        let stem = pool.pop()?;
        self.dealt.entry(dir.to_string()).or_default().push(stem.clone());
        Some(PickedImage {
            dir: dir.to_string(),
            stem,
        })
    }

    /// 按类别记一次「成功出图」（三个成功返回点各调一次，None 分支不调）。
    /// 男球员是默认类别、不计数；职员男女合并成一项 —— 汇总里关心的是"有没有走对类别"，
    /// 而不是再拆一层性别。
    fn note_family(&mut self, family: PersonKind) {
        match family {
            PersonKind::PlayerFemale => self.wf_hits += 1,
            PersonKind::StaffMale | PersonKind::StaffFemale => self.staff_hits += 1,
            PersonKind::PlayerMale => {}
        }
    }
}

impl PickRule for Standard3Rule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        // 按 from 全路径精确匹配，不跨目录误伤同名图。
        // 【剩牌与已发牌两份都要清】只清一份的话，被占用的图会在重洗后被二次发出。
        retain_unused(&mut self.pools, used);
        retain_unused(&mut self.dealt, used);
    }

    fn pick(&mut self, player: &PlayerRecord, duplicates: bool) -> Option<PickedImage> {
        // 不使用第二国籍：标准三的每条规则已经把 (国籍, 种族码) 完全决定了目标目录。
        let res = self.cfg.resolve(&player.primary_nat, &player.ethnicity_code);
        let family = player.kind;
        let (candidates, owner) = match family {
            // 男球员：三级规则链照旧 —— 规则块本身就是男球员口径。
            PersonKind::PlayerMale => (res.candidates.clone(), res.owner_folder.clone()),
            // 女球员 / 职员：规则块不适用，改走「男族裔目录 → 本类目录」的映射表。
            // 只做换类与过滤：三级查找的**顺序与层数都不变**，也不新增兜底层。
            family => {
                let dirs = self.cfg.family_dirs(res.owner_folder.as_deref(), family);
                (shape_of(dirs.clone()), dirs.into_iter().next())
            }
        };
        if let Some(candidates) = &candidates {
            if let Some(dir) = self.choose_dir(candidates) {
                if let Some(img) = self.take_from(&dir, duplicates) {
                    self.note_family(family);
                    return Some(img);
                }
            }
        }
        if let Some(owner) = &owner {
            if let Some(img) = self.take_from(owner, duplicates) {
                // 走到这里说明候选目录没出图（缺失/为空），属于第一级回落。
                self.owner_fallbacks += 1;
                self.note_family(family);
                return Some(img);
            }
        }
        match self.fallback {
            // 兜底仍只在 FolderList 内、且**只在本类目录**里抽：越过清单会抽到 `WF/*`（45,006 张）
            // 与 `Staff/*`、`Scandinavian`、包根图，男球员可能拿到女职员/女足头像。抽不到 ⇒ 视同空包。
            Fallback::Random => {
                let picked = self.fallback_dirs.get(&family).and_then(|allowed| {
                    pick_from_dirs(&mut self.pools, &mut self.dealt, allowed, duplicates)
                });
                match picked {
                    Some(img) => {
                        self.pool_fallbacks += 1;
                        self.note_family(family);
                        Some(img)
                    }
                    None => {
                        self.last_skip = SkipReason::EmptyPack;
                        None
                    }
                }
            }
            Fallback::Skip => {
                self.last_skip = SkipReason::RuleSkip;
                None
            }
        }
    }

    fn last_skip_reason(&self) -> SkipReason {
        self.last_skip
    }

    fn stats(&self) -> Vec<(&'static str, usize)> {
        vec![
            ("owner_fallback", self.owner_fallbacks),
            ("pool_fallback", self.pool_fallbacks),
            ("wf_family", self.wf_hits),
            ("staff_family", self.staff_hits),
        ]
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

    /// 带数据源类别的拓展记录，用于验证女球员与职员路由。
    fn detailed_player(uid: &str, nat: &str, code: &str, kind: PersonKind) -> PlayerRecord {
        let mut p = player(uid, nat, code);
        p.kind = kind;
        p.detailed = Some(crate::rtf_scanner::DetailedInfo::default());
        p
    }

    fn sset(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// 测试辅助：由一级目录图集构造递归池（根 + 各目录）。
    fn recursive(faces_map: &BTreeMap<String, BTreeSet<String>>) -> BTreeMap<String, BTreeSet<String>> {
        let mut pools = faces_map.clone();
        pools.entry(String::new()).or_default();
        pools
    }

    fn num(v: i64) -> serde_json::Number {
        serde_json::Number::from(v)
    }

    // ---------------- 解析（组 A） ----------------

    #[test]
    fn resolve_covers_chain_defaults_and_unknown_nationality() {
        // 主题：`resolve()` 的全部语义 —— 国家块优先 / 族群块 / 全局块 / 缺码穿透 /
        // "default" 回归属者 / 全局码 9 / 未知国籍无归属者。
        let cfg = RuleConfig::builtin().standard3;
        // ① 国家块优先于族群块：COL 的 Ethnics=Mestizo（族群块存在），但 COL 自己也有国家级块。
        assert_eq!(cfg.ethnics["COL"], "Mestizo");
        assert!(cfg.ethnicity_mapping_rules.contains_key("COL"));
        let mestizo = &cfg.ethnicity_mapping_rules["Mestizo"]["0"];
        let col = cfg.resolve("COL", "0");
        // 两块对码 0 的规则不同：COL 块是 random 三选一，Mestizo 块是 random 两选一。
        assert_ne!(&to_candidates(mestizo, Some("Mestizo")).unwrap(), col.candidates.as_ref().unwrap());
        assert_eq!(col.owner_folder.as_deref(), Some("Mestizo")); // 归属者仍是本国基础池
        // ② 族群块：JAM 无国家块，Ethnics=Afrocaribbean → 用族群块。
        assert!(!cfg.ethnicity_mapping_rules.contains_key("JAM"));
        assert_eq!(cfg.ethnics["JAM"], "Afrocaribbean");
        assert_eq!(cfg.resolve("JAM", "7").candidates, Some(Candidates::Single("IndigenousSA".into())));
        // ③ 全局块：CHN 既无国家块、Ethnics=China 也不是族群块键 → 落到 _DEFAULT_。
        assert!(!cfg.ethnicity_mapping_rules.contains_key("CHN"));
        let chn = cfg.resolve("CHN", "10");
        assert_eq!(chn.candidates, Some(Candidates::Single("China".into()))); // "default" → Ethnics[CHN]
        assert_eq!(chn.owner_folder.as_deref(), Some("China"));
        // ④ 块存在但缺该码：继续往下找 —— 对手 50 个块里 33 个缺码（`Rules[788]` 缺 9），
        // 先定块再取码会把这些行截断在第一级、退回归属者目录。
        let mut sparse = RuleConfig::builtin().standard3;
        let only_code_one: BTreeMap<String, MappingRule> =
            [("1".to_string(), MappingRule::Directly("Japan".into()))].into_iter().collect();
        sparse.ethnicity_mapping_rules.insert("ESP".into(), only_code_one);
        assert_eq!(sparse.resolve("ESP", "1").candidates, Some(Candidates::Single("Japan".into())));
        assert_eq!(sparse.resolve("ESP", "9").candidates, Some(Candidates::Single("MixedRace".into())));
        assert_eq!(sparse.resolve("ESP", "0").candidates, Some(Candidates::Single("Spain".into())));
        // ⑤ "default" 在三种块里都回归属者目录：国家块（ARM 码 1）、族群块（SouthConeSA 的某个
        // default 码）、全局块（CHN 码 0）。下面三条 `contains_key` / `ethnics[...]` 是**前置守卫**
        //（断言内置数据与分支前提，不是 resolve 的输出）：规则本身必须是 Default，
        // 否则"结果等于 owner"在 `directly → owner` 下同样成立，等于没测到 default 语义。
        assert!(cfg.ethnicity_mapping_rules.contains_key("ARM"));
        assert_eq!(cfg.ethnicity_mapping_rules["ARM"]["1"], MappingRule::Default);
        assert_eq!(cfg.resolve("ARM", "1").candidates, Some(Candidates::Single(cfg.ethnics["ARM"].clone())));
        assert_eq!(&cfg.ethnics["ARG"], "SouthConeSA");
        let default_code = cfg.ethnicity_mapping_rules["SouthConeSA"].iter()
            .find(|(_, r)| **r == MappingRule::Default).map(|(c, _)| c.clone()).unwrap();
        assert_eq!(cfg.resolve("ARG", &default_code).candidates, Some(Candidates::Single("SouthConeSA".into())));
        assert_eq!(cfg.ethnicity_mapping_rules["_DEFAULT_"]["0"], MappingRule::Default);
        assert_eq!(cfg.resolve("CHN", "0").candidates, Some(Candidates::Single("China".into())));
        // ⑥ 全局块唯一的跨国家硬编码：码 9 恒为 MixedRace；其余码回本国基础池。
        assert_eq!(cfg.resolve("CHN", "9").candidates, Some(Candidates::Single("MixedRace".into())));
        assert_eq!(cfg.resolve("JPN", "9").candidates, Some(Candidates::Single("MixedRace".into())));
        assert_eq!(cfg.resolve("JPN", "10").candidates, Some(Candidates::Single("Japan".into())));
        // ⑦ 未知国籍：没有归属者目录，全局块的 "default" 也解析不出候选。
        let unknown = cfg.resolve("ZZZ", "1");
        assert_eq!(unknown.owner_folder, None);
        assert_eq!(unknown.candidates, None);
    }

    #[test]
    fn mapping_rule_roundtrips_all_four_shapes() {
        let shapes = [
            (MappingRule::Default, "\"default\""),
            (MappingRule::Directly("Spain".into()),
             r#"{"typeOfMapping":"directly","directory":"Spain"}"#),
            (MappingRule::Random(vec!["Japan".into(), "China".into()]),
             r#"{"typeOfMapping":"random","directory":["Japan","China"]}"#),
            (MappingRule::Weighted { dirs: vec!["Mestizo".into(), "NorthernSA".into()], weights: vec![num(66), num(33)] },
             r#"{"typeOfMapping":"randomWithWeight","directory":{"ethnicities":["Mestizo","NorthernSA"],"weights":[66,33]}}"#),
        ];
        for (rule, json) in shapes {
            let parsed: MappingRule = serde_json::from_str(json).unwrap();
            assert_eq!(parsed, rule, "解析 {json} 应得到 {rule:?}");
            assert_eq!(serde_json::to_string(&rule).unwrap(), json, "写回应与原文一致");
        }
        // 错误信息指名道姓：未知 typeOfMapping、缺字段、字符串值不对。
        let bad = [
            (r#"{"typeOfMapping":"direct","directory":"Spain"}"#, "direct"),
            (r#"{"typeOfMapping":"directly"}"#, "directory"),
            (r#"{"directory":"Spain"}"#, "typeOfMapping"),
            ("\"defaults\"", "defaults"),
        ];
        for (json, needle) in bad {
            let err = serde_json::from_str::<MappingRule>(json).unwrap_err().to_string();
            assert!(err.contains(needle), "错误信息应含 {needle:?}：{err}");
        }
    }

    #[test]
    fn weighted_index_normalizes_by_sum() {
        // 权重和为 99（实测配置里存在）：分界在 66/99 处，而不是 66/100。
        let w = [66.0, 33.0];
        assert_eq!(Standard3Config::weighted_index(&w, 0.0), 0);
        assert_eq!(Standard3Config::weighted_index(&w, 0.5), 0); // 49.5 < 66
        assert_eq!(Standard3Config::weighted_index(&w, 66.0 / 99.0 - 1e-9), 0);
        assert_eq!(Standard3Config::weighted_index(&w, 66.0 / 99.0), 1); // 正好落在第二项
        assert_eq!(Standard3Config::weighted_index(&w, 0.999999), 1);
        // 单元素与全零权重：不 panic、恒取首项。
        assert_eq!(Standard3Config::weighted_index(&[5.0], 0.7), 0);
        assert_eq!(Standard3Config::weighted_index(&[0.0, 0.0], 0.5), 0);
        assert_eq!(Standard3Config::weighted_index(&[], 0.5), 0);
    }

    #[test]
    fn validate_rejects_broken_configs() {
        let base = || RuleConfig::builtin().standard3;
        // 内置默认本身必须通过校验。
        base().validate().unwrap();
        // ① 版本不对
        let mut c = base();
        c.version = 3;
        assert!(c.validate().unwrap_err().contains("Version"));
        // ② 未知块键（既非国籍码也非目录名）
        let mut c = base();
        c.ethnicity_mapping_rules.insert("_country_specific_default_".into(), BTreeMap::new());
        let err = c.validate().unwrap_err();
        assert!(err.contains("_country_specific_default_"), "{err}");
        // ③ 块缺码 ⇒ **放行**（缺码 = 未命中、继续下一级，与 NG 出厂配置同语义）。
        let mut c = base();
        c.ethnicity_mapping_rules.get_mut("ARM").unwrap().remove("5");
        c.validate().unwrap();
        // ③′ 非数字种族码（NG 的 `_country_specific_default_`）⇒ 跳过、不报错；三级查找顺序不变，
        // 故该键不会出现在任何解析结果里。WARN 走 `log`，单测没装 logger，这里只能验到
        // 「不报错 + `skipped_codes` 能列出该键」。
        let mut c = base();
        c.ethnicity_mapping_rules.get_mut("ARM").unwrap()
            .insert("_country_specific_default_".into(),
                    MappingRule::Directly("Anglosphere".into()));
        c.validate().unwrap();
        assert!(c.skipped_codes().iter()
            .any(|(k, v)| *k == "ARM" && *v == "_country_specific_default_"));
        // ④ 规则指向清单外目录
        let mut c = base();
        c.ethnicity_mapping_rules.get_mut("ARM").unwrap()
            .insert("0".into(), MappingRule::Directly("NotAFolder".into()));
        let err = c.validate().unwrap_err();
        assert!(err.contains("NotAFolder") && err.contains("FolderList"), "{err}");
        // ⑤ Ethnics 指向清单外目录
        let mut c = base();
        c.ethnics.insert("XXX".into(), "NotAFolder".into());
        assert!(c.validate().unwrap_err().contains("Ethnics[XXX]"));
        // ⑥ 加权两列不等长 / 含非正权重
        let mut c = base();
        c.ethnicity_mapping_rules.get_mut("DOM").unwrap().insert("1".into(), MappingRule::Weighted {
            dirs: vec!["Mestizo".into(), "NorthernSA".into()], weights: vec![num(66)],
        });
        assert!(c.validate().unwrap_err().contains("等长"));
        let mut c = base();
        c.ethnicity_mapping_rules.get_mut("DOM").unwrap().insert("1".into(), MappingRule::Weighted {
            dirs: vec!["Mestizo".into(), "NorthernSA".into()], weights: vec![num(66), num(0)],
        });
        assert!(c.validate().unwrap_err().contains("大于 0"));
        // ⑦ 缺全局默认块 / FolderList 为空或有重复
        let mut c = base();
        c.ethnicity_mapping_rules.remove(DEFAULT_BLOCK);
        assert!(c.validate().unwrap_err().contains(DEFAULT_BLOCK));
        let mut c = base();
        c.folder_list = vec![];
        assert!(c.validate().unwrap_err().contains("FolderList"));
        let mut c = base();
        c.folder_list.push(c.folder_list[0].clone());
        assert!(c.validate().unwrap_err().contains("重复"));
        // ⑧ 目录类别映射表：目标先查在册，再查目录类别形态。`Spain` 是男球员族裔目录（在册），
        // 却被指成女球员目标 —— 女球员会静默拿到男脸，必须在配置期拦下。
        let mut c = base();
        c.folder_mapping_female.insert("Spain".into(), vec!["NotAFolder".into()]);
        let err = c.validate().unwrap_err();
        assert!(
            err.contains("FolderMappingFemale[Spain]") && err.contains("FolderList"),
            "{err}"
        );
        let mut c = base();
        c.folder_mapping_female.insert("Spain".into(), vec!["Spain".into()]);
        let err = c.validate().unwrap_err();
        assert!(
            err.contains("FolderMappingFemale[Spain]") && err.contains("WF/"),
            "{err}"
        );
    }

    // ---------------- 抽图行为（组 B） ----------------

    #[test]
    fn builtin_rules_resolve_every_country_and_code() {
        // 全量回归：233 国 × 11 码 = 2563 组合，每一组都要解析出目录，且目录都在 FolderList 内。
        let cfg = RuleConfig::builtin().standard3;
        cfg.validate().unwrap();
        let folders: BTreeSet<&str> = cfg.folder_list.iter().map(String::as_str).collect();
        let mut total = 0usize;
        let mut kinds = BTreeMap::new();
        let mut targeted: BTreeSet<String> = BTreeSet::new();
        for nat in cfg.ethnics.keys() {
            for code in 0..=10u8 {
                total += 1;
                let r = cfg.resolve(nat, &code.to_string());
                let c = r.candidates.clone().unwrap_or_else(|| panic!("{nat}/{code} 无法解析"));
                match &c {
                    Candidates::Single(d) => {
                        *kinds.entry("single").or_insert(0) += 1;
                        targeted.insert(d.clone());
                    }
                    Candidates::Uniform(v) => {
                        *kinds.entry("uniform").or_insert(0) += 1;
                        targeted.extend(v.iter().cloned());
                    }
                    Candidates::Weighted { dirs, weights } => {
                        *kinds.entry("weighted").or_insert(0) += 1;
                        targeted.extend(dirs.iter().cloned());
                        assert_eq!(dirs.len(), weights.len(), "{nat}/{code} 加权两列不等长");
                    }
                }
                for d in match &c {
                    Candidates::Single(d) => vec![d.clone()],
                    Candidates::Uniform(v) => v.clone(),
                    Candidates::Weighted { dirs, .. } => dirs.clone(),
                } {
                    assert!(folders.contains(d.as_str()), "{nat}/{code} 指向清单外目录 {d}");
                }
            }
        }
        assert_eq!(total, 2563);
        assert_eq!(kinds["single"], 1833 + 591, "default 与 directly 都解析成单一目录");
        assert_eq!(kinds["uniform"], 128);
        assert_eq!(kinds["weighted"], 11);
        // 每个**男球员类别**目录都至少被一条规则指到（无死目录）。
        // `WF/*` 与 `Staff/*/*` 不由规则指向——规则块是男球员口径，那两类走目录类别映射表。
        let male_dirs: BTreeSet<String> = cfg
            .folder_list
            .iter()
            .filter(|d| !d.contains('/'))
            .cloned()
            .collect();
        assert_eq!(targeted, male_dirs, "存在从未被规则指向的男球员目录");
    }

    #[test]
    fn pick_walks_the_fallback_chain_then_skips() {
        // 主题：回退链与跳过成因。空包 → 本国基础池 → 兜底池 → Skip；未知国籍没有归属者，
        // 直接进兜底。stats 让每一级对上层可见。
        let mut cfg = RuleConfig::builtin().standard3;
        // ① 空包（包里一张图都没有）+ Random ⇒ EmptyPack，不是"禁重复抽干"。
        let empty: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut rule = Standard3Rule::new(&recursive(&empty), &cfg);
        assert!(rule.pick(&player("r-0", "ESP", "1"), true).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::EmptyPack);
        // ② 第一级回落：目标目录缺失、本国有图 ⇒ 用 Ethnics[ESP]。
        cfg.ethnicity_mapping_rules.entry("ESP".into()).or_default().insert("1".into(), MappingRule::Directly("NorthernSA".into()));
        let owner_pool: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("Spain".into(), sset(&["sp1"])),
            ("Anglosphere".into(), sset(&["an1"])),
        ]);
        let mut rule = Standard3Rule::new(&recursive(&owner_pool), &cfg);
        let img = rule.pick(&player("r-1", "ESP", "1"), true).unwrap();
        assert_eq!((img.dir.as_str(), img.stem.as_str()), ("Spain", "sp1"), "应回退到 Ethnics[ESP]");
        assert_eq!(rule.stats(), vec![
            ("owner_fallback", 1), ("pool_fallback", 0), ("wf_family", 0), ("staff_family", 0)]);
        // ③ 第二级兜底：本国基础池也没建 ⇒ 从兜底池抽（目录可以不是 Spain）。
        let pool_only: BTreeMap<String, BTreeSet<String>> =
            BTreeMap::from([("Anglosphere".into(), sset(&["an1"]))]);
        let mut rule = Standard3Rule::new(&recursive(&pool_only), &cfg);
        let img = rule.pick(&player("r-2", "ESP", "1"), true).unwrap();
        assert_eq!(img.dir, "Anglosphere");
        assert_eq!(rule.stats(), vec![
            ("owner_fallback", 0), ("pool_fallback", 1), ("wf_family", 0), ("staff_family", 0)]);
        // ④ 未知国籍：归属者未知 → 全局块的 "default" 也解析不出候选 → 直接兜底。
        let one: BTreeMap<String, BTreeSet<String>> =
            BTreeMap::from([("Spain".into(), sset(&["sp1"]))]);
        let mut rule = Standard3Rule::new(&recursive(&one), &cfg);
        assert_eq!(rule.pick(&player("r-3", "ZZZ", "1"), true).unwrap().dir, "Spain");
        // ⑤ Fallback::Skip：候选失效与未知国籍都不产出映射，成因 = 规则主动跳过。
        cfg.fallback = Fallback::Skip;
        let mut rule = Standard3Rule::new(&recursive(&pool_only), &cfg);
        assert!(rule.pick(&player("r-4", "ESP", "1"), true).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::RuleSkip);
        let mut rule = Standard3Rule::new(&recursive(&one), &cfg);
        assert!(rule.pick(&player("r-5", "ZZZ", "1"), true).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::RuleSkip);
    }

    #[test]
    fn sampling_models_cover_deck_and_iid() {
        // 主题：取样模型。`duplicates=false` 是 Deck（一轮不放回、全池用尽、发完原地重洗、
        // 不越类/不越清单）；`duplicates=true` 与 Deck 无关，仍是有放回独立均匀。
        let cfg = RuleConfig::builtin().standard3;
        // 陪跑池：清单内异类（WF / Staff）与清单外（Scandinavian）各一张 —— 类别口径失效就会被抽到。
        let pools: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("Spain".into(), sset(&["sp1", "sp2", "sp3", "sp4"])),
            ("WF/African".into(), sset(&["wf1"])),
            ("Staff/China/male".into(), sset(&["st1"])),
            ("Scandinavian".into(), sset(&["sc1"])),
        ]);
        let mut rule = Standard3Rule::new(&recursive(&pools), &cfg);
        // 一轮：四张全部用尽且互不重复（ESP 码 1 = "default" ⇒ 归属者目录 Spain）。
        let mut round1 = Vec::new();
        for i in 0..4 {
            let img = rule.pick(&player(&format!("r-{i}"), "ESP", "1"), false).unwrap();
            assert_eq!(img.dir, "Spain", "第 {i} 次越出本类/清单");
            round1.push(img.stem);
        }
        round1.sort();
        assert_eq!(round1, vec!["sp1", "sp2", "sp3", "sp4"], "一轮内不放回应恰好用尽全池");
        // 第二轮：原地重洗后再发满一轮，仍互不重复 —— 重洗是重开整副牌，不是"随便重复"。
        let mut round2 = Vec::new();
        for i in 0..4 {
            let img = rule
                .pick(&player(&format!("q-{i}"), "ESP", "1"), false)
                .unwrap_or_else(|| panic!("第 {i} 张应仍能发出"));
            assert_eq!(img.dir, "Spain");
            round2.push(img.stem);
        }
        round2.sort();
        assert_eq!(round2, vec!["sp1", "sp2", "sp3", "sp4"], "第二轮应复用同一批图、且仍不重复");
        // 第三轮第一张：仍来自第一轮那四张（重洗复用，永不因"池抽干"丢人）。
        let tail = rule.pick(&player("t-0", "ESP", "1"), false).unwrap();
        assert_eq!(tail.dir, "Spain");
        assert!(round1.contains(&tail.stem), "第三轮开出了不在第一轮的新图：{}", tail.stem);
        // duplicates=true：单图池连抽 8 次都拿到同一张（池内容不变）。
        let one: BTreeMap<String, BTreeSet<String>> =
            BTreeMap::from([("Spain".into(), sset(&["only1"]))]);
        let mut rule = Standard3Rule::new(&recursive(&one), &cfg);
        for i in 0..8 {
            let img = rule.pick(&player(&format!("i-{i}"), "ESP", "1"), true).unwrap();
            assert_eq!(img.stem, "only1");
        }
    }

    #[test]
    fn pool_state_covers_usable_and_exclude_used() {
        // 主题：池状态。可用性同时看剩余牌与已发牌；Preserve 剔除按 from 全路径清两份。
        let cfg = RuleConfig::builtin().standard3;
        let faces_map: BTreeMap<String, BTreeSet<String>> =
            BTreeMap::from([("Spain".into(), sset(&["sp1", "sp2"]))]);
        // ① 建池后：剩余牌非空 ⇒ 可用；清单外目录恒不可用。
        let mut rule = Standard3Rule::new(&recursive(&faces_map), &cfg);
        assert!(rule.usable("Spain"));
        assert!(!rule.usable("Scandinavian"), "清单外目录恒不可用");
        // ② 抽走两张后：剩余牌空但已发牌非空 ⇒ **仍可用**（一轮发完原地重洗）。
        assert!(rule.pick(&player("a", "ESP", "1"), false).is_some());
        assert!(rule.pick(&player("b", "ESP", "1"), false).is_some());
        assert!(rule.pools["Spain"].is_empty());
        assert_eq!(rule.dealt["Spain"].len(), 2);
        assert!(rule.usable("Spain"), "剩牌空但已发牌非空 ⇒ 仍可用");
        // ③ Preserve 剔除必须清两份：把已发牌里的两张都判为占用 ⇒ 目录彻底不可用、抽不出图。
        rule.exclude_used(&sset(&["Spain/sp1", "Spain/sp2"]));
        assert!(rule.pools["Spain"].is_empty() && rule.dealt["Spain"].is_empty());
        assert!(!rule.usable("Spain"));
        assert!(rule.pick(&player("c", "ESP", "1"), false).is_none());
        // ④ 只剔除一张（有放回 iid）：剩下一张被反复发出，被占用的那张永不出现。
        let mut rule = Standard3Rule::new(&recursive(&faces_map), &cfg);
        rule.exclude_used(&sset(&["Spain/sp1"]));
        for _ in 0..5 {
            assert_eq!(rule.pick(&player("r-1", "ESP", "1"), true).unwrap().stem, "sp2");
        }
    }

    #[test]
    fn fallback_random_never_leaves_the_folder_list() {
        // 主题：兜底边界。包里有清单外的图（WF / Staff / Scandinavian / 包根），
        // 且规则目标目录缺失 ⇒ 兜底只能在 FolderList 内的目录里抽。
        let mut cfg = RuleConfig::builtin().standard3;
        cfg.ethnicity_mapping_rules.entry("ESP".into()).or_default()
            .insert("1".into(), MappingRule::Directly("NorthernSA".into()));
        // 归属者目录也失效，兜底才会真正被触发（否则第二级 owner 回落先接住，兜底永远测不到）。
        cfg.ethnics.remove("ESP");
        let mut pools: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        pools.insert("Spain".into(), sset(&["sp1"]));   // 清单内、男球员类别
        pools.insert("Japan".into(), sset(&["jp1"]));   // 清单内、男球员类别
        pools.insert("WF/African".into(), sset(&["wf1"]));       // 清单内、女球员类别
        pools.insert("Staff/China/male".into(), sset(&["st1"]));   // 清单内、男职员类别
        pools.insert("Scandinavian".into(), sset(&["sc1"]));      // 清单外
        pools.insert(String::new(), sset(&["root1"]));          // 包根，清单外
        let mut rule = Standard3Rule::new(&pools, &cfg);
        let mut seen = BTreeSet::new();
        for i in 0..80 {
            let img = rule.pick(&player(&format!("r-{i}"), "ESP", "1"), true)
                .unwrap_or_else(|| panic!("清单内还有图，不应跳过"));
            seen.insert(img.dir);
        }
        // 男球员路径：兜底不得产出 WF/*、Staff/*、Scandinavian、包根图。
        assert_eq!(seen, sset(&["Spain", "Japan"]), "兜底越过了 FolderList 的类别口径");
        assert_eq!(rule.stats(), vec![
            ("owner_fallback", 0), ("pool_fallback", 80), ("wf_family", 0), ("staff_family", 0)]);
        // ④ 清单内所有目录都空 ⇒ 一律 EmptyPack 跳过（Q4 裁定：与空包同处置）。
        let mut drained: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        drained.insert("Spain".into(), BTreeSet::new());
        drained.insert("Japan".into(), BTreeSet::new());
        let mut rule = Standard3Rule::new(&drained, &cfg);
        assert!(rule.pick(&player("z", "ESP", "1"), true).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::EmptyPack);
    }

    #[test]
    fn family_routing_covers_female_and_staff_dirs() {
        // 主题：类别路由 + 类别命中计数。同一条 rule 连跑四类，最后核对计数口径：
        // 男球员是默认类别不计数，失败的 pick 也不计数。
        let mut cfg = RuleConfig::builtin().standard3;
        cfg.ethnics.insert("TST".into(), "Spain".into());
        cfg.folder_mapping_female.insert("Spain".into(), vec!["WF/Spain".into()]);
        // 职员区取 Armenian，好让④走到 FolderMappingStaffFemale 的覆盖表（Armenian → MiddleEast）。
        cfg.folder_mapping_staff.insert("Spain".into(), vec!["Staff/Armenian".into()]);
        cfg.validate().unwrap();

        let mut pools: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        pools.insert("Spain".into(), sset(&["sp1"]));
        pools.insert("WF/Spain".into(), sset(&["wf1"]));
        pools.insert("Staff/Armenian/male".into(), sset(&["stm1"]));
        pools.insert("Staff/MiddleEast/female".into(), sset(&["stf1"]));

        let mut rule = Standard3Rule::new(&recursive(&pools), &cfg);
        // ① 基础名单（detailed=None）与男球员 → 62 个族裔目录
        assert_eq!(rule.pick(&player("m1", "TST", "1"), true).unwrap().dir, "Spain");
        // ② 女球员 → WF 类别（男目录 Spain 经映射换成 WF/Spain）
        let img = rule.pick(&detailed_player("w1", "TST", "1", PersonKind::PlayerFemale), true).unwrap();
        assert_eq!(img.dir, "WF/Spain");
        // ③ 男职员 → Staff/<region>/male
        let img = rule.pick(&detailed_player("s1", "TST", "1", PersonKind::StaffMale), true).unwrap();
        assert_eq!(img.dir, "Staff/Armenian/male");
        // ④ 女职员：区域先过 FolderMappingStaffFemale 覆盖表（Armenian → MiddleEast）
        let img = rule.pick(&detailed_player("sf1", "TST", "1", PersonKind::StaffFemale), true).unwrap();
        assert_eq!(img.dir, "Staff/MiddleEast/female");
        // 4 次 pick 里男球员那次不计入类别计数 ⇒ 1 + 2 = 3，而不是 4。
        assert_eq!(rule.stats(), vec![
            ("owner_fallback", 0), ("pool_fallback", 0), ("wf_family", 1), ("staff_family", 2)]);
        // ⑤ 失败/跳过不计数：空包下女球员拿不到图，类别计数保持 0（口径是「成功出图」）。
        let nothing: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut rule = Standard3Rule::new(&recursive(&nothing), &cfg);
        assert!(rule.pick(&detailed_player("w9", "TST", "1", PersonKind::PlayerFemale), true).is_none());
        assert_eq!(rule.stats(), vec![
            ("owner_fallback", 0), ("pool_fallback", 0), ("wf_family", 0), ("staff_family", 0)]);
    }

    #[test]
    fn staff_rosters_from_real_rtfs_land_in_their_own_family() {
        // 真实名单解析与标准三选图必须共享同一类别；短名单包含职位和性别同时为空的未加载职员。
        let cfg = RuleConfig::builtin().standard3;
        let mut pools: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for d in &cfg.folder_list {
            pools.insert(d.clone(), sset(&["only"]));
        }
        for (fixture, expected_blank_rows) in [
            ("ext_staff_long_chn.rtf", None),
            ("ext_staff_short_chn.rtf", Some(37usize)),
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/testing_data").join(fixture);
            let (logger, _log_rx) = crate::logger::ChannelLogger::channel();
            let mut scanner = crate::rtf_scanner::RtfScanner::new();
            scanner.detect_format(&path, &logger)
                .unwrap_or_else(|e| panic!("{fixture} 的格式识别应通过：{e}"));
            let roster = scanner.parse_roster(&path, false, &logger)
                .unwrap_or_else(|e| panic!("{fixture} 应能解析：{e}"));
            assert!(!roster.is_empty(), "{fixture} 不应解析出 0 行");
            assert!(roster.iter().all(|p| p.kind.is_staff()),
                "{fixture} 的记录必须逐行保留职员类别");
            if let Some(expected) = expected_blank_rows {
                // 源文件的 398 个数据行里有一个两位 UID（45），按名单校验规则不会生成记录。
                assert_eq!(roster.len(), 397, "短名单应保留全部合法记录");
                let blank_rows = roster.iter().filter(|p| {
                    p.detailed.as_ref().is_some_and(|d|
                        d.gender.is_empty() && d.preferred_job.is_empty())
                }).count();
                assert_eq!(blank_rows, expected, "短名单的未加载职员行数");
            }
            let mut rule = Standard3Rule::new(&recursive(&pools), &cfg);
            let mut females = 0usize;
            for p in &roster {
                let img = rule.pick(p, true)
                    .unwrap_or_else(|| panic!("{fixture} 中 UID {} 没抽到图", p.uid));
                let want = if p.kind.is_female() {
                    females += 1;
                    "/female"
                } else {
                    "/male"
                };
                assert!(img.dir.starts_with("Staff/") && img.dir.ends_with(want),
                    "{fixture} 中 UID {} 应进 Staff/*{}，实际 {}", p.uid, want, img.dir);
            }
            assert!(females > 0, "{fixture} 应覆盖女职员目录分支");
        }
    }

    #[test]
    fn pick_weighted_rule_only_uses_available_dirs() {
        let cfg = RuleConfig::builtin().standard3;
        // 前置：DOM 码 1 必须确实是加权规则，否则下面的分布断言没有意义。
        assert!(matches!(cfg.ethnicity_mapping_rules["DOM"]["1"], MappingRule::Weighted { .. }));
        // DOM 码 1 = 加权 {Mestizo:66, NorthernSA:33}；包内只建 Mestizo → 必然落 Mestizo。
        let faces_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("Mestizo".into(), sset(&["m1", "m2"])),
            ("Spain".into(), sset(&["sp1"])),
        ]);
        let mut rule = Standard3Rule::new(&recursive(&faces_map), &cfg);
        for i in 0..8 {
            let img = rule.pick(&player(&format!("r-{i}"), "DOM", "1"), true).unwrap();
            assert_eq!(img.dir, "Mestizo", "NorthernSA 不在包内，加权应只剩 Mestizo");
        }
        // 两个目录都在时：两项都要可达，且占比必须贴近 66:33 —— 只断言"都出现过"的话，
        // 等概率实现照样能过，等于没测加权。400 次抽样下 0.55~0.78 的误报率约 1e-6。
        let faces_map2: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("Mestizo".into(), sset(&["m1"])),
            ("NorthernSA".into(), sset(&["n1"])),
        ]);
        let mut rule = Standard3Rule::new(&recursive(&faces_map2), &cfg);
        let mut seen = BTreeSet::new();
        let draws = 400usize;
        let mut mestizo = 0usize;
        for i in 0..draws {
            let dir = rule.pick(&player(&format!("r-{i}"), "DOM", "1"), true).unwrap().dir;
            if dir == "Mestizo" {
                mestizo += 1;
            }
            seen.insert(dir);
        }
        assert_eq!(seen, sset(&["Mestizo", "NorthernSA"]));
        let share = mestizo as f64 / draws as f64;
        assert!(
            (0.55..=0.78).contains(&share),
            "加权 66:33 的实测占比应接近 2/3，实际 {share:.3}（{mestizo}/{draws}）"
        );
    }

    // ---------------- 真实包形态（组 C）：126 目录超集包的完整链路 ----------------
    // 形态取自实测的社区标准三包：清单目录全在且各有几千张图，另多出三个包外目录——
    // Scandinavian（老头像包平铺残留）与 Staff/WF（嵌套子树）。专门覆盖手搓 faces_map 测不到的
    // 「扫描 → 判定 → 建池 → 抽图」全程，尤其是包外目录与递归池的关系。

    /// 在临时目录搭一个「真实包形态」的标准三包，返回写入的图片总数。
    fn build_real_shape_pack(
        root: &Path,
        cfg: &Standard3Config,
        imgs_per_dir: usize,
    ) -> std::io::Result<usize> {
        let mut files = 0usize;
        for d in &cfg.folder_list {
            let dir = root.join(d);
            std::fs::create_dir_all(&dir)?;
            // FolderList 里有多级条目（`WF/China`、`Staff/African/male`），文件名只能取末段，
            // 否则会把 "/" 写进文件名、新建出多余的子目录。
            let label = d.rsplit('/').next().unwrap_or(d);
            for i in 0..imgs_per_dir {
                std::fs::write(dir.join(format!("{label}_{i}.png")), b"png")?;
                files += 1;
            }
        }
        // 真实包多出的三个目录：Scandinavian 平铺有图；Staff/WF 各含一层嵌套子树。
        let scan_dir = root.join("Scandinavian");
        std::fs::create_dir_all(&scan_dir)?;
        std::fs::write(scan_dir.join("old_0.png"), b"png")?;
        std::fs::write(scan_dir.join("old_1.png"), b"png")?;
        files += 2;
        for (top, sub) in [("Staff", "African"), ("WF", "BrazilMixed")] {
            let dir = root.join(top).join(sub);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join(format!("{sub}.png")), b"png")?;
            files += 1;
        }
        Ok(files)
    }

    #[test]
    fn real_shape_pack_scans_classifies_and_picks_without_leaving_the_list() {
        // 主题：真实包形态的端到端链路 —— 扫描 → 判定 → 建池 → 233 国 × 11 码全量抽图。
        // 只有真实形态才暴露的关系：包外目录（Scandinavian）、嵌套子树（Staff/WF）、
        // 清单多级条目与一级目录名的不一致。
        let rcfg = RuleConfig::builtin();
        let cfg = &rcfg.standard3;
        let dir = tempfile::tempdir().unwrap();
        build_real_shape_pack(dir.path(), cfg, 2).unwrap();
        let scan = crate::facepack_scanner::scan_pack(dir.path().to_str().unwrap()).unwrap();
        // ① 一级目录 65 个：62 个男球员族裔目录 + Staff + WF + Scandinavian（真实包同款）。
        // 清单里的 `WF/*` 与 `Staff/*/*` 是一级目录的**子/孙**目录，不出现在一级目录名里。
        assert_eq!(scan.faces_map.len(), 65);
        let refs: Vec<_> = rcfg.folder_lists().iter().map(|(t, f)| (*t, *f)).collect();
        let layout = crate::facepack_scanner::classify_scan(&scan, &refs);
        // 清单 126 条（62 男球员 + 32 WF + 18 男职员 + 14 女职员）全在 → 判标准三完整包；
        // 多出的 Scandinavian 不在任何清单里，不影响判定。
        assert_eq!(layout, crate::facepack_scanner::PackLayout::Standard { standard: 3, complete: true });
        let mut rule = Standard3Rule::new(&scan.recursive_pools, cfg);
        // ② 规则可用目录只收清单内且非空的；包外目录不进规则池。
        assert_eq!(rule.rule_dirs.len(), cfg.folder_list.len());
        assert!(!rule.rule_dirs.contains("Scandinavian"));
        // ③ 唯一的全池仍收录包外目录与嵌套子树（Unknown 布局的抽样来源），根池键恒存在。
        assert!(rule.pools.contains_key("Scandinavian"));
        assert!(rule.pools.contains_key("Staff/African"));
        assert!(rule.pools.contains_key("WF/BrazilMixed"));
        assert!(rule.pools.contains_key(""));
        // ④ 233 国 × 11 码 = 2563 组全量抽图：清单内目录全非空 ⇒ 每次都直中候选目录
        //（不碰回退链），且抽中的必须是包里真实存在的文件（from 不带扩展名，逐个试扩展名）。
        let folders: BTreeSet<&str> = cfg.folder_list.iter().map(String::as_str).collect();
        for nat in cfg.ethnics.keys() {
            for code in 0..=10u8 {
                let res = cfg.resolve(nat, &code.to_string());
                let img = rule.pick(&player(&format!("{nat}/{code}"), nat, &code.to_string()), true)
                    .unwrap_or_else(|| panic!("{nat}/{code} 没抽到图"));
                let cands: Vec<&str> = match res.candidates.as_ref().unwrap() {
                    Candidates::Single(d) => vec![d.as_str()],
                    Candidates::Uniform(v) => v.iter().map(String::as_str).collect(),
                    Candidates::Weighted { dirs, .. } => dirs.iter().map(String::as_str).collect(),
                };
                assert!(cands.contains(&img.dir.as_str()),
                    "{nat}/{code} 抽到 {}/{}，候选只有 {cands:?}", img.dir, img.stem);
                assert!(folders.contains(img.dir.as_str()), "{nat}/{code} 抽到清单外目录");
                let base = dir.path().join(&img.dir).join(&img.stem);
                assert!(["png", "jpg", "jpeg"].iter().any(|e| std::path::Path::new(&format!("{}.{e}", base.display())).is_file()),
                    "{nat}/{code} → {}/{} 在包里不存在", img.dir, img.stem);
            }
        }
    }

    #[test]
    fn dotted_filename_has_single_identity_in_the_pool() {
        // 含点号文件名（"sp.1.png"）的词干是 "sp.1" 而非 "sp"：from 必须能定位到真实文件，
        // 且规则与兜底共用同一份池，同一张图不得被发出两次。
        let rcfg = RuleConfig::builtin();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Spain")).unwrap();
        std::fs::write(dir.path().join("Spain").join("sp.1.png"), b"png").unwrap();
        let scan = crate::facepack_scanner::scan_pack(dir.path().to_str().unwrap()).unwrap();
        // ① 禁重复 = Deck：一轮内不放回，池里只有这一张，所以第二轮重洗后仍发它（不丢人）。
        let mut rule = Standard3Rule::new(&scan.recursive_pools, &rcfg.standard3);
        let img = rule.pick(&player("r-1", "ESP", "1"), false).unwrap();
        assert_eq!((img.dir.as_str(), img.stem.as_str()), ("Spain", "sp.1"));
        let again = rule.pick(&player("r-2", "ESP", "1"), false).unwrap();
        assert_eq!(
            (again.dir.as_str(), again.stem.as_str()),
            ("Spain", "sp.1"),
            "单图池重洗后仍是同一张，词干必须带点号才能定位真实文件"
        );
        // ② Preserve 剔除：from "Spain/sp.1" 同时从剩余牌与已发牌里摘掉，抽不出第二张。
        let mut rule = Standard3Rule::new(&scan.recursive_pools, &rcfg.standard3);
        rule.exclude_used(&sset(&["Spain/sp.1"]));
        assert!(rule.pick(&player("r-3", "ESP", "1"), true).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::EmptyPack);
    }

    #[test]
    fn standard3_rule_file_registration() {
        let rf = rule_file();
        assert_eq!(rf.name, "standard3");
        let data = tempfile::tempdir().unwrap();
        let p = (rf.path)(data.path());
        assert_eq!(p.file_name().unwrap(), "standard3.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap(); // 真实流程里 rules/ 由 ensure_user_files 先建好
        assert!((rf.save)(&p, "{ broken").is_err());
        assert!(!p.exists(), "解析失败不得写盘");
        // 格式对但语义不成立（缺 Version、缺 _DEFAULT_）→ 同样拒绝写盘。
        assert!((rf.save)(&p, "{\"FolderList\":[\"China\"]}").is_err());
        assert!(!p.exists(), "校验失败不得写盘");
        // 合法内容 → 写盘并可读回。
        let ok = r#"{"FolderList":["China"],"Ethnics":{"CHN":"China"},"DefaultCodes":{"CHN":[10]},
            "EthnicityMappingRules":{"_DEFAULT_":{"0":"default","1":"default","2":"default","3":"default",
            "4":"default","5":"default","6":"default","7":"default","8":"default",
            "9":{"typeOfMapping":"directly","directory":"China"},"10":"default"}},"Version":4}"#;
        (rf.save)(&p, ok).unwrap();
        let back: Standard3Config = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(back.version, 4);
        assert_eq!(back.ethnics["CHN"], "China");
    }
}
