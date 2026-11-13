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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
    Flat,                                      // 标准一：无子目录 + 根目录有图片
    Empty,                                     // 无子目录且无图片（空包）
    Standard { standard: u8, complete: bool }, // 归属标准号；complete = FolderList 目录全部存在（可有 extra）
    Unknown,                                   // 与任何标准的 FolderList 都不构成包含关系
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

/// 判定与「检查完整性」共用的目录口径：包内**实际存在**的目录集合。
///
/// 输入是「一级子目录名」与「多级相对路径」（递归池的键，或浅层遍历得到的路径）。
/// 输出扣掉两类条目：
///   ① 包根 `""` —— 它不是目录名，放进白名单比对只会制造假 Unknown；
///   ② **纯容器目录**（`WF` / `Staff`：自身只是给子树提供路径前缀，且**不在任何 FolderList 里**）
///      —— 不扣的话，带 `WF/<ethnic>` 子树的包会因多出一个 `WF` 名字而在子集轮里落空。
///      只扣「非白名单名」：标准二的 `African` 本身是白名单条目，当它下面再分
///      `African/African1` 子目录（GUNZO 编号包）时若一并扣掉，14 项目录全部消失 → 整包误判 Unknown。
///      判定「是容器」因此需要两条：存在别的键以它 + "/" 为前缀，且它不在任何标准的 FolderList 中。
fn effective_dirs<'a>(
    subdirs: &'a BTreeSet<String>,
    pool_keys: &'a BTreeSet<String>,
    configured: &BTreeSet<&str>,
) -> BTreeSet<&'a str> {
    let containers: BTreeSet<&str> = pool_keys
        .iter()
        .flat_map(|k| k.match_indices('/').map(move |(i, _)| &k[..i]))
        .collect();
    pool_keys
        .iter()
        .map(String::as_str)
        .chain(subdirs.iter().map(String::as_str))
        .filter(|n| !n.is_empty() && (!containers.contains(*n) || configured.contains(*n)))
        .collect()
}

/// 所有标准 FolderList 的目录名并集：判定「纯容器」时用来排除本身是白名单条目的目录
/// （如 `African` 虽是 `African/African1` 的前缀，但它本身必须留在判定集里）。
fn configured_names<'b>(folder_lists: &'b [(u8, &Vec<String>)]) -> BTreeSet<&'b str> {
    folder_lists
        .iter()
        .flat_map(|(_, folders)| folders.iter().map(String::as_str))
        .collect()
}

/// 唯一的标准判定入口（纯函数，不读盘）。
/// subdirs：包内**一级**子目录名；pool_keys：递归全池的键（含 ""、一级名、多级路径）。
/// folder_lists：各标准的 (标准号, FolderList)，来自 templates（用户可配置），按登记顺序传入。
/// 判定顺序：**超集优先**（S ⊇ F → 归属该标准），其次**子集**（S ⊂ F → 就近归属，标 fixable）。
/// `complete` 的口径是「该标准配置的目录**都在**包内」，**允许包外还有别的目录**（超集 → true）：
/// 用户常在一个包里放多套素材，多余目录不参与抽图即可，不必因此判成「不完整」并催用户删目录。
/// 交叉/未知名 → Unknown。
/// 【为什么先命中先赢是安全的】`templates::validate_registry` 保证任意两份清单交集为 0，
/// 于是一个包的目录集不可能同时是两份清单的超集；子集轮里多个标准命中同一包的情况也被同一守卫排除。
/// 【为什么要 pool_keys】FolderList 里可以有 `WF/x`、`Staff/r/male` 这类**多级**条目，
/// 它们在一级目录名里根本不存在；判定要沿递归池的相对路径比对，否则带子树的包会被误判 Unknown。
pub fn classify(
    subdirs: &BTreeSet<String>,
    pool_keys: &BTreeSet<String>,
    has_root_images: bool,
    folder_lists: &[(u8, &Vec<String>)],
) -> PackLayout {
    if subdirs.is_empty() {
        return if has_root_images { PackLayout::Flat } else { PackLayout::Empty };
    }
    let names = effective_dirs(subdirs, pool_keys, &configured_names(folder_lists));
    // 第一轮：超集判定（包含某标准全部目录 → 归属且视为完整；超集之间互斥——两份清单交集为 0）。
    // complete = 标准配置的全部目录都在包内（S 可以多出 extra 目录，由调用方按 FolderList 排除）。
    for (n, folders) in folder_lists {
        let f: BTreeSet<&str> = folders.iter().map(String::as_str).collect();
        if !f.is_empty() && f.is_subset(&names) {
            return PackLayout::Standard { standard: *n, complete: true };
        }
    }
    // 第二轮：子集判定（观察到的目录全部落在某标准清单内 → 仍归属该标准、标 fixable，可补全缺失目录）。
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

/// 头像包目录的轻量签名：用来判断缓存的扫描索引是否还能代表磁盘现状。
///
/// 【为什么需要】索引按 `img_dir` 字符串命中就复用（见 `commands::FaceIndex`），而用户完全可能在
/// 两次替换之间动过包里的文件。索引一旦过期，抽图就会写出**指向不存在图片**的映射
/// （FM 里那名球员显示空白头像），而且**静默**——不校验的话没有任何人会发现。
///
/// 【只统计「会影响抽图池」的条目】每个非垃圾一级子目录记 (名字, mtime, 直接子条目数)，
/// 每个非隐藏根图记 (名字, mtime, 0)。判定只看扩展名，所以 `config.xml` / `config_backup_*.xml` / README
/// 这类非图片文件天然不入签名——
/// 否则替换自己写完 config.xml 就会让缓存失效，下次又要重扫 28 万文件（这正是要避免的）。
///
/// 【为什么 mtime 与条目数都要】两者互补：改名只动 mtime（条目数不变），同秒内「删一张再加一张」
/// 只动条目数（mtime 可能相同）。名字也在签名里，所以一级目录被改名同样能被发现。
///
/// 【能拦住】删/改名某张图片（所在一级目录 mtime 变）✓ 增删一级目录或根图（名字集合变）✓
///   一级目录内增删子目录/图片（直接子条目数变）✓
/// 【拦不住】孙目录内的**改名**（一级目录的 mtime 与直接条目数都可能不变）——已知的残余窗口，
///   只影响会递归到孙目录的池（Unknown / 标准三兜底），用「不重扫 28 万文件」换这一小段窗口。
/// 【为什么不递归算】递归一遍就等于把扫盘又做了一次，缓存也就失去意义了。
/// 【失败方向一律是安全的】目录 mtime 在 NTFS 上是「句柄关闭时」才落定的（实测刚写完立刻读，
/// 与 1.6 ms 后再读能差 15969 个 100 ns tick），加上 FAT32/SMB 的粗精度，签名可能**多失配一次**
/// → 白扫一遍盘；但不会在包真变了的时候误判命中。宁可偶尔多扫一次，也不写出指向不存在图片的映射。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackSig {
    entries: Vec<(String, Option<std::time::SystemTime>, usize)>,
}

