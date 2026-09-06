//! 【新手导读】头像包布局扫描与判定。
//! 社区头像包有多套组织"模板"（模板一：图片直铺；模板二：14 种族目录；模板三：细分目录；
//! 未来可扩展模板四/五/六……）。本模块职责：
//!   1. 扫描：scan_pack 一次遍历建立图片索引（纯收集，不做判定）；
//!   2. 判定：classify 按「各模板 json 的 FolderList」推断包属于哪个模板——
//!      超集（含全部目录、可多出）或子集（有缺失）都算归属该模板，交叉/未知名 → Unknown。
//! FolderList 来自 rule_config（用户可配置），本模块不再硬编码任何目录清单。
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 14 个种族目录（模板二出厂默认的 FolderList 数据，也是 validate_image_dirs 的对照基准）。
pub const FACEPACK_DIRS: [&str; 14] = [
    "African", "Asian", "Caucasian", "Central European", "EECA", "Italmed",
    "MENA", "MESA", "SAMed", "Scandinavian", "Seasian", "South American",
    "SpanMed", "YugoGreek",
];

/// 头像包布局判定结果。
/// 【泛化】Standard 携带模板号而非固定枚举二/三——新增模板四/五/六时判定层零改动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackLayout {
    Flat,                                   // 模板一：无子目录 + 根目录有图片
    Empty,                                  // 无子目录且无图片（空包）
    Standard { template: u8, complete: bool }, // 归属模板号 template；complete = FolderList 目录全部存在（可有 extra）
    Unknown,                                // 与任何模板的 FolderList 都不构成包含关系
}

impl PackLayout {
    /// 映射为对外的"标准号"：模板号原样返回；模板一/空包 → 1；无法识别 → 0。
    pub fn standard(&self) -> u8 {
        match self {
            PackLayout::Flat | PackLayout::Empty => 1,
            PackLayout::Standard { template, .. } => *template,
            PackLayout::Unknown => 0,
        }
    }
    pub fn is_flat(&self) -> bool { matches!(self, PackLayout::Flat) }
}

/// 唯一的模板判定入口（纯函数，不读盘）。
/// folder_lists：各模板的 (标准号, FolderList)，来自 rule_config（用户可配置），按模板号序传入。
/// 判定顺序：**超集优先**（S ⊇ F，含全部目录、可多出 → 归属该模板，complete = 恰好相等），
/// 其次**子集**（S ⊂ F，有缺失 → fixable；多个模板同时子集命中时按传入顺序，
/// 仅 {Scandinavian} 一名重合的历史包因此归模板二），交叉/未知名 → Unknown。
pub fn classify(
    subdirs: &BTreeSet<String>,
    has_root_images: bool,
    folder_lists: &[(u8, &Vec<String>)],
) -> PackLayout {
    if subdirs.is_empty() {
        return if has_root_images { PackLayout::Flat } else { PackLayout::Empty };
    }
    let names: BTreeSet<&str> = subdirs.iter().map(String::as_str).collect();
    // 第一轮：超集判定（包含某模板全部目录 → 归属且视为完整；超集间互斥——各模板清单互不包含）。
    // complete = 模板配置的全部目录都在包内（S 可以多出 extra 目录，由调用方按 FolderList 排除）。
    for (template, folders) in folder_lists {
        let f: BTreeSet<&str> = folders.iter().map(String::as_str).collect();
        if !f.is_empty() && f.is_subset(&names) {
            return PackLayout::Standard { template: *template, complete: true };
        }
    }
    // 第二轮：子集判定（观察到的目录全部落在某模板清单内 → 就近归属，可补全缺失目录）。
    for (template, folders) in folder_lists {
        let f: BTreeSet<&str> = folders.iter().map(String::as_str).collect();
        if names.is_subset(&f) {
            return PackLayout::Standard { template: *template, complete: false };
        }
    }
    PackLayout::Unknown
}

/// 一次目录扫描的产物：一级子目录图片索引 + 根图片索引 + 递归全量池（纯收集，无判定）。
#[derive(Debug, Clone)]
pub struct PackScan {
    pub faces_map: BTreeMap<String, BTreeSet<String>>, // 一级子目录名 → 图片名（去扩展名，标准二口径）
    pub root_images: BTreeSet<String>,                 // 根目录图片名（去扩展名）
    /// 递归全量池：相对目录路径（"" = 根，多级用 "/" 分隔）→ 该目录下全部图片词干。
    /// 供 Unknown/兜底"全随机"规则使用；垃圾目录（is_junk_dir）整棵子树不收录。
    pub recursive_pools: BTreeMap<String, BTreeSet<String>>,
}

fn is_image_ext(name: &str) -> bool {
    matches!(name.rsplit('.').next().map(|s| s.to_ascii_lowercase()).as_deref(),
             Some("png") | Some("jpg") | Some("jpeg"))
}

/// 打包/系统垃圾目录：macOS 压缩包解出的 `__MACOSX`、各类 `.` 开头目录（.git 等）。
/// 仅用于过滤 classify 的输入与递归池；faces_map 仍原样收录，标准二 generic 路径行为不受影响。
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
                let stem = Path::new(&fname).file_stem()
                    .map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if !stem.is_empty() { imgs.insert(stem); }
            }
        }
    }
    pools.insert(rel, imgs);
}

