//! 头像包「标准」的抽图模型总入口：**一个标准 = 一个自包含模块**。
//! 各 standardN.rs 自带规则结构体、抽图算法、单测，标准之间互不 import ——
//! 每套头像包目录组织不同，抽图逻辑天然不同，强行共享只会让改一处影响另一处。
//! 本文件只放跨标准共享的部分：
//!   1. `PickRule` trait 与 `PickedImage`：统一接口与返回值；
//!   2. `for_layout`：唯一分发入口（按扫描出的布局挑规则）；
//!   3. `RuleConfig`：规则聚合与加载（内置默认 + 用户可编辑 json）；
//!   4. `filter_pools` / `pick_from_dirs` / `pick_from_pools_flat` / `sorted_unique`：纯机械工具；
//!   5. `validate_registry`：标准间登记冲突守卫。
//!
//! 布局判定在 `facepack_scanner::classify`，本模块不重复实现 —— 判定与分发各只有一处真源。
//!
//! 【扩展点】新增标准：①`assets/standardN.json` ②`templates/standardN.rs`
//! （自带 schema + validate + 规则 + 单测，只 import 本文件的 trait 与机械工具）
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

/// 当前已实现抽图规则的标准号；replace_flow 据此拦截清单外的标准。
/// 标准一（平铺）与空包/无法识别布局不走 `PackLayout::Standard`，不经过此拦截。
pub const IMPLEMENTED_STANDARDS: [u8; 2] = [2, 3];

/// 一次抽图结果：图片相对头像包根的目录（"" = 根，多级 "子目录/孙目录"）+ 文件词干。
/// 写 config.xml 时 dir + stem 经 record_from 拼成 from。
#[derive(Debug)]
pub struct PickedImage {
    pub dir: String,
    pub stem: String,
}

/// 一名球员没抽到图（`pick` 返回 None）的成因：face_mapper 按它分组记日志。
/// 不同成因的处置完全不同（改规则文件 / 改兜底策略 / 常规结局 / 换头像包），
/// 混在一句固定文案里只会把排查的人带偏。
/// 【目标目录缺失或为空不在其中】那已不再导致跳过：规则会先回落到他处取图，
/// 由 `stats()` 的回落计数承载（见 face_mapper 的汇总日志）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkipReason {
    /// 国籍不在规则的 Ethnics 表内（规则的国籍映射缺失）。
    NoNationRule,
    /// 规则主动放弃该球员（标准三 Fallback::Skip）。
    RuleSkip,
    /// 禁重复模式下图片池被抽干（包里有图、但已发完）。
    PoolExhausted,
    /// 池内一张图都没有（空包 / 该标准口径下无可用目录）。
    EmptyPack,
}

/// 抽图规则统一接口：每个标准一套实现。pick 不带 config 参数 ——
/// 各规则构造时只拿自己那份配置，整套行为封闭在自己的文件里，读一个文件即知全部逻辑。
pub trait PickRule {
    /// Preserve+禁重复：把已有映射占用的图片从池中剔除。
    /// used 为「from 全路径」键集合（record_from 形式：种族目录非空 → "目录/词干"，平铺 → "词干"）。
    fn exclude_used(&mut self, used: &BTreeSet<String>);
    /// 为一名球员抽图；返回 None 表示该球员无法映射（跳过）。
    fn pick(&mut self, player: &PlayerRecord, duplicates: bool) -> Option<PickedImage>;
    /// 最近一次 `pick` 返回 None 的成因（规则在 None 分支顺手记录；调用方只在拿到 None 时读）。
    /// 无默认实现：新增规则必须显式回答「为什么跳过」，不许退回一句含混文案。
    fn last_skip_reason(&self) -> SkipReason;
    /// 本轮抽图的内部统计（事件名 → 次数，如目标目录不可用改走他处的回落计数）：
    /// 选图内部过程对上层不可见，映射结束后由 face_mapper 一次性汇总落日志。
    /// 默认空：没有内部回落概念的规则（标准一 / Unknown 布局）。
    fn stats(&self) -> Vec<(&'static str, usize)> {
        Vec::new()
    }
}

