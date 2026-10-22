//! 标准三：头像包按细分目录组织（国籍级 + 族群级，如 Spain、China、Mestizo…）。
//! 抽图逻辑 = **规则块模型**：先按「国家 → 族群 → 全局默认」三级选出规则块，
//! 再取该块里球员种族码对应的那条规则，把规则解析成候选目录，最后从目录池里随机抽图。
//! 规则全部来自 `assets/standard3.json`（内置默认）与 `data/rules/standard3.json`（用户可编辑）。
//!
//! 规则块的键有三类：国籍三字码（国家级块）、目录名（族群/群组块）、`_DEFAULT_`（全局块）。
//! 每条规则有四种形态：`"default"`（归属者自己的目录）、`directly`（单一目录）、
//! `random`（等概率多选一）、`randomWithWeight`（按权重多选一）。
use super::{pick_from_pools_flat, sorted_unique, PickRule, PickedImage, Pools, RuleFile, SkipReason};
use crate::json_store;
use crate::rtf_scanner::PlayerRecord;
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
    pub folder_list: Vec<String>,                 // "FolderList"：图像池目录清单（判定 + 池过滤基准）
    #[serde(default)]
    pub ethnics: BTreeMap<String, String>,        // "Ethnics"：国籍三字码 → 本国基础池目录
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
    /// 选规则块，优先级从高到低：① 国籍块 ② 族群块 ③ 全局默认块。
    /// 【为什么②不需要判断"是否属于某族群"】族群块的键就是目录名，而国籍块的键是三字码，
    /// 两者不可能相撞，所以直接拿 `Ethnics[国籍]` 去查表即可：命中就是族群块，没命中自然落到③。
    fn rules_block(&self, nat: &str) -> Option<&BTreeMap<String, MappingRule>> {
        if let Some(b) = self.ethnicity_mapping_rules.get(nat) { return Some(b); }
        if let Some(base) = self.ethnics.get(nat) {
            if let Some(b) = self.ethnicity_mapping_rules.get(base) { return Some(b); }
        }
        self.ethnicity_mapping_rules.get(DEFAULT_BLOCK)
    }

    /// 解析 (国籍, 种族码) → 候选目录 + 归属者目录。
    /// 【`DefaultCodes` 不参与】抽图用的种族码来自 RTF 名单里球员自己的 `ethnicity_code`；
    /// `DefaultCodes` 是"该国 regen 可能落到哪些码"的清单，只在配置校验里用来对照，不做路由。
    pub fn resolve(&self, nat: &str, code: &str) -> Resolution {
        let owner_folder = self.ethnics.get(nat).cloned();
        let candidates = self.rules_block(nat)
            .and_then(|block| block.get(code))
            .and_then(|rule| to_candidates(rule, owner_folder.as_deref()));
        Resolution { candidates, owner_folder }
    }

    /// 纯函数：按权重取候选下标（r ∈ [0,1)）。Σ 归一在函数内做，
    /// 因此权重和是 99 还是 100 都不影响分布（实测配置里两种和都存在）。
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
        for (key, block) in &self.ethnicity_mapping_rules {
            // 块键合法域：国籍三字码 / FolderList 里的目录名（族群块）/ _DEFAULT_。
            if key != DEFAULT_BLOCK && !self.ethnics.contains_key(key) && !folders.contains(key.as_str()) {
                return Err(format!("未知规则块键 {key:?}：既不是 Ethnics 里的国籍码，也不是 FolderList 里的目录名"));
            }
            for code in 0..=10u8 {
                if !block.contains_key(&code.to_string()) {
                    return Err(format!("规则块 {key:?} 缺少种族码 {code}（每个块必须 0-10 全枚举）"));
                }
            }
            for code in block.keys() {
                if !code.parse::<u8>().is_ok_and(|c| c <= 10) {
                    return Err(format!("规则块 {key:?} 含非法种族码 {code:?}（应为 \"0\"-\"10\"）"));
                }
            }
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
    /// 唯一的图片存储：目录路径 → 词干池（含清单内、清单外与根 ""）。
    pools: Pools,
    /// 清单内（FolderList）、且构造时非空的目录名 —— 规则候选目录只有这些算「可用」。
    /// 只存目录名不存图，所以包外目录（Scandinavian / Staff/WF 之类）天然进不了规则池，
    /// 但它们的图仍在 `pools` 里，供 `Fallback::Random` 兜底。
    rule_dirs: BTreeSet<String>,
    cfg: Standard3Config, // 构造时只拿自己这份配置
    fallback: Fallback,
    /// 构造时的全池图数：区分「池本来就空」与「禁重复下被摘干」两种 None 成因。
    initial_total: usize,
    /// 最近一次 None 的成因（只在 pick 返回 None 后被读，初始值无意义）。
    last_skip: SkipReason,
    /// 候选目录不可用、改从本国基础池取图的次数（stats 键 "owner_fallback"）。
    owner_fallbacks: usize,
    /// 候选与本国目录都不可用、经全池兜底取图的次数（stats 键 "pool_fallback"）。
    pool_fallbacks: usize,
}

