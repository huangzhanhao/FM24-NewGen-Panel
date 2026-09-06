//! 【新手导读】头像包"标准识别"的唯一来源：标准常量 + 布局分类 + 目录扫描。
//! 社区头像包有三套组织标准：
//!   标准一：图片直铺（包根全是图片，无子目录）；
//!   标准二：14 个种族目录（African / Asian / ...，历史主格式）；
//!   标准三：65 个细分种族目录（社区新版）。
//! 本模块供 commands.rs（检查完整性/扫描）、face_mapper.rs（映射）、replace_flow.rs（拦截）共用，
//! 不反向依赖它们。
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 14 个种族目录（标准二）。保持历史拼写（含 Seasian）。
pub const FACEPACK_DIRS: [&str; 14] = [
    "African", "Asian", "Caucasian", "Central European", "EECA", "Italmed",
    "MENA", "MESA", "SAMed", "Scandinavian", "Seasian", "South American",
    "SpanMed", "YugoGreek",
];

/// 65 个细分种族目录（标准三）。保持社区原始拼写（含 Finstonia / Staff / WF）。
pub const FACEPACK_DIRS_65: [&str; 65] = [
    "Afroamerican", "Afrocaribbean", "Albania", "AlbanianGreek", "Anglosphere",
    "ArabGulf", "Armenian", "Azerbaijan", "Baltics", "BrazilMixed",
    "CaucasianNA", "Caucasus", "CentralAfrica", "CentralAsian", "CentralEurope",
    "China", "Denmark", "EastAfrica", "EastBalkan", "EastSlavic",
    "Filipino", "Finstonia", "France", "HornOfAfrica", "Hungary",
    "Iceland", "IndigenousSA", "Indonesia", "Iran", "Ireland",
    "Israel", "Italia", "Japan", "Korea", "Maghreb",
    "MainlandSEA", "Malaysia", "Mashriq", "Mestizo", "MixedRace",
    "Mongolia", "Netherlands", "NorthernSA", "Norway", "PacificIslanders",
    "Poland", "Portugal", "Romania", "SahelianAfrica", "Scandinavian",
    "Singapore", "SouthAsia", "SouthConeSA", "SouthernAfrica", "Spain",
    "Staff", "Sweden", "Tajikistan", "Turkish", "Uzbekistan",
    "Vietnam", "WF", "WestAfrica", "WestBalkan", "WestSlavic",
];

/// 头像包布局（三套标准 + 兜底）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackLayout {
    Flat,                          // 标准一：无子目录 + 根目录有图片
    Empty,                         // 无子目录且无图片（空包）
    Standard2 { complete: bool },  // 子目录集合 ⊆ 14 组；complete=恰等于
    Standard3 { complete: bool },  // 子目录集合 ⊆ 65 组；complete=恰等于
    Unknown,                       // 含未知名或跨两套混用
}

impl PackLayout {
    /// 映射为对外的"标准号"：1/2/3；0=无法识别。
    pub fn standard(&self) -> u8 {
        match self {
            PackLayout::Flat | PackLayout::Empty => 1,
            PackLayout::Standard2 { .. } => 2,
            PackLayout::Standard3 { .. } => 3,
            PackLayout::Unknown => 0,
        }
    }
    pub fn is_flat(&self) -> bool { matches!(self, PackLayout::Flat) }
}

/// 唯一的标准判定入口（纯函数，不读盘）。
pub fn classify(subdirs: &BTreeSet<String>, has_root_images: bool) -> PackLayout {
    if subdirs.is_empty() {
        return if has_root_images { PackLayout::Flat } else { PackLayout::Empty };
    }
    let s14: BTreeSet<&str> = FACEPACK_DIRS.iter().copied().collect();
    let s65: BTreeSet<&str> = FACEPACK_DIRS_65.iter().copied().collect();
    let names: BTreeSet<&str> = subdirs.iter().map(String::as_str).collect();
    if names == s14 { return PackLayout::Standard2 { complete: true }; }
    if names == s65 { return PackLayout::Standard3 { complete: true }; }
    // 仅 Scandinavian 一名重合；同时 ⊆ 两套时先命中标准二（历史主格式）。
    if names.is_subset(&s14) { return PackLayout::Standard2 { complete: false }; }
    if names.is_subset(&s65) { return PackLayout::Standard3 { complete: false }; }
    PackLayout::Unknown
}

