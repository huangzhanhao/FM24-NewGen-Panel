//! 【新手导读】Tauri 命令层：对等旧版 services/* + app.py。数据留在 Rust，IPC 只传摘要/分页。
//!
//! 【核心概念：什么是 Tauri 命令】带 #[tauri::command] 的函数会自动暴露给前端。
//! 前端写 invoke("create_profile", { name: "档A" })，就会调到本文件里的 create_profile 函数。
//! 参数里的 State<'_, AppState> 由 Tauri 自动注入（就是 lib.rs 里 app.manage 托管的那个状态）。
//! 返回 Result<T, String> 时：Ok → 前端 Promise resolve；Err(String) → 前端 Promise reject（抛异常）。
use crate::assets::NAT_TRANSLATION;
use crate::face_mapper::Mode;
use crate::facepack::{self, PackLayout, PackScan}; // 标准识别/扫描的唯一来源（facepack.rs）
use crate::logger::{LogPage, Logger};
use crate::profile_manager::ProfileManager;
use crate::replace_flow::{self, Outcome, ReplaceParams};
use crate::rtf_parser::{PlayerRecord, RtfParser, NEWGAN_UID_PREFIX};
use crate::xml_parser::{self, MappingEntry};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State}; // AppHandle=应用句柄（发事件用）；Emitter 提供 emit；State 注入托管状态

// 标准常量已迁移至 facepack.rs（识别的单一来源）；此处 re-export，
// 使既有引用路径 commands::FACEPACK_DIRS / commands::FACEPACK_DIRS_65（含测试）无需改动即可解析。
pub use crate::facepack::{FACEPACK_DIRS, FACEPACK_DIRS_65};

/// 应用全局状态（在 lib.rs 的 setup 里用 app.manage 交给 Tauri 托管）。
/// 【为何这样包】命令可能在不同线程被调用，后台替换也跑在单独线程；
/// 所以共享数据用 Mutex（互斥锁）保护，跨线程引用用 Arc（共享所有权），标志用 AtomicBool。
pub struct AppState {
    pub pm: Mutex<ProfileManager>,       // 存档管理器（Mutex 保证同一时刻只有一个线程操作）
    pub logger: Arc<Logger>,             // Logger 字段全部是 Mutex/Sender，包 Arc 即可跨线程
    pub cancel: Arc<AtomicBool>,         // 请求取消标志
    pub running: Arc<AtomicBool>,        // 是否正在替换
    pub rtf_cache: Arc<Mutex<Vec<PlayerRecord>>>,   // 最近一次解析的球员缓存
    pub mapping_cache: Arc<Mutex<Vec<MappingEntry>>>, // 最近一次生成的映射缓存
    pub faces_cache: Arc<Mutex<Option<FaceIndex>>>,   // 「扫描头像」的包索引缓存（替换可复用）
}

/// 「扫描头像」产出的索引缓存：目录 + 完整扫描结果（仅按 img_dir 命中，切组即失效）。
pub struct FaceIndex { pub img_dir: String, pub scan: PackScan }

// ---------------- 传给前端的类型（统一 camelCase） ----------------
// 【说明】下面这些结构体是「前后端的数据契约」：Rust 端用蛇形命名，通过 serde 自动转成前端的驼峰命名。
// 前端 types.ts 里有与之对应的 TypeScript 接口，两边字段必须一致。

/// 一个头像包（组）的摘要：目录 + 已设 RTF。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo { pub img_dir: String, pub rtf: String }

/// 应用全局快照：一次性把前端需要的所有状态打包返回（前端拿到后整体替换本地状态）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub profiles: Vec<String>,      // 所有存档名
    pub current_profile: String,    // 当前激活存档
    pub groups: Vec<GroupInfo>,     // 当前存档下的所有组
    pub current_group: Option<String>, // 当前选中组（Option 对应前端 null）
    pub current_rtf: String,        // 当前组的 RTF
    pub replacing: bool,            // 是否正在替换
    pub log_path: String,           // 日志文件路径
}

/// 进度事件的载荷（通过 replace:progress 事件发给前端）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPayload { pub status: String, pub value: u8 }

