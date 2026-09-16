//! 头像包布局扫描与判定。
//!
//! 社区头像包有多套组织"标准"（标准一：图片直铺；标准二：14 种族目录；标准三：细分目录；
//! 未来可扩展标准四/五/六……）。本模块职责：
//!   1. 扫描：scan_pack 一次遍历建立图片索引（纯收集，不做判定）；
//!   2. 判定：classify 按「各标准 json 的 FolderList」推断包属于哪个标准——
//!      超集（含全部目录、可多出）或子集（有缺失）都算归属该标准，交叉/未知名 → Unknown。
//!
//! FolderList 来自 templates（用户可配置），本模块不硬编码任何目录清单。
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 14 个种族目录（标准二出厂默认的 FolderList 数据，也是 validate_image_dirs 的对照基准）。
pub const FACEPACK_DIRS: [&str; 14] = [
    "African", "Asian", "Caucasian", "Central European", "EECA", "Italmed",
    "MENA", "MESA", "SAMed", "Scandinavian", "Seasian", "South American",
    "SpanMed", "YugoGreek",
];

/// 头像包布局判定结果。
/// 【泛化】Standard 携带标准号而非固定枚举二/三——新增标准四/五/六时判定层零改动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackLayout {
    Flat,                                   // 标准一：无子目录 + 根目录有图片
    Empty,                                  // 无子目录且无图片（空包）
    Standard { standard: u8, complete: bool }, // 归属标准号；complete = FolderList 目录全部存在（可有 extra）
    Unknown,                                // 与任何标准的 FolderList 都不构成包含关系
}

impl PackLayout {
    /// 映射为对外的"标准号"：标准号原样返回；标准一/空包 → 1；无法识别 → 0。
    pub fn standard(&self) -> u8 {
        match self {
            PackLayout::Flat | PackLayout::Empty => 1,
            PackLayout::Standard { standard, .. } => *standard,
            PackLayout::Unknown => 0,
        }
    }
    /// 是否标准一（图片直铺）布局；「扫描头像」据它选图片总数口径：平铺数根图，其余数递归全池。
    pub fn is_flat(&self) -> bool { matches!(self, PackLayout::Flat) }
}

/// 唯一的标准判定入口（纯函数，不读盘）。
/// folder_lists：各标准的 (标准号, FolderList)，来自 templates（用户可配置），按登记顺序传入。
/// 判定顺序：**超集优先**（S ⊇ F，含全部目录、可多出 → 归属该标准，complete = 恰好相等），
/// 其次**子集**（S ⊂ F，有缺失 → fixable；多个标准同时子集命中时按传入顺序，
/// 仅 {Scandinavian} 一名重合的历史包因此归标准二），交叉/未知名 → Unknown。
pub fn classify(
    subdirs: &BTreeSet<String>,
    has_root_images: bool,
    folder_lists: &[(u8, &Vec<String>)],
) -> PackLayout {
    if subdirs.is_empty() {
        return if has_root_images { PackLayout::Flat } else { PackLayout::Empty };
    }
    let names: BTreeSet<&str> = subdirs.iter().map(String::as_str).collect();
    // 第一轮：超集判定（包含某标准全部目录 → 归属且视为完整；超集间互斥——各标准清单互不包含）。
    // complete = 标准配置的全部目录都在包内（S 可以多出 extra 目录，由调用方按 FolderList 排除）。
    for (n, folders) in folder_lists {
        let f: BTreeSet<&str> = folders.iter().map(String::as_str).collect();
        if !f.is_empty() && f.is_subset(&names) {
            return PackLayout::Standard { standard: *n, complete: true };
        }
    }
    // 第二轮：子集判定（观察到的目录全部落在某标准清单内 → 就近归属，可补全缺失目录）。
    for (n, folders) in folder_lists {
        let f: BTreeSet<&str> = folders.iter().map(String::as_str).collect();
        if names.is_subset(&f) {
            return PackLayout::Standard { standard: *n, complete: false };
        }
    }
    PackLayout::Unknown
}