impl Standard3Rule {
    /// 构造规则：以 `recursive_pools`（递归全池，含包外目录与根）为**唯一存储**，
    /// 按 `cfg.folder_list` 过滤出「规则可用目录」的名字集合；`cfg.fallback` 决定第二级兜底策略。
    /// 【faces_map 只用来判名】扫描侧保证 `faces_map[dir]` 与 `recursive_pools[dir]` 逐字同键同值
    /// （主池直接取自递归池），所以拿哪一份判断「该目录非空」都等价；这里只为判定，不再复制图。
    pub fn new(
        faces_map: &BTreeMap<String, BTreeSet<String>>,
        recursive_pools: &BTreeMap<String, BTreeSet<String>>,
        cfg: &Standard3Config,
    ) -> Self {
        let mut pools: Pools = recursive_pools
            .iter()
            .map(|(dir, stems)| (dir.clone(), sorted_unique(stems.iter().cloned())))
            .collect();
        // 根键兜底：扫描侧恒会写入 "" 键（无根图时为空集），这里补一次是为了不依赖该前提。
        pools.entry(String::new()).or_default();
        let initial_total = pools.values().map(|s| s.len()).sum();
        let rule_dirs: BTreeSet<String> = cfg
            .folder_list
            .iter()
            .filter(|d| faces_map.get(*d).is_some_and(|s| !s.is_empty()))
            .cloned()
            .collect();
        Self {
            pools,
            rule_dirs,
            fallback: cfg.fallback,
            cfg: cfg.clone(),
            initial_total,
            last_skip: SkipReason::EmptyPack,
            owner_fallbacks: 0,
            pool_fallbacks: 0,
        }
    }

    /// 目录是否「可用」：在清单内、且池里还有图。规则只允许从这样的目录取图。
    fn usable(&self, d: &str) -> bool {
        self.rule_dirs.contains(d) && self.pools.get(d).is_some_and(|p| !p.is_empty())
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

    /// 从指定目录池随机取一张图；禁重复时把它从池中移除。目录不可用返回 None。
    fn take_from(&mut self, dir: &str, duplicates: bool) -> Option<PickedImage> {
        // 清单外目录不得被规则取图（可用性判定与 choose_dir 同一处，owner_folder 路径也走这里）。
        if !self.usable(dir) {
            return None;
        }
        let pool = self.pools.get_mut(dir)?;
        let idx = rand::rng().random_range(0..pool.len());
        // 下标直取 O(1)；禁重复就地 swap_remove 摘除。
        let stem = if duplicates {
            pool[idx].clone()
        } else {
            pool.swap_remove(idx)
        };
        Some(PickedImage {
            dir: dir.to_string(),
            stem,
        })
    }
}

impl PickRule for Standard3Rule {
    fn exclude_used(&mut self, used: &BTreeSet<String>) {
        // 按 from 全路径精确匹配，不跨目录误伤同名图。
        for (dir, pool) in self.pools.iter_mut() {
            pool.retain(|s| !used.contains(&record_from(dir, s)));
        }
    }