/// 「检查完整性」结果：头像包命中的标准 + 可修复的缺失目录（供 check_integrity 命令返回）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IntegrityResult {
    pub standard: u8,          // 1=图片直铺 / 2=14 种族 / 3=65 细分；0=无法识别
    pub status: String,        // "complete" | "fixable" | "unknown"
    pub missing: Vec<String>,  // fixable 时可一键创建的缺失目录（按标准原顺序）
    pub found: usize,          // 已命中的子目录数
    pub expected_total: usize, // 该标准应有的目录总数（标准一/无法识别为 0）
}

/// 「扫描头像」返回摘要：图片总数 + 命中的标准号（0=无法识别）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImageScanResult { pub total: usize, pub standard: u8 }

/// 「扫描RTF」返回摘要。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RtfScanResult {
    pub total: usize,     // 名单总条数
    pub newgan: usize,    // 随机人（r- 前缀）条数
    pub detailed: bool,   // 是否详细名单
    pub language: String, // "English" / "简体中文"
}

/// 前端发起替换时传来的参数（Deserialize = 从前端 JSON 解析而来）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceArgs {
    pub mode: String,
    pub filter_newgan: bool,
    pub allow_duplicates: bool,
    pub save_backup: bool,
}

/// 名单分页查询参数。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerQuery {
    pub offset: usize,
    pub limit: usize,
    pub search: String,
    pub sort_key: String,
    pub sort_desc: bool,
}

/// 通用分页结果（<T> 是泛型，可装任何类型的条目）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageResult<T> { pub total: usize, pub items: Vec<T> }

/// 单人预览的返回体：球员资料 + 映射 + 实际图片路径（三者都可能为 None/null）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPreview {
    pub record: Option<PlayerRecord>,
    pub mapping: Option<MappingEntry>,
    pub image_path: Option<String>,
}

// ---------------- 命令实现 ----------------

/// 把 ProfileManager 的当前状态汇总成一个 AppSnapshot（发给前端的快照）。
/// 【复用】几乎每个命令改完状态后都会调它，把最新状态回传前端，前端据此刷新界面。
fn snapshot(state: &State<'_, AppState>) -> AppSnapshot {
    let pm = state.pm.lock().unwrap(); // 加锁读取存档管理器（lock 期间独占）
    AppSnapshot {
        profiles: pm.config.profile.keys().cloned().collect(),
        current_profile: pm.cur_prf.clone(),
        groups: pm
            .prf_cfg
            .groups
            .iter()
            .map(|(dir, g)| GroupInfo { img_dir: dir.clone(), rtf: g.rtf.clone() })
            .collect(),
        current_group: pm.cur_group.clone(),
        current_rtf: pm.get_cur_rtf(),
        replacing: state.running.load(Ordering::SeqCst), // SeqCst 是最严格的内存顺序（跨线程同步标志）
        log_path: state.logger.path().to_string_lossy().to_string(),
    }
}

// #[tauri::command] 是关键：把这个函数注册为前端可调用的命令（还需在 lib.rs 的 invoke_handler 里登记）。

/// 获取应用全局快照（前端启动时、或需要强制刷新时调）。
#[tauri::command]
pub fn get_app_state(state: State<'_, AppState>) -> AppSnapshot { snapshot(&state) }

/// 新建并激活存档。name 会先 trim 去首尾空白。
#[tauri::command]
pub fn create_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let name = name.trim().to_string();
    state.pm.lock().unwrap().create_profile(&name)?; // ? 将底层错误直接回传前端
    state.logger.info("profile", format!("创建并激活 Profile: {name}"));
    Ok(snapshot(&state))
}

/// 删除存档。'No Profile' 不可删（返回错误提示）。
#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if !pm.delete_profile(&name)? {
        return Err("Can't delete 'No Profile'".into());
    }
    drop(pm); // 显式释放锁（因为后面 snapshot() 会再次加锁，不先释放会死锁）
    Ok(snapshot(&state))
}