/// 一次目录扫描的产物：一级子目录图片索引 + 根图片索引 + 递归全量池（纯收集，无判定）。
#[derive(Debug, Clone)]
pub struct PackScan {
    /// 一级子目录名 → 该目录直接图片的词干（image_stem 口径）。
    /// 与 recursive_pools 同源同键：标准二/标准三的主池都从这里按 FolderList 过滤而来，
    /// 而标准三的兜底池取 recursive_pools——两者同键才能保证禁重复 / Preserve 剔除摘一份即两份生效。
    pub faces_map: BTreeMap<String, BTreeSet<String>>,
    pub root_images: BTreeSet<String>,                 // 根目录图片名（去扩展名）
    /// 递归全量池：相对目录路径（"" = 根，多级用 "/" 分隔）→ 该目录下全部图片词干。
    /// 供 Unknown/兜底"全随机"规则使用；垃圾目录（is_junk_dir）整棵子树不收录。
    pub recursive_pools: BTreeMap<String, BTreeSet<String>>,
}

impl PackScan {
    /// 图片总数口径（「扫描头像」用）：平铺包数根图，其余（含 Unknown 的子/孙目录）数递归全量池。
    pub fn total_for(&self, layout: &PackLayout) -> usize {
        if layout.is_flat() {
            self.root_images.len()
        } else {
            self.recursive_pools.values().map(|s| s.len()).sum()
        }
    }

    /// 裸词干递归兜底：在递归全量池里按目录字典序找第一个含该词干的目录，返回其相对路径。
    /// 供「头像」列预览在包根未命中时到子/孙目录里找同名图（确定性行为）。
    pub fn find_stem_dir(&self, stem: &str) -> Option<&String> {
        self.recursive_pools
            .iter()
            .find(|(_, stems)| stems.contains(stem))
            .map(|(dir, _)| dir)
    }
}

/// 按扩展名判断是否图片文件（png/jpg/jpeg，大小写不敏感）。
pub fn is_image_ext(name: &str) -> bool {
    matches!(name.rsplit('.').next().map(|s| s.to_ascii_lowercase()).as_deref(),
             Some("png") | Some("jpg") | Some("jpeg"))
}

/// 图片词干的**唯一口径**：取 file_stem（"a.1.png" → "a.1"），扫描池（faces_map / root_images /
/// recursive_pools 全部经它）与 xml_parser 解析/回填一致，保证同一张图在各层只有一个身份。
/// 空词干返回 None（调用方另用 `!starts_with('.')` 排除隐藏文件）。
pub fn image_stem(name: &str) -> Option<String> {
    let stem = Path::new(name).file_stem()?.to_string_lossy().to_string();
    if stem.is_empty() { None } else { Some(stem) }
}

/// 打包/系统垃圾目录：macOS 压缩包解出的 `__MACOSX`、各类 `.` 开头目录（.git 等）。
/// 仅用于过滤 classify 的输入与递归池；faces_map 仍收录该键（值为空集）——判定层按本函数过滤键名。
pub fn is_junk_dir(name: &str) -> bool { name.starts_with('.') || name == "__MACOSX" }

/// 递归收集 dir 及其全部子/孙目录的图片（词干用 file_stem，跳过 `.` 开头文件）。
/// 键为相对头像包根的目录路径（"" = 根，多级用 "/" 分隔）；垃圾目录整棵子树跳过。
fn walk_recursive(dir: &Path, rel: String, pools: &mut BTreeMap<String, BTreeSet<String>>) {
    let mut imgs = BTreeSet::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            let fname = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if is_junk_dir(&fname) { continue; } // 垃圾目录整棵不入池
                let child = if rel.is_empty() { fname } else { format!("{rel}/{fname}") };
                walk_recursive(&p, child, pools);
            } else if is_image_ext(&fname) && !fname.starts_with('.') {
                if let Some(stem) = image_stem(&fname) { imgs.insert(stem); }
            }
        }
    }
    pools.insert(rel, imgs);
}

/// 重扫描：建 faces_map + root_images + 递归全量池（纯收集；布局判定由调用方用 classify 完成）。
/// 【仅测试】生产的两条路径（「扫描头像」命令、替换流水线）都要报进度，一律走 scan_pack_progress；
/// 留这个无进度版只是让测试少写一个空回调，故标 cfg(test) 免得在发布构建里报 dead_code。
#[cfg(test)]
pub fn scan_pack(img_dir: &str) -> Result<PackScan, String> {
    scan_pack_progress(img_dir, &mut |_, _| {})
}