/// 读取头像包目录的签名（只看顶层 + 每个一级目录的直接子条目数，不递归）。
/// 目录读不到时返回 None —— 调用方据此判定「无法确认缓存有效」而重扫（安全方向）。
pub fn pack_sig(img_dir: &str) -> Option<PackSig> {
    let rd = std::fs::read_dir(img_dir).ok()?;
    let mut entries: Vec<(String, Option<std::time::SystemTime>, usize)> = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let is_dir = entry_is_dir(&e);
        // 只跟踪会进池的条目：非垃圾目录、非隐藏图片。其余（config.xml / 备份 / 说明文件）跳过。
        if is_dir {
            if is_junk_dir(&name) { continue; }
        } else if !is_image_ext(&name) || name.starts_with('.') {
            continue;
        }
        // DirEntry::metadata 不跟随符号链接（软链自身的时间戳即可反映它有没有被换掉）。
        let mtime = e.metadata().and_then(|m| m.modified()).ok();
        // 用条目名排序而非 read_dir 顺序：ext4/APFS 给的是哈希序，NTFS 近似名字序，
        // 不排序的话两次计算无法逐项比对。
        let count = if is_dir { pool_entry_count(&e.path()) } else { 0 };
        entries.push((name, mtime, count));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Some(PackSig { entries })
}

/// 一级目录里「会影响抽图池的直接子条目」数：非垃圾子目录 + 非隐藏图片。
/// 口径与 walk_recursive 的收录规则一致，所以 Thumbs.db / desktop.ini 之类不会造成无谓的重扫。
fn pool_entry_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    if entry_is_dir(e) { !is_junk_dir(&n) } else { is_image_ext(&n) && !n.starts_with('.') }
                })
                .count()
        })
        .unwrap_or(0)
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

/// 目录判定（扫描侧**唯一口径**）：优先用 readdir 自带的文件类型（Unix 的 `d_type` / Windows 的
/// `FIND_DATA` 属性），省掉每个条目一次 metadata 系统调用 —— 大包（28 万条目）扫盘的主要成本就在这里。
///
/// 【为什么不能直接写 `e.file_type().is_dir()`】两者只在**符号链接**上分道扬镳，而那个分歧恰好跨平台：
/// `file_type()` **不跟随**符号链接，`Path::is_dir()` **跟随**。Linux/macOS 上把大包软链进包根
/// （`~/packs/NG -> /mnt/big/NG`）是常规用法；若把符号链接目录判成文件，`is_image_ext` 不认它，
/// **整棵子树会静默消失**（索引为空 → 布局退化成 Empty → 名单里所有球员被跳过，只留一条 WARNING）。
/// 因此只在「符号链接」这一种情况下回退一次 `is_dir()`；普通目录/文件仍零额外系统调用。
/// Windows 的 junction 自带 DIRECTORY 属性，`file_type().is_dir()` 直接为真，走不进回退分支。
/// 【d_type 缺失】XFS(ftype=0)/NFS/FUSE/exFAT 上 readdir 不给类型（`file_type()` 返回 Err），
/// 此时退回 `is_dir()`（一次 stat），与改造前行为一致 —— 这些环境省不掉系统调用。
///
/// 【为何是自由函数】扫盘的两趟递归、第一趟列顶层、以及「检查完整性」四处都靠它判目录；
/// 抽出来才有单一真源 —— 否则「检查完整性」认符号链接目录、抽图池不认，missing/extra 与池内容对不上。
fn entry_is_dir(e: &std::fs::DirEntry) -> bool {
    match e.file_type() {
        Ok(t) => t.is_dir() || (t.is_symlink() && e.path().is_dir()),
        Err(_) => e.path().is_dir(),
    }
}

/// 递归收集 dir 及其全部子/孙目录的图片（词干用 file_stem，跳过 `.` 开头文件）。
/// 键为相对头像包根的目录路径（"" = 根，多级用 "/" 分隔）；垃圾目录整棵子树跳过。
fn walk_recursive(dir: &Path, rel: String, pools: &mut BTreeMap<String, BTreeSet<String>>) {
    let mut imgs = BTreeSet::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            // 目录判定走 entry_is_dir：普通条目零额外系统调用。
            if entry_is_dir(&e) {
                if is_junk_dir(&fname) { continue; } // 垃圾目录整棵不入池
                let child = if rel.is_empty() { fname } else { format!("{rel}/{fname}") };
                walk_recursive(&e.path(), child, pools);
            } else if is_image_ext(&fname) && !fname.starts_with('.') {
                if let Some(stem) = image_stem(&fname) { imgs.insert(stem); }
            }
        }
    }
    pools.insert(rel, imgs);
}

/// 并行扫盘的最小一级子目录数：低于它时不起线程（起线程与合并的开销摊不平）。
/// 标准二 14 个目录、标准三 62 个目录；实测 28 万图那次扫盘 14.6–39 s，正是这里的目标。
const PARALLEL_SCAN_MIN_DIRS: usize = 4;

/// 并行度上限。再多也只是把并发的 metadata 请求堆在文件系统与杀软实时扫描那一层，
/// 反而加剧争抢（实测同一目录树两次扫描 24 s ↔ 36 s，说明瓶颈在 I/O 而不在 CPU）。
const PARALLEL_SCAN_MAX_THREADS: usize = 8;