/// 切换存档（含快照存取：保存旧档各组 config.xml、恢复新档各组快照）。
#[tauri::command]
pub fn switch_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    pm.load_profile(&name)?;
    // 切档后把全局配置 cfg.json 也落盘（记住新的激活位）。
    crate::json_store::save(
        &pm.user_cfg_path().join("cfg.json"),
        &pm.config,
    )?;
    drop(pm);
    state.logger.info("profile", format!("激活 Profile: {name}"));
    Ok(snapshot(&state))
}

/// 关联头像包目录（新目录自动建组并设为当前组）。未激活存档时不允许。
#[tauri::command]
pub fn select_image_directory(state: State<'_, AppState>, path: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if pm.cur_prf == "No Profile" {
        return Err("请先创建并激活一个 Profile".into());
    }
    pm.ensure_group(&path).ok_or("目录不可用")?; // ok_or 把 Option::None 转为 Err
    drop(pm);
    Ok(snapshot(&state))
}

/// 设置当前组的 RTF 名单路径。
#[tauri::command]
pub fn set_current_rtf(state: State<'_, AppState>, path: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if !pm.set_cur_rtf(&path)? {
        return Err("请先选择头像包目录（当前无组）".into());
    }
    drop(pm);
    Ok(snapshot(&state))
}

/// 切换当前头像包（组）。
#[tauri::command]
pub fn set_current_group(state: State<'_, AppState>, img_dir: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if !pm.set_cur_group(&img_dir)? {
        return Err("组不存在".into());
    }
    drop(pm);
    Ok(snapshot(&state))
}

/// 删除组（同时删掉其快照）。
#[tauri::command]
pub fn delete_group(state: State<'_, AppState>, img_dir: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if !pm.delete_group(&img_dir)? {
        return Err("组不存在".into());
    }
    drop(pm);
    Ok(snapshot(&state))
}

/// 校验头像包目录，返回缺失的种族子目录名（前端弹一次确认框后批量创建）。
/// 【注意】这个命令不需要 State（不访问全局状态），只根据传入的 img_dir 扫描目录。
#[tauri::command]
pub fn validate_image_dirs(img_dir: String) -> Vec<String> {
    // 先把目录下已有的子目录名收集到一个 HashSet（方便快速判存在）。
    let existing: std::collections::HashSet<String> = std::fs::read_dir(&img_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    // 对照标准 14 个目录，筛出「不在已有集合里」的，即缺失目录。
    FACEPACK_DIRS
        .iter()
        .filter(|d| !existing.contains(**d))
        .map(|d| d.to_string())
        .collect()
}

/// 「检查完整性」：识别头像包属于三套标准中的哪一套，并给出可一键补全的缺失目录。
/// 判定逻辑统一走 facepack::classify（唯一来源）；本命令只做"轻扫描"——
/// 列一级子目录名 + 探测根是否有图，不读子目录内文件（保持现状开销）。
/// 子目录名先经 facepack::is_junk_dir 过滤（__MACOSX / . 开头目录不参与判定）。
#[tauri::command]
pub fn check_integrity(img_dir: String) -> IntegrityResult {
    let mut subdirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut has_images = false;
    if let Ok(rd) = std::fs::read_dir(&img_dir) {
        for e in rd.flatten() {
            let p = e.path();
            let fname = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !facepack::is_junk_dir(&fname) { subdirs.insert(fname); } // 跳过 __MACOSX 等
            }
            else if !fname.starts_with('.')   // 跳过 ._x.png 等隐藏文件，与 scan_pack 口径一致
                && p.extension().and_then(|x| x.to_str())
                .map(|x| matches!(x.to_ascii_lowercase().as_str(), "png"|"jpg"|"jpeg")).unwrap_or(false) {
                has_images = true;
            }
        }
    }

    let layout = facepack::classify(&subdirs, has_images);
    let standard = layout.standard();
    let status = match layout {
        PackLayout::Flat => "complete",
        PackLayout::Empty | PackLayout::Unknown => "unknown",
        PackLayout::Standard2 { complete: true } | PackLayout::Standard3 { complete: true } => "complete",
        PackLayout::Standard2 { .. } | PackLayout::Standard3 { .. } => "fixable",
    };
    // fixable 时给出"该标准还有哪些目录没出现"，按常量原顺序返回（便于前端稳定展示与创建）。
    let missing: Vec<String> = match layout {
        PackLayout::Standard2 { complete: false } =>
            FACEPACK_DIRS.iter().filter(|d| !subdirs.contains(**d)).map(|d| d.to_string()).collect(),
        PackLayout::Standard3 { complete: false } =>
            FACEPACK_DIRS_65.iter().filter(|d| !subdirs.contains(**d)).map(|d| d.to_string()).collect(),
        _ => Vec::new(),
    };
    let expected_total = match standard { 2 => 14, 3 => 65, _ => 0 };
    // 标准一/无法识别的 found 与现状一致：标准一记 0，其余记命中的子目录数。
    let found = if standard == 1 { 0 } else { subdirs.len() };
    IntegrityResult { standard, status: status.into(), missing, found, expected_total }
}