/// 同 `scan_pack`，额外把「已扫完第 n 个 / 共 total 个一级子目录」上报给进度回调。
/// 【为何以一级子目录为进度单位】第一趟只列目录拿到分母、第二趟逐个扫（每个含整棵子树），
/// 不必为算进度多遍历一遍磁盘，又让「扫描头像」的进度条真实反映工作量（标准二 14 目录 → 14 步）。
pub fn scan_pack_progress(
    img_dir: &str,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PackScan, String> {
    let root = Path::new(img_dir);
    let mut faces_map = BTreeMap::new();
    let mut root_images = BTreeSet::new();
    let mut recursive_pools: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // 第一趟：只列顶层（不递归），分出「一级子目录」与根图——拿到子目录清单就有进度分母了。
    let mut subdirs: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| format!("读取头像包目录失败: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let fname = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            subdirs.push((fname, path));
        } else if is_image_ext(&fname) && !fname.starts_with('.') {
            // 根图词干走 image_stem 唯一口径（与 parse_image_path_in_pack 同规则，"a.1.png"→"a.1"）；
            // 跳过 . 开头文件（macOS AppleDouble 的 ._x.png 会以 "._x" 入池、污染 resolve）。
            if let Some(stem) = image_stem(&fname) { root_images.insert(stem); }
        }
    }
    let total = subdirs.len(); // 进度分母（0 = 平铺包/空包，订阅方按 total==0 直接给 100%）

    // 第二趟：逐个一级子目录建索引，每扫完一个上报一次进度。
    for (i, (fname, path)) in subdirs.iter().enumerate() {
        // 递归全量池：垃圾目录整棵跳过（目录本身仍进 faces_map，只是值为空集）。
        if !is_junk_dir(fname) {
            walk_recursive(path, fname.clone(), &mut recursive_pools);
        }
        // faces_map：一级子目录 → 其直接图片词干。**直接从 recursive_pools 取**，不再单独 read_dir：
        // 主池（标准二/三按 FolderList 过滤 faces_map）与兜底池（recursive_pools）是同一批图的两份索引，
        // 同源取值才保证两者逐字同键——否则词干规则一有分歧（历史上 "a.1.png" 在主池是 "a"、在兜底池是
        // "a.1"），同一张图就有两个身份，禁重复与 Preserve 剔除只摘得掉一份、这张图会被发出两次。
        // 顺带省掉每个一级目录的第二遍 read_dir。
        let imgs = recursive_pools.get(fname).cloned().unwrap_or_default();
        faces_map.insert(fname.clone(), imgs);
        progress(i + 1, total);
    }
    // 根目录图片也归入递归池（键 = 空串），全随机规则据此连根图一起抽。
    recursive_pools.entry(String::new()).or_default().extend(root_images.iter().cloned());
    Ok(PackScan { faces_map, root_images, recursive_pools })
}

/// 扫描结果的布局判定：垃圾目录（__MACOSX 等）不参与，与 FaceMapper::from_scan 同口径。
/// 「扫描头像」与「建立映射索引」两处共用此前导，避免各自重列一遍 faces_map 过滤。
pub fn classify_scan(scan: &PackScan, folder_lists: &[(u8, &Vec<String>)]) -> PackLayout {
    let subdirs: BTreeSet<String> = scan
        .faces_map
        .keys()
        .filter(|n| !is_junk_dir(n))
        .cloned()
        .collect();
    classify(&subdirs, !scan.root_images.is_empty(), folder_lists)
}