/// 一次目录扫描的产物：子目录图片索引 + 根图片索引 + 布局。
#[derive(Debug, Clone)]
pub struct PackScan {
    pub faces_map: BTreeMap<String, BTreeSet<String>>, // 一级子目录名 → 图片名（去扩展名，标准二口径）
    pub root_images: BTreeSet<String>,                 // 根目录图片名（去扩展名）
    /// 递归全量池：相对目录路径（"" = 根，多级用 "/" 分隔）→ 该目录下全部图片词干。
    /// 供 Unknown 模板的"全随机"规则使用；垃圾目录（is_junk_dir）整棵子树不收录。
    pub recursive_pools: BTreeMap<String, BTreeSet<String>>,
    pub layout: PackLayout,
}

fn is_image_ext(name: &str) -> bool {
    matches!(name.rsplit('.').next().map(|s| s.to_ascii_lowercase()).as_deref(),
             Some("png") | Some("jpg") | Some("jpeg"))
}

/// 打包/系统垃圾目录：macOS 压缩包解出的 `__MACOSX`、各类 `.` 开头目录（.git 等）。
/// 仅用于过滤 classify 的输入；faces_map 仍原样收录，标准二 generic 路径行为不受影响。
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

/// 重扫描：建 faces_map + root_images + 递归全量池，并调用 classify 得出 layout。
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
    // 根目录图片也归入递归池（键 = 空串），Unknown/全随机规则据此连根图一起抽。
    recursive_pools.entry(String::new()).or_default().extend(root_images.iter().cloned());
    // classify 输入剔除垃圾目录（如 __MACOSX），否则 Mac 打包的平铺包会被判 Unknown。
    let classify_input: BTreeSet<String> =
        faces_map.keys().filter(|n| !is_junk_dir(n)).cloned().collect();
    let layout = classify(&classify_input, !root_images.is_empty());
    Ok(PackScan { faces_map, root_images, recursive_pools, layout })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn classify_flat_and_empty() {
        assert_eq!(classify(&names(&[]), true), PackLayout::Flat);
        assert_eq!(classify(&names(&[]), false), PackLayout::Empty);
    }

    #[test]
    fn classify_standard2_exact_and_subset() {
        assert_eq!(classify(&names(&FACEPACK_DIRS), false), PackLayout::Standard2 { complete: true });
        let subset: BTreeSet<String> = FACEPACK_DIRS[..13].iter().map(|s| s.to_string()).collect();
        assert_eq!(classify(&subset, false), PackLayout::Standard2 { complete: false });
    }

    #[test]
    fn classify_standard3_exact_and_subset() {
        assert_eq!(classify(&names(&FACEPACK_DIRS_65), false), PackLayout::Standard3 { complete: true });
        assert_eq!(classify(&names(&["China", "Japan", "Korea"]), false),
                   PackLayout::Standard3 { complete: false });
    }

    #[test]
    fn classify_unknown_and_scandinavian_priority() {
        assert_eq!(classify(&names(&["African", "NotARealRace"]), false), PackLayout::Unknown);
        assert_eq!(classify(&names(&["African", "China"]), false), PackLayout::Unknown); // 跨两套
        // 仅 Scandinavian（两套唯一重合名）→ 优先标准二（历史主格式）。
        assert_eq!(classify(&names(&["Scandinavian"]), false), PackLayout::Standard2 { complete: false });
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
        assert_eq!(scan.layout, PackLayout::Flat);
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
        assert_eq!(scan.layout, PackLayout::Standard2 { complete: true });
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
        assert_eq!(scan.layout, PackLayout::Unknown); // 非法目录名 → Unknown
        let pools = &scan.recursive_pools;
        assert_eq!(pools.get("").map(|s| s.len()), Some(0)); // 根无图 → 空集（键存在）
        assert_eq!(pools.get("weirdA").map(|s| s.iter().cloned().collect::<Vec<_>>()),
                   Some(vec!["i".to_string()]));
        assert_eq!(pools.get("weirdA/sub").map(|s| s.iter().cloned().collect::<Vec<_>>()),
                   Some(vec!["j.1".to_string()]));
        assert!(!pools.contains_key("weirdA/__MACOSX")); // 垃圾目录整棵跳过
    }
}