/// 批量创建缺失目录（前端确认后调用）。
#[tauri::command]
pub fn create_missing_dirs(img_dir: String, names: Vec<String>) -> Result<(), String> {
    for name in names {
        std::fs::create_dir_all(Path::new(&img_dir).join(&name))
            .map_err(|e| format!("创建目录 {name} 失败: {e}"))?;
    }
    Ok(())
}

// ---------------- 扫描（「扫描头像 / 扫描RTF」按钮） ----------------

/// 标准号的中文标注（写日志用）。
fn layout_label(standard: u8) -> &'static str {
    match standard {
        1 => "标准一/图片直铺",
        2 => "标准二/14 目录",
        3 => "标准三/65 目录",
        _ => "无法识别",
    }
}

/// 「扫描头像」核心（纯函数，便于单测）：重扫描头像包，返回 (图片总数, 标准号, 完整扫描结果)。
fn scan_images_core(img_dir: &str) -> Result<(usize, u8, PackScan), String> {
    let scan = facepack::scan_pack(img_dir)?;
    // 平铺包图片数 = 根图数；其余（含 Unknown 的子/孙目录）= 递归全量池图片总数。
    let total = if scan.layout.is_flat() {
        scan.root_images.len()
    } else {
        scan.recursive_pools.values().map(|s| s.len()).sum()
    };
    Ok((total, scan.layout.standard(), scan))
}

/// 「扫描RTF」核心（纯函数，便于单测）：校验 + 全量解析名单，返回 (摘要, 球员列表)。
fn scan_rtf_core(rtf_path: &str) -> Result<(RtfScanResult, Vec<PlayerRecord>), String> {
    let mut parser = RtfParser::new();
    if !parser.check_rtf_valid(Path::new(rtf_path))? {
        return Err("RTF 文件格式无效".into());
    }
    // filter=false：扫描展示全量名单（Replace 流程才按「仅随机人」过滤）。
    let data = parser.parse_rtf(Path::new(rtf_path), false, &NAT_TRANSLATION)?;
    let summary = RtfScanResult {
        total: data.len(),
        newgan: data.iter().filter(|p| p.uid.starts_with(NEWGAN_UID_PREFIX)).count(),
        detailed: data.iter().any(|p| p.detailed.is_some()),
        language: parser.rtf_language.clone(),
    };
    Ok((summary, data))
}

/// 「扫描头像」：扫描当前组头像包、缓存索引（供替换复用），结果写日志。
#[tauri::command]
pub fn scan_images(state: State<'_, AppState>) -> Result<ImageScanResult, String> {
    let img_dir = { state.pm.lock().unwrap().cur_group.clone().ok_or("请先选择头像包目录")? };
    let (total, standard, scan) = scan_images_core(&img_dir)?;
    *state.faces_cache.lock().unwrap() = Some(FaceIndex { img_dir, scan });
    state.logger.info("scan", format!("扫描头像完成：{total} 张图片（{}）", layout_label(standard)));
    Ok(ImageScanResult { total, standard })
}

