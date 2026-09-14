//! Tauri 命令层（对等旧版 services/* + app.py）：前端 invoke 的全部入口。
//!
//! 【设计】数据留在 Rust，IPC 只传摘要/分页：大名单与日志在缓存里按页查询，
//! 每次状态变更后回传 AppSnapshot 让前端整体刷新。命令可能在不同线程被调用、后台替换
//! 也跑在单独线程，因此 AppState 的共享字段用 Mutex 保护、跨线程引用用 Arc、
//! 运行/取消标志用 AtomicBool。返回 Result<T, String> 时，Err 的中文描述由前端直接弹窗展示。
use crate::face_mapper::Mode;
use crate::facepack_scanner::{self, IntegrityResult, PackScan};
use crate::logger::{LogPage, LogSink, Logger};
use crate::paging::PageResult;
use crate::profile_manager::ProfileManager;
use crate::replace_flow::{self, Outcome, ReplaceParams};
use crate::rtf_scanner::{self, PlayerQuery, PlayerRecord, RosterSummary, RtfScanner};
use crate::templates::{self, RuleConfig};
use crate::xml_parser::{self, MappingEntry, SnapshotEntry, SnapshotQuery};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

// 标准二的 14 目录常量保留在此 re-export（validate_image_dirs 与既有测试引用路径不变）；
// 标准三目录清单已由 rule_config 的 standard3.json（FolderList）取代，不再有硬编码常量。
pub use crate::facepack_scanner::FACEPACK_DIRS;

/// 应用全局状态（在 lib.rs 的 setup 里用 app.manage 交给 Tauri 托管）。
/// 【为何这样包】命令可能在不同线程被调用，后台替换也跑在单独线程；
/// 所以共享数据用 Mutex（互斥锁）保护，跨线程引用用 Arc（共享所有权），标志用 AtomicBool。
pub struct AppState {
    pub pm: Mutex<ProfileManager>, // 存档管理器（Mutex 保证同一时刻只有一个线程操作）
    pub logger: Arc<Logger>,       // 日志门面（Arc 跨线程共享；写入走 log 门面，回查走内存缓冲）
    pub cancel: Arc<AtomicBool>,   // 请求取消标志
    pub running: Arc<AtomicBool>,  // 是否正在替换
    pub rtf_cache: Arc<Mutex<Vec<PlayerRecord>>>, // 最近一次解析的球员缓存
    pub mapping_cache: Arc<Mutex<Vec<MappingEntry>>>, // 最近一次生成的映射缓存
    pub faces_cache: Arc<Mutex<Option<FaceIndex>>>, // 「扫描头像」的包索引缓存（替换可复用）
    pub snapshot_cache: Arc<Mutex<Option<SnapshotCache>>>, // 快照解析缓存（快照预览分页复用；「读取快照」与替换收尾都会写）
    pub data_dir: PathBuf, // 便携数据目录（标准规则 data/rules/ 在其下）
}

/// 「扫描头像」产出的索引缓存：目录 + 完整扫描结果（仅按 img_dir 命中，切组即失效）。
pub struct FaceIndex {
    pub img_dir: String,
    pub scan: Arc<PackScan>,
}

/// 「读取快照」产出的解析缓存：快照文件路径 + 有序映射条目（快照预览分页查询复用）。
/// 查询时按路径比对，路径不符即失效（切档/切组后由「读取快照」重新填充）。
/// 两个写入点：「读取快照」命令；替换成功收尾的自动同步（sync_group_snapshot）。
pub struct SnapshotCache {
    pub xml_path: String,
    pub entries: Vec<MappingEntry>,
}

// ---------------- 传给前端的类型（统一 camelCase） ----------------
// 下面这些结构体是「前后端的数据契约」：Rust 端蛇形命名经 serde 自动转成前端的驼峰命名，
// 前端 types.ts 里有与之对应的 TypeScript 接口，两边字段必须一致。

/// 一个头像包（组）的摘要：目录 + 已设 RTF。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    pub img_dir: String,
    pub rtf: String,
}

/// 当前组 config.xml 的路径与存在性（快照附带，前端渲染 config.xml 行）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigXmlInfo {
    pub path: String,
    pub exists: bool,
}

/// 应用全局快照：一次性把前端需要的所有状态打包返回（前端拿到后整体替换本地状态）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub profiles: Vec<String>,             // 所有存档名
    pub current_profile: String,           // 当前激活存档
    pub groups: Vec<GroupInfo>,            // 当前存档下的所有组
    pub current_group: Option<String>,     // 当前选中组（Option 对应前端 null）
    pub current_rtf: String,               // 当前组的 RTF
    pub replacing: bool,                   // 是否正在替换
    pub config_xml: Option<ConfigXmlInfo>, // 当前组的 config.xml（无选中组为 None）
    pub log_path: String,                  // 日志文件路径
}