/// 遍历一个一级子目录（含其全部子孙），返回「相对头像包根的目录路径 → 词干集合」。
/// 键空间只覆盖该一级目录及其子孙（前缀就是它的名字），因此不同目录的成果互不相交，可并行后合并。
/// 垃圾目录整棵不入池（该目录自身仍会由调用方在 faces_map 里留下空集键）。
fn scan_one_subdir(name: &str, path: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let mut local = BTreeMap::new();
    if !is_junk_dir(name) {
        walk_recursive(path, name.to_string(), &mut local);
    }
    local
}

/// 把一个一级子目录的遍历成果合并进全局递归池。
/// 【为何用 insert 而非 extend】键空间本不相交，两者等价；但万一上游键规则变了导致撞键，
/// insert 会因为 debug 断言立刻暴露，而 extend 会静默把两份合成一份（图凭空少一半且无声）。
fn merge_into(
    target: &mut BTreeMap<String, BTreeSet<String>>,
    local: BTreeMap<String, BTreeSet<String>>,
) {
    for (k, v) in local {
        debug_assert!(!target.contains_key(&k), "一级子目录的键空间不应相交：{k}");
        target.insert(k, v);
    }
}

/// 本次扫盘用几个线程：拿不到 CPU 数、只剩一个 CPU、或目录太少时都退化为 1（顺序遍历）。
fn scan_threads(total_dirs: usize) -> usize {
    if total_dirs < PARALLEL_SCAN_MIN_DIRS {
        return 1;
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(PARALLEL_SCAN_MAX_THREADS)
}

/// 顺序遍历全部一级子目录并报进度（每完成一个报一次）。`cancel` 置位即停在中途（不再开新目录）。
fn build_pools_sequential(
    subdirs: &[(String, PathBuf)],
    recursive_pools: &mut BTreeMap<String, BTreeSet<String>>,
    progress: &mut dyn FnMut(usize, usize),
    cancel: &AtomicBool,
) {
    for (i, (fname, path)) in subdirs.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        merge_into(recursive_pools, scan_one_subdir(fname, path));
        progress(i + 1, subdirs.len());
    }
}

/// 并行遍历全部一级子目录：N 个 worker 用原子下标抢任务（work-stealing，谁快谁多干），
/// 每个子目录的成果单独发回主线程，由主线程合并并上报进度。
/// 【为什么进度只能在主线程报】进度回调是 `&mut dyn FnMut`，既不是 Sync 也不能并发调用；
/// 所以 worker 只负责遍历、把结果发回来，所有的合并与报进度都在主线程做。
/// 【为什么结果与顺序遍历逐字相同】各子目录键空间互不相交，合并顺序无关；
/// 池内部是 BTreeMap/BTreeSet，插入顺序也不影响内容。
fn build_pools_parallel(
    subdirs: &[(String, PathBuf)],
    recursive_pools: &mut BTreeMap<String, BTreeSet<String>>,
    progress: &mut dyn FnMut(usize, usize),
    threads: usize,
    cancel: &AtomicBool,
) {
    let total = subdirs.len();
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..threads.min(total) {
            let tx = tx.clone();
            let next = &next;
            scope.spawn(move || loop {
                // worker 也要看取消标志：主线程只是不再收成果，光靠它 break 的话
                // 剩下的目录照样被各 worker 扫完，取消等于没生效。
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((fname, path)) = subdirs.get(i) else { return };
                if tx.send(scan_one_subdir(fname, path)).is_err() { return; }
            });
        }
        // 主线程自己那份 Sender 必须丢掉：否则 rx 要等它被 drop 才结束（worker 退出后仍然挂着）。
        drop(tx);
        let mut done = 0usize;
        for local in rx {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            merge_into(recursive_pools, local);
            done += 1;
            progress(done, total);
        }
    });
}

/// 第二趟：为每个一级子目录建递归池（目录足够多时并行），每完成一个报一次进度。
fn build_pools(
    subdirs: &[(String, PathBuf)],
    recursive_pools: &mut BTreeMap<String, BTreeSet<String>>,
    progress: &mut dyn FnMut(usize, usize),
    cancel: &AtomicBool,
) {
    let threads = scan_threads(subdirs.len());
    if threads <= 1 {
        build_pools_sequential(subdirs, recursive_pools, progress, cancel);
    } else {
        build_pools_parallel(subdirs, recursive_pools, progress, threads, cancel);
    }
}

/// 重扫描：建 faces_map + root_images + 递归全量池（纯收集；布局判定由调用方用 classify 完成）。
/// 【仅测试】生产的两条路径（「扫描头像」命令、替换流水线）都要报进度，一律走 scan_pack_progress；
/// 留这个无进度版只是让测试少写一个空回调，故标 cfg(test) 免得在发布构建里报 dead_code。
#[cfg(test)]
pub fn scan_pack(img_dir: &str) -> Result<PackScan, String> {
    scan_pack_progress(img_dir, &mut |_, _| {}, &AtomicBool::new(false))
}