/// 「扫描RTF」：解析当前组 RTF 并填充名单预览缓存（无需先 Replace），结果写日志。
#[tauri::command]
pub fn scan_rtf(state: State<'_, AppState>) -> Result<RtfScanResult, String> {
    let rtf = { state.pm.lock().unwrap().get_cur_rtf() };
    if rtf.is_empty() { return Err("请先选择 RTF 名单".into()); }
    let (summary, data) = scan_rtf_core(&rtf)?;
    *state.rtf_cache.lock().unwrap() = data; // 名单预览（query_players）直接读这份缓存
    state.logger.info("scan", format!("扫描 RTF 完成：{} 条（随机人 {}，{}，{}）",
        summary.total, summary.newgan, if summary.detailed { "详细名单" } else { "基础名单" }, summary.language));
    Ok(summary)
}

/// 【最复杂的命令】启动批量替换。开一个后台线程跑流水线，避免阻塞界面。
/// app: AppHandle 用于在线程里向前端 emit 事件；state 提供共享状态；args 是前端传来的参数。
#[tauri::command]
pub fn start_replace(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ReplaceArgs,
) -> Result<(), String> {
    // swap(true) 原子地把 running 置 true 并返回旧值；若旧值已是 true，说明有任务在跑，拒绝重入。
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("已有替换任务在执行".into());
    }
    state.cancel.store(false, Ordering::SeqCst); // 重置取消标志
    // 先在锁内取出当前 RTF 和组目录，随即释锁（不能把 Mutex 带进新线程）。
    let (rtf, img_dir) = {
        let pm = state.pm.lock().unwrap();
        (pm.get_cur_rtf(), pm.cur_group.clone())
    };
    // let ... else：img_dir 为 None 就报错退出（退出前记得把 running 置回 false）。
    let Some(img_dir) = img_dir else {
        state.running.store(false, Ordering::SeqCst);
        return Err("请先选择头像包目录".into());
    };
    if rtf.is_empty() {
        state.running.store(false, Ordering::SeqCst);
        return Err("请先选择 RTF 名单".into());
    }
    let mode = Mode::parse(&args.mode)?; // 把字符串模式解析为枚举
    // 「扫描头像」缓存命中同一目录则复用其索引（避免重复扫盘）；切组即失效、现场新扫描。
    let pre_scan = { state.faces_cache.lock().unwrap()
        .as_ref().filter(|c| c.img_dir == img_dir).map(|c| c.scan.clone()) };
    let params = ReplaceParams {
        rtf_path: rtf,
        img_dir,
        mode,
        filter_newgen: args.filter_newgan,
        allow_duplicates: args.allow_duplicates,
        save_backup: args.save_backup,
        pre_scan,
    };
    // 把需要跨线程使用的东西先 clone 出来（Arc::clone 只增加引用计数，不拷贝真实数据）。
    let (cancel, running) = (state.cancel.clone(), state.running.clone());
    let (rtf_cache, mapping_cache) = (state.rtf_cache.clone(), state.mapping_cache.clone());
    let logger = state.logger.clone();
    // std::thread::spawn 开后台线程；move 把上面 clone 的变量所有权移进线程。
    std::thread::spawn(move || {
        let emit_app = app.clone();
        // 跑流水线；progress 回调里通过 emit 把进度发给前端（事件名 replace:progress）。
        let result = replace_flow::run(
            &params,
            &mut |status: &str, value: u8| {
                let _ = emit_app.emit("replace:progress", ProgressPayload { status: status.into(), value });
            },
            &cancel,
        );
        // 把解析结果与映射写入缓存（供后续预览/分页查询使用）。
        *rtf_cache.lock().unwrap() = result.rtf_data.clone();
        *mapping_cache.lock().unwrap() = result.mapping.clone();
        if matches!(result.outcome, Outcome::Finished) && result.mapping.is_empty() {
            // 跑完却一条映射也没有：config.xml 会被写成空表，必须留 WARNING
            logger.warn("replace", "映射结果为空：本次写入的 config.xml 不含任何映射（多为「仅随机人」过滤掉了全部名单，或头像包内无图片）");
        }
        // 把结果映射为前端认的字符串；失败时额外记一条 ERROR 日志。
        let result_str = match result.outcome {
            Outcome::Finished => "finished",
            Outcome::Cancelled => "cancelled",
            Outcome::Failed(ref m) => { logger.error("replace", m.clone()); "failed" }
        };
        // 发「完成」事件（前端收到后刷新快照并弹提示）。
        let _ = app.emit("replace:finished", serde_json::json!({ "result": result_str }));
        running.store(false, Ordering::SeqCst); // 标记任务结束，解除全局禁用
    });
    Ok(())
}