/// 全部标准规则的集合。字段名与标准号一一对应（standardN）。
#[derive(Debug, Clone)]
pub struct RuleConfig {
    pub standard2: standard2::Standard2Config,
    pub standard3: standard3::Standard3Config,
}

/// 可编辑规则文件的登记表项：命令层只查这张表，不再按标准号写 match。
/// 用函数指针而非泛型/闭包 —— 每项 Config 类型不同，函数指针把「解析+校验+写回」
/// 擦除到同一张表里，新增标准时 commands.rs 零改动。
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

    /// 参与目录判定的标准 (标准号, FolderList)，按标准号升序。
    /// 顺序即判定真源：`classify` 先命中先赢，故两份清单交集必须为空（`validate_registry` 守卫）。
    pub fn folder_lists(&self) -> Vec<(u8, &Vec<String>)> {
        vec![
            (2, &self.standard2.folder_list),
            (3, &self.standard3.folder_list),
        ]
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
        cfg.standard2
            .validate()
            .map_err(|e| format!("标准二规则无效：{}（{e}）", s2.display()))?;
        let s3 = dir.join("standard3.json");
        match json_store::load::<standard3::Standard3Config>(&s3) {
            Ok(user) => cfg.standard3 = user,
            Err(LoadError::Missing) => {}
            Err(LoadError::Corrupt(e)) => {
                return Err(format!("标准三规则文件无法解析：{}（{e}）", s3.display()))
            }
        }
        // 结构校验：用户改坏规则 json 时在这里拦下（不静默回退内置默认），错误信息含文件路径。
        cfg.standard3
            .validate()
            .map_err(|e| format!("标准三规则无效：{}（{e}）", s3.display()))?;
        validate_registry(&cfg)?;
        Ok(cfg)
    }
}