/// 同 `scan_pack`，额外把「已扫完第 n 个 / 共 total 个一级子目录」上报给进度回调。
/// 【为何以一级子目录为进度单位】第一趟只列目录拿到分母、第二趟逐个扫（每个含整棵子树），
/// 不必为算进度多遍历一遍磁盘，又让「扫描头像」的进度条真实反映工作量（标准二 14 目录 → 14 步）。
/// `cancel` 置位即中止并返回 Err（替换流水线的取消要能在最耗时这一步生效；无取消入口的调用方
/// 给一个恒为 false 的临时值即可）。半途而废的索引一律以 Err 抛出，绝不返回残缺的 PackScan——
/// 那会被 classify 判成另一种布局，把「用户按了取消」说成「这个包不合法」。
pub fn scan_pack_progress(
    img_dir: &str,
    progress: &mut dyn FnMut(usize, usize),
    cancel: &AtomicBool,
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
        // 目录判定走 entry_is_dir（唯一口径）——软链目录在这里也必须算目录，否则整棵子树不进索引。
        if entry_is_dir(&entry) {
            subdirs.push((fname, path));
        } else if is_image_ext(&fname) && !fname.starts_with('.') {
            // 根图词干走 image_stem 唯一口径（与 parse_image_path_in_pack 同规则，"a.1.png"→"a.1"）；
            // 跳过 . 开头文件（macOS AppleDouble 的 ._x.png 会以 "._x" 入池、污染 resolve）。
            if let Some(stem) = image_stem(&fname) { root_images.insert(stem); }
        }
    }
    // 第二趟：逐个一级子目录建递归池（目录够多时并行遍历，见 build_pools），每完成一个报一次进度。
    build_pools(&subdirs, &mut recursive_pools, progress, cancel);
    // 取消检查放在合并之后、建索引之前：半途的 recursive_pools 缺目录，喂给 classify 会判出
    // 另一个标准（甚至判成不合法），必须在这里就断掉，让调用方按「已取消」收场。
    if cancel.load(Ordering::Relaxed) {
        return Err("头像包扫描已取消".into());
    }
    // faces_map：一级子目录 → 其直接图片词干。**直接从 recursive_pools 取**，不单独 read_dir：
    // 标准二主池按 FolderList 过滤 faces_map、标准三也拿它判目录名，而标准三 / Unknown 的池取
    // recursive_pools——两份索引同源取值才保证逐字同键同值；词干规则一旦分歧（"a.1.png" 在两边
    // 一个成 "a" 一个成 "a.1"），同一张图就有两个身份，禁重复与 Preserve 剔除只摘得掉一份、
    // 这张图会被发出两次。顺带省掉每个一级目录的第二遍 read_dir。
    for (fname, _path) in subdirs.iter() {
        let imgs = recursive_pools.get(fname).cloned().unwrap_or_default();
        faces_map.insert(fname.clone(), imgs);
    }
    // 根目录图片也归入递归池（键 = 空串），全随机规则据此连根图一起抽。
    recursive_pools.entry(String::new()).or_default().extend(root_images.iter().cloned());
    Ok(PackScan { faces_map, root_images, recursive_pools })
}

/// 扫描结果的布局判定：垃圾目录（__MACOSX 等）不参与，与 FaceMapper::from_scan 同口径。
/// 「扫描头像」与「建立映射索引」两处共用本函数，避免各自重列一遍 faces_map 过滤。
pub fn classify_scan(scan: &PackScan, folder_lists: &[(u8, &Vec<String>)]) -> PackLayout {
    let subdirs: BTreeSet<String> = scan
        .faces_map
        .keys()
        .filter(|n| !is_junk_dir(n))
        .cloned()
        .collect();
    // 判定要看到多级目录（`WF/x`、`Staff/r/male`），所以把递归池的键一起喂进去 ——
    // 只给一级目录名的话，带子树的包会被判成 Unknown。
    let pool_keys: BTreeSet<String> = scan.recursive_pools.keys().cloned().collect();
    classify(&subdirs, &pool_keys, !scan.root_images.is_empty(), folder_lists)
}