/// 单人换脸（对等旧版 PlayerService.replace_single_face）。只改 config.xml 里该 UID 那一行。
/// 支持标准一平铺：选中的图片在包根时，写入 from="图名"（ethnicity 为空串）。
#[tauri::command]
pub fn replace_single_face(
    state: State<'_, AppState>,
    uid: String,
    image_path: String,
    save_backup: bool,
) -> Result<bool, String> {
    let uid = uid.trim().to_string();
    // 先取当前组目录（parse_image_path_in_pack 需要它来判定"图片是否在包根"）。
    let img_dir = {
        let pm = state.pm.lock().unwrap();
        pm.cur_group.clone().ok_or("当前没有选中组")?
    };
    let Some((pack, image)) = xml_parser::parse_image_path_in_pack(&img_dir, &image_path) else {
        return Err("图片路径不合法，需为头像包内的图片文件".into());
    };
    let hit = xml_parser::single_replacement(&uid, &pack, &image, Path::new(&img_dir), save_backup)?;
    if hit {
        // 同步内存映射缓存（对等旧版 update_mapping_cache），保证预览与实际一致。
        let mut cache = state.mapping_cache.lock().unwrap();
        for m in cache.iter_mut() {
            if m.uid == uid {
                m.ethnicity = pack.clone(); // 平铺时为空串，正常
                m.image = image.clone();
                break;
            }
        }
    }
    state.logger.info("viewer", format!("单人换脸 {uid} → {}", xml_parser::record_from(&pack, &image)));
    Ok(hit) // 返回是否命中（未命中 = config.xml 里本来就没这个 UID）
}

/// 查询/预览（对等旧版 PlayerService 查询链）。按优先级从多个数据源拼出单人预览。
#[tauri::command]
pub fn preview_player(state: State<'_, AppState>, uid: String) -> Result<PlayerPreview, String> {
    let uid = uid.trim().to_string();
    // UID 可能带 r- 也可能不带，两种形态都试。
    let variants = xml_parser::with_newgan_prefix_variants(&uid);
    // 1) 先从最近一次批量映射缓存里找。
    let mut mapping = {
        let cache = state.mapping_cache.lock().unwrap();
        cache.iter().find(|m| variants.contains(&m.uid)).cloned()
    };
    let pm = state.pm.lock().unwrap();
    // 2) 缓存没命中，就读当前组的 config.xml 找。
    if mapping.is_none() {
        if let Some(g) = &pm.cur_group {
            if let Ok(xml) = xml_parser::parse_xml(&Path::new(g).join("config.xml")) {
                mapping = variants.iter().find_map(|u| xml.get(u).cloned());
            }
        }
    }
    // 3) 根据映射解析实际图片文件路径（供前端加载显示）。
    let image_path = mapping.as_ref().and_then(|m| {
        let g = pm.cur_group.as_ref()?;
        xml_parser::resolve_image_file(Path::new(g), &m.ethnicity, &m.image)
    });
    // 4) RTF 球员资料缓存（展示国籍/姓名等）。
    let record = {
        let cache = state.rtf_cache.lock().unwrap();
        cache.iter().find(|p| variants.contains(&p.uid)).cloned()
    };
    Ok(PlayerPreview { record, mapping, image_path })
}