    fn pick(&mut self, player: &PlayerRecord, duplicates: bool) -> Option<PickedImage> {
        // 三级链条：规则解析出的候选目录 → 本国基础池 → Fallback。
        // 不使用第二国籍：标准三的每条规则已经把 (国籍, 种族码) 完全决定了目标目录。
        let res = self.cfg.resolve(&player.primary_nat, &player.ethnicity_code);
        if let Some(candidates) = &res.candidates {
            if let Some(dir) = self.choose_dir(candidates) {
                if let Some(img) = self.take_from(&dir, duplicates) {
                    return Some(img);
                }
            }
        }
        if let Some(owner) = &res.owner_folder {
            if let Some(img) = self.take_from(owner, duplicates) {
                // 走到这里说明候选目录没出图（缺失/为空/被抽空），属于第一级回落。
                self.owner_fallbacks += 1;
                return Some(img);
            }
        }
        match self.fallback {
            // 兜底从同一份全池抽：已摘除的图天然不会再发出，无需同步。
            Fallback::Random => {
                let img = pick_from_pools_flat(&mut self.pools, duplicates);
                match &img {
                    Some(_) => self.pool_fallbacks += 1,
                    None => {
                        self.last_skip = if self.initial_total == 0 {
                            SkipReason::EmptyPack
                        } else {
                            SkipReason::PoolExhausted
                        };
                    }
                }
                img
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
            detailed: None,
        }
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
    fn resolve_prefers_country_block_over_group_block() {
        let cfg = RuleConfig::builtin().standard3;
        // COL 的 Ethnics 是 Mestizo（族群块存在），但 COL 自己也有国家级块 → 国家块优先。
        assert_eq!(cfg.ethnics["COL"], "Mestizo");
        assert!(cfg.ethnicity_mapping_rules.contains_key("COL"));
        let mestizo = &cfg.ethnicity_mapping_rules["Mestizo"]["0"];
        let col = cfg.resolve("COL", "0");
        // 两块对码 0 的规则不同：COL 块是 random 三选一，Mestizo 块是 random 两选一。
        assert_ne!(&to_candidates(mestizo, Some("Mestizo")).unwrap(), col.candidates.as_ref().unwrap());
        assert_eq!(col.owner_folder.as_deref(), Some("Mestizo")); // 归属者仍是本国基础池
    }

    #[test]
    fn resolve_falls_through_to_group_block_then_default_block() {
        let cfg = RuleConfig::builtin().standard3;
        // ② 族群块：JAM 无国家块，Ethnics=Afrocaribbean → 用族群块。
        assert!(!cfg.ethnicity_mapping_rules.contains_key("JAM"));
        assert_eq!(cfg.ethnics["JAM"], "Afrocaribbean");
        let jam = cfg.resolve("JAM", "7");
        assert_eq!(jam.candidates, Some(Candidates::Single("IndigenousSA".into())));
        // ③ 全局块：CHN 既无国家块、Ethnics=China 也不是族群块键 → 落到 _DEFAULT_。
        assert!(!cfg.ethnicity_mapping_rules.contains_key("CHN"));
        let chn = cfg.resolve("CHN", "10");
        assert_eq!(chn.candidates, Some(Candidates::Single("China".into()))); // "default" → Ethnics[CHN]
        assert_eq!(chn.owner_folder.as_deref(), Some("China"));
    }

    #[test]
    fn resolve_default_means_owner_folder_in_every_block_kind() {
        let cfg = RuleConfig::builtin().standard3;
        // 国家块内的 "default"：ARM 码 1 → Ethnics[ARM]。
        assert!(cfg.ethnicity_mapping_rules.contains_key("ARM"));
        let arm = cfg.resolve("ARM", "1");
        assert_eq!(arm.candidates, Some(Candidates::Single(cfg.ethnics["ARM"].clone())));
        // 族群块内的 "default"：SouthConeSA 块含 default → 归属该族群国家的 Ethnics。
        let arg_nat = &cfg.ethnics["ARG"];
        assert_eq!(arg_nat, "SouthConeSA");
        let default_code = cfg.ethnicity_mapping_rules["SouthConeSA"].iter().find(|(_, r)| **r == MappingRule::Default).map(|(c, _)| c.clone()).unwrap();
        let arg = cfg.resolve("ARG", &default_code);
        assert_eq!(arg.candidates, Some(Candidates::Single("SouthConeSA".into())));
        // 全局块内的 "default"：CHN 码 0 → Ethnics[CHN]。
        assert_eq!(cfg.resolve("CHN", "0").candidates, Some(Candidates::Single("China".into())));
    }

    #[test]
    fn resolve_global_block_routes_code9_to_mixed_race() {
        let cfg = RuleConfig::builtin().standard3;
        // 全局块唯一的跨国家硬编码：码 9 恒为 MixedRace。
        assert_eq!(cfg.resolve("CHN", "9").candidates, Some(Candidates::Single("MixedRace".into())));
        assert_eq!(cfg.resolve("JPN", "9").candidates, Some(Candidates::Single("MixedRace".into())));
        // 其余码回本国基础池。
        assert_eq!(cfg.resolve("JPN", "10").candidates, Some(Candidates::Single("Japan".into())));
    }

    #[test]
    fn resolve_unknown_nationality_has_no_owner() {
        let cfg = RuleConfig::builtin().standard3;
        let r = cfg.resolve("ZZZ", "1");
        assert_eq!(r.owner_folder, None);
        // 仍能落到全局块（"default" 无归属者 → 候选为空）。
        assert_eq!(r.candidates, None);
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
        // ③ 块缺码
        let mut c = base();
        c.ethnicity_mapping_rules.get_mut("ARM").unwrap().remove("5");
        let err = c.validate().unwrap_err();
        assert!(err.contains("ARM") && err.contains("种族码 5"), "{err}");
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
                // 归属者目录恒等于 Ethnics[国籍]，且必在清单内。
                assert_eq!(r.owner_folder.as_deref(), Some(cfg.ethnics[nat].as_str()));
            }
        }
        assert_eq!(total, 2563);
        assert_eq!(kinds["single"], 1833 + 591, "default 与 directly 都解析成单一目录");
        assert_eq!(kinds["uniform"], 128);
        assert_eq!(kinds["weighted"], 11);
        // FolderList 里每个目录都至少被一条规则指到（无死目录）。
        assert_eq!(targeted.len(), folders.len(), "存在从未被规则指向的目录");
    }