/// 「扫描头像」核心（纯函数）：重扫描头像包，返回 (图片总数, 标准号, 完整扫描结果)。
/// 标准号按 templates 的 FolderList 判定（用户可配置，不硬编码）；总数口径见 PackScan::total_for。
/// 额外把扫描进度透传给 `scan_pack_progress`（「扫描头像」状态栏进度用）。
pub fn scan_and_total_progress(
    img_dir: &str,
    folder_lists: &[(u8, &Vec<String>)],
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(usize, u8, PackScan), String> {
    let scan = scan_pack_progress(img_dir, progress)?;
    let layout = classify_scan(&scan, folder_lists);
    let total = scan.total_for(&layout);
    Ok((total, layout.standard(), scan))
}

/// 「检查完整性」结果（对应前端 IntegrityResult）：命中的标准、可补全的缺失目录、多余目录等。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityResult {
    pub standard: u8,          // 命中的标准号（2=14 种族 / 3=细分…）；1=图片直铺；0=无法识别
    pub status: String,        // "complete" | "fixable" | "unknown"
    pub missing: Vec<String>,  // fixable 时可一键创建的缺失目录（按标准 FolderList 原顺序）
    pub extra: Vec<String>,    // 包内多出、未在标准 FolderList 配置的目录（不参与抽图）
    pub found: usize,          // 已命中的子目录数
    pub expected_total: usize, // 该标准 FolderList 的目录总数（标准一/无法识别为 0）
}