/// RTF 名单分页查询（大名单预览；数据留 Rust，IPC 只传当前页）。
/// 【为何分页】名单可能上万条，一次全传会卡顿，所以只传当前页（前端每页 200 条）。
#[tauri::command]
pub fn query_players(state: State<'_, AppState>, query: PlayerQuery) -> PageResult<PlayerRecord> {
    // 先把缓存整体 clone 出来（避免长时间持锁），再做搜索/排序/分页。
    let mut rows = state.rtf_cache.lock().unwrap().clone();
    let search = query.search.to_lowercase();
    if !search.is_empty() {
        // retain 保留满足条件的项：UID/姓名/国籍 任一包含搜索词即命中。
        rows.retain(|r| {
            r.uid.to_lowercase().contains(&search)
                || r.name.to_lowercase().contains(&search)
                || r.primary_nat.to_lowercase().contains(&search)
        });
    }
    // 按前端指定的列排序；sort_desc 为 true 时反转。
    rows.sort_by(|a, b| {
        let ord = match query.sort_key.as_str() {
            "name" => a.name.cmp(&b.name),
            "primaryNat" => a.primary_nat.cmp(&b.primary_nat),
            "ethnicityCode" => a.ethnicity_code.cmp(&b.ethnicity_code),
            _ => a.uid.cmp(&b.uid),
        };
        if query.sort_desc { ord.reverse() } else { ord }
    });
    let total = rows.len();
    // skip + take 实现分页，只返回当前页。
    PageResult { total, items: rows.into_iter().skip(query.offset).take(query.limit).collect() }
}

/// 读取预览图片字节（前端转 blob URL 显示）。
/// 【为何不直接用文件路径】WebView 无权直接读本地磁盘图片，所以由 Rust 读出字节传给前端。
/// 返回 tauri::ipc::Response（二进制响应），前端收到的是 ArrayBuffer。
#[tauri::command]
pub fn read_image(path: String) -> Result<tauri::ipc::Response, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取图片失败: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// 分页查询日志（前端打开日志面板、或切换过滤条件时调）。
#[tauri::command]
pub fn query_logs(state: State<'_, AppState>, level: String, only: bool, offset: usize, limit: usize) -> LogPage {
    state.logger.query(&level, only, offset, limit)
}