/// 「扫描头像」核心（纯函数）：重扫描头像包，返回 (图片总数, 标准号, 完整扫描结果)。
/// 标准号按 templates 的 FolderList 判定（用户可配置，不硬编码）；总数口径见 PackScan::total_for。
/// 额外把扫描进度透传给 `scan_pack_progress`（「扫描头像」状态栏进度用）。
pub fn scan_and_total_progress(
    img_dir: &str,
    folder_lists: &[(u8, &Vec<String>)],
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(usize, u8, PackScan), String> {
    // 「扫描头像」命令没有取消入口（状态栏进度只报不做），给它一个恒为 false 的标志走同一条实现。
    let scan = scan_pack_progress(img_dir, progress, &AtomicBool::new(false))?;
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
            let fname = e.file_name().to_string_lossy().to_string();
            // 目录判定与扫盘同源（entry_is_dir）：否则「检查完整性」认软链目录、「扫描头像」不认，
            // missing/extra 与实际抽图池就对不上了。
            let is_dir = entry_is_dir(&e);
            // 子目录全收（垃圾目录在下面判定时再滤）；根图只认非隐藏的图片文件（与 scan_pack 口径一致）。
            if is_dir || (!fname.starts_with('.') && is_image_ext(&fname)) {
                entries.push((is_dir, fname));
            }
        }
    }
    let total = entries.len(); // 进度分母（空包/读不到目录时为 0）

    // 第二趟：逐条核对，每核对一条上报一次进度。
    let mut subdirs: BTreeSet<String> = BTreeSet::new();
    let mut pool_keys: BTreeSet<String> = BTreeSet::new();
    let mut has_images = false;
    for (i, (is_dir, fname)) in entries.iter().enumerate() {
        if *is_dir {
            if !is_junk_dir(fname) {
                subdirs.insert(fname.clone());
                pool_keys.insert(fname.clone());
                // 再下沉两层：覆盖 `WF/<ethnic>` 与 `Staff/<region>/{male,female}`。
                // 只做浅层遍历、不递归全树 —— 「检查完整性」必须比全量扫盘快一个数量级。
                if let Ok(rd) = std::fs::read_dir(Path::new(img_dir).join(fname)) {
                    for sub in rd.flatten() {
                        let subname = sub.file_name().to_string_lossy().to_string();
                        if !entry_is_dir(&sub) || is_junk_dir(&subname) {
                            continue;
                        }
                        let two = format!("{fname}/{subname}");
                        pool_keys.insert(two.clone());
                        if let Ok(rd2) = std::fs::read_dir(Path::new(img_dir).join(&two)) {
                            for leaf in rd2.flatten() {
                                let leafname = leaf.file_name().to_string_lossy().to_string();
                                if entry_is_dir(&leaf) && !is_junk_dir(&leafname) {
                                    pool_keys.insert(format!("{two}/{leafname}"));
                                }
                            }
                        }
                    }
                }
            } // 跳过 __MACOSX 等
        } else {
            has_images = true; // 走到这里的一定是非隐藏图片文件（隐藏文件上一步已被滤掉）
        }
        progress(i + 1, total);
    }

    let layout = classify(&subdirs, &pool_keys, has_images, folder_lists);
    let standard = layout.standard();
    let status = match layout {
        PackLayout::Flat => "complete",
        PackLayout::Empty | PackLayout::Unknown => "unknown",
        PackLayout::Standard { complete: true, .. } => "complete",
        PackLayout::Standard { .. } => "fixable",
    };
    // missing / extra / expected_total 按归属标准的 FolderList 计算：
    // missing = FolderList 中尚未出现的目录（可一键补全）；extra = 包内多出的未配置目录（不进抽图池）。
    // 【必须按 `pool_keys` 判存在】清单含 `WF/x`、`Staff/r/male` 这类多级条目，只看一级目录名
    // 会把 64 条全部报成"缺失"，让完美包在 UI 上显示为需要补全。
    let (missing, extra, expected_total) = match &layout {
        PackLayout::Standard { standard: n, .. } => {
            let folders = folder_lists
                .iter()
                .find(|(t, _)| t == n)
                .map(|(_, f)| (*f).clone())
                .unwrap_or_default();
            let missing: Vec<String> = folders
                .iter()
                .filter(|d| !pool_keys.contains(*d))
                .cloned()
                .collect();
            // 纯容器目录（`WF` / `Staff`）只是多级条目的路径前缀，不算多余目录。
            let containers: BTreeSet<&str> = folders
                .iter()
                .flat_map(|d| d.match_indices('/').map(move |(i, _)| &d[..i]))
                .collect();
            let extra: Vec<String> = subdirs
                .iter()
                .filter(|d| !folders.contains(*d) && !containers.contains(d.as_str()))
                .cloned()
                .collect();
            (missing, extra, folders.len())
        }
        _ => (Vec::new(), Vec::new(), 0),
    };
    // 标准一/无法识别的 found 与现状一致：标准一记 0，其余记命中的子目录数；
    // 有归属标准时记**已就位的必需目录数**（= expected_total − missing），与 missing 同口径。
    let found = if standard == 1 {
        0
    } else if expected_total > 0 {
        expected_total - missing.len()
    } else {
        subdirs.len()
    };
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

    /// 单参版：把一级目录名同时当作「递归池的键」（无嵌套子树的包就是这种形态）。
    fn classify_with(subdirs: &[&str], has_root: bool) -> PackLayout {
        let fl = folder_lists();
        let refs: Vec<(u8, &Vec<String>)> = fl.iter().map(|(t, f)| (*t, f)).collect();
        classify(&names(subdirs), &names(subdirs), has_root, &refs)
    }

    /// 带多级递归池键的版本：`extra` 是包里除一级目录外的相对路径（如 `WF/China`）。
    fn classify_with_pools(subdirs: &[&str], extra: &[&str], has_root: bool) -> PackLayout {
        let fl = folder_lists();
        let refs: Vec<(u8, &Vec<String>)> = fl.iter().map(|(t, f)| (*t, f)).collect();
        let mut keys = names(extra);
        keys.extend(names(subdirs));
        classify(&names(subdirs), &keys, has_root, &refs)
    }

    #[test]
    fn classify_covers_all_layouts() {
        // 主题：布局判定矩阵 —— 平铺/空包、两套标准的恰好/子集/超集、女性包、未知名与跨套混用。
        // ① 平铺与空包。
        assert_eq!(classify_with(&[], true), PackLayout::Flat);
        assert_eq!(classify_with(&[], false), PackLayout::Empty);
        // ② 标准二：恰好 14 个 → complete；缺 1 个 → fixable；超集（甚至含他标准目录名）→
        // 仍归标准二、complete，多余目录由规则构造时按 FolderList 过滤、不进抽图池。
        let all: Vec<&str> = FACEPACK_DIRS.to_vec();
        assert_eq!(classify_with(&all, false),
                   PackLayout::Standard { standard: 2, complete: true });
        assert_eq!(classify_with(&all[..13], false),
                   PackLayout::Standard { standard: 2, complete: false });
        let mut superset2: Vec<&str> = FACEPACK_DIRS.to_vec();
        superset2.push("Spain");
        superset2.push("Extra1");
        assert_eq!(classify_with(&superset2, false),
                   PackLayout::Standard { standard: 2, complete: true });
        // ③ 标准三：恰好全量 → complete；子集 → fixable；超集 → complete。
        let cfg = crate::templates::RuleConfig::builtin();
        let f3: Vec<&str> = cfg.standard3.folder_list.iter().map(String::as_str).collect();
        assert_eq!(classify_with(&f3, false),
                   PackLayout::Standard { standard: 3, complete: true });
        assert_eq!(classify_with(&["China", "Japan", "Korea"], false),
                   PackLayout::Standard { standard: 3, complete: false });
        let mut superset3 = f3.clone();
        superset3.push("Scandinavian");
        superset3.push("SomeUnusedDir");
        assert_eq!(classify_with(&superset3, false),
                   PackLayout::Standard { standard: 3, complete: true });
        // ④ 只有 `WF` 一级目录 + 其子树的女性包：容器名不算白名单条目，按子集判 → 标准三、不完整。
        let wf_count = cfg.standard3.folder_list.iter().filter(|d| d.starts_with("WF/")).count();
        assert_eq!(wf_count, 32, "内置清单里应有 32 条 WF 目录");
        assert_eq!(classify_with_pools(&["WF"], &["WF/African", "WF/China"], false),
                   PackLayout::Standard { standard: 3, complete: false });
        // ⑤ 未知名/跨套混用 → Unknown；仅 Scandinavian（只属标准二）→ 标准二子集。
        assert_eq!(classify_with(&["African", "NotARealRace"], false), PackLayout::Unknown);
        assert_eq!(classify_with(&["African", "China"], false), PackLayout::Unknown);
        assert_eq!(classify_with(&["Scandinavian"], false),
                   PackLayout::Standard { standard: 2, complete: false });
        // ⑥ 标准二的种族目录下再分子目录（GUNZO 编号包，如 African/African1）：
        //    种族名本身是白名单条目，不能因为「有子目录」就被当纯容器剥掉，否则整包误判 Unknown。
        assert_eq!(
            classify_with_pools(&all, &["African/African1", "Asian/Asian1"], false),
            PackLayout::Standard { standard: 2, complete: true }
        );
    }

    #[test]
    fn scan_pack_covers_flat_standard2_and_nesting() {
        // 主题：扫盘的三种形态 —— 平铺（含垃圾/隐藏/辅助文件）、标准二 14 目录、多层嵌套。
        // ① 平铺：根图只收非隐藏图片，垃圾目录与辅助文件不入池。
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
        // ② 标准二 14 目录：每个目录 2 张（e1 png + e2 jpg，词干唯一口径），根池空。
        let dir2 = tempfile::tempdir().unwrap();
        for eth in FACEPACK_DIRS {
            let d = dir2.path().join(eth);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("e1.png"), b"png").unwrap();
            std::fs::write(d.join("e2.jpg"), b"jpg").unwrap();
        }
        let scan2 = scan_pack(dir2.path().to_str().unwrap()).unwrap();
        assert_eq!(scan2.faces_map.len(), 14);
        assert!(scan2.faces_map.values().all(|s| s.len() == 2));
        assert!(scan2.root_images.is_empty());
        // 递归池 = 根池（空集，键恒存在）+ 14 个种族目录。
        assert_eq!(scan2.recursive_pools.len(), 15);
        assert!(scan2.recursive_pools.get("").unwrap().is_empty());
        assert!(scan2.recursive_pools.values().skip(1).all(|s| s.len() == 2));
        // ③ 多层嵌套：子/孙目录全部图片进入递归池，键为 "/" 分隔的相对路径；垃圾子树跳过。
        let dir3 = tempfile::tempdir().unwrap();
        let p = dir3.path();
        std::fs::create_dir_all(p.join("weirdA/sub")).unwrap();
        std::fs::create_dir_all(p.join("weirdA/__MACOSX")).unwrap();
        std::fs::write(p.join("weirdA/i.png"), b"png").unwrap();
        std::fs::write(p.join("weirdA/sub/j.1.png"), b"png").unwrap();
        std::fs::write(p.join("weirdA/__MACOSX/x.png"), b"png").unwrap();
        let scan3 = scan_pack(p.to_str().unwrap()).unwrap();
        let pools = &scan3.recursive_pools;
        assert_eq!(pools.get("").map(|s| s.len()), Some(0)); // 根无图 → 空集（键存在）
        assert_eq!(pools.get("weirdA").map(|s| s.iter().cloned().collect::<Vec<_>>()),
                   Some(vec!["i".to_string()]));
        assert_eq!(pools.get("weirdA/sub").map(|s| s.iter().cloned().collect::<Vec<_>>()),
                   Some(vec!["j.1".to_string()]));
        assert!(!pools.contains_key("weirdA/__MACOSX")); // 垃圾目录整棵跳过
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

    /// 等到目录签名稳定后再取值。
    /// 【为什么必须等】NTFS 的目录 mtime 是**句柄关闭时**才落定的：刚写完文件立刻读可能读到旧值，
    /// 过一两毫秒再读才是最终值（实测第一次取值与 1.6 ms 后的取值差 15969 个 100ns tick）。
    /// 生产侧这条只会让缓存「多失效一次」（＝重扫，安全方向），但测试里必须等到稳定才谈得上比对。
    fn stable_sig(dir: &Path) -> PackSig {
        let mut prev = pack_sig(dir.to_str().unwrap()).unwrap();
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(25));
            let now = pack_sig(dir.to_str().unwrap()).unwrap();
            if now == prev { return now; }
            prev = now;
        }
        prev
    }

    #[test]
    fn pack_sig_tracks_only_pool_affecting_entries() {
        // 签名是「索引缓存能不能复用」的唯一判据，两条都必须成立：
        //   ① 影响抽图池的改动 → 签名变（否则缓存过期，会写出指向不存在图片的映射）；
        //   ② 与池无关的改动 → 签名不变（否则替换自己写 config.xml 就会让缓存失效、下次白扫 28 万图）。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        std::fs::create_dir_all(p.join("African")).unwrap();
        std::fs::write(p.join("African").join("a.png"), b"png").unwrap();
        std::fs::write(p.join("African").join("Thumbs.db"), b"x").unwrap(); // 非图片，不进池
        std::fs::write(p.join("root.png"), b"png").unwrap();
        std::fs::write(p.join("config.xml"), b"<x/>").unwrap();
        let s1 = stable_sig(p);

        // ② 与池无关的改动：替换收尾写 config.xml、生成备份、用户丢个 README —— 签名必须不变。
        std::fs::write(p.join("config.xml"), b"<y/>").unwrap();
        std::fs::write(p.join("config_backup_20260101-000000.xml"), b"<z/>").unwrap();
        std::fs::write(p.join("README.md"), b"md").unwrap();
        assert_eq!(stable_sig(p), s1, "非池文件不得影响签名");

        // ① 一级目录内新增图片 → 直接子条目数变 → 签名变。
        std::fs::write(p.join("African").join("b.png"), b"png").unwrap();
        let s2 = stable_sig(p);
        assert_ne!(s2, s1, "一级目录内新增图片应改变签名");

        // ① 一级目录内**改名**（条目数不变，只有 mtime 变）→ 也要能被发现。
        std::fs::rename(p.join("African").join("b.png"), p.join("African").join("c.png")).unwrap();
        let s3 = stable_sig(p);
        assert_ne!(s3, s2, "一级目录内改名应改变签名（靠 mtime）");

        // ① 新增/删除一级目录与根图 → 名字集合变。
        std::fs::create_dir_all(p.join("Asian")).unwrap();
        let s4 = stable_sig(p);
        assert_ne!(s4, s3, "新增一级目录应改变签名");
        std::fs::remove_file(p.join("root.png")).unwrap();
        assert_ne!(stable_sig(p), s4, "删除根图应改变签名");

        // ① 垃圾目录不进池，也不该进签名（否则 __MACOSX 一出现就白扫一遍）。
        let s5 = stable_sig(p);
        std::fs::create_dir_all(p.join("__MACOSX")).unwrap();
        std::fs::create_dir_all(p.join(".git")).unwrap();
        assert_eq!(stable_sig(p), s5, "垃圾目录不得影响签名");

        // 读不到的目录 → None（调用方据此判定「无法确认缓存有效」而重扫）。
        assert!(pack_sig(p.join("nonexistent").to_str().unwrap()).is_none());
    }

    #[test]
    fn parallel_scan_merges_identically_and_gates_thread_count() {
        // 主题：并行扫盘。① 合并结果与顺序遍历逐字相同；② 线程数按目录量闸门。
        // 并行只改「遍历的调度」，不得改变任何池内容。这里直接调并行实现（不依赖测试机的 CPU 数），
        // 与顺序遍历逐字对比——覆盖垃圾子树跳过、dot 文件排除、孙目录独立成键三个易错点。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        for (d, n) in [
            ("African", 3),
            ("Asian", 2),
            ("Caucasian", 4),
            ("EECA", 1),
            ("Italmed", 2),
        ] {
            std::fs::create_dir_all(p.join(d)).unwrap();
            for i in 0..n {
                std::fs::write(p.join(d).join(format!("f{i}.png")), b"png").unwrap();
            }
        }
        std::fs::create_dir_all(p.join("EECA").join("sub")).unwrap();
        std::fs::write(p.join("EECA").join("sub").join("deep.1.jpg"), b"jpg").unwrap();
        std::fs::write(p.join("EECA").join("._hidden.png"), b"png").unwrap(); // 隐藏名不入池
        std::fs::write(p.join("EECA").join("notes.txt"), b"t").unwrap();      // 非图片不入池
        std::fs::create_dir_all(p.join("African").join("__MACOSX")).unwrap();
        std::fs::write(p.join("African").join("__MACOSX").join("x.png"), b"png").unwrap();

        // 与 scan_pack_progress 第一趟同口径的顶层目录清单。
        let subdirs: Vec<(String, PathBuf)> = ["African", "Asian", "Caucasian", "EECA", "Italmed"]
            .iter()
            .map(|n| (n.to_string(), p.join(n)))
            .collect();

        let mut seq: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        build_pools_sequential(&subdirs, &mut seq, &mut |_, _| {}, &AtomicBool::new(false));

        let mut par: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut seen: Vec<(usize, usize)> = Vec::new();
        build_pools_parallel(
            &subdirs,
            &mut par,
            &mut |d, t| seen.push((d, t)),
            4,
            &AtomicBool::new(false),
        );

        assert_eq!(par, seq, "并行扫盘结果必须与顺序扫盘逐字相同");
        // 每个一级目录都报一次进度，且最终走满（订阅方靠它把进度条推到区间右端）。
        assert_eq!(seen.len(), subdirs.len());
        assert_eq!(seen.last().copied(), Some((subdirs.len(), subdirs.len())));
        // 抽查：孙目录独立成键、垃圾子树整棵不入池、一级目录自己的图不被子孙覆盖。
        assert_eq!(par.get("EECA/sub").map(|s| s.iter().cloned().collect::<Vec<_>>()),
                   Some(vec!["deep.1".to_string()]));
        assert!(!par.contains_key("African/__MACOSX"));
        assert_eq!(par.get("EECA").map(|s| s.iter().cloned().collect::<Vec<_>>()),
                   Some(vec!["f0".to_string()]));
        assert_eq!(par.get("African").map(|s| s.len()), Some(3));
        // ② 线程数闸门：目录太少（空包 0 个、小包不足 PARALLEL_SCAN_MIN_DIRS）不起线程 ——
        // 起线程与合并的开销摊不平；多核机器上目录够多则并行且不超过上限，单核恒为 1。
        assert_eq!(scan_threads(0), 1);
        assert_eq!(scan_threads(PARALLEL_SCAN_MIN_DIRS - 1), 1);
        let many = scan_threads(62);
        if std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) >= 2 {
            assert!(many >= 2, "多核机器上 62 个目录应并行");
        }
        assert!(many <= PARALLEL_SCAN_MAX_THREADS);
    }

    #[test]
    fn cancel_stops_pack_scan_and_never_returns_a_partial_index() {
        // 取消检查点必须落在最耗时的这一步（冷扫整包实测 14.6–39 s）：
        // 只在阶段之间检查会让用户按取消后几十秒无反应 → 判定假死 → 强杀进程。
        // 半途的索引尤其不能当结果返回——残缺的目录集会把包判成另一个标准。
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        // 目录数要够多：取消检查点若被改成"每 N 个目录一次"，只建 3 个目录会直接扫完 →
        // expect_err 假失败，看起来像取消功能坏了。
        for d in ["African", "Asian", "Caucasian", "EECA", "Italmed", "MENA", "MESA", "SAMed"] {
            std::fs::create_dir_all(p.join(d)).unwrap();
            std::fs::write(p.join(d).join("f0.png"), b"png").unwrap();
        }
        let cancel = AtomicBool::new(false);
        let err = scan_pack_progress(
            p.to_str().unwrap(),
            &mut |done, _total| {
                if done >= 1 {
                    cancel.store(true, Ordering::Relaxed); // 扫完第一个目录就请求取消
                }
            },
            &cancel,
        )
        .expect_err("取消后必须报错，而不是返回残缺索引");
        assert!(err.contains("取消"), "{err}");
    }

    /// 【为什么只在 Unix 上跑这个用例】两个原因，都不是「懒得写」：
    ///
    /// ① **危险场景本来就长在 Unix 上**：`~/packs/NG -> /mnt/big/NG`（把大包软链进包根）是
    ///    Linux/macOS 的常规用法。Windows 上用户用的是 **junction**，而 junction 同时带 DIRECTORY
    ///    属性，`file_type().is_dir()` 直接为真，根本走不到 `entry_is_dir` 的符号链接回退分支。
    /// ② **Windows 上跑它要付 30 秒**：建目录符号链接需要管理员权限或开发者模式；实测在缺权限的
    ///    机器上，这一次 `CreateSymbolicLinkW` 调用要等 **约 30 秒**才返回失败（授权路径的等待；
    ///    对照 PowerShell 自己的同等操作是 0.007 s 秒回权限错误）。把它放进常规循环等于让每次
    ///    `cargo test` 多花半分钟，换不来任何断言。
    #[test]
    #[cfg(unix)]
    fn symlinked_directory_is_still_treated_as_directory() {
        // entry_is_dir 的核心回归：`DirEntry::file_type()` **不跟随**符号链接，直接拿它判目录会让
        // 软链目录被当成文件 → `is_image_ext` 不认 → **整棵子树静默消失**（索引为空 → 布局退化 →
        // 全部球员被跳过，只剩一条 WARNING）。所以符号链接必须回退一次 is_dir()。
        let dir = tempfile::tempdir().unwrap();
        let real = tempfile::tempdir().unwrap();
        std::fs::write(real.path().join("a.png"), b"png").unwrap();
        std::fs::create_dir_all(real.path().join("sub")).unwrap();
        std::fs::write(real.path().join("sub").join("b.png"), b"png").unwrap();
        // 本用例唯一需要的 OS 调用；Unix 上建软链无需特权。
        std::os::unix::fs::symlink(real.path(), dir.path().join("African")).unwrap();

        let scan = scan_pack(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(scan.faces_map["African"], BTreeSet::from(["a".to_string()]),
                   "符号链接目录必须被当作目录遍历（否则整棵子树静默消失）");
        // 经软链的子孙目录同样递归到。
        assert_eq!(scan.recursive_pools["African/sub"], BTreeSet::from(["b".to_string()]));

        // 「检查完整性」与扫盘同口径（都走 entry_is_dir）：软链目录算目录 → 单目录包判标准二子集。
        let cfg = crate::templates::RuleConfig::builtin();
        let r = check_integrity_progress(dir.path().to_str().unwrap(), &cfg.folder_lists(), &mut |_, _| {});
        assert_eq!((r.standard, r.found), (2, 1), "完整性检查也必须把软链目录算作目录");
    }

    // ---- 完整性检查 / 扫描总数 --------------------------------------------------------------------------------------

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

    /// 完整性判定的表驱动主矩阵：包形态 → (标准号, 状态, found, expected_total) + missing/extra。
    /// 覆盖：平铺/空包、标准二三恰好、乱序不敏感、超集（多余目录进 extra、不进抽图池）、
    /// 子集（missing 可一键补全，found + missing = expected_total）、未知名/跨套混用 → Unknown、
    /// 垃圾目录不参与判定。missing/extra 按归属标准的 FolderList 原顺序计算。
    #[test]
    fn integrity_covers_layout_matrix_and_aux_files() {
        let cfg = crate::templates::RuleConfig::builtin();
        let f3: Vec<&str> = cfg.standard3.folder_list.iter().map(String::as_str).collect();
        let mut shuffled: Vec<&str> = FACEPACK_DIRS.to_vec();
        shuffled.reverse(); // 顺序不同仍应命中
        let mut superset3 = f3.clone();
        superset3.push("Scandinavian"); // 清单外（标准二的目录）
        superset3.push("SomeUnusedDir"); // 清单外
        // 标准三子集的缺失 = 126 清单按原顺序去掉已建 3 目录。
        let subset3_missing: Vec<String> = cfg
            .standard3
            .folder_list
            .iter()
            .filter(|d| !["China", "Japan", "Korea"].contains(&d.as_str()))
            .cloned()
            .collect();
        // (说明, 建包子目录, 根文件, (standard, status, found, expected_total), missing, extra)
        let cases: Vec<(&str, Vec<&str>, Vec<&str>, (u8, &str, usize, usize), Vec<String>, Vec<String>)> = vec![
            ("平铺单图", vec![], vec!["face.png"], (1, "complete", 0, 0), vec![], vec![]),
            ("空目录", vec![], vec![], (1, "unknown", 0, 0), vec![], vec![]),
            ("标准二乱序全量", shuffled, vec![], (2, "complete", 14, 14), vec![], vec![]),
            // found 记已就位的必需目录数（与 missing 同口径）：126 条里 64 条多级条目也真实存在。
            ("标准三恰好全量", f3.clone(), vec![], (3, "complete", 126, 126), vec![], vec![]),
            // 超集仍 complete；extra 只列一级目录里未配置且非容器的项。
            ("标准三超集", superset3, vec![], (3, "complete", 126, 126), vec![],
             vec!["Scandinavian".into(), "SomeUnusedDir".into()]),
            // 子集 fixable：缺失目录可一键补全。
            ("标准二缺一", FACEPACK_DIRS[..13].to_vec(), vec![], (2, "fixable", 13, 14),
             vec!["YugoGreek".into()], vec![]),
            ("标准三子集", vec!["China", "Japan", "Korea"], vec![], (3, "fixable", 3, 126),
             subset3_missing, vec![]),
            ("未知名", vec!["African", "Asian", "NotARealRace"], vec![], (0, "unknown", 3, 0), vec![], vec![]),
            // African 属标准二（14 目录清单）、China 属标准三（62 目录清单），跨套混用无法识别。
            ("跨套混用", vec!["African", "China"], vec![], (0, "unknown", 2, 0), vec![], vec![]),
            // 垃圾目录不进判定：平铺包不因 __MACOSX/.git 误判 Unknown。
            ("垃圾目录不参与", vec!["__MACOSX", ".git"], vec!["face.png"], (1, "complete", 0, 0), vec![], vec![]),
        ];
        for (name, dirs, files, expected, missing, extra) in cases {
            let tmp = tempfile::tempdir().unwrap();
            mk_dirs(tmp.path(), &dirs);
            for f in &files {
                std::fs::write(tmp.path().join(f), b"png").unwrap();
            }
            let r = integrity(tmp.path());
            assert_eq!(
                (r.standard, r.status.as_str(), r.found, r.expected_total),
                expected,
                "用例 {name}"
            );
            assert_eq!(r.missing, missing, "用例 {name} 的缺失目录");
            assert_eq!(r.extra, extra, "用例 {name} 的多余目录");
        }
        // 辅助文件（config.xml / README / 备注.txt）不得把判定带偏：三种布局各验一份。
        // (说明, 建包子目录, 根图, (standard, status, missing 数, expected_total))
        // 只留两例：标准二全量的期望与矩阵里那条完全同参（aux 文件是非图片，扫盘本就不收），
        // 重跑同一组数字没有新信息。留下的是"最容易被根级非图片文件带偏"的两例：
        // 子集包（缺 123 个目录、fixable）与平铺包（根级 aux 不得把它判成 Unknown）。
        let aux_cases: Vec<(&str, Vec<&str>, bool, (u8, &str, usize, usize))> = vec![
            ("标准三子集", vec!["China", "Japan", "Korea"], false, (3, "fixable", 123, 126)),
            ("标准一平铺", vec![], true, (1, "complete", 0, 0)),
        ];
        for (name, dirs, with_root_image, expected) in aux_cases {
            let tmp = tempfile::tempdir().unwrap();
            mk_dirs(tmp.path(), &dirs);
            drop_aux_files(tmp.path());
            if with_root_image {
                std::fs::write(tmp.path().join("face.png"), b"png").unwrap();
            }
            let r = integrity(tmp.path());
            assert_eq!(
                (r.standard, r.status.as_str(), r.missing.len(), r.expected_total),
                expected,
                "辅助文件用例 {name}"
            );
        }
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