/// 检查完整性（纯函数，读盘但无副作用）：轻扫描子目录名 + 探测根是否有图 → classify →
/// 汇总缺失/多余目录。判定与缺失/多余计算全部以标准 FolderList 为基准（用户改 json 即改判定口径）。
/// 额外把「已核对第 n 个 / 共 total 个顶层条目」上报给进度回调（「检查完整性」状态栏进度用）。
/// 【为何先把条目收集到 Vec】read_dir 是迭代器、事先不知道总数，先收集一遍才有进度分母。
pub fn check_integrity_progress(
    img_dir: &str,
    folder_lists: &[(u8, &Vec<String>)],
    progress: &mut dyn FnMut(usize, usize),
) -> IntegrityResult {
    // 第一趟：只收集关心的顶层条目（子目录 / 图片文件），拿到分母。
    let mut entries: Vec<(bool, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(img_dir) {
        for e in rd.flatten() {
            let p = e.path();
            let fname = e.file_name().to_string_lossy().to_string();
            let is_dir = p.is_dir();
            // 子目录全收（垃圾目录在下面判定时再滤）；根图只认非隐藏的图片文件（与 scan_pack 口径一致）。
            if is_dir || (!fname.starts_with('.') && is_image_ext(&fname)) {
                entries.push((is_dir, fname));
            }
        }
    }
    let total = entries.len(); // 进度分母（空包/读不到目录时为 0）

    // 第二趟：逐条核对，每核对一条上报一次进度。
    let mut subdirs: BTreeSet<String> = BTreeSet::new();
    let mut has_images = false;
    for (i, (is_dir, fname)) in entries.iter().enumerate() {
        if *is_dir {
            if !is_junk_dir(fname) {
                subdirs.insert(fname.clone());
            } // 跳过 __MACOSX 等
        } else {
            has_images = true; // 走到这里的一定是非隐藏图片文件（隐藏文件上一步已被滤掉）
        }
        progress(i + 1, total);
    }

    let layout = classify(&subdirs, has_images, folder_lists);
    let standard = layout.standard();
    let status = match layout {
        PackLayout::Flat => "complete",
        PackLayout::Empty | PackLayout::Unknown => "unknown",
        PackLayout::Standard { complete: true, .. } => "complete",
        PackLayout::Standard { .. } => "fixable",
    };
    // missing / extra / expected_total 按归属标准的 FolderList 计算：
    // missing = FolderList 中尚未出现的目录（可一键补全）；extra = 包内多出的未配置目录（不进抽图池）。
    let (missing, extra, expected_total) = match &layout {
        PackLayout::Standard { standard: n, .. } => {
            let folders = folder_lists
                .iter()
                .find(|(t, _)| t == n)
                .map(|(_, f)| (*f).clone())
                .unwrap_or_default();
            let missing = folders
                .iter()
                .filter(|d| !subdirs.contains(*d))
                .cloned()
                .collect();
            let extra: Vec<String> = subdirs
                .iter()
                .filter(|d| !folders.contains(*d))
                .cloned()
                .collect();
            (missing, extra, folders.len())
        }
        _ => (Vec::new(), Vec::new(), 0),
    };
    // 标准一/无法识别的 found 与现状一致：标准一记 0，其余记命中的子目录数。
    let found = if standard == 1 { 0 } else { subdirs.len() };
    IntegrityResult {
        standard,
        status: status.into(),
        missing,
        extra,
        found,
        expected_total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() }

    /// 构造与内置 standard2/standard3 默认 FolderList 对应的判定输入。
    fn folder_lists() -> Vec<(u8, Vec<String>)> {
        let cfg = crate::templates::RuleConfig::builtin();
        vec![
            (2, cfg.standard2.folder_list.clone()),
            (3, cfg.standard3.folder_list.clone()),
        ]
    }

    fn classify_with(subdirs: &[&str], has_root: bool) -> PackLayout {
        let s = names(subdirs);
        let fl = folder_lists();
        let refs: Vec<(u8, &Vec<String>)> = fl.iter().map(|(t, f)| (*t, f)).collect();
        classify(&s, has_root, &refs)
    }

    #[test]
    fn classify_flat_and_empty() {
        assert_eq!(classify_with(&[], true), PackLayout::Flat);
        assert_eq!(classify_with(&[], false), PackLayout::Empty);
    }

    #[test]
    fn classify_standard2_exact_subset_and_superset() {
        // 恰好 14 个 → complete
        let all: Vec<&str> = FACEPACK_DIRS.to_vec();
        assert_eq!(classify_with(&all, false),
                   PackLayout::Standard { standard: 2, complete: true });
        // 子集（缺 1 个）→ fixable
        assert_eq!(classify_with(&all[..13], false),
                   PackLayout::Standard { standard: 2, complete: false });
        // 超集（14 全在 + 多出目录，甚至含其他标准的目录名）→ 仍归标准二、complete，
        // 多余目录由规则构造时按 FolderList 过滤、不进抽图池。
        let mut superset: Vec<&str> = FACEPACK_DIRS.to_vec();
        superset.push("Spain");
        superset.push("Extra1");
        assert_eq!(classify_with(&superset, false),
                   PackLayout::Standard { standard: 2, complete: true });
    }

    #[test]
    fn classify_standard3_exact_and_superset() {
        let cfg = crate::templates::RuleConfig::builtin();
        let f3: Vec<&str> = cfg.standard3.folder_list.iter().map(String::as_str).collect();
        assert_eq!(classify_with(&f3, false),
                   PackLayout::Standard { standard: 3, complete: true });
        // 子集
        assert_eq!(classify_with(&["China", "Japan", "Korea"], false),
                   PackLayout::Standard { standard: 3, complete: false });
        // 超集：62 清单全在 + 多出清单外的目录（用户包里常见的未使用目录）→ 仍归标准三
        let mut superset = f3.clone();
        superset.push("Scandinavian");
        superset.push("SomeUnusedDir");
        assert_eq!(classify_with(&superset, false),
                   PackLayout::Standard { standard: 3, complete: true });
    }

    #[test]
    fn classify_unknown_and_scandinavian_priority() {
        assert_eq!(classify_with(&["African", "NotARealRace"], false), PackLayout::Unknown);
        // 跨两套混用（非超集、非任何子集）→ Unknown
        assert_eq!(classify_with(&["African", "China"], false), PackLayout::Unknown);
        // 仅 Scandinavian（只属于标准二清单）→ 子集判定归标准二、目录不全。
        assert_eq!(classify_with(&["Scandinavian"], false),
                   PackLayout::Standard { standard: 2, complete: false });
    }

    #[test]
    fn scan_pack_flat_with_junk_and_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"png").unwrap();
        std::fs::write(dir.path().join("b.1.png"), b"png").unwrap();
        std::fs::write(dir.path().join("._c.png"), b"png").unwrap();   // AppleDouble 垃圾
        std::fs::write(dir.path().join("notes.txt"), b"t").unwrap();   // 非图片
        std::fs::write(dir.path().join("config.xml"), b"<x/>").unwrap(); // 辅助文件
        std::fs::create_dir(dir.path().join("__MACOSX")).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join("__MACOSX").join("._a.png"), b"png").unwrap();
        let scan = scan_pack(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(scan.root_images, BTreeSet::from(["a".to_string(), "b.1".to_string()]));
        // faces_map 原样收录垃圾目录键（不参与判定、值恒为空集），其内 ._a.png 因隐藏文件过滤被排除。
        assert!(scan.faces_map.contains_key("__MACOSX"));
        assert!(scan.faces_map.contains_key(".git"));
        assert!(scan.faces_map["__MACOSX"].is_empty());
        // 递归池：根图在 "" 键下；垃圾目录整棵不入池。
        assert_eq!(scan.recursive_pools.get(""), Some(&scan.root_images));
        assert!(!scan.recursive_pools.contains_key("__MACOSX"));
        assert!(!scan.recursive_pools.contains_key(".git"));
    }

    #[test]
    fn scan_pack_standard2() {
        let dir = tempfile::tempdir().unwrap();
        for eth in FACEPACK_DIRS {
            let d = dir.path().join(eth);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("e1.png"), b"png").unwrap();
            std::fs::write(d.join("e2.jpg"), b"jpg").unwrap();
        }
        let scan = scan_pack(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(scan.faces_map.len(), 14);
        assert!(scan.faces_map.values().all(|s| s.len() == 2)); // e1 + e2（image_stem 唯一口径）
        assert!(scan.root_images.is_empty());
        // 递归池 = 根池（空集，键恒存在）+ 14 个种族目录。
        assert_eq!(scan.recursive_pools.len(), 15);
        assert!(scan.recursive_pools.get("").unwrap().is_empty());
        assert!(scan.recursive_pools.values().skip(1).all(|s| s.len() == 2));
    }

    #[test]
    fn faces_map_and_recursive_pools_share_stem_rule() {
        // 含点号文件名（"a.1.png"）在整条链路上只有一个身份 "a.1"：主池（faces_map）与兜底池
        // （recursive_pools）必须逐字同键，否则禁重复 / Preserve 剔除只摘得掉一份、同一张图会被发出两次。
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("SpanMed");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.png"), b"png").unwrap();
        std::fs::write(d.join("a.1.png"), b"png").unwrap();
        std::fs::write(d.join("._hidden.png"), b"png").unwrap(); // AppleDouble：隐藏名不入池
        std::fs::write(d.join("notes.txt"), b"t").unwrap();       // 非图片不入池
        std::fs::write(d.join("config.xml"), b"<x/>").unwrap();   // 辅助文件不入池
        let scan = scan_pack(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(scan.faces_map["SpanMed"], BTreeSet::from(["a".to_string(), "a.1".to_string()]));
        // 同源：每个一级目录在两份索引里的键集逐字相同（faces_map 直接取自 recursive_pools）。
        for (k, v) in &scan.faces_map {
            assert_eq!(scan.recursive_pools.get(k), Some(v), "目录 {k} 的图集在两份索引中应完全一致");
        }
    }

    #[test]
    fn scan_pack_recursive_deep_nesting() {
        // 子/孙目录全部图片进入递归池，键为 "/" 分隔的相对路径；垃圾子树跳过。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::create_dir_all(p.join("weirdA/sub")).unwrap();
        std::fs::create_dir_all(p.join("weirdA/__MACOSX")).unwrap();
        std::fs::write(p.join("weirdA/i.png"), b"png").unwrap();
        std::fs::write(p.join("weirdA/sub/j.1.png"), b"png").unwrap();
        std::fs::write(p.join("weirdA/__MACOSX/x.png"), b"png").unwrap();
        let scan = scan_pack(p.to_str().unwrap()).unwrap();
        let pools = &scan.recursive_pools;
        assert_eq!(pools.get("").map(|s| s.len()), Some(0)); // 根无图 → 空集（键存在）
        assert_eq!(pools.get("weirdA").map(|s| s.iter().cloned().collect::<Vec<_>>()),
                   Some(vec!["i".to_string()]));
        assert_eq!(pools.get("weirdA/sub").map(|s| s.iter().cloned().collect::<Vec<_>>()),
                   Some(vec!["j.1".to_string()]));
        assert!(!pools.contains_key("weirdA/__MACOSX")); // 垃圾目录整棵跳过
    }

    // ---------------- 完整性检查 / 扫描总数（原 commands.rs，随 check_integrity/scan_and_total 下沉） ----------------

    /// 在临时目录下按 names 建子目录。
    fn mk_dirs(root: &Path, names: &[&str]) {
        for n in names {
            std::fs::create_dir_all(root.join(n)).unwrap();
        }
    }

    /// 在根目录放标准说明里点名的辅助文件（config.xml / .md / 备注.txt），验证其不参与判定。
    fn drop_aux_files(root: &Path) {
        std::fs::write(root.join("config.xml"), b"<config/>").unwrap();
        std::fs::write(root.join("README.md"), b"# md").unwrap();
        std::fs::write(root.join("备注.txt"), b"note").unwrap();
    }

    /// 以内置 FolderList 判定完整性（cfg 只活在函数内，folder_lists 借用它）；不关心进度传空回调。
    fn integrity(dir: &Path) -> IntegrityResult {
        let cfg = crate::templates::RuleConfig::builtin();
        check_integrity_progress(dir.to_str().unwrap(), &cfg.folder_lists(), &mut |_, _| {})
    }

    /// 以内置 FolderList 扫描并计数；不关心进度传空回调。
    fn scan_total(dir: &Path) -> Result<(usize, u8, PackScan), String> {
        let cfg = crate::templates::RuleConfig::builtin();
        scan_and_total_progress(dir.to_str().unwrap(), &cfg.folder_lists(), &mut |_, _| {})
    }

    #[test]
    fn standard1_images_only() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("face.png"), b"png").unwrap();
        let r = integrity(tmp.path());
        assert_eq!((r.standard, r.status.as_str()), (1, "complete"));
    }

    #[test]
    fn empty_dir_is_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let r = integrity(tmp.path());
        assert_eq!((r.standard, r.status.as_str()), (1, "unknown"));
    }

    #[test]
    fn standard2_exact_regardless_of_order() {
        let tmp = tempfile::tempdir().unwrap();
        let mut shuffled: Vec<&str> = FACEPACK_DIRS.to_vec();
        shuffled.reverse(); // 顺序不同仍应命中
        mk_dirs(tmp.path(), &shuffled);
        std::fs::write(tmp.path().join(FACEPACK_DIRS[0]).join("a.jpg"), b"x").unwrap();
        let r = integrity(tmp.path());
        assert_eq!(
            (r.standard, r.status.as_str(), r.missing.len(), r.expected_total),
            (2, "complete", 0, 14)
        );
    }

    #[test]
    fn standard3_exact() {
        let tmp = tempfile::tempdir().unwrap();
        // 按内置标准三 FolderList（62 目录）建包 → 恰好相等 → complete。
        let cfg = crate::templates::RuleConfig::builtin();
        mk_dirs(
            tmp.path(),
            &cfg.standard3.folder_list.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let r = integrity(tmp.path());
        assert_eq!(
            (r.standard, r.status.as_str(), r.expected_total),
            (3, "complete", 62)
        );
        assert!(r.extra.is_empty());
    }

    #[test]
    fn standard3_superset_still_matches_with_extra() {
        // 62 目录全在 + 多出清单外的未使用目录 → 仍判标准三 complete；多余目录进 extra、不参与抽图。
        let tmp = tempfile::tempdir().unwrap();
        let cfg = crate::templates::RuleConfig::builtin();
        let mut dirs: Vec<String> = cfg.standard3.folder_list.clone();
        dirs.push("Scandinavian".into()); // 清单外
        dirs.push("SomeUnusedDir".into()); // 清单外
        mk_dirs(tmp.path(), &dirs.iter().map(String::as_str).collect::<Vec<_>>());
        let r = integrity(tmp.path());
        assert_eq!(
            (r.standard, r.status.as_str(), r.expected_total),
            (3, "complete", 62)
        );
        assert_eq!(
            r.extra,
            vec!["Scandinavian".to_string(), "SomeUnusedDir".to_string()]
        );
    }

    #[test]
    fn standard2_fixable_reports_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // 标准二去掉最后一个（YugoGreek）→ 13 个，应判 fixable 且缺失正好是 YugoGreek。
        let present: Vec<&str> = FACEPACK_DIRS[..13].to_vec();
        mk_dirs(tmp.path(), &present);
        let r = integrity(tmp.path());
        assert_eq!(
            (r.standard, r.status.as_str(), r.found, r.expected_total),
            (2, "fixable", 13, 14)
        );
        assert_eq!(r.missing, vec!["YugoGreek".to_string()]);
    }

    #[test]
    fn standard3_fixable_subset() {
        let tmp = tempfile::tempdir().unwrap();
        mk_dirs(tmp.path(), &["China", "Japan", "Korea"]); // 全部 ⊆ 标准三 FolderList(62)
        let r = integrity(tmp.path());
        assert_eq!(
            (r.standard, r.status.as_str(), r.found, r.expected_total),
            (3, "fixable", 3, 62)
        );
        assert_eq!(r.missing.len(), 59); // 62 - 3
        assert!(!r.missing.contains(&"China".to_string()));
    }

    #[test]
    fn unknown_when_unrecognized_name_present() {
        let tmp = tempfile::tempdir().unwrap();
        mk_dirs(tmp.path(), &["African", "Asian", "NotARealRace"]);
        let r = integrity(tmp.path());
        assert_eq!((r.standard, r.status.as_str()), (0, "unknown"));
    }

    #[test]
    fn unknown_when_mixing_two_sets() {
        let tmp = tempfile::tempdir().unwrap();
        // African 属 14 组、China 属 65 组，跨两套混用 → 无法识别。
        mk_dirs(tmp.path(), &["African", "China"]);
        let r = integrity(tmp.path());
        assert_eq!((r.standard, r.status.as_str()), (0, "unknown"));
    }

    #[test]
    fn standard2_ignores_aux_files() {
        let tmp = tempfile::tempdir().unwrap();
        mk_dirs(tmp.path(), &FACEPACK_DIRS);
        drop_aux_files(tmp.path());
        let r = integrity(tmp.path());
        assert_eq!(
            (r.standard, r.status.as_str(), r.missing.len()),
            (2, "complete", 0)
        );
    }

    #[test]
    fn standard3_fixable_ignores_aux_files() {
        let tmp = tempfile::tempdir().unwrap();
        mk_dirs(tmp.path(), &["China", "Japan", "Korea"]);
        drop_aux_files(tmp.path());
        let r = integrity(tmp.path());
        assert_eq!(
            (r.standard, r.status.as_str(), r.expected_total),
            (3, "fixable", 62)
        );
    }

    #[test]
    fn standard1_complete_with_aux_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("face.png"), b"png").unwrap();
        drop_aux_files(tmp.path());
        let r = integrity(tmp.path());
        assert_eq!((r.standard, r.status.as_str()), (1, "complete"));
    }

    #[test]
    fn check_integrity_ignores_junk_dirs() {
        // __MACOSX / .git 等垃圾目录不参与判定：平铺包不因它们误判 Unknown。
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("face.png"), b"png").unwrap();
        std::fs::create_dir(tmp.path().join("__MACOSX")).unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let r = integrity(tmp.path());
        assert_eq!((r.standard, r.status.as_str()), (1, "complete"));
    }

    #[test]
    fn scan_and_total_flat_and_standard2() {
        // 平铺包：根 3 图 → (3, 1)
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..3 {
            std::fs::write(tmp.path().join(format!("f{i}.png")), b"png").unwrap();
        }
        let (total, standard, _) = scan_total(tmp.path()).unwrap();
        assert_eq!((total, standard), (3, 1));
        // 14 目录包：各目录图数之和 → (28, 2)
        let tmp2 = tempfile::tempdir().unwrap();
        for eth in FACEPACK_DIRS {
            let d = tmp2.path().join(eth);
            std::fs::create_dir_all(&d).unwrap();
            for i in 0..2 {
                std::fs::write(d.join(format!("e{i}.png")), b"png").unwrap();
            }
        }
        let (total, standard, _) = scan_total(tmp2.path()).unwrap();
        assert_eq!((total, standard), (28, 2));
    }
}