/// 请求取消正在进行的替换。只把标志置 true，后台线程会在下一次检查时自行停下。
#[tauri::command]
pub fn cancel_replace(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
    state.logger.info("replace", "用户请求取消");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在临时目录下按 names 建子目录，返回根路径字符串。
    fn mk_dirs(root: &Path, names: &[&str]) {
        for n in names {
            std::fs::create_dir_all(root.join(n)).unwrap();
        }
    }

    #[test]
    fn standard1_images_only() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("face.png"), b"png").unwrap();
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str()), (1, "complete"));
    }

    #[test]
    fn empty_dir_is_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str()), (1, "unknown"));
    }

    #[test]
    fn standard2_exact_regardless_of_order() {
        let tmp = tempfile::tempdir().unwrap();
        let mut shuffled: Vec<&str> = FACEPACK_DIRS.to_vec();
        shuffled.reverse(); // 顺序不同仍应命中
        mk_dirs(tmp.path(), &shuffled);
        std::fs::write(tmp.path().join(FACEPACK_DIRS[0]).join("a.jpg"), b"x").unwrap();
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str(), r.missing.len(), r.expected_total), (2, "complete", 0, 14));
    }

    #[test]
    fn standard3_exact() {
        let tmp = tempfile::tempdir().unwrap();
        mk_dirs(tmp.path(), &FACEPACK_DIRS_65);
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str(), r.expected_total), (3, "complete", 65));
    }

    #[test]
    fn standard2_fixable_reports_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // 标准二去掉最后一个（YugoGreek）→ 13 个，应判 fixable 且缺失正好是 YugoGreek。
        let present: Vec<&str> = FACEPACK_DIRS[..13].to_vec();
        mk_dirs(tmp.path(), &present);
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str(), r.found, r.expected_total), (2, "fixable", 13, 14));
        assert_eq!(r.missing, vec!["YugoGreek".to_string()]);
    }

    #[test]
    fn standard3_fixable_subset() {
        let tmp = tempfile::tempdir().unwrap();
        mk_dirs(tmp.path(), &["China", "Japan", "Korea"]); // 全部 ⊆ 65 组
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str(), r.found, r.expected_total), (3, "fixable", 3, 65));
        assert_eq!(r.missing.len(), 62);
        assert!(!r.missing.contains(&"China".to_string()));
    }

    #[test]
    fn unknown_when_unrecognized_name_present() {
        let tmp = tempfile::tempdir().unwrap();
        mk_dirs(tmp.path(), &["African", "Asian", "NotARealRace"]);
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str()), (0, "unknown"));
    }

    #[test]
    fn unknown_when_mixing_two_sets() {
        let tmp = tempfile::tempdir().unwrap();
        // African 属 14 组、China 属 65 组，跨两套混用 → 无法识别。
        mk_dirs(tmp.path(), &["African", "China"]);
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str()), (0, "unknown"));
    }

    /// 在根目录放标准说明里点名的辅助文件（config.xml / .md / 备注.txt），验证其不参与判定。
    fn drop_aux_files(root: &Path) {
        std::fs::write(root.join("config.xml"), b"<config/>").unwrap();
        std::fs::write(root.join("README.md"), b"# md").unwrap();
        std::fs::write(root.join("备注.txt"), b"note").unwrap();
    }

    #[test]
    fn standard2_ignores_aux_files() {
        let tmp = tempfile::tempdir().unwrap();
        mk_dirs(tmp.path(), &FACEPACK_DIRS);
        drop_aux_files(tmp.path());
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str(), r.missing.len()), (2, "complete", 0));
    }

    #[test]
    fn standard3_fixable_ignores_aux_files() {
        let tmp = tempfile::tempdir().unwrap();
        mk_dirs(tmp.path(), &["China", "Japan", "Korea"]);
        drop_aux_files(tmp.path());
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str(), r.expected_total), (3, "fixable", 65));
    }

    #[test]
    fn standard1_complete_with_aux_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("face.png"), b"png").unwrap();
        drop_aux_files(tmp.path());
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str()), (1, "complete"));
    }

    #[test]
    fn scan_images_core_flat_and_standard2() {
        // 平铺包：根 3 图 → (3, 1)
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..3 {
            std::fs::write(tmp.path().join(format!("f{i}.png")), b"png").unwrap();
        }
        let (total, standard, scan) = scan_images_core(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!((total, standard), (3, 1));
        assert_eq!(scan.layout, PackLayout::Flat);
        // 14 目录包：各目录图数之和 → (28, 2)
        let tmp2 = tempfile::tempdir().unwrap();
        for eth in FACEPACK_DIRS {
            let d = tmp2.path().join(eth);
            std::fs::create_dir_all(&d).unwrap();
            for i in 0..2 {
                std::fs::write(d.join(format!("e{i}.png")), b"png").unwrap();
            }
        }
        let (total, standard, _) = scan_images_core(tmp2.path().to_str().unwrap()).unwrap();
        assert_eq!((total, standard), (28, 2));
    }

    #[test]
    fn scan_rtf_core_parses_summary() {
        let dir = tempfile::tempdir().unwrap();
        let rtf = dir.path().join("list.rtf");
        std::fs::write(
            &rtf,
            "| UID       | Nat  | 2nd Nat | Name          |    |    |   |\n\
             | 2000472008| ESP  | USA     | Pepe Sáenz    | 1  | 12 | 1 |\n\
             | 2002161479| BRA  |         | José·Milhomem | 1  | 16 | 3 |\n",
        ).unwrap();
        let (summary, data) = scan_rtf_core(rtf.to_str().unwrap()).unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.newgan, 2); // 两条 UID 均带 r- 前缀
        assert!(!summary.detailed);    // 基础名单
        assert_eq!(summary.language, "English");
        assert_eq!(data.len(), 2);
        // 非法 RTF → Err
        let bad = dir.path().join("bad.rtf");
        std::fs::write(&bad, "乱七八糟没有数据行").unwrap();
        assert!(scan_rtf_core(bad.to_str().unwrap()).is_err());
    }

    #[test]
    fn check_integrity_ignores_junk_dirs() {
        // __MACOSX / .git 等垃圾目录不参与判定：平铺包不因它们误判 Unknown。
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("face.png"), b"png").unwrap();
        std::fs::create_dir(tmp.path().join("__MACOSX")).unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let r = check_integrity(tmp.path().to_string_lossy().to_string());
        assert_eq!((r.standard, r.status.as_str()), (1, "complete"));
    }
}