/// 重扫描：建 faces_map + root_images + 递归全量池（纯收集；布局判定由调用方用 classify 完成）。
pub fn scan_pack(img_dir: &str) -> Result<PackScan, String> {
    let root = Path::new(img_dir);
    let mut faces_map = BTreeMap::new();
    let mut root_images = BTreeSet::new();
    let mut recursive_pools: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in std::fs::read_dir(root).map_err(|e| format!("读取头像包目录失败: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let fname = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            // faces_map：一级子目录 → 其直接图片（保持旧 FaceMapper::new 口径，标准二专用）。
            let mut imgs = BTreeSet::new();
            if let Ok(files) = std::fs::read_dir(&path) {
                for f in files.flatten() {
                    if f.path().is_file() {
                        let n = f.file_name().to_string_lossy().to_string();
                        let stem = n.split('.').next().unwrap_or("").to_string();
                        if !stem.is_empty() { imgs.insert(stem); }
                    }
                }
            }
            faces_map.insert(fname.clone(), imgs);
            // 递归全量池：垃圾目录整棵跳过。
            if !is_junk_dir(&fname) {
                walk_recursive(&path, fname, &mut recursive_pools);
            }
        } else if is_image_ext(&fname) && !fname.starts_with('.') {
            // 根图词干用 file_stem（与 parse_image_path_in_pack 同规则，"a.1.png"→"a.1"）；
            // 跳过 . 开头文件（macOS AppleDouble 的 ._x.png 会以 "._x" 入池、污染 resolve）。
            let stem = Path::new(&fname).file_stem()
                .map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if !stem.is_empty() { root_images.insert(stem); }
        }
    }
    // 根目录图片也归入递归池（键 = 空串），全随机规则据此连根图一起抽。
    recursive_pools.entry(String::new()).or_default().extend(root_images.iter().cloned());
    Ok(PackScan { faces_map, root_images, recursive_pools })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() }

    /// 构造与内置 template2/template3 默认 FolderList 对应的判定输入。
    fn folder_lists() -> Vec<(u8, Vec<String>)> {
        let cfg = crate::rule_config::RuleConfig::builtin();
        vec![
            (2, cfg.template2.folder_list.clone()),
            (3, cfg.template3.folder_list.clone()),
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
                   PackLayout::Standard { template: 2, complete: true });
        // 子集（缺 1 个）→ fixable
        assert_eq!(classify_with(&all[..13], false),
                   PackLayout::Standard { template: 2, complete: false });
        // 超集（14 全在 + 多出目录，甚至含其他模板的目录名）→ 仍归模板二、complete，
        // 多余目录由规则构造时按 FolderList 过滤、不进抽图池。
        let mut superset: Vec<&str> = FACEPACK_DIRS.to_vec();
        superset.push("Spain");
        superset.push("Extra1");
        assert_eq!(classify_with(&superset, false),
                   PackLayout::Standard { template: 2, complete: true });
    }

    #[test]
    fn classify_standard3_exact_and_superset() {
        let cfg = crate::rule_config::RuleConfig::builtin();
        let f3: Vec<&str> = cfg.template3.folder_list.iter().map(String::as_str).collect();
        assert_eq!(classify_with(&f3, false),
                   PackLayout::Standard { template: 3, complete: true });
        // 子集
        assert_eq!(classify_with(&["China", "Japan", "Korea"], false),
                   PackLayout::Standard { template: 3, complete: false });
        // 超集：50 清单全在 + 旧 65 表的 Denmark/Norway 等多出目录 → 仍归模板三
        let mut superset = f3.clone();
        superset.push("Denmark");
        superset.push("Norway");
        assert_eq!(classify_with(&superset, false),
                   PackLayout::Standard { template: 3, complete: true });
    }

    #[test]
    fn classify_unknown_and_scandinavian_priority() {
        assert_eq!(classify_with(&["African", "NotARealRace"], false), PackLayout::Unknown);
        // 跨两套混用（非超集、非任何子集）→ Unknown
        assert_eq!(classify_with(&["African", "China"], false), PackLayout::Unknown);
        // 仅 Scandinavian（两套清单重合名）→ 子集判定按模板号序优先模板二。
        assert_eq!(classify_with(&["Scandinavian"], false),
                   PackLayout::Standard { template: 2, complete: false });
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
        // faces_map 原样收录垃圾目录（不参与判定），其内 ._a.png 因空词干被排除。
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
        assert!(scan.faces_map.values().all(|s| s.len() == 2)); // e1 + e2（split 词干）
        assert!(scan.root_images.is_empty());
        // 递归池 = 根池（空集，键恒存在）+ 14 个种族目录。
        assert_eq!(scan.recursive_pools.len(), 15);
        assert!(scan.recursive_pools.get("").unwrap().is_empty());
        assert!(scan.recursive_pools.values().skip(1).all(|s| s.len() == 2));
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
}