/// 标准间登记冲突守卫：任意两份 FolderList 交集必须为空。
/// 【为什么查交集而非「互不包含」】classify 先命中先赢，互不包含拦不住部分交叉：
/// 共享目录会让「只用这几个目录的包」被判给登记在前的标准，整包用错模型且全程无报错
/// （新增标准时最易踩中，设计文档标准二.md §5 本就要求交集为 0）。
/// 空清单跳过（空集是任何集合的子集，属退化情形）。
pub fn validate_registry(cfg: &RuleConfig) -> Result<(), String> {
    let lists = cfg.folder_lists();
    for i in 0..lists.len() {
        for j in (i + 1)..lists.len() {
            let (na, fa) = lists[i];
            let (nb, fb) = lists[j];
            if fa.is_empty() || fb.is_empty() {
                continue;
            }
            let a: BTreeSet<&str> = fa.iter().map(String::as_str).collect();
            let b: BTreeSet<&str> = fb.iter().map(String::as_str).collect();
            let shared: Vec<&str> = a.intersection(&b).copied().collect();
            if !shared.is_empty() {
                let head: Vec<&str> = shared.iter().copied().take(5).collect();
                return Err(format!(
                    "标准{na} 与 标准{nb} 的目录清单有 {n} 个共同目录（如 {eg}），\
                     只用这些目录的包会被判给登记在前的那个标准；请改 data/rules 下的 \
                     standard{na}.json / standard{nb}.json，让两份 FolderList 交集为 0",
                    n = shared.len(),
                    eg = head.join(", "),
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
/// 只借 `&PackScan`、由各规则自己转池 —— 大包索引（28 万图片名）整份 clone 是几十 MB 白复制，
/// 各规则构造时按需把要用的部分转成自己的池。
pub fn for_layout(layout: PackLayout, scan: &PackScan, config: &RuleConfig) -> Box<dyn PickRule> {
    match layout {
        // 标准一：包根单一池，完全随机。
        PackLayout::Flat => Box::new(standard1::Standard1Rule::new(&scan.root_images)),
        PackLayout::Standard { standard, .. } => match standard {
            2 => Box::new(standard2::Standard2Rule::new(
                &scan.faces_map,
                &config.standard2,
            )),
            3 => Box::new(standard3::Standard3Rule::new(
                &scan.recursive_pools,
                &config.standard3,
            )),
            // 未实现标准号已被 replace_flow 拦截；防御性返回永不命中的空规则。
            _ => Box::new(standard1::Standard1Rule::new(&BTreeSet::new())),
        },
        // 空包 / 无法识别：递归全池全随机（Empty 池为空 → 全部球员跳过）。
        PackLayout::Empty | PackLayout::Unknown => {
            Box::new(unknown::UnknownRule::new(&scan.recursive_pools))
        }
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
/// 【去重】上游 BTreeSet 天生按词干去重，但同目录 `face1.png`/`face1.jpg` 词干相同且合法并存；
/// 上游若换成非去重来源，禁重复时 swap_remove 只摘走一项，两人会拿到同一张图。
/// 【排序】上游本就升序，对已序输入接近线性，成本可忽略。
pub fn sorted_unique(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut v: Vec<String> = items.into_iter().collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// 构造"目录 → 图片集合"池的通用规则：只保留 FolderList 内且非空的目录，
/// json 未配置的多余子目录不进池。
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

/// Deck 的「发完原地重洗」：把某目录已发的牌洗回剩余牌堆。
/// 【为什么不整份克隆一副 deck】剩余牌与已发牌互补，两者总量恒等于建池全集；
/// 摘一张只需动一处，于是建池时那 28 万词条的整份复制（约几十 MB）可以全省掉。
/// 洗回时用 `SliceRandom::shuffle`，与 NG 侧 `random` 动作同构（§8.5）。
fn refill_into(pool: &mut Vec<String>, dealt: Vec<String>) {
    let mut back = dealt;
    rand::seq::SliceRandom::shuffle(&mut back[..], &mut rand::rng());
    pool.extend(back);
}

/// 与 `pick_from_pools_flat` 同口径（展平计数按下标取），但**只在 `allowed` 里的目录之间抽**。
/// 【为什么不能复用 flat】flat 会越过 FolderList 抽到 `WF/*`（45,006 张）与 `Staff/*`、
/// `Scandinavian`、包根图 —— 男球员可能拿到女职员/女足头像。NG 实测 172,494 条映射
/// **从不越过 FolderList**：94 个池的使用次数全部落在 `{q, q+1}`，越过清单的映射 0 条。
/// `allowed` 内一张不剩（含"只剩已发牌但该目录不在 allowed 内"）⇒ None。
fn pick_from_dirs(
    pools: &mut Pools,
    dealt: &mut BTreeMap<String, Vec<String>>,
    allowed: &BTreeSet<String>,
    duplicates: bool,
) -> Option<PickedImage> {
    if !duplicates {
        // 先把 allowed 内已发完的目录洗回来，否则这一轮它们一张都贡献不了。
        for (dir, pool) in pools.iter_mut() {
            if pool.is_empty() && allowed.contains(dir) {
                refill_into(pool, dealt.remove(dir).unwrap_or_default());
            }
        }
    }
    let total: usize = pools
        .iter()
        .filter(|(d, _)| allowed.contains(*d))
        .map(|(_, p)| p.len())
        .sum();
    if total == 0 {
        return None;
    }
    let idx = rand::rng().random_range(0..total);
    let mut passed = 0usize;
    for (dir, pool) in pools.iter_mut() {
        if !allowed.contains(dir) {
            continue;
        }
        if idx < passed + pool.len() {
            let i = idx - passed;
            // 下标直取 O(1)；禁重复就地 swap_remove 摘除，并记进已发牌堆。
            let stem = if duplicates {
                pool[i].clone()
            } else {
                pool.swap_remove(i)
            };
            if !duplicates {
                dealt.entry(dir.clone()).or_default().push(stem.clone());
            }
            return Some(PickedImage { dir: dir.clone(), stem });
        }
        passed += pool.len();
    }
    None // total > 0 时上面必然 return，这里只为穷尽分支
}

/// 从多级池中全局随机取一张（展平计数按下标取，有序池保证可复现）。
/// 禁重复时 swap_remove 就地摘除（O(1)）。Unknown 布局的「全池全随机」专用；
/// 标准三的兜底**不**走这里——它必须留在 FolderList 内（见 `pick_from_dirs`）。
fn pick_from_pools_flat(pools: &mut Pools, duplicates: bool) -> Option<PickedImage> {
    let total: usize = pools.values().map(|s| s.len()).sum();
    if total == 0 {
        return None;
    }
    let idx = rand::rng().random_range(0..total);
    let mut passed = 0usize;
    let mut chosen: Option<(String, String)> = None;
    for (dir, pool) in pools.iter_mut() {
        if idx < passed + pool.len() {
            let i = idx - passed;
            // 下标直取 O(1)；禁重复就地 swap_remove 摘除。
            let stem = if duplicates {
                pool[i].clone()
            } else {
                pool.swap_remove(i)
            };
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
    /// `NEWGAN_REAL_PACK_DIR=<包根> cargo test --manifest-path src-tauri/Cargo.toml real_pack_smoke -- --ignored --nocapture`
    ///
    /// 全链路验证 scan → classify → 标准三抽图（233 国 × 11 码）。硬断言仅两条：
    /// 每人抽到图、抽中的图在包里真实存在（from 不带扩展名，逐个试图片扩展名）；
    /// 直中/回退占比与缺失目录只打印不拦 —— 部分目录缺失时回退是正确行为而非错误。
    #[test]
    #[ignore]
    fn real_pack_smoke_scan_classify_pick() {
        use super::standard3::Candidates;
        use crate::xml_parser::record_from;
        let Some(pack) = std::env::var_os("NEWGAN_REAL_PACK_DIR") else {
            eprintln!("未设 NEWGAN_REAL_PACK_DIR，跳过真实包冒烟测试");
            return;
        };
        let pack = pack.to_string_lossy().to_string();
        // 规则来源与真实运行同源（开发数据目录的 rules/）；用户文件损坏不中断冒烟，退回内置默认。
        let data_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("data");
        let config = RuleConfig::load(&data_dir).unwrap_or_else(|e| {
            eprintln!("用户规则不可用（{e}），改用内置默认");
            RuleConfig::builtin()
        });
        let t = std::time::Instant::now();
        let scan = crate::facepack_scanner::scan_pack(&pack).unwrap();
        let total_imgs: usize = scan.recursive_pools.values().map(|s| s.len()).sum();
        eprintln!(
            "扫描：{} 个一级目录，递归池 {total_imgs} 张图，耗时 {:?}",
            scan.faces_map.len(),
            t.elapsed()
        );
        let refs: Vec<_> = config
            .folder_lists()
            .iter()
            .map(|(t, f)| (*t, *f))
            .collect();
        let layout = crate::facepack_scanner::classify_scan(&scan, &refs);
        eprintln!("判定：{layout:?}");
        assert!(
            matches!(layout, PackLayout::Standard { standard: 3, .. }),
            "该包未被判为标准三：{layout:?}"
        );
        // 清单目录在包内的健康度：缺失 / 空目录列出来（对应「一键创建缺失目录」的语义）。
        // 判据走递归池（键是完整相对路径）—— 清单里 64 条多级条目（`WF/x`、`Staff/r/male`）
        // 在一级目录 map 里根本不存在，拿它判会把它们全报成缺失。
        let unhealthy: Vec<&String> = config
            .standard3
            .folder_list
            .iter()
            .filter(|d| scan.recursive_pools.get(*d).is_none_or(|s| s.is_empty()))
            .collect();
        if !unhealthy.is_empty() {
            eprintln!(
                "清单内缺失或为空的目录 {} 个：{unhealthy:?}",
                unhealthy.len()
            );
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
                let img = rule
                    .pick(
                        &PlayerRecord {
                            uid: format!("{nat}/{code}"),
                            primary_nat: nat.into(),
                            sec_nat: String::new(),
                            name: "T".into(),
                            hair_length: "1".into(),
                            hair_color: "1".into(),
                            ethnicity_code: code.to_string(),
                            detailed: None,
                        },
                        true,
                    )
                    .unwrap_or_else(|| panic!("{nat}/{code} 没抽到图"));
                let in_cands = match res.candidates.as_ref() {
                    Some(Candidates::Single(d)) => *d == img.dir,
                    Some(Candidates::Uniform(v)) => v.iter().any(|d| *d == img.dir),
                    Some(Candidates::Weighted { dirs, .. }) => dirs.iter().any(|d| *d == img.dir),
                    None => false,
                };
                if in_cands {
                    direct += 1
                } else {
                    fallback += 1
                }
                *by_dir.entry(img.dir.clone()).or_insert(0) += 1;
                let base = std::path::Path::new(&pack).join(&img.dir).join(&img.stem);
                if !["png", "jpg", "jpeg"]
                    .iter()
                    .any(|e| std::path::Path::new(&format!("{}.{e}", base.display())).is_file())
                {
                    missing.push(format!("{}/{}", img.dir, img.stem));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "{} 条映射指向包里不存在的文件，如 {:?}",
            missing.len(),
            &missing[..missing.len().min(5)]
        );
        let mut ranked: Vec<_> = by_dir.iter().collect();
        ranked.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        eprintln!(
            "全量 2563 组：直中候选 {direct}，走回退 {fallback}；目录命中数 Top8：{:?}",
            &ranked[..ranked.len().min(8)]
        );
        // 第二轮：禁重复再扫一遍，验证真实规模下不产生重复映射。
        let mut rule = for_layout(layout, &scan, &config);
        let mut issued = BTreeSet::new();
        for (i, nat) in cfg.ethnics.keys().enumerate() {
            for code in 0..=10u8 {
                let img = rule
                    .pick(
                        &PlayerRecord {
                            uid: format!("nd-{nat}/{code}"),
                            primary_nat: nat.into(),
                            sec_nat: String::new(),
                            name: "T".into(),
                            hair_length: "1".into(),
                            hair_color: "1".into(),
                            ethnicity_code: code.to_string(),
                            detailed: None,
                        },
                        false,
                    )
                    .unwrap_or_else(|| panic!("禁重复第 {i} 组 {nat}/{code} 没抽到图"));
                assert!(
                    issued.insert(record_from(&img.dir, &img.stem)),
                    "禁重复抽到重复图 {img:?}"
                );
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
        // 标准三：126 目录清单（62 男球员族裔 + 32 女球员 + 32 职员）+ Ethnics（233 国）
        // + 规则块（国家/族群/全局共 43 个）+ Version 4 + Random 兜底。
        assert_eq!(cfg.standard3.folder_list.len(), 126);
        // 目录族映射表：男族裔目录 → 女球员/职员目录（58 条，含多值），女职员换区仅 4 条。
        assert_eq!(cfg.standard3.folder_mapping_female.len(), 58);
        assert_eq!(cfg.standard3.folder_mapping_staff.len(), 58);
        assert_eq!(cfg.standard3.folder_mapping_staff_female.len(), 4);
        assert!(cfg
            .standard3
            .folder_list
            .contains(&"SouthConeSA".to_string()));
        assert!(cfg
            .standard3
            .folder_list
            .contains(&"WestSlavic".to_string()));
        assert_eq!(cfg.standard3.ethnics.len(), 233);
        assert_eq!(cfg.standard3.ethnics["ESP"], "Spain");
        assert_eq!(cfg.standard3.ethnics["COL"], "Mestizo");
        assert_eq!(cfg.standard3.default_codes["ARG"], vec![1, 7]);
        assert_eq!(cfg.standard3.default_codes["BRA"], vec![1, 3, 9]);
        assert_eq!(cfg.standard3.version, 4);
        let rules = &cfg.standard3.ethnicity_mapping_rules;
        assert_eq!(rules.len(), 43);
        // 三类块各抽查一条：国家块（BRA）、族群块（Mestizo）、全局块（_DEFAULT_）。
        assert!(
            rules.contains_key("BRA")
                && rules.contains_key("Mestizo")
                && rules.contains_key("_DEFAULT_")
        );
        assert_eq!(
            rules["_DEFAULT_"]["9"],
            standard3::MappingRule::Directly("MixedRace".into())
        );
        assert_eq!(
            rules["Mestizo"]["7"],
            standard3::MappingRule::Directly("IndigenousSA".into())
        );
        assert!(matches!(
            rules["DOM"]["1"],
            standard3::MappingRule::Weighted { .. }
        ));
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
        // 二次 ensure：已存在不覆盖（写个标记内容验证不被冲掉）。
        let s2 = data.path().join("rules/standard2.json");
        std::fs::write(
            &s2,
            "{\"FolderList\":[\"African\"],\"Ethnics\":{\"XXL\":\"African\"}}",
        )
        .unwrap();
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
        std::fs::write(
            &s3,
            "{\"FolderList\":[\"China\"],\"Ethnics\":{\"CHN\":\"China\"}}",
        )
        .unwrap();
        let err = RuleConfig::load(data.path()).unwrap_err();
        assert!(err.contains("standard3.json"), "错误应含文件路径: {err}");
        assert!(err.contains("Version"), "应说明版本不符: {err}");
        std::fs::remove_file(&s3).unwrap();
        assert_eq!(
            RuleConfig::load(data.path())
                .unwrap()
                .standard3
                .folder_list
                .len(),
            126
        );
    }

    #[test]
    fn registry_registers_all_standards_and_rejects_overlapping_lists() {
        // 主题：标准登记表与冲突守卫。
        let cfg = RuleConfig::builtin();
        let lists = cfg.folder_lists();
        // ① 登记顺序即判定优先级：标准二在前、标准三在后；两份清单都非空。
        assert_eq!(
            lists.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert!(lists.iter().all(|(_, f)| !f.is_empty()));
        // ② 出厂两份清单交集为 0 → 通过（标准二 14 目录、标准三 126 目录，实测无一名重合）。
        validate_registry(&RuleConfig::builtin()).unwrap();
        // ②′ 再单独钉一次「新增的 64 条多级目录与标准二 14 条交集为 0」——
        // 这是 `classify` 先命中先赢的前提，也是这次扩清单最容易踩坏的地方。
        let multi_level: BTreeSet<&str> = cfg
            .standard3
            .folder_list
            .iter()
            .filter(|d| d.contains('/'))
            .map(String::as_str)
            .collect();
        assert_eq!(multi_level.len(), 64, "应新增 32 条 WF/* + 18 男职员 + 14 女职员");
        assert!(
            multi_level
                .iter()
                .all(|d| !crate::facepack_scanner::FACEPACK_DIRS.contains(d)),
            "新增的多级目录不得与标准二清单重合"
        );
        // 包含关系（标准二清单 ⊂ 标准三清单）→ 报错并指名两个标准号。
        let mut cfg = RuleConfig::builtin();
        cfg.standard2.folder_list = vec!["China".into(), "Japan".into()];
        let err = validate_registry(&cfg).unwrap_err();
        assert!(
            err.contains("标准2") && err.contains("标准3"),
            "应指名冲突标准: {err}"
        );
        assert!(err.contains("China"), "应给出重合目录名: {err}");
        // 部分交叉：两份清单互不包含但共享「Sudan」——「互不包含」式守卫拦不住，
        // 只用 Sudan 的包会被 classify 判给登记在前的标准2，整包用错抽图模型。
        let mut cfg = RuleConfig::builtin();
        cfg.standard2.folder_list = vec!["Sudan".into(), "标准二独有".into()];
        cfg.standard3.folder_list = vec!["Sudan".into(), "标准三独有".into()];
        let err = validate_registry(&cfg).unwrap_err();
        assert!(
            err.contains("Sudan") && err.contains("交集为 0"),
            "部分交叉必须被拒: {err}"
        );
        // 空清单不参与判定（否则空集恒为子集，误报）。
        let mut cfg = RuleConfig::builtin();
        cfg.standard2.folder_list = Vec::new();
        validate_registry(&cfg).unwrap();
    }
}