/// 进度事件的载荷（通过 replace:progress 事件发给前端）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPayload {
    pub status: String,
    pub value: u8,
}

/// 「检查完整性」结果 IntegrityResult 已下沉到 facepack_scanner（属头像包分类判定的领域产物）。

/// 「扫描头像」返回摘要：图片总数 + 命中的标准号（0=无法识别）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImageScanResult {
    pub total: usize,
    pub standard: u8,
}

/// 「扫描RTF」返回摘要直接复用 rtf_scanner::RosterSummary（camelCase 序列化结构与前端一致），
/// 不再在命令层维护一份字段一一对应的 RtfScanResult——「扫描RTF」与「替换」两条路径本就共用同一口径。

/// 前端发起替换时传来的参数（Deserialize = 从前端 JSON 解析而来）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceArgs {
    pub mode: String,
    pub filter_newgan: bool,
    pub allow_duplicates: bool,
    pub save_backup: bool,
}

/// 名单分页查询参数 PlayerQuery 与筛选/排序/分页逻辑已下沉到 rtf_scanner::query_page（名单领域）。

/// 单人预览的返回体：球员资料 + 映射 + 实际图片路径（三者都可能为 None/null）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPreview {
    pub record: Option<PlayerRecord>,
    pub mapping: Option<MappingEntry>,
    pub image_path: Option<String>,
}

/// 「读取快照」返回摘要：快照里的映射总条数。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub total: usize,
}

/// 快照预览的 SnapshotEntry / SnapshotQuery 及筛选分页逻辑已下沉到 xml_parser（config.xml 映射领域）。

// ---------------- 命令实现 ----------------

/// 把 ProfileManager 的当前状态汇总成一个 AppSnapshot（发给前端的快照）。
/// 【复用】几乎每个命令改完状态后都会调它，把最新状态回传前端，前端据此刷新界面。
fn snapshot(state: &State<'_, AppState>) -> AppSnapshot {
    let pm = state.pm.lock().unwrap(); // 加锁读取存档管理器（lock 期间独占）
                                       // 当前组的存档 XML 快照路径（.user/<存档名>/<目录末段>.xml，存档资料里的那份）。
                                       // 切组/替换后随快照自动刷新；从未替换过的组无快照（exists=false）。
    let config_xml = match pm.cur_group.as_ref() {
        Some(g) => {
            let path = crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, g);
            Some(ConfigXmlInfo {
                path: path.to_string_lossy().to_string(),
                exists: path.is_file(),
            })
        }
        None => None,
    };
    AppSnapshot {
        profiles: pm.config.profile.keys().cloned().collect(),
        current_profile: pm.cur_prf.clone(),
        groups: pm
            .prf_cfg
            .groups
            .iter()
            .map(|(dir, g)| GroupInfo {
                img_dir: dir.clone(),
                rtf: g.rtf.clone(),
            })
            .collect(),
        current_group: pm.cur_group.clone(),
        current_rtf: pm.get_cur_rtf(),
        replacing: state.running.load(Ordering::SeqCst), // SeqCst 是最严格的内存顺序（跨线程同步标志）
        config_xml,
        log_path: state.logger.path().to_string_lossy().to_string(),
    }
}

// 命令函数带 #[tauri::command]（还需在 lib.rs 的 invoke_handler 里登记）。

/// 获取应用全局快照（前端启动时、或需要强制刷新时调）。
#[tauri::command]
pub fn get_app_state(state: State<'_, AppState>) -> AppSnapshot {
    snapshot(&state)
}

/// 新建并激活存档。name 会先 trim 去首尾空白。
#[tauri::command]
pub fn create_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let name = name.trim().to_string();
    state.pm.lock().unwrap().create_profile(&name)?;
    state
        .logger
        .info("profile", format!("创建并激活 Profile: {name}"));
    Ok(snapshot(&state))
}

/// 删除存档。'No Profile' 不可删（返回错误提示）。
#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    if !pm.delete_profile(&name)? {
        return Err("无法删除缺省档 No Profile".into());
    }
    drop(pm); // 显式释放锁（因为后面 snapshot() 会再次加锁，不先释放会死锁）
    Ok(snapshot(&state))
}

/// 切换存档（含快照存取：保存旧档各组 config.xml、恢复新档各组快照）。
#[tauri::command]
pub fn switch_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    pm.load_profile(&name)?;
    // 切档后把全局配置 profiles.json 也落盘（记住新的激活位）。
    crate::json_store::save(&pm.user_cfg_path().join("profiles.json"), &pm.config)?;
    drop(pm);
    state
        .logger
        .info("profile", format!("激活 Profile: {name}"));
    Ok(snapshot(&state))
}