    #[test]
    fn pick_falls_back_to_owner_folder_when_target_dir_missing() {
        let mut cfg = RuleConfig::builtin().standard3;
        // 人为把 ESP 码 1 指到包内不存在的目录，验证第一级回退用本国基础池。
        cfg.ethnicity_mapping_rules.entry("ESP".into()).or_default().insert("1".into(), MappingRule::Directly("NorthernSA".into()));
        let faces_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("Spain".into(), sset(&["sp1"])),
            ("Anglosphere".into(), sset(&["an1"])),
        ]);
        let mut rule = Standard3Rule::new(&faces_map, &recursive(&faces_map), &cfg);
        let img = rule.pick(&player("r-1", "ESP", "1"), true).unwrap();
        assert_eq!(img.dir, "Spain", "应回退到 Ethnics[ESP]");
        assert_eq!(img.stem, "sp1");
        // 第一级回落要对上层可见：stats 累计一次「候选目录不可用改走本国基础池」。
        assert_eq!(rule.stats(), vec![("owner_fallback", 1), ("pool_fallback", 0)]);
    }

    #[test]
    fn pick_second_level_fallback_random_and_skip() {
        let mut cfg = RuleConfig::builtin().standard3;
        // ESP 码 1 指向包内没有的目录，且本国基础池 Spain 也没建 → 只能走第二级。
        cfg.ethnicity_mapping_rules.entry("ESP".into()).or_default().insert("1".into(), MappingRule::Directly("NorthernSA".into()));
        let faces_map: BTreeMap<String, BTreeSet<String>> =
            BTreeMap::from([("Anglosphere".into(), sset(&["an1"]))]);
        // random：从递归全池抽到图（目录可以不是 Spain）；stats 累计一次全池兜底。
        let mut rule = Standard3Rule::new(&faces_map, &recursive(&faces_map), &cfg);
        let img = rule.pick(&player("r-1", "ESP", "1"), true).unwrap();
        assert_eq!(img.dir, "Anglosphere");
        assert_eq!(rule.stats(), vec![("owner_fallback", 0), ("pool_fallback", 1)]);
        // skip：直接不给映射，成因 = 规则主动跳过。
        cfg.fallback = Fallback::Skip;
        let mut rule = Standard3Rule::new(&faces_map, &recursive(&faces_map), &cfg);
        assert!(rule.pick(&player("r-2", "ESP", "1"), true).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::RuleSkip);
    }

    #[test]
    fn standard3_skip_reasons_distinguish_empty_pack_from_exhausted() {
        let cfg = RuleConfig::builtin().standard3;
        // 全池本来就空（包里一张图都没有）→ EmptyPack。
        let empty: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut rule = Standard3Rule::new(&empty, &recursive(&empty), &cfg);
        assert!(rule.pick(&player("r-1", "ESP", "1"), true).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::EmptyPack);
        // 池有图但禁重复摘干：Spain 仅一张，抽走后候选与本国目录同池已空、全池也空 → PoolExhausted。
        let faces_map: BTreeMap<String, BTreeSet<String>> =
            BTreeMap::from([("Spain".into(), sset(&["sp1"]))]);
        let mut rule = Standard3Rule::new(&faces_map, &recursive(&faces_map), &cfg);
        assert!(rule.pick(&player("r-2", "ESP", "1"), false).is_some());
        assert!(rule.pick(&player("r-3", "ESP", "1"), false).is_none());
        assert_eq!(rule.last_skip_reason(), SkipReason::PoolExhausted);
    }

    #[test]
    fn pick_unknown_nationality_goes_straight_to_fallback() {
        let cfg = RuleConfig::builtin().standard3;
        let faces_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([("Spain".into(), sset(&["sp1"])),]);
        // ZZZ 不在 Ethnics：归属者目录未知 → 全局块的 "default" 也解析不出候选 → 兜底。
        let mut rule = Standard3Rule::new(&faces_map, &recursive(&faces_map), &cfg);
        let img = rule.pick(&player("r-1", "ZZZ", "1"), true).unwrap();
        assert_eq!(img.dir, "Spain"); // 递归全池里只有这一张
        // skip 模式下同一球员不产出映射。
        let mut cfg = cfg.clone();
        cfg.fallback = Fallback::Skip;
        let mut rule = Standard3Rule::new(&faces_map, &recursive(&faces_map), &cfg);
        assert!(rule.pick(&player("r-2", "ZZZ", "1"), true).is_none());
    }

    #[test]
    fn pick_weighted_rule_only_uses_available_dirs() {
        let cfg = RuleConfig::builtin().standard3;
        // DOM 码 1 = 加权 {Mestizo:66, NorthernSA:33}；包内只建 Mestizo → 必然落 Mestizo。
        let faces_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("Mestizo".into(), sset(&["m1", "m2"])),
            ("Spain".into(), sset(&["sp1"])),
        ]);
        let mut rule = Standard3Rule::new(&faces_map, &recursive(&faces_map), &cfg);
        for i in 0..8 {
            let img = rule.pick(&player(&format!("r-{i}"), "DOM", "1"), true).unwrap();
            assert_eq!(img.dir, "Mestizo", "NorthernSA 不在包内，加权应只剩 Mestizo");
        }
        // 两个目录都在时，加权抽样必须落在候选集内（大样本覆盖两项）。
        let faces_map2: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
            ("Mestizo".into(), sset(&["m1"])),
            ("NorthernSA".into(), sset(&["n1"])),
        ]);
        let mut rule = Standard3Rule::new(&faces_map2, &recursive(&faces_map2), &cfg);
        let mut seen = BTreeSet::new();
        for i in 0..60 {
            seen.insert(rule.pick(&player(&format!("r-{i}"), "DOM", "1"), true).unwrap().dir);
        }
        assert_eq!(seen, sset(&["Mestizo", "NorthernSA"]));
        assert!(matches!(cfg.ethnicity_mapping_rules["DOM"]["1"], MappingRule::Weighted { .. }));
    }

    #[test]
    fn exclude_used_removes_occupied_images() {
        // Preserve 禁重复：已占用的图按 from 全路径从全池剔除（池耗尽语义由真实包形态组覆盖）。
        let cfg = RuleConfig::builtin().standard3;
        let faces_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([("Spain".into(), sset(&["sp1", "sp2"]))]);
        let mut rule = Standard3Rule::new(&faces_map, &recursive(&faces_map), &cfg);
        rule.exclude_used(&sset(&["Spain/sp1"]));
        for _ in 0..5 {
            assert_eq!(rule.pick(&player("r-1", "ESP", "1"), true).unwrap().stem, "sp2");
        }
    }

    // ---------------- 真实包形态（组 C）：62 目录超集包的完整链路 ----------------
    // 形态取自实测的社区标准三包：62 个清单目录全在且各有几千张图，另多出三个包外目录——
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
            for i in 0..imgs_per_dir {
                std::fs::write(dir.join(format!("{d}_{i}.png")), b"png")?;
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
    fn real_shape_pack_classifies_standard3_and_isolates_extras() {
        let rcfg = RuleConfig::builtin();
        let cfg = &rcfg.standard3;
        let dir = tempfile::tempdir().unwrap();
        build_real_shape_pack(dir.path(), cfg, 1).unwrap();
        let scan = crate::facepack_scanner::scan_pack(dir.path().to_str().unwrap()).unwrap();
        // 一级目录 65 个：62 清单 + 3 包外（真实包同款）。
        assert_eq!(scan.faces_map.len(), cfg.folder_list.len() + 3);
        let refs: Vec<_> = rcfg.folder_lists().iter().map(|(t, f)| (*t, *f)).collect();
        let layout = crate::facepack_scanner::classify(
            &scan.faces_map.keys().cloned().collect::<BTreeSet<_>>(), false, &refs);
        // 清单 62 目录全在 → 判标准三完整包；多出的 Staff/WF/Scandinavian 不影响判定。
        assert_eq!(layout, crate::facepack_scanner::PackLayout::Standard { standard: 3, complete: true });
        let rule = Standard3Rule::new(&scan.faces_map, &scan.recursive_pools, cfg);
        // 规则可用目录只收清单内且非空的：包外目录不进规则池（含与清单目录同在一级的 Scandinavian）。
        assert_eq!(rule.rule_dirs.len(), cfg.folder_list.len());
        assert!(["Scandinavian", "Staff", "WF"].iter().all(|d| !rule.rule_dirs.contains(*d)));
        // 唯一的全池仍收录包外目录与嵌套子树（Fallback::Random 的兜底来源），根池键恒存在。
        assert!(rule.pools.contains_key("Scandinavian"));
        assert!(rule.pools.contains_key("Staff/African"));
        assert!(rule.pools.contains_key("WF/BrazilMixed"));
        assert!(rule.pools.contains_key(""));
    }

    #[test]
    fn real_shape_full_coverage_pick_hits_candidate_dirs_and_real_files() {
        // 真实跑法第一道回归：扫描真实形态的包后，233 国 × 11 码 = 2563 组全量抽图。
        // 62 个目录全部非空 → 每次都必须直中规则解析出的候选目录（不碰回退链），
        // 且抽中的必须是包里真实存在的文件（from 不带扩展名，逐个试图片扩展名）。
        let rcfg = RuleConfig::builtin();
        let cfg = &rcfg.standard3;
        let dir = tempfile::tempdir().unwrap();
        build_real_shape_pack(dir.path(), cfg, 2).unwrap();
        let scan = crate::facepack_scanner::scan_pack(dir.path().to_str().unwrap()).unwrap();
        let mut rule = Standard3Rule::new(&scan.faces_map, &scan.recursive_pools, cfg);
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
    fn real_shape_no_duplicates_exhausts_every_image_once() {
        // 每目录 1 张 + Scandinavian 2 张 + 两棵嵌套子树各 1 张。
        // 禁重复持续抽：规则候选抽空 → 归属者目录（同池，也空）→ 递归兜底收尾包外目录的图，
        // 必须恰好抽完全部图片、无一张重复，耗尽后稳定返回 None。
        let rcfg = RuleConfig::builtin();
        let cfg = &rcfg.standard3;
        let dir = tempfile::tempdir().unwrap();
        let total = build_real_shape_pack(dir.path(), cfg, 1).unwrap();
        let scan = crate::facepack_scanner::scan_pack(dir.path().to_str().unwrap()).unwrap();
        let mut rule = Standard3Rule::new(&scan.faces_map, &scan.recursive_pools, cfg);
        let nats: Vec<&String> = cfg.ethnics.keys().collect();
        let mut issued = BTreeSet::new();
        for i in 0..total + 10 {
            let nat = nats[i % nats.len()];
            let code = ((i / nats.len()) % 11).to_string();
            match rule.pick(&player(&format!("p-{i}"), nat, &code), false) {
                Some(img) => assert!(issued.insert(record_from(&img.dir, &img.stem)),
                    "第 {i} 次抽到重复图 {img:?}"),
                None => break,
            }
        }
        assert_eq!(issued.len(), total, "禁重复应恰好抽完全部 {total} 张图（含包外目录）");
        // 耗尽后继续抽仍为 None（不 panic、不复活已抽走的图）。
        assert!(rule.pick(&player("tail", "ESP", "1"), false).is_none());
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
        // ① 禁重复连抽两次：第一次拿走 "sp.1"，第二次池已摘空 → None。
        let mut rule = Standard3Rule::new(&scan.faces_map, &scan.recursive_pools, &rcfg.standard3);
        let img = rule.pick(&player("r-1", "ESP", "1"), false).unwrap();
        assert_eq!((img.dir.as_str(), img.stem.as_str()), ("Spain", "sp.1"));
        assert!(
            rule.pick(&player("r-2", "ESP", "1"), false).is_none(),
            "同一张图不得被第二次发出"
        );
        // ② Preserve 剔除：from "Spain/sp.1" 命中池中剩余的唯一一项，抽不出第二张。
        let mut rule = Standard3Rule::new(&scan.faces_map, &scan.recursive_pools, &rcfg.standard3);
        rule.exclude_used(&sset(&["Spain/sp.1"]));
        assert!(rule.pick(&player("r-3", "ESP", "1"), true).is_none());
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