/// 关联头像包目录（新目录自动建组并设为当前组）。未激活存档时不允许。
#[tauri::command]
pub fn select_image_directory(
    state: State<'_, AppState>,
    path: String,
) -> Result<AppSnapshot, String> {
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
pub fn set_current_group(
    state: State<'_, AppState>,
    img_dir: String,
) -> Result<AppSnapshot, String> {
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
/// 这个命令不需要 State（不访问全局状态），只根据传入的 img_dir 扫描目录。
#[tauri::command]
pub fn validate_image_dirs(img_dir: String) -> Vec<String> {
    // 目录下已有的子目录名收集成集合，再对照标准 14 个目录筛出缺失项。
    let existing: std::collections::HashSet<String> = std::fs::read_dir(&img_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    FACEPACK_DIRS
        .iter()
        .filter(|d| !existing.contains(**d))
        .map(|d| d.to_string())
        .collect()
}

/// 「检查完整性」：按 templates 的 FolderList（用户可配置）识别头像包归属哪个标准，
/// 并给出可一键补全的缺失目录与未配置的多余目录。判定逻辑在 facepack_scanner::check_integrity，
/// 命令层只负责加载规则配置并把结果回传前端（IPC）。
#[tauri::command]
pub fn check_integrity(
    state: State<'_, AppState>,
    img_dir: String,
) -> Result<IntegrityResult, String> {
    let config = RuleConfig::load(&state.data_dir)?;
    Ok(facepack_scanner::check_integrity(&img_dir, &config.folder_lists()))
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

/// 标准号的中文标注（写日志用）。目录数取自规则配置的 FolderList，不在这里硬编码。
fn layout_label(standard: u8, config: &RuleConfig) -> String {
    let dirs = |n: u8| {
        config
            .folder_lists()
            .into_iter()
            .find(|(t, _)| *t == n)
            .map(|(_, f)| f.len())
            .unwrap_or(0)
    };
    match standard {
        1 => "标准一/图片直铺".to_string(),
        2 => format!("标准二/{} 目录", dirs(2)),
        3 => format!("标准三/{} 目录", dirs(3)),
        _ => "无法识别".to_string(),
    }
}

/// 「扫描RTF」核心：格式识别 + 全量解析名单（全字段入库），返回 (摘要, 记录列表)。
/// 识别失败 → Err("RTF 格式不合法：…")，文案带首个不合法字段定位，前端弹窗直接展示。
fn scan_rtf_core(
    rtf_path: &str,
    logger: &impl LogSink,
) -> Result<(RosterSummary, Vec<PlayerRecord>), String> {
    let mut scanner = RtfScanner::new();
    scanner.detect_format(Path::new(rtf_path), logger)?;
    // filter=false：扫描展示全量名单（Replace 流程才按「仅随机人」过滤）。
    let data = scanner.parse_roster(Path::new(rtf_path), false, logger)?;
    // 摘要口径统一收在 RosterSummary（随机人前缀判定、拓展信息判定都不在这里重复一遍）。
    Ok((RosterSummary::from_records(&scanner, &data), data))
}

/// 「扫描头像」：扫描当前组头像包、缓存索引（供替换复用），结果写日志。
/// 【async】重扫描（递归遍历包内全部目录）跑在异步运行时线程，避免阻塞主线程导致界面卡住。
#[tauri::command]
pub async fn scan_images(state: State<'_, AppState>) -> Result<ImageScanResult, String> {
    let img_dir = {
        state
            .pm
            .lock()
            .unwrap()
            .cur_group
            .clone()
            .ok_or("请先选择头像包目录")?
    };
    let config = RuleConfig::load(&state.data_dir)?; // 用户改坏规则 json → 报错提示
    let (total, standard, scan) = facepack_scanner::scan_and_total(&img_dir, &config.folder_lists())?;
    // Arc 包一层入缓存：替换复用时只涨引用计数，不复制整份图片索引。
    *state.faces_cache.lock().unwrap() = Some(FaceIndex {
        img_dir,
        scan: Arc::new(scan),
    });
    state.logger.info(
        "scan",
        format!(
            "扫描头像完成：{total} 张图片（{}）",
            layout_label(standard, &config)
        ),
    );
    Ok(ImageScanResult { total, standard })
}

/// 「扫描RTF」：解析当前组 RTF 并填充名单预览缓存（无需先 Replace），结果写日志。
/// 【async】大名单解析跑在异步运行时线程，避免阻塞主线程导致界面卡住。
#[tauri::command]
pub async fn scan_rtf(state: State<'_, AppState>) -> Result<RosterSummary, String> {
    let rtf = { state.pm.lock().unwrap().get_cur_rtf() };
    if rtf.is_empty() {
        return Err("请先选择 RTF 名单".into());
    }
    // &*state.logger：&Arc<Logger> 显式解引用成 &Logger（泛型参数处不会自动解强制引用）。
    let (summary, data) = scan_rtf_core(&rtf, &*state.logger)?;
    *state.rtf_cache.lock().unwrap() = data; // 名单预览（query_players）直接读这份缓存（全字段）
    state.logger.info(
        "scan",
        format!(
            "扫描 RTF 完成：{} 条（随机人 {}，{}，{}）",
            summary.total, summary.newgan, summary.format, summary.language
        ),
    );
    Ok(summary)
}

/// 用资源管理器打开目录（便携应用面向 Windows 的统一出口）。
/// 组 key 经 norm_dir 带尾部分隔符，而 explorer 对引号内的尾反斜杠解析脆弱
/// （`\"` 被当作转义引号，含空格的路径会打不开正确位置），传入前必须去掉尾部分隔符。
fn explorer_open(dir: &Path, what: &str) -> Result<(), String> {
    let s = dir.to_string_lossy();
    let trimmed = s.trim_end_matches(['\\', '/']);
    // 盘根（`C:\`）去尾后是 `C:`（表示「该盘当前目录」而非根），需保留原样；裸分隔符同理。
    let target = if trimmed.is_empty() || trimmed.ends_with(':') {
        s.as_ref()
    } else {
        trimmed
    };
    std::process::Command::new("explorer")
        .arg(target)
        .spawn()
        .map_err(|e| format!("打开{what}失败: {e}"))?;
    Ok(())
}

/// 打开标准规则目录（data/rules/）供用户编辑 standard2/standard3.json；
/// 目录或文件缺失时先导出内置默认。编辑保存后下次替换/检查完整性即生效。
#[tauri::command]
pub fn open_rules_dir(state: State<'_, AppState>) -> Result<(), String> {
    templates::ensure_user_files(&state.data_dir)?;
    let dir = state.data_dir.join("rules");
    explorer_open(&dir, "规则目录")
}

/// 启动批量替换：校验参数后开一个后台线程跑流水线，避免阻塞界面。
/// app 用于在线程里向前端 emit 事件；args 是前端传来的参数。
#[tauri::command]
pub fn start_replace(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ReplaceArgs,
) -> Result<(), String> {
    // swap(true) 原子地把 running 置 true 并返回旧值；旧值已是 true 说明有任务在跑，拒绝重入。
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("已有替换任务在执行".into());
    }
    state.cancel.store(false, Ordering::SeqCst); // 重置取消标志
                                                 // 先在锁内取出当前 RTF 和组目录，随即释锁（不能把 Mutex 带进新线程）。
                                                 // 顺带取该档当前组的存档快照路径：替换成功后要把新 config.xml 同步到这份快照位置
                                                 // （快照预览的数据源，与「读取快照」按钮读的是同一路径）。
    let (rtf, img_dir, snapshot_path) = {
        let pm = state.pm.lock().unwrap();
        let snap = pm
            .cur_group
            .as_ref()
            .map(|g| crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, g));
        (pm.get_cur_rtf(), pm.cur_group.clone(), snap)
    };
    // 以下每条校验失败路径都必须先把 running 复位，否则界面全局禁用到重启。
    let Some(img_dir) = img_dir else {
        state.running.store(false, Ordering::SeqCst);
        return Err("请先选择头像包目录".into());
    };
    if rtf.is_empty() {
        state.running.store(false, Ordering::SeqCst);
        return Err("请先选择 RTF 名单".into());
    }
    let mode = match Mode::parse(&args.mode) {
        Ok(m) => m,
        Err(e) => {
            state.running.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    // 加载标准规则配置（data/rules/standardN.json，用户改坏 json → 报错中止，错误含文件路径）。
    let rule_config = match RuleConfig::load(&state.data_dir) {
        Ok(c) => c,
        Err(e) => {
            state.running.store(false, Ordering::SeqCst);
            return Err(e);
        }
    };
    // 「扫描头像」缓存命中同一目录则复用其索引（避免重复扫盘）；切组即失效、现场新扫描。
    let pre_scan = {
        state
            .faces_cache
            .lock()
            .unwrap()
            .as_ref()
            .filter(|c| c.img_dir == img_dir)
            .map(|c| c.scan.clone())
    };
    let params = ReplaceParams {
        rtf_path: rtf,
        img_dir,
        mode,
        filter_newgen: args.filter_newgan,
        allow_duplicates: args.allow_duplicates,
        save_backup: args.save_backup,
        pre_scan,
        rule_config,
    };
    // 后台线程需要的句柄先 clone 出来（Arc::clone 只涨引用计数，不拷贝真实数据）。
    let (cancel, running) = (state.cancel.clone(), state.running.clone());
    let (rtf_cache, mapping_cache) = (state.rtf_cache.clone(), state.mapping_cache.clone());
    let snapshot_cache = state.snapshot_cache.clone(); // 替换成功时同步刷新（快照预览的数据源）
    let logger = state.logger.clone();
    std::thread::spawn(move || {
        let emit_app = app.clone();
        // 跑流水线；progress 回调里通过 emit 把进度发给前端（事件名 replace:progress）。
        // 流水线关键步骤的日志经 logger 记录，由 log:batch 事件实时回显前端日志面板。
        let result = replace_flow::run(
            &params,
            &mut |status: &str, value: u8| {
                let _ = emit_app.emit(
                    "replace:progress",
                    ProgressPayload {
                        status: status.into(),
                        value,
                    },
                );
            },
            &cancel,
            &*logger, // &Arc<Logger> 显式解引用成 &Logger（泛型参数处不会自动解强制引用）
        );
        // 把解析结果与映射写入缓存（供后续预览/分页查询使用）。
        *rtf_cache.lock().unwrap() = result.rtf_data.clone();
        *mapping_cache.lock().unwrap() = result.mapping.clone();
        // 快照预览的数据源也在收尾处刷新（与「读取快照」按钮写的是同一份缓存）：
        // 替换成功 → 把刚写好的 config.xml 备份到该档快照位置并重解析入缓存，让快照预览
        // 无需再点「读取快照」就能看到本次结果；失败/取消不改 config.xml，快照保持原样
        //（已读过的那份仍然有效，所以这里只写不清）。
        if matches!(result.outcome, Outcome::Finished) {
            if let Some(snap_path) = snapshot_path.as_deref() {
                sync_group_snapshot(snap_path, &params.img_dir, &snapshot_cache, &*logger);
            }
        }
        if matches!(result.outcome, Outcome::Finished) && result.mapping.is_empty() {
            // 跑完却一条映射也没有：config.xml 会被写成空表，必须留 WARNING
            logger.warn("replace", "映射结果为空：本次写入的 config.xml 不含任何映射（多为「仅随机人」过滤掉了全部名单，或头像包内无图片）");
        }
        // 名单摘要（替换时后端已经解析过名单）：随事件回传前端，名单预览不必再点一次「扫描RTF」
        // 就知道名单格式；失败/取消为 None（与 rtf_cache 被写回空表一致）。
        let roster = result.roster;
        // 把结果映射为前端认的字符串；失败时额外记一条 ERROR 日志。
        let result_str = match result.outcome {
            Outcome::Finished => "finished",
            Outcome::Cancelled => "cancelled",
            Outcome::Failed(ref m) => {
                logger.error("replace", m.clone());
                "failed"
            }
        };
        // 发「完成」事件（前端收到后刷新快照并弹提示）。
        let _ = app.emit(
            "replace:finished",
            serde_json::json!({ "result": result_str, "roster": roster }),
        );
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
        return Err("所选图片不在当前头像包内（或文件不存在），请从当前头像包目录或其子目录中选择图片".into());
    };
    let hit =
        xml_parser::single_replacement(&uid, &pack, &image, Path::new(&img_dir), save_backup)?;
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
    state.logger.info(
        "viewer",
        format!(
            "单人换脸 {uid} → {}",
            xml_parser::record_from(&pack, &image)
        ),
    );
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
    Ok(PlayerPreview {
        record,
        mapping,
        image_path,
    })
}

/// RTF 名单分页查询（大名单预览；数据留 Rust，IPC 只传当前页）。
/// 名单可能上万条，一次全传会卡顿，所以只传当前页。筛选/排序/分页逻辑在 rtf_scanner::query_page，
/// 命令层只负责取出缓存（整体 clone 避免长时间持锁）并转交。
#[tauri::command]
pub fn query_players(state: State<'_, AppState>, query: PlayerQuery) -> PageResult<PlayerRecord> {
    let rows = state.rtf_cache.lock().unwrap().clone();
    rtf_scanner::query_page(rows, &query)
}

// ---------------- 快照预览（「读取快照」按钮 → 结果区「快照预览」Tab） ----------------

/// 「读取快照」：解析存档 XML 快照（.user/<存档>/<组>_snapshot.xml，config.xml 的存档副本）
/// 并缓存有序条目，供「快照预览」分页查询。xml_path 由前端传当前展示档的快照全路径（浏览态自洽）；
/// 每次点击都重读文件，展示的始终是快照文件的最新内容。
/// 【async】大快照（可达数万行）的正则解析跑在异步运行时线程，避免阻塞主线程导致界面卡住。
#[tauri::command]
pub async fn read_snapshot(
    state: State<'_, AppState>,
    xml_path: String,
) -> Result<SnapshotInfo, String> {
    let path = PathBuf::from(&xml_path);
    if !path.is_file() {
        return Err("存档 XML 快照尚未生成（请先执行一次替换）".into());
    }
    let total = cache_snapshot(&state.snapshot_cache, &path)?;
    state
        .logger
        .info("scan", format!("读取快照完成：{total} 条映射"));
    Ok(SnapshotInfo { total })
}

/// 解析快照文件并写入快照缓存，返回条目数。两条路径共用：「读取快照」命令（用户显式读），
/// 以及替换收尾的自动同步（sync_group_snapshot）。缓存里存的路径即传入路径的字符串形式——
/// 前端两处拿到的都是同一套 `group_xml_path` 结果，快照预览查询时按此路径比对命中。
fn cache_snapshot(cache: &Mutex<Option<SnapshotCache>>, xml_path: &Path) -> Result<usize, String> {
    let entries = xml_parser::parse_xml_entries(xml_path)?;
    let total = entries.len();
    *cache.lock().unwrap() = Some(SnapshotCache {
        xml_path: xml_path.to_string_lossy().to_string(),
        entries,
    });
    Ok(total)
}

/// 替换收尾时同步该档的存档 XML 快照：把刚写入的 config.xml 备份到该档快照位置
/// （等价于切档时 save_group_xml 做的备份，保证「存档资料」里那份也是最新的），
/// 再按新文件重写快照缓存——快照预览因此无需再点「读取快照」就能看到本次结果。
/// 任何失败都只记 WARNING 不返回 Err：快照新旧不影响本次替换的结论（config.xml 已写好）。
/// 【只写不清】失败/取消时调用方根本不会调它（config.xml 未被改写，已读的那份仍有效）。
fn sync_group_snapshot(
    snapshot_path: &Path,
    img_dir: &str,
    cache: &Mutex<Option<SnapshotCache>>,
    logger: &impl LogSink,
) {
    let src = Path::new(img_dir).join("config.xml");
    if !src.is_file() {
        return; // 没有 config.xml（替换成功时必有，这里只做防御）
    }
    if let Some(p) = snapshot_path.parent() {
        if let Err(e) = std::fs::create_dir_all(p) {
            logger.write(
                log::Level::Warn,
                "replace",
                format!("创建快照目录失败：{e}"),
            );
            return;
        }
    }
    if let Err(e) = std::fs::copy(&src, snapshot_path) {
        logger.write(
            log::Level::Warn,
            "replace",
            format!("同步存档快照失败：{e}"),
        );
        return;
    }
    match cache_snapshot(cache, snapshot_path) {
        Ok(total) => logger.info("replace", format!("同步存档快照完成：{total} 条映射")),
        // 解析失败：宁缺勿旧——清空缓存而不是留着可能正是这份路径的旧数据（留着会让预览显示替换前的旧表）；
        // 清空后快照预览查询会按磁盘上新文件重读一次。
        Err(e) => {
            *cache.lock().unwrap() = None;
            logger.write(
                log::Level::Warn,
                "replace",
                format!("同步存档快照后解析失败：{e}"),
            );
        }
    }
}

/// 快照预览分页查询：UID 筛选 + 分页，数据留在缓存里，IPC 只传当前页。
/// 只对当前页的条目解析实际图片路径——整表几万条逐一探测文件开销太大，按页解析才现实。
#[tauri::command]
pub fn query_snapshot(
    state: State<'_, AppState>,
    xml_path: String,
    img_dir: String,
    query: SnapshotQuery,
) -> Result<PageResult<SnapshotEntry>, String> {
    let mut cache = state.snapshot_cache.lock().unwrap();
    // 正常流程「读取快照」已填好缓存且路径一致；路径不符（缓存被换走）时按请求路径重读兜底。
    let hit = cache
        .as_ref()
        .map(|c| c.xml_path == xml_path)
        .unwrap_or(false);
    if !hit {
        let path = PathBuf::from(&xml_path);
        if !path.is_file() {
            return Ok(PageResult {
                total: 0,
                items: Vec::new(),
            });
        }
        *cache = Some(SnapshotCache {
            xml_path: xml_path.clone(),
            entries: xml_parser::parse_xml_entries(&path)?,
        });
    }
    let entries = &cache.as_ref().unwrap().entries;
    Ok(xml_parser::query_snapshot_page(entries, &img_dir, &query))
}

// ---------------- 存档浏览预览（下拉选择存档 = 浏览，不切档） ----------------

/// 预览指定存档的快照（仅供前端下拉浏览展示，不改变后端当前档/激活状态）。
/// currentProfile 字段返回被浏览的存档名；激活仍需前端显式调 switch_profile。
#[tauri::command]
pub fn preview_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let pm = state.pm.lock().unwrap();
    let (prf_cfg, cur_group) = pm.preview_cfg(&name)?;
    // 浏览档的存档 XML 快照路径与存在性（与激活档口径一致）。
    let config_xml = cur_group.as_ref().map(|g| {
        let path = crate::paths::group_xml_path(&pm.user_cfg_path(), &name, g);
        ConfigXmlInfo {
            path: path.to_string_lossy().to_string(),
            exists: path.is_file(),
        }
    });
    Ok(AppSnapshot {
        profiles: pm.config.profile.keys().cloned().collect(),
        current_profile: name, // 浏览口径：返回被浏览的存档名
        groups: prf_cfg
            .groups
            .iter()
            .map(|(dir, g)| GroupInfo {
                img_dir: dir.clone(),
                rtf: g.rtf.clone(),
            })
            .collect(),
        current_group: cur_group.clone(),
        current_rtf: cur_group
            .as_ref()
            .and_then(|g| prf_cfg.groups.get(g))
            .map(|g| g.rtf.clone())
            .unwrap_or_default(),
        replacing: state.running.load(Ordering::SeqCst),
        config_xml,
        log_path: state.logger.path().to_string_lossy().to_string(),
    })
}

// ---------------- 目录打开（「当前任务」卡 config.xml 行：读取快照 / 查看位置） ----------------

/// 打开存档 XML 快照所在目录（.user/<存档名>/，「查看位置」按钮）。
/// 路径由前端传当前展示档的快照 XML 全路径（同上，浏览态自洽）。
#[tauri::command]
pub fn open_snapshot_dir(path: String) -> Result<(), String> {
    let xml = PathBuf::from(path);
    if !xml.is_file() {
        return Err("存档 XML 快照尚未生成（请先执行一次替换）".into());
    }
    let dir = xml.parent().ok_or_else(|| "无法定位快照目录".to_string())?;
    explorer_open(dir, "存档快照目录")
}

// ---------------- 标准规则文件的查看/保存（规则编辑对话框） ----------------

/// 按前端传来的 name 查规则文件登记表（唯一白名单来源：`templates::rule_files()`）。
/// 【为什么不在这里 match】每加一个标准就要在此处加一个分支，还容易漏挂结构校验；
/// 登记表由各标准模块自己提供「路径 + 解析校验 + 写回」，命令层只负责查表与转发。
fn rule_file(name: &str) -> Result<&'static templates::RuleFile, String> {
    // 表在首次调用时构造一次并常驻，换来命令层零 match（新增标准时本文件不用改）。
    static TABLE: std::sync::LazyLock<Vec<templates::RuleFile>> =
        std::sync::LazyLock::new(templates::rule_files);
    TABLE
        .iter()
        .find(|f| f.name == name)
        .ok_or_else(|| format!("未知规则文件: {name}"))
}

/// 读取标准规则文件内容（供规则编辑对话框；缺失时先导出内置默认）。
#[tauri::command]
pub fn read_rule_file(state: State<'_, AppState>, name: String) -> Result<String, String> {
    let rf = rule_file(&name)?;
    let path = (rf.path)(&state.data_dir);
    if !path.is_file() {
        templates::ensure_user_files(&state.data_dir)?;
    }
    std::fs::read_to_string(&path).map_err(|e| format!("读取规则文件失败: {e}"))
}

/// 保存标准规则文件：先按该标准的结构解析并校验，合法才原子写回
/// （json_store 自动 pretty 格式化，相当于顺带规范化用户手改的格式）。
#[tauri::command]
pub fn save_rule_file(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<(), String> {
    let rf = rule_file(&name)?;
    let path = (rf.path)(&state.data_dir);
    (rf.save)(&path, &content)
}

/// 名单预览「头像」列预览：把列原文解析成当前头像包内的实际图片路径（找不到返回 null）。
/// 先走确定性路径探测（xml_parser::resolve_avatar_path）；裸词干在包根没命中时，
/// 再借「扫描头像」的递归索引在全部子/孙目录里找同名图（缓存缺失/切组则不做递归，避免现场扫盘卡顿）。
#[tauri::command]
pub fn resolve_avatar(
    state: State<'_, AppState>,
    avatar: String,
) -> Result<Option<String>, String> {
    let avatar = avatar.trim();
    if avatar.is_empty() {
        return Ok(None);
    }
    let pm = state.pm.lock().unwrap();
    let img_dir = pm.cur_group.clone().ok_or("请先选择头像包目录")?;
    drop(pm); // 与后续 faces_cache 是两把锁，显式提前释放避免嵌套持锁。
              // 1) 绝对路径 / 相对包根（含补扩展名）。
    if let Some(p) = xml_parser::resolve_avatar_path(Path::new(&img_dir), avatar) {
        return Ok(Some(p));
    }
    // 2) 裸词干（无分隔符、无扩展名）才允许递归兜底；带路径的输入已在上面按其指定位置探测过。
    if !xml_parser::is_bare_stem(avatar) {
        return Ok(None);
    }
    let cache = state.faces_cache.lock().unwrap();
    let Some(idx) = cache.as_ref().filter(|c| c.img_dir == img_dir) else {
        return Ok(None); // 没有同目录的扫描索引：提示用户可先「扫描头像」，不在悬停时现场扫大包
    };
    // 递归兜底的目录选取策略在 PackScan::find_stem_dir（按字典序取最前命中目录）。
    if let Some(dir) = idx.scan.find_stem_dir(avatar) {
        if let Some(p) = xml_parser::resolve_image_file(Path::new(&img_dir), dir, avatar) {
            return Ok(Some(p));
        }
    }
    Ok(None)
}

/// 读取预览图片字节（前端转 blob URL 显示）。
/// WebView 无权直接读本地磁盘图片，所以由 Rust 读出字节传给前端：
/// 返回 tauri::ipc::Response（二进制响应），前端收到的是 ArrayBuffer。
#[tauri::command]
pub fn read_image(path: String) -> Result<tauri::ipc::Response, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取图片失败: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// 分页查询日志（前端打开日志面板、或切换过滤条件时调）。
#[tauri::command]
pub fn query_logs(
    state: State<'_, AppState>,
    level: String,
    only: bool,
    offset: usize,
    limit: usize,
) -> LogPage {
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
    use crate::logger::ChannelLogger;

    #[test]
    fn scan_rtf_core_parses_summary() {
        let dir = tempfile::tempdir().unwrap();
        let rtf = dir.path().join("list.rtf");
        std::fs::write(
            &rtf,
            "| UID       | Nat  | 2nd Nat | Name          |    |    |   |\n\
             | 2000472008| ESP  | USA     | Pepe Sáenz    | 1  | 12 | 1 |\n\
             | 2002161479| BRA  |         | José·Milhomem | 1  | 16 | 3 |\n",
        )
        .unwrap();
        let logger = ChannelLogger::channel().0;
        let (summary, data) = scan_rtf_core(rtf.to_str().unwrap(), &logger).unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.newgan, 2); // 两条 UID 均带 r- 前缀
        assert!(!summary.detailed); // 基础名单无拓展信息
        assert_eq!(summary.format, "基础名单");
        assert_eq!(summary.language, "English");
        assert_eq!(data.len(), 2);
        // 非法 RTF → Err，文案带「RTF 格式不合法」
        let bad = dir.path().join("bad.rtf");
        std::fs::write(&bad, "乱七八糟没有数据行").unwrap();
        let err = scan_rtf_core(bad.to_str().unwrap(), &logger).err().unwrap();
        assert!(err.contains("RTF 格式不合法"), "{err}");
    }

    #[test]
    fn sync_group_snapshot_copies_config_and_fills_cache() {
        // 替换收尾的自动同步：把阶段4写下的 config.xml 备份到该档快照位置，并立即刷新快照缓存。
        let tmp = tempfile::tempdir().unwrap();
        let img = tmp.path().join("pack");
        std::fs::create_dir_all(&img).unwrap();
        let entries = vec![MappingEntry {
            uid: "r-1001".into(),
            ethnicity: "SpanMed".into(),
            image: "img1".into(),
        }];
        xml_parser::write_xml(&entries, &img, false).unwrap(); // 替换阶段4写下的那份
        let snap_path = tmp.path().join("档1").join("graphics_pack_snapshot.xml");
        let cache = Mutex::new(None);
        let (logger, _rx) = ChannelLogger::channel();
        sync_group_snapshot(&snap_path, img.to_str().unwrap(), &cache, &logger);

        assert!(snap_path.is_file()); // 快照文件已生成（父目录一并补建）
        let got = cache.lock().unwrap();
        let c = got.as_ref().expect("快照缓存应被填充");
        assert_eq!(c.xml_path, snap_path.to_str().unwrap()); // 路径即快照预览查询的比对键
        assert_eq!(c.entries.len(), 1);
        assert_eq!(c.entries[0].uid, "r-1001");
    }

    #[test]
    fn sync_group_snapshot_without_config_xml_keeps_cache_untouched() {
        // 包内没有 config.xml（替换成功时不会发生）：不生成快照、也不动缓存里已有的数据。
        let tmp = tempfile::tempdir().unwrap();
        let img = tmp.path().join("pack");
        std::fs::create_dir_all(&img).unwrap();
        let snap_path = tmp.path().join("档1").join("graphics_pack_snapshot.xml");
        let cache = Mutex::new(Some(SnapshotCache {
            xml_path: "old_snapshot.xml".into(),
            entries: Vec::new(),
        }));
        let (logger, _rx) = ChannelLogger::channel();
        sync_group_snapshot(&snap_path, img.to_str().unwrap(), &cache, &logger);

        assert!(!snap_path.is_file());
        let got = cache.lock().unwrap();
        assert_eq!(got.as_ref().unwrap().xml_path, "old_snapshot.xml");
    }
}
