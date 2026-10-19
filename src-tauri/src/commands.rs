//! Tauri 命令层：前端 invoke 的全部入口。
//!
//! 【设计】数据留在 Rust，IPC 只传摘要/分页，状态变更后回传 AppSnapshot 整体刷新。
//! 命令与后台替换线程并发，故共享字段用 Mutex/Arc、运行与取消标志用 AtomicBool。
//! 返回 Result<T, String> 的 Err 文案由前端直接弹窗展示。
use crate::face_mapper::{FaceMapper, Mode};
use crate::facepack_scanner::{self, IntegrityResult, PackScan, PackSig};
use crate::image_ai::{self, Config};
use crate::image_ai_queue;
use crate::json_store;
use crate::logger::{LogPage, LogSink, Logger};
use crate::profile_manager::ProfileManager;
use crate::replace_flow::{self, Outcome, ReplaceParams};
use crate::rtf_scanner::{PlayerRecord, RosterCounts, RosterSummary, RtfScanner};
use crate::templates::{self, RuleConfig};
use crate::xml_parser::{self, MappingEntry, SnapshotEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

// 标准二的 14 目录常量保留在此 re-export（validate_image_dirs 与既有测试引用路径不变）；
// 标准三目录清单已由 rule_config 的 standard3.json（FolderList）取代，不再有硬编码常量。
pub use crate::facepack_scanner::FACEPACK_DIRS;

/// 应用全局状态（lib.rs 的 setup 里 app.manage 交给 Tauri 托管）。
/// 命令与后台替换线程并发，故共享数据用 Mutex、跨线程引用用 Arc、标志用 AtomicBool。
pub struct AppState {
    pub pm: Mutex<ProfileManager>, // 存档管理器（Mutex 保证同一时刻只有一个线程操作）
    pub logger: Arc<Logger>,       // 日志门面（Arc 跨线程共享；写入走 log 门面，回查走内存缓冲）
    pub cancel: Arc<AtomicBool>,   // 请求取消标志
    pub running: Arc<AtomicBool>,  // 是否正在替换
    pub rtf_cache: Arc<Mutex<Vec<PlayerRecord>>>, // 最近一次解析的球员缓存（作用域见 clear_caches_if_scope_changed）
    pub mapping_cache: Arc<Mutex<Vec<MappingEntry>>>, // 最近一次生成的映射缓存（同上；两者切档/切组/换名单一律清空）
    pub faces_cache: Arc<Mutex<Option<FaceIndex>>>, // 「扫描头像」的包索引缓存（替换可复用）
    pub snapshot_cache: Arc<Mutex<Option<SnapshotCache>>>, // 快照解析缓存（快照预览分页复用；「读取快照」与替换收尾都会写）
    pub image: Arc<Mutex<Config>>,                         // 生图生效配置（image_ai.json + 默认双来源）
    pub image_keys: Arc<Mutex<HashMap<String, String>>>,   // 会话内的明文 Key（密文随配置行落盘，本表只缓存解密结果）
    pub queue: Arc<Mutex<image_ai_queue::QueueState>>,     // 批量生成队列（条目列表 + cancel 标志）
    pub data_dir: PathBuf,                                 // 便携数据目录（标准规则 data/rules/ 在其下）
}

/// 「扫描头像」产出的索引缓存：目录 + 完整扫描结果 + 目录签名。
/// 【命中条件】`img_dir` 相同**且**签名一致（见 facepack_scanner::PackSig）。
/// 映射（FM 显示空白头像）且全无提示；签名把这一情况拦在复用之前。切组（img_dir 变）仍然直接失效。
pub struct FaceIndex {
    pub img_dir: String,
    pub scan: Arc<PackScan>,
    pub sig: Option<PackSig>,
}

/// 「读取快照」产出的解析缓存：快照文件路径 + 有序映射条目（快照预览分页查询复用）。
/// 查询时按路径比对，路径不符即失效（切档/切组后由「读取快照」重新填充）。
/// 两个写入点：「读取快照」命令；替换成功收尾的自动同步（sync_group_snapshot）。
pub struct SnapshotCache {
    pub xml_path: String,
    pub entries: Vec<MappingEntry>,
}

// ---------------- 跟着「当前档 / 当前组」走的内存缓存：作用域变更即作废 ----------------

/// 「缓存作用域」：决定 `rtf_cache` / `mapping_cache` 归谁用的三个量——当前档、当前组、当前组的名单路径。
/// 【为什么需要它】这两个缓存是「最近一次解析/生成的」语义，**没有路径键**
/// （不像 `snapshot_cache`按快照路径命中、`faces_cache` 按目录 + 目录签名），它们的归属完全由这三项决定。
fn cache_scope(pm: &ProfileManager) -> (String, Option<String>, String) {
    (pm.cur_prf.clone(), pm.cur_group.clone(), pm.get_cur_rtf())
}

/// 存档/组/名单变更后调用：作用域变了就清掉那两个缓存（宁缺勿旧）。
/// 清空后前端据快照把 `rosterReady` 复位，，各预览回到引导空态，等下一次「扫描RTF」或替换重新填充。
fn clear_caches_if_scope_changed(state: &State<'_, AppState>, before: (String, Option<String>, String)) {
    // 锁只在这一句里持有：先取出改后的作用域，随后的两把锁与 pm 无嵌套关系。
    let after = cache_scope(&state.pm.lock().unwrap());
    if after == before {
        return; // 作用域没变（例如删的是别的组）：手上这份缓存仍然对应当前档当前组
    }
    state.rtf_cache.lock().unwrap().clear();
    state.mapping_cache.lock().unwrap().clear();
}

// ---------------- 传给前端的类型（统一 camelCase） ----------------
// 这些结构体是前后端的数据契约：serde 把蛇形字段转成驼峰，前端 types.ts 有对应接口，两边字段必须一致。

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

/// 替换进度载荷（replace:progress 事件）。current/total 是阶段内计数，无可数单位时为 0/0（前端只显示百分比）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPayload {
    pub status: String,
    pub value: u8,
    pub current: usize,
    pub total: usize,
}

/// 轻量任务（检查完整性 / 扫描头像 / 扫描RTF / 读取快照）的进度载荷：
/// 除任务名与百分比外还带「第 current 个 / 共 total 个」，状态栏据此显示原始计数。
/// 不复用 ProgressPayload：替换流水线只报阶段与百分比，两者形状与消费方不同。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgressPayload {
    pub status: String,
    pub value: u8,
    pub current: usize,
    pub total: usize,
}

/// 「轻量任务」进度事件名（前端 stores 里 listen("task:progress")）。
const TASK_PROGRESS_EVENT: &str = "task:progress";

/// 轻量任务的进度上报器：把「第 n 个 / 共 total 个」折算成百分比 emit 给前端状态栏。
/// 【为何按百分比去重】几个扫描都是逐项上报（大名单可达数万行），每次都 emit 会刷爆 IPC——
/// 只在百分比真的变了才发，最多 101 条事件。
struct TaskProgress {
    app: AppHandle,
    status: &'static str,
    current: usize,   // 最近一次上报的「已完成个数」
    total: usize,     // 最近一次上报的「总个数」
    last: Option<u8>, // 上一次发出的百分比（None = 还没发过）
}

impl TaskProgress {
    /// 新建上报器；status 是状态栏上显示的任务名（如「扫描RTF」）。
    fn new(app: &AppHandle, status: &'static str) -> Self {
        Self {
            app: app.clone(),
            status,
            current: 0,
            total: 0,
            last: None,
        }
    }

    /// 上报「已完成第 current 个 / 共 total 个」（total == 0 视为无事可做 → 直接 100%）。
    fn report(&mut self, current: usize, total: usize) {
        self.current = current;
        self.total = total;
        let value = if total == 0 {
            100
        } else {
            ((current * 100) / total).min(100) as u8
        };
        self.emit(value);
    }

    /// 收尾上报 100%：命令成功返回前必须调一次，否则总数为 0 或任务过快时进度条会停在半路。
    fn finish(&mut self) {
        self.emit(100);
    }

    fn emit(&mut self, value: u8) {
        if self.last == Some(value) {
            return;
        }
        self.last = Some(value);
        let _ = self.app.emit(
            TASK_PROGRESS_EVENT,
            TaskProgressPayload {
                status: self.status.into(),
                value,
                current: self.current,
                total: self.total,
            },
        );
    }
}

/// `replace` 通道的事件去重器：只在「阶段文案 或 百分比」变化时放行。
/// 【为何键里不能带 current/total】替换流水线的阶段内部现在按项上报（逐球员抽图可达数万次），
/// 计数每项都在变——把计数放进键等于没去重，照样刷爆 IPC。
/// 百分比本就是由 current/total 折算来的，放行时把当时的计数一并带出去，两者自洽。
struct ProgressDedup {
    last: Option<(String, u8)>,
}

impl ProgressDedup {
    /// 返回 true 表示这次该 emit（并把本次的 (阶段文案, 百分比) 记为上次）。
    fn should_emit(&mut self, status: &str, value: u8) -> bool {
        if self.last.as_ref().is_some_and(|(s, v)| s == status && *v == value) {
            return false;
        }
        self.last = Some((status.to_string(), value));
        true
    }
}

// 「检查完整性」结果 IntegrityResult 已下沉到 facepack_scanner（属头像包分类判定的领域产物）。

/// 「扫描头像」返回摘要：图片总数 + 命中的标准号（0=无法识别）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImageScanResult {
    pub total: usize,
    pub standard: u8,
}

// 「扫描RTF」返回摘要复用 rtf_scanner::RosterSummary（与替换路径同一口径），命令层不再另建类型。

/// 前端发起替换时传来的参数（Deserialize = 从前端 JSON 解析而来）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceArgs {
    pub mode: String,
    pub filter_newgan: bool,
    pub allow_duplicates: bool,
    pub save_backup: bool,
    /// 用户已在发起前的预检（`preflight_replace`）里确认过「本次可能把本档既有映射清空」。
    /// 前端只在预检要求确认、且用户点了确认时才置 true——它不能由界面勾选，否则等于没有守卫。
    pub allow_blank: bool,
}

/// 「发起前预检」的返回体（对应前端 preflight_replace）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightResult {
    /// 是否需要用户确认后才发起。判定只有一条：本次有可能把本档既有映射写成空表。
    pub needs_confirm: bool,
    /// 本档本组档景里现有的映射条数（确认后最坏情况会被整体覆盖掉的条数）。
    pub existing: usize,
}

// 名单分页查询参数 PlayerQuery 与筛选/排序/分页逻辑已下沉到 rtf_scanner::query_page（名单领域）。

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

// 快照预览的 SnapshotEntry / SnapshotQuery 及筛选分页逻辑已下沉到 xml_parser（config.xml 映射领域）。

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

// 命令函数需带 #[tauri::command]，并在 lib.rs 的 invoke_handler 里登记。

/// 获取应用全局快照（前端启动时、或需要强制刷新时调）。
#[tauri::command]
pub fn get_app_state(state: State<'_, AppState>) -> AppSnapshot {
    snapshot(&state)
}

/// 新建并激活存档。name 会先 trim 去首尾空白。
#[tauri::command]
pub fn create_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let name = name.trim().to_string();
    let mut pm = state.pm.lock().unwrap();
    let before = cache_scope(&pm); // 新建即激活＝切档：改前记作用域，改完比对作废名单/映射缓存
    pm.create_profile(&name)?;
    drop(pm);
    clear_caches_if_scope_changed(&state, before);
    state.logger.info("profile", format!("创建并激活 Profile: {name}"));
    Ok(snapshot(&state))
}

/// 删除存档。'No Profile' 不可删（返回错误提示）。
#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    let before = cache_scope(&pm); // 删的是当前激活档时会连带切回 No Profile → 作用域变化
    let was_current = pm.cur_prf == name; // 删的是当前激活档时，delete_profile 会自动切回 No Profile
    if !pm.delete_profile(&name)? {
        return Err("无法删除缺省档 No Profile".into());
    }
    drop(pm); // 显式释放锁（因为后面 snapshot() 会再次加锁，不先释放会死锁）
    clear_caches_if_scope_changed(&state, before);
    let suffix = if was_current {
        "（当前激活档，已切回 No Profile）"
    } else {
        ""
    };
    state.logger.info("profile", format!("删除 Profile: {name}{suffix}"));
    Ok(snapshot(&state))
}

/// 切换存档（含快照存取：保存旧档各组 config.xml、恢复新档各组快照）。
#[tauri::command]
pub fn switch_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    let before = cache_scope(&pm); // 切档＝换了一套名单/映射：改前记下作用域，改完作废缓存
    pm.load_profile(&name)?;
    // 切档后把全局配置 profiles.json 也落盘（记住新的激活位）。
    crate::json_store::save(&pm.user_cfg_path().join("profiles.json"), &pm.config)?;
    drop(pm);
    clear_caches_if_scope_changed(&state, before);
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
    let before = cache_scope(&pm); // 当前组会变（新组）
    pm.ensure_group(&path).ok_or("目录不可用")?;
    drop(pm);
    clear_caches_if_scope_changed(&state, before);
    Ok(snapshot(&state))
}

/// 设置当前组的 RTF 名单路径。
#[tauri::command]
pub fn set_current_rtf(state: State<'_, AppState>, path: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    let before = cache_scope(&pm); // 名单路径属作用域：换名单后旧解析结果不再对应当前组
    if !pm.set_cur_rtf(&path)? {
        return Err("请先选择头像包目录（当前无组）".into());
    }
    drop(pm);
    clear_caches_if_scope_changed(&state, before);
    Ok(snapshot(&state))
}

/// 切换当前头像包（组）。
#[tauri::command]
pub fn set_current_group(state: State<'_, AppState>, img_dir: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    let before = cache_scope(&pm); // 换组：名单/映射缓存都属上一个组
    if !pm.set_cur_group(&img_dir)? {
        return Err("组不存在".into());
    }
    drop(pm);
    clear_caches_if_scope_changed(&state, before);
    Ok(snapshot(&state))
}

/// 删除组（同时删掉其快照）。
#[tauri::command]
pub fn delete_group(state: State<'_, AppState>, img_dir: String) -> Result<AppSnapshot, String> {
    let mut pm = state.pm.lock().unwrap();
    let before = cache_scope(&pm); // 删当前组会使当前组回退到剩下的组 → 作用域变化
    if !pm.delete_group(&img_dir)? {
        return Err("组不存在".into());
    }
    drop(pm);
    clear_caches_if_scope_changed(&state, before);
    Ok(snapshot(&state))
}

/// 校验头像包目录，返回缺失的种族子目录名（前端确认后批量创建）。不访问全局状态，无需 State。
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
    FACEPACK_DIRS.iter().filter(|d| !existing.contains(**d)).map(|d| d.to_string()).collect()
}

/// 「检查完整性」：按规则配置的 FolderList 识别头像包归属哪个标准，返回可补全的缺失目录与多余目录。
/// 判定逻辑在 facepack_scanner::check_integrity_progress。命令层只负责加载规则配置、把结果回传前端（IPC），并把核对进度发给状态栏。
/// 采用 async 是因为平铺包顶层条目可达数万，同步执行会把界面重绘与进度事件一起堵住。
#[tauri::command]
pub async fn check_integrity(
    app: AppHandle,
    state: State<'_, AppState>,
    img_dir: String,
) -> Result<IntegrityResult, String> {
    let config = RuleConfig::load(&state.data_dir)?;
    // 进度以「头像包顶层条目」为单位（先列目录拿到分母再逐条核对），状态栏显示 n/total。
    let mut progress = TaskProgress::new(&app, "检查完整性");
    let result = facepack_scanner::check_integrity_progress(&img_dir, &config.folder_lists(), &mut |done, total| {
        progress.report(done, total)
    });
    state.logger.info(
        "scan",
        format!(
            "检查完整性完成：{}（缺失 {} 个、多余 {} 个目录）",
            layout_label(result.standard, &config),
            result.missing.len(),
            result.extra.len()
        ),
    );
    progress.finish();
    Ok(result)
}

/// 批量创建缺失目录（前端确认后调用）。
#[tauri::command]
pub fn create_missing_dirs(img_dir: String, names: Vec<String>) -> Result<(), String> {
    let count = names.len();
    for name in names {
        std::fs::create_dir_all(Path::new(&img_dir).join(&name)).map_err(|e| format!("创建目录 {name} 失败: {e}"))?;
    }
    log::info!(target: "scan", "已创建缺失目录 {count} 个");
    Ok(())
}

// ---------------- 扫描（「扫描头像 / 扫描RTF」按钮） ----------------

/// 标准号的中文标注（写日志用）。目录数取自规则配置的 FolderList，不在这里硬编码。
fn layout_label(standard: u8, config: &RuleConfig) -> String {
    let dirs = |n: u8| config.folder_lists().into_iter().find(|(t, _)| *t == n).map(|(_, f)| f.len()).unwrap_or(0);
    match standard {
        1 => "标准一/图片直铺".to_string(),
        2 => format!("标准二/{} 目录", dirs(2)),
        3 => format!("标准三/{} 目录", dirs(3)),
        _ => "无法识别".to_string(),
    }
}

/// 「扫描RTF」核心：格式识别 + 全量解析名单（全字段入库），返回 (摘要, 记录列表, 行数账目)。
/// 识别失败 → Err("RTF 格式不合法：…")，文案带首个不合法字段定位，前端弹窗直接展示。
/// 第三个返回值（保留/过滤/跳过/非数据行）只给调用方的 INFO 摘要用：本工具的职责是
/// 「N 个人 → N 张脸」，行消失了必须在日志里看得见，否则用户以为名单本来就只有这么短。
fn scan_rtf_core(
    rtf_path: &str,
    logger: &impl LogSink,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(RosterSummary, Vec<PlayerRecord>, RosterCounts), String> {
    let mut scanner = RtfScanner::new();
    scanner.detect_format(Path::new(rtf_path), logger)?;
    // filter=false：展示全量名单（替换流程才按「仅随机人」过滤）；按钮无取消入口，故传恒 false 的标志。
    let (data, counts) =
        scanner.parse_roster_progress(Path::new(rtf_path), false, logger, &AtomicBool::new(false), progress)?;
    // 摘要口径统一收在 RosterSummary（随机人前缀判定、拓展信息判定都不在这里重复一遍）。
    Ok((RosterSummary::from_records(&scanner, &data), data, counts))
}

/// 「扫描头像」：扫描当前组头像包并缓存索引（供替换复用），结果写日志。采用 async 避免阻塞界面。
#[tauri::command]
pub async fn scan_images(app: AppHandle, state: State<'_, AppState>) -> Result<ImageScanResult, String> {
    let img_dir = { state.pm.lock().unwrap().cur_group.clone().ok_or("请先选择头像包目录")? };
    let config = RuleConfig::load(&state.data_dir)?;

    // 进度以「一级子目录」为单位（见 scan_pack_progress），扫完一个大包能明显看到进度在走。
    let mut progress = TaskProgress::new(&app, "扫描头像");
    let t = Instant::now(); // 扫盘是替换流水线的最大单项开销，单独记一条供对比
    let (total, standard, scan) =
        facepack_scanner::scan_and_total_progress(&img_dir, &config.folder_lists(), &mut |done, total| {
            progress.report(done, total)
        })?;
    let scan_elapsed = t.elapsed();
    // 目录签名随索引一起存：替换复用前先比对，包内文件被增删时重扫，避免拿过期索引写错映射。
    // 计时与扫盘分开记：pack_sig 要逐个一级目录数一遍（标准三 28 万图约 0.2 s，与扫盘同量级）。
    let t_sig = Instant::now();
    let sig = facepack_scanner::pack_sig(&img_dir);
    let sig_elapsed = t_sig.elapsed();
    // Arc 包一层入缓存：复用时只涨引用计数，不复制整份索引。
    *state.faces_cache.lock().unwrap() = Some(FaceIndex {
        img_dir,
        scan: Arc::new(scan),
        sig,
    });
    state.logger.info(
        "scan",
        format!(
            "扫描头像完成：{total} 张图片（{}），扫盘耗时 {:.1?}，目录签名 {:.1?}",
            layout_label(standard, &config),
            scan_elapsed,
            sig_elapsed
        ),
    );
    progress.finish();
    Ok(ImageScanResult { total, standard })
}

/// 「扫描RTF」：解析当前组 RTF 并填充名单预览缓存，结果写日志。采用 async 避免阻塞界面。
#[tauri::command]
pub async fn scan_rtf(app: AppHandle, state: State<'_, AppState>) -> Result<RosterSummary, String> {
    let rtf = { state.pm.lock().unwrap().get_cur_rtf() };
    if rtf.is_empty() {
        return Err("请先选择 RTF 名单".into());
    }
    // 进度以「数据行」为单位（解析前先数一遍行数拿到分母，见 parse_roster_progress）。
    let mut progress = TaskProgress::new(&app, "扫描RTF");
    let t = Instant::now(); // 解析耗时（识别 + 全量解析），与替换流水线同一段同口径
    let (summary, data, counts) = scan_rtf_core(&rtf, &*state.logger, &mut |done, total| progress.report(done, total))?;
    *state.rtf_cache.lock().unwrap() = data; // 名单预览（query_players）直接读这份缓存（全字段）
    state.logger.info(
        "scan",
        format!(
            "扫描 RTF 完成：{} 条（随机人 {}，{}，{}）{}，耗时 {:.1?}",
            summary.total,
            summary.newgan,
            summary.format,
            summary.language,
            counts.log_tail(),
            t.elapsed()
        ),
    );
    progress.finish();
    Ok(summary)
}

/// 用Windows资源管理器打开目录。组 key 经 norm_dir 带尾部分隔符，而 explorer 对引号内的尾反斜杠解析脆弱
/// （含空格的路径会打不开正确位置），故传入前先去掉尾部分隔符。
fn explorer_open(dir: &Path, what: &str) -> Result<(), String> {
    let s = dir.to_string_lossy();
    let trimmed = s.trim_end_matches(['\\', '/']);
    // 盘根（`C:\`）去尾后是 `C:`（表示「该盘当前目录」而非根），需保留原样；裸分隔符同理。
    let target = if trimmed.is_empty() || trimmed.ends_with(':') {
        s.as_ref()
    } else {
        trimmed
    };
    std::process::Command::new("explorer").arg(target).spawn().map_err(|e| format!("打开{what}失败: {e}"))?;
    Ok(())
}

/// 打开标准规则目录（data/rules/）供用户编辑；缺失时先导出内置默认。
/// 编辑保存后下次替换/检查完整性即生效。
#[tauri::command]
pub fn open_rules_dir(state: State<'_, AppState>) -> Result<(), String> {
    templates::ensure_user_files(&state.data_dir)?;
    let dir = state.data_dir.join("rules");
    explorer_open(&dir, "规则目录")
}

/// 「正在替换」标志的 RAII 守卫：Drop 时复位，让「校验提前 return」「线程正常收尾」「线程 panic」 三条路径共用同一出口。
/// 只靠 store(false) 不行：复位点与线程末尾相隔很远，中间任何一次 panic 都会跳过复位：running 永久为 true，导致界面全局禁用且无法再发起替换。
struct RunningGuard(Arc<AtomicBool>);

impl RunningGuard {
    /// 主动复位。**两处补发 `replace:finished` 的调用方都必须先调它、再 emit。**
    /// 【为什么不能只靠 Drop】Drop 要等线程闭包结束，而它前面那句 `app.emit` 是把事件投递给主线程
    /// 再等主线程接手；主线程那一刻若正忙在别的事情上（例如 tauri-plugin-dialog 那个原生弹窗的嵌套
    /// 消息循环），这次 emit 就会把本线程按在原地震不动——running 跟着一起被按住，界面持续全局禁用。
    /// 复位挪到 emit 之前，emit 卡多久都不再影响解锁。
    /// store(false) 幂等，Drop 再走一遍无害，于是 panic、参数校验失败这些没走到 release 的路径仍然由 Drop 兜住。
    fn release(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.release();
    }
}

/// 后台替换线程的入参：收成结构体是为了让线程体能被 catch_unwind 包住，而 panic 后还需句柄发补救事件。
struct ReplaceCtx {
    app: AppHandle,
    logger: Arc<Logger>,
    cancel: Arc<AtomicBool>,
    /// 唯一的 running 复位出口：线程函数返回（正常或 unwind）时 Drop。
    _running: RunningGuard,
    rtf_cache: Arc<Mutex<Vec<PlayerRecord>>>,
    mapping_cache: Arc<Mutex<Vec<MappingEntry>>>,
    faces_cache: Arc<Mutex<Option<FaceIndex>>>,
    snapshot_cache: Arc<Mutex<Option<SnapshotCache>>>,
    snapshot_path: Option<PathBuf>,
    params: ReplaceParams,
}

/// 缓存被 panic 毒化时才作废（未中毒的保留原样）。
/// 先判中毒：`replace_flow::run` 不持锁，多数 panic 不会毒化谁，顺手清缓存会把已载入的名单一起丢掉。
/// 必须 clear_poison：中毒会永久粘在 Mutex 上，之后每次 lock().unwrap() 都 panic ，等于界面解禁了但任何命令一调就炸。
fn wipe_if_poisoned<T: Default>(cell: &Mutex<T>) -> bool {
    if !cell.is_poisoned() {
        return false;
    }
    *cell.lock().unwrap_or_else(|e| e.into_inner()) = T::default();
    cell.clear_poison();
    true
}

impl ReplaceCtx {
    /// 线程正常路径：比对目录签名决定复用还是现扫 → 跑流水线 → 收尾写缓存/同步快照 → 发完成事件。
    fn body(&mut self) {
        // 签名比对挪进线程：pack_sig 要逐个一级目录数一遍（标准三的 28 万图约 0.2 s），
        // 留在同步命令里会让点击「替换」后先有一段无响应（约 0.2 s）。
        let t_sig = Instant::now();
        let want_sig = facepack_scanner::pack_sig(&self.params.img_dir);
        let reuse = self
            .faces_cache
            .lock()
            .unwrap()
            .as_ref()
            .filter(|c| c.img_dir == self.params.img_dir)
            .filter(|c| c.sig.is_some() && c.sig == want_sig)
            .map(|c| c.scan.clone());
        self.logger.info(
            "replace",
            format!(
                "头像包索引缓存：{}，目录签名比对耗时 {:.1?}",
                if reuse.is_some() {
                    "命中（复用，不扫盘）"
                } else {
                    "未命中（将现场扫盘）"
                },
                t_sig.elapsed()
            ),
        );
        self.params.pre_scan = reuse;
        // 跑流水线；progress 回调里通过 emit 把进度发给前端（事件名 replace:progress）。
        // 流水线关键步骤的日志经 logger 记录，由 log:batch 事件实时回显前端日志面板。
        // 阶段内部按项上报（逐球员抽图可达数万次），去重后才 emit——见 ProgressDedup。
        let emit_app = self.app.clone();
        let mut dedup = ProgressDedup { last: None };
        let result = replace_flow::run(
            &self.params,
            &mut |status: &str, value: u8, current: usize, total: usize| {
                if !dedup.should_emit(status, value) {
                    return;
                }
                let _ = emit_app.emit(
                    "replace:progress",
                    ProgressPayload {
                        status: status.into(),
                        value,
                        current,
                        total,
                    },
                );
            },
            &self.cancel,
            &*self.logger,
        );
        // 【收尾计时】下面这段（把两份缓存写回去 + 同步存档快照 + 发完成事件）全在 run() 返回之后，
        // 不属于流水线自己的「总耗时」，却卡着 running 复位（界面全局禁用）——必须单独计时才看得见。
        let t_tail = Instant::now();
        // 把解析结果与映射写入缓存（供后续预览/分页查询使用）。
        // rtf_data / mapping 后面不再需要整份，直接移动（整表 clone 约 0.1 s）；mapping 留一份给快照直填。
        let mapping = result.mapping;
        let mapping_empty = mapping.is_empty();
        // 【失败/取消 不动这两份缓存】带回来的是空表，无条件写回会清掉「扫描RTF」载入的名单，
        // 而作用域根本没变（用户还得重扫并丢掉筛选、排序与页码）。
        if matches!(result.outcome, Outcome::Finished) {
            *self.rtf_cache.lock().unwrap() = result.rtf_data;
            *self.mapping_cache.lock().unwrap() = mapping.clone();
        }
        // 【索引回填】本轮现场扫盘的话，把索引连同目录签名写回缓存，下次同目录只付签名比对成本，省掉整包重扫
        // （实测：旧口径单线程 14.6–39 s，改并行 + `file_type()` 后约 0.13 s）。
        // 签名复用线程开头那份 want_sig：替换只写 config.xml 与 config_backup_*.xml，两者都不入签名，
        // 所以 want_sig 此刻仍代表磁盘现状；包被外部改动则下次签不上 → 重扫（失败方向安全）。
        // 命中缓存（fresh_scan 为 None）与失败/取消都不动缓存。
        if matches!(result.outcome, Outcome::Finished) {
            if let Some(scan) = result.scan {
                *self.faces_cache.lock().unwrap() = Some(FaceIndex {
                    img_dir: self.params.img_dir.clone(),
                    scan,
                    sig: want_sig,
                });
                self.logger.info("replace", "头像包索引已回填缓存：下次替换同一目录可免重扫");
            }
        }
        // 快照预览数据源也在此刷新：成功则把刚写好的 config.xml 备份到该档快照位置并直填本次映射，
        // 无需再点「读取快照」；失败/取消不改 config.xml，已读的那份仍有效（只写不清）。
        if matches!(result.outcome, Outcome::Finished) {
            if let Some(snap_path) = self.snapshot_path.as_deref() {
                sync_group_snapshot(snap_path, &self.params.img_dir, &mapping, &self.snapshot_cache, &*self.logger);
            }
        }
        if matches!(result.outcome, Outcome::Finished) && mapping_empty {
            // 跑完却一条映射也没有：config.xml 会被写成空表，必须留 WARNING
            self.logger.warn("replace", "映射结果为空：本次写入的 config.xml 不含任何映射（多为「仅随机人」过滤掉了全部名单，或头像包内无图片）");
        }
        // 名单摘要随事件回传，前端不必再点「扫描RTF」；失败/取消为 None。
        let roster = result.roster;
        // 把结果映射为前端认的字符串；失败时额外记一条 ERROR 日志。
        let result_str = match result.outcome {
            Outcome::Finished => "finished",
            Outcome::Cancelled => "cancelled",
            Outcome::Failed(ref m) => {
                self.logger.error("replace", m.clone());
                "failed"
            }
        };
        // 收尾耗时（写两份缓存 + 同步快照）：这段在流水线「总耗时」之外，但它是 running 复位（按钮解禁）之前最后一段阻塞，单独记一条方便对比。
        self.logger.info("replace", format!("收尾完成：耗时 {:.1?}（写缓存 + 快照同步）", t_tail.elapsed()));
        // 先复位再 emit：因为 emit 可能被主线程按住，而界面解锁只认这次 finished 事件。
        self._running.release();
        let _ = self.app.emit("replace:finished", serde_json::json!({ "result": result_str, "roster": roster }));
    }

    /// 线程 panic 后的补救：留 ERROR、作废被毒化的缓存、补发 finished{failed} 让前端解锁。
    /// 进度条会停在最后百分比而 replacing 已复位，不发事件前端就没机会把收尾态改成「失败」。
    fn on_panic(&self) {
        // 先复位：下面的日志写入与 emit 都可能卡在主线程序列上。
        self._running.release();
        let mut wiped: Vec<&str> = Vec::new();
        if wipe_if_poisoned(&self.rtf_cache) {
            wiped.push("名单");
        }
        if wipe_if_poisoned(&self.mapping_cache) {
            wiped.push("映射");
        }
        if wipe_if_poisoned(&self.faces_cache) {
            wiped.push("头像包索引");
        }
        if wipe_if_poisoned(&self.snapshot_cache) {
            wiped.push("快照");
        }
        self.logger.error(
            "replace",
            format!(
                "替换线程发生 panic，本轮中止{}。请查看日志文件里的 panic 现场后重试。",
                if wiped.is_empty() {
                    "（共享缓存未受影响，保留原样）".to_string()
                } else {
                    format!("：已作废被毒化的缓存——{}", wiped.join("、"))
                },
            ),
        );
        let _ = self.app.emit("replace:finished", serde_json::json!({ "result": "failed", "roster": null }));
    }
}

/// 发起替换前的预检：本次会不会把本档既有映射清空，需要则让前端先确认。
///
/// 【只有 Generate 可能】Preserve 携带全部既有条目、Overwrite 保留名单外的球员，两者写出的文件
/// 都不可能比档景更空；「生成」从零起算，名单被筛空／包里没有图时就是一轮清空。非 Generate 因此
/// 连档景都不读，直接返回 needs_confirm=false。
/// 【为什么问在发起前，而不是等流水线报错】流水线的写前拦截（replace_flow::run_body）只会中止，
/// 于是「我就是要把这个 config.xml 清空」这个合法意图在应用里没有通路（绕过守卫只剩手工删档景文件）。
/// 预检把选择权前置：确认过的这一轮带 allow_blank 发起，那条拦截改为记 WARN 后照写。
/// 【条数优先取快照缓存】命中即免解析；未命中才整表解析（15 万行实测 1.4–1.5 s），并按条数上报
/// 进度。所以本命令是 async —— 同步命令跑在主线程上，会把界面重绘和 task:progress 一起堵住。
#[tauri::command]
pub async fn preflight_replace(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: String,
) -> Result<PreflightResult, String> {
    let none = || PreflightResult {
        needs_confirm: false,
        existing: 0,
    };
    if Mode::parse(&mode)? != Mode::Generate {
        return Ok(none());
    }
    // 判据源与 start_replace 完全一致：本档本组的档景路径（同一个 group_xml_path）。
    let snapshot_path = {
        let pm = state.pm.lock().unwrap();
        pm.cur_group.as_ref().map(|g| crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, g))
    };
    // 还没有档景 = 本档在这个包从没换过脸，生成覆盖不掉任何东西。
    let Some(path) = snapshot_path.filter(|p| p.is_file()) else {
        return Ok(none());
    };
    // 缓存里正是这份档景（用户点过「读取快照」，或刚替换完自动同步）时条数白拿。
    // 锁在这里就释放：下面那条解析要跑上秒级，抱着锁会把替换收尾的同一把锁一起堵住。
    let cached = {
        let cache = state.snapshot_cache.lock().unwrap();
        cache.as_ref().filter(|c| c.xml_path == path.to_string_lossy()).map(|c| c.entries.len())
    };
    let existing = match cached {
        Some(n) => n,
        _ => {
            // 解析失败（档景读不出）直接 Err：预检的意义是「动手前把要丢多少说清楚」，
            // 说不清就不发起，而不是把「读不到」当成 0 条照常覆盖。
            let mut progress = TaskProgress::new(&app, "检查既有映射");
            let entries =
                xml_parser::parse_xml_entries_progress(&path, &mut |done, total| progress.report(done, total))?;
            progress.finish();
            entries.len()
        }
    };
    Ok(PreflightResult {
        needs_confirm: existing > 0,
        existing,
    })
}

/// 启动批量替换：校验参数后开一个后台线程跑流水线，避免阻塞界面。
/// app 用于在线程里向前端 emit 事件；args 是前端传来的参数。
#[tauri::command]
pub fn start_replace(app: AppHandle, state: State<'_, AppState>, args: ReplaceArgs) -> Result<(), String> {
    // swap(true) 原子地把 running 置 true 并返回旧值；旧值已是 true 说明有任务在跑，拒绝重入。
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("已有替换任务在执行".into());
    }
    // 从这里起下面的每条提前 return、以及后台线程的整个生命周期，都由这个守卫负责复位。
    let running = RunningGuard(Arc::clone(&state.running));
    state.cancel.store(false, Ordering::SeqCst);
    // 先在锁内取出当前 RTF 和组目录，随即释锁（不能把 Mutex 带进新线程）。
    // 顺带取该档当前组的存档快照路径：替换成功后要把新 config.xml 同步到这份快照位置
    // （快照预览的数据源，与「读取快照」按钮读的是同一路径）。
    let (rtf, img_dir, snapshot_path) = {
        let pm = state.pm.lock().unwrap();
        let snap = pm.cur_group.as_ref().map(|g| crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, g));
        (pm.get_cur_rtf(), pm.cur_group.clone(), snap)
    };
    // 下面每条校验失败路径直接 return：running 由上面的守卫在 drop 时复位。
    let Some(img_dir) = img_dir else {
        return Err("请先选择头像包目录".into());
    };
    if rtf.is_empty() {
        return Err("请先选择 RTF 名单".into());
    }
    let mode = Mode::parse(&args.mode)?;
    // 加载标准规则配置（data/rules/standardN.json，用户改坏 json → 报错中止，错误含文件路径）。
    let rule_config = RuleConfig::load(&state.data_dir)?;
    // 「扫描头像」缓存命中同一目录**且目录签名一致**时才复用其索引（避免重复扫盘）；
    // 切组即失效；签名不一致（包内图片被增删改名）也重扫——宁可慢几秒，也不拿过期索引
    // 写出指向不存在图片的映射（那类错误完全静默，事后无从发现）。
    let params = ReplaceParams {
        rtf_path: rtf,
        img_dir,
        mode,
        filter_newgen: args.filter_newgan,
        allow_duplicates: args.allow_duplicates,
        save_backup: args.save_backup,
        allow_blank: args.allow_blank,
        pre_scan: None, // 由后台线程按「缓存命中 + 目录签名」填，见下
        // 保留/覆写的判据是本档本组的存档快照，不是包里的 live config.xml（live 全档共享，会残留别档内容）。
        source_xml: snapshot_path.clone(),
        rule_config,
    };
    // 句柄交给 ctx（Arc::clone 只涨引用计数）；running 守卫一起搬进线程，返回（正常或 unwind）时复位。
    let ctx = ReplaceCtx {
        app,
        logger: state.logger.clone(),
        cancel: state.cancel.clone(),
        _running: running,
        rtf_cache: state.rtf_cache.clone(),
        mapping_cache: state.mapping_cache.clone(),
        faces_cache: state.faces_cache.clone(), // 复用前比对 + 现场扫盘后回填，都在这份缓存上
        snapshot_cache: state.snapshot_cache.clone(), // 替换成功时同步刷新（快照预览的数据源）
        snapshot_path,
        params,
    };
    std::thread::spawn(move || {
        let mut ctx = ctx;
        // 【为什么要 catch_unwind】running 已由守卫复位，但 panic 若不接住就没人发 replace:finished，
        // 前端进度条会永远停在最后上报的那个百分比上（界面解锁与「失败」提示是两件事）。
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctx.body())).is_err() {
            ctx.on_panic();
        }
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
    // 手动「单人微调」只改已有映射行；没有该 UID 就报未命中，不擅自给包加行。
    replace_single_face_impl(&state, uid, image_path, save_backup, false)
}

/// 单人换脸主体。`insert_missing=true`（仅「保存并替换」传）时，映射表没有该 UID 就追加一条新记录。
fn replace_single_face_impl(
    state: &State<'_, AppState>,
    uid: String,
    image_path: String,
    save_backup: bool,
    insert_missing: bool,
) -> Result<bool, String> {
    let uid = uid.trim().to_string();
    // 【与批量替换互斥】两边都写同一个 live config.xml，并发必有一方被无声吞掉——这里的拒绝是唯一能让用户看见的机会。
    if state.running.load(Ordering::SeqCst) {
        state.logger.warn("viewer", "单人换脸被拒绝：批量替换正在进行");
        return Err("批量替换正在进行，请等它完成或先取消，再做单人微调".into());
    }
    // 先取当前组目录（parse_image_path_in_pack 需要它来判定"图片是否在包根"）。
    let img_dir = {
        let pm = state.pm.lock().unwrap();
        pm.cur_group.clone().ok_or("当前没有选中组")?
    };
    let Some((pack, image)) = xml_parser::parse_image_path_in_pack(&img_dir, &image_path) else {
        return Err("所选图片不在当前头像包内（或文件不存在），请从当前头像包目录或其子目录中选择图片".into());
    };
    // 【先归位再改】live config.xml 全档共享，切档 restore 有静默跳过分支，跳过时 live 可能还是别档内容；
    // 先按本档快照把 live 恢复一遍，这行替换才落在本档现有映射上。
    // 快照不存在＝本档在此目录没有历史，live 归谁无从判断，宁可不改（报错让用户先跑替换）。
    let snap = {
        let pm = state.pm.lock().unwrap();
        crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, &img_dir)
    };
    if !snap.is_file() {
        state.logger.warn("viewer", "单人换脸已跳过：当前档在此头像包目录下还没有存档快照");
        return Err("当前存档在此头像包目录下还没有存档快照，无法确定本档的现有映射。请先执行一次替换后再微调。".into());
    }
    // 【原子恢复】目标是用户头像包里的 config.xml，直接 copy 是截断覆写：中途失败会让整包无脸，
    // 而这里的失败本就只 WARN 后继续（见下），更不该把用户文件写成半截。
    if let Err(e) = crate::json_store::copy_atomic(&snap, &Path::new(&img_dir).join("config.xml")) {
        // 恢复失败不致命，退化为在 live 现况上改，但必须留痕。
        state.logger.warn("viewer", format!("单人换脸前恢复本档现有映射失败，将按头像包当前内容继续：{e}"));
    }
    let hit = xml_parser::single_replacement(&uid, &pack, &image, Path::new(&img_dir), save_backup, insert_missing)?;
    if hit {
        // 同步内存映射缓存，保证预览与实际一致。
        let mut cache = state.mapping_cache.lock().unwrap();
        match cache.iter_mut().find(|m| m.uid == uid) {
            Some(m) => {
                m.ethnicity = pack.clone(); // 平铺时为空串，正常
                m.image = image.clone();
            }
            // 新增行：缓存里也没有，补进去；否则预览还要等下一次整表解析才认。
            None => cache.push(MappingEntry {
                uid: uid.clone(),
                ethnicity: pack.clone(),
                image: image.clone(),
            }),
        }
        drop(cache); // 下面要锁 snapshot_cache，先放开这把，避免无谓的嵌套持锁
                     // 【微调写回本档快照】快照才是本档本组的现有映射，替换的保留/覆写判据取自它；
                     // 不写回的话下一轮替换仍按微调前的快照判断，刚改的这行会被当成新球员重抽。
        sync_live_to_snapshot(&state, &snap, &img_dir, &uid, &pack, &image);
    }
    if hit {
        state.logger.info("viewer", format!("单人换脸 {uid} → {}", xml_parser::record_from(&pack, &image)));
    }
    // 未命中时 single_replacement 已经 WARN 过「文件未改动」，这里不能再报成功。
    Ok(hit)
}

/// 单人微调后把 live config.xml 同步到 `snap`（本档该组快照），并就地更新快照预览缓存那一条。
/// `snap` 必须来自 paths::group_xml_path（与快照预览、收尾同步同一口径）。
fn sync_live_to_snapshot(state: &State<'_, AppState>, snap: &Path, img_dir: &str, uid: &str, pack: &str, image: &str) {
    // 【原子写档景】快照是 Preserve/Overwrite 的判据源，截断写坏了它＝下次替换按半截映射判断；
    // 覆盖前同样先轮转备份旧的那一份（见 xml_parser::overwrite_snapshot）。
    match xml_parser::overwrite_snapshot(&Path::new(img_dir).join("config.xml"), snap) {
        Ok(_) => {
            // 预览缓存按路径命中：路径一致时就地改这一条（条目顺序不变，其余字段原样）。
            // 路径不符/缓存为空则不动——查询侧发现路径不匹配会按请求路径重读兜底，不会展示错数据。
            let snap_str = snap.to_string_lossy().to_string();
            let mut cache = state.snapshot_cache.lock().unwrap();
            if let Some(c) = cache.as_mut() {
                if c.xml_path == snap_str {
                    match c.entries.iter_mut().find(|e| e.uid == uid) {
                        Some(e) => {
                            e.ethnicity = pack.to_string();
                            e.image = image.to_string();
                        }
                        // 新增行：快照缓存里也没有，补进去（快照文件本身已整份覆盖为 live）。
                        None => c.entries.push(MappingEntry {
                            uid: uid.to_string(),
                            ethnicity: pack.to_string(),
                            image: image.to_string(),
                        }),
                    }
                }
            }
        }
        Err(e) => state.logger.warn("viewer", format!("单人换脸同步存档快照失败：{e}")),
    }
}

/// 单人重掷：把抽图阶段单独跑在这 1 名球员上，重新抽图覆盖其现有映射行，其余行不动。
/// 语义固定为「重抽」，不跟随替换面板的模式/选项：面板默认「保留」，对已有映射的球员重跑等于空操作，
/// 「重掷」会失去撤销手动指定的意义（过滤也会把按钮变成静默空操作）。
#[tauri::command]
pub fn reset_single_face(state: State<'_, AppState>, uid: String, save_backup: bool) -> Result<bool, String> {
    let uid = uid.trim().to_string();
    // 【与批量替换互斥】同 replace_single_face：两边写的是同一个 live config.xml，并发必有一方被吞。
    if state.running.load(Ordering::SeqCst) {
        state.logger.warn("viewer", "单人重掷被拒绝：批量替换正在进行");
        return Err("批量替换正在进行，请等它完成或先取消，再做单人微调".into());
    }
    let img_dir = {
        let pm = state.pm.lock().unwrap();
        pm.cur_group.clone().ok_or("当前没有选中组")?
    };
    // 判据与归位同 replace_single_face：先按本档快照恢复 live 再抽图；快照不存在则让用户先跑一次替换。
    let snap = {
        let pm = state.pm.lock().unwrap();
        crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, &img_dir)
    };
    if !snap.is_file() {
        state.logger.warn("viewer", "单人重掷已跳过：当前档在此头像包目录下还没有存档快照");
        return Err("当前存档在此头像包目录下还没有存档快照，无法确定本档的现有映射。请先执行一次替换后再微调。".into());
    }
    if let Err(e) = crate::json_store::copy_atomic(&snap, &Path::new(&img_dir).join("config.xml")) {
        state.logger.warn("viewer", format!("单人重掷前恢复本档现有映射失败，将按头像包当前内容继续：{e}"));
    }
    // 抽图对象是名单记录（规则检索靠国籍/种族码），从名单缓存取；两种 UID 形态都试。
    let record = {
        let variants = xml_parser::with_newgan_prefix_variants(&uid);
        let cache = state.rtf_cache.lock().unwrap();
        cache.iter().find(|p| variants.contains(&p.uid)).cloned()
    };
    let Some(record) = record else {
        return Err("名单缓存中找不到该球员，请先扫描RTF名单（或重新执行一次替换）后再重掷".into());
    };
    let rule_config = RuleConfig::load(&state.data_dir)?;
    // 包索引按目录 + 签名命中即复用；未命中现场扫盘并回填（与替换流水线同口径）。
    let want_sig = facepack_scanner::pack_sig(&img_dir);
    let scan = {
        let hit = state
            .faces_cache
            .lock()
            .unwrap()
            .as_ref()
            .filter(|c| c.img_dir == img_dir)
            .filter(|c| c.sig.is_some() && c.sig == want_sig)
            .map(|c| c.scan.clone());
        match hit {
            Some(scan) => scan,
            None => {
                let scan =
                    Arc::new(facepack_scanner::scan_pack_progress(&img_dir, &mut |_, _| {}, &AtomicBool::new(false))?);
                *state.faces_cache.lock().unwrap() = Some(FaceIndex {
                    img_dir: img_dir.clone(),
                    scan: scan.clone(),
                    sig: want_sig,
                });
                scan
            }
        }
    };
    let mut mapper = FaceMapper::from_scan(&scan, &rule_config, Some(snap.clone()))?;
    // 标准规则清单外（未实现抽图规则）与批量替换同口径拦截。
    if let facepack_scanner::PackLayout::Standard { standard, .. } = mapper.layout {
        if !templates::IMPLEMENTED_STANDARDS.contains(&standard) {
            return Err(format!("标准{standard}的抽图规则暂未实现，无法重掷该球员"));
        }
    }
    let mapping = mapper
        .generate_mapping(
            std::slice::from_ref(&record),
            Mode::Generate,          // 固定重抽：无视该球员的现有映射，从零抽一张
            true,                    // 单人视角没有同轮球员占用图片，复用恒放开
            &AtomicBool::new(false), // 单人重抽没有取消入口，恒不取消
            &mut |_, _, _| {},
            &*state.logger,
        )
        .ok_or("单人重掷被取消")?; // cancel 恒为 false，实际走不到
    let Some(entry) = mapping.first() else {
        // 1 人名单映射为空＝没抽到图（多为包内无可用图片），facemap 已留痕。
        return Err("未能为该球员重新抽到图（详见日志）".into());
    };
    // 写入复用单人换脸的逐行改写（只动该 UID 那一行）；未命中（live 里没有他的行）原样返回 false。
    let hit = xml_parser::single_replacement(
        &entry.uid,
        &entry.ethnicity,
        &entry.image,
        Path::new(&img_dir),
        save_backup,
        false,
    )?;
    if hit {
        // 与 replace_single_face 同一套收尾：内存映射缓存就地改 + 写回本档快照。
        let mut cache = state.mapping_cache.lock().unwrap();
        for m in cache.iter_mut() {
            if m.uid == entry.uid {
                m.ethnicity = entry.ethnicity.clone();
                m.image = entry.image.clone();
                break;
            }
        }
        drop(cache);
        sync_live_to_snapshot(&state, &snap, &img_dir, &entry.uid, &entry.ethnicity, &entry.image);
        state.logger.info(
            "viewer",
            format!("单人重掷 {} → {}", entry.uid, xml_parser::record_from(&entry.ethnicity, &entry.image)),
        );
    }
    Ok(hit)
}

/// 单人预览：按优先级拼出 ① 映射缓存 → ② 本档本组快照（缓存命中同路径免整表解析）→ ③ 包内实际图片 → ④ 名单资料。
/// 读快照而非 live config.xml：替换判据同源，避免拿别档的映射显示本档抽图结果；快照不存在则保持 None。
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
    // 2) 缓存未命中就读本档本组快照——与替换判据同源；不读 live config.xml（全档共享，可能残留别档内容）。
    if mapping.is_none() {
        if let Some(g) = &pm.cur_group {
            let snap = crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, g);
            let snap_str = snap.to_string_lossy().to_string();
            // 快照预览缓存命中同一路径时直接查（大快照整表解析要 1 s 以上，同步命令能省则省）；否则读文件兜底。
            mapping = {
                let cache = state.snapshot_cache.lock().unwrap();
                cache
                    .as_ref()
                    .filter(|c| c.xml_path == snap_str)
                    .and_then(|c| c.entries.iter().find(|e| variants.contains(&e.uid)).cloned())
            };
            if mapping.is_none() && snap.is_file() {
                if let Ok(xml) = xml_parser::parse_xml(&snap) {
                    mapping = variants.iter().find_map(|u| xml.get(u).cloned());
                }
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

/// 名单预览「头像」列：按一页 UID 批量取头像绝对路径（一次 IPC）。
/// 查找口径与单人预览同源：映射缓存 → 本档本组快照（缓存命中同路径免解析）→ 读快照兜底 → 包内实际文件。
/// 【为什么是批量】：名单「头像」列原文导出恒为空，只能按 UID 反查；逐行调用还会把整表解析重复多遍。
/// 映射缺失/图片不在包内/未选组 → null。未命中缓存时会读整表（秒级），故为 async；函数内不跨 await 持锁。
#[tauri::command]
pub async fn query_avatars(
    state: State<'_, AppState>,
    uids: Vec<String>,
) -> Result<std::collections::HashMap<String, Option<String>>, String> {
    // 先按「全部无头像」铺底；下面只覆盖命中的（未选组、没读过快照时就是这个结果）。
    let mut out: std::collections::HashMap<String, Option<String>> = uids.iter().map(|u| (u.clone(), None)).collect();
    let Some(img_dir) = state.pm.lock().unwrap().cur_group.clone() else {
        return Ok(out); // 未选组：没有头像包可解析
    };
    // 变体 → 原始 UID：同一球员在缓存里可能带 r- 也可能不带（与单人预览同一口径）。
    let mut variants: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for uid in &uids {
        for v in xml_parser::with_newgan_prefix_variants(uid.trim()) {
            variants.entry(v).or_insert_with(|| uid.clone());
        }
    }
    // 命中表：原始 UID → 映射（三级来源依次补，先到先得，与单人预览的优先级一致）。
    let mut found: std::collections::HashMap<String, MappingEntry> = std::collections::HashMap::new();
    // 1) 最近一次解析/替换的映射缓存（一趟扫完所有候选）。
    {
        let cache = state.mapping_cache.lock().unwrap();
        for m in cache.iter() {
            if let Some(orig) = variants.get(&m.uid) {
                found.entry(orig.clone()).or_insert_with(|| m.clone());
            }
        }
    }
    if found.len() < uids.len() {
        // 2) 本档本组的存档快照：快照预览缓存里正好是这份时直接查（路径命中，免解析）。
        let snap = {
            let pm = state.pm.lock().unwrap();
            crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, &img_dir)
        };
        let snap_str = snap.to_string_lossy().to_string();
        {
            let cache = state.snapshot_cache.lock().unwrap();
            if let Some(c) = cache.as_ref().filter(|c| c.xml_path == snap_str) {
                for m in c.entries.iter() {
                    if let Some(orig) = variants.get(&m.uid) {
                        found.entry(orig.clone()).or_insert_with(|| m.clone());
                    }
                }
            }
        }
        // 3) 仍未命中且快照文件在：读一次整表兜底（整批最多解析一次，绝不按 UID 重复解析）。
        if found.len() < uids.len() && snap.is_file() {
            if let Ok(xml) = xml_parser::parse_xml(&snap) {
                for uid in &uids {
                    if found.contains_key(uid) {
                        continue;
                    }
                    if let Some(m) = xml_parser::with_newgan_prefix_variants(uid.trim()).iter().find_map(|v| xml.get(v))
                    {
                        found.insert(uid.clone(), m.clone());
                    }
                }
            }
        }
    }
    // 4) 命中的映射 → 头像包内的实际图片文件（只对当前页这几条探测扩展名）。
    let root = Path::new(&img_dir);
    for (uid, entry) in found {
        out.insert(uid, xml_parser::resolve_image_file(root, &entry.ethnicity, &entry.image));
    }
    Ok(out)
}

/// 名单预览取全量（解析一次进 rtf_cache）；分页/筛选/排序都在前端做。
#[tauri::command]
pub fn query_players(state: State<'_, AppState>) -> Vec<PlayerRecord> {
    state.rtf_cache.lock().unwrap().clone()
}

/// 按 uid 批量取球员记录（队列坞条目行的身份信息：年龄/所在地帮认人）。
/// 【为什么批量】：原先逐条调 query_players，N 条 = N 次 IPC + N×M 线扫；这里一次 IPC、一遍哈希命中。
/// 在锁内过滤、只克隆命中行（N 条），避免 clone 整份大名单；未命中的 uid 直接缺席，缓存未填时返回空表。
#[tauri::command]
pub fn query_players_by_uids(state: State<'_, AppState>, uids: Vec<String>) -> Vec<PlayerRecord> {
    let want: std::collections::HashSet<&str> = uids.iter().map(|s| s.as_str()).collect();
    state.rtf_cache.lock().unwrap().iter().filter(|r| want.contains(r.uid.as_str())).cloned().collect()
}

// ---------------- 快照预览（「读取快照」按钮 → 结果区「快照预览」Tab） ----------------

/// 「读取快照」：解析存档 XML 快照（config.xml 的存档副本）并缓存有序条目，供「快照预览」分页查询。
/// xml_path 由前端传当前展示档的快照全路径（浏览态自洽）；每次点击都重读文件，展示最新内容。大快照解析耗时，故 async。
#[tauri::command]
pub async fn read_snapshot(
    app: AppHandle,
    state: State<'_, AppState>,
    xml_path: String,
) -> Result<SnapshotInfo, String> {
    let path = PathBuf::from(&xml_path);
    if !path.is_file() {
        return Err("存档 XML 快照尚未生成（请先执行一次替换）".into());
    }
    // 进度以「映射条数」为单位（解析前先数一遍条目拿到分母，见 parse_xml_entries_progress）。
    let mut progress = TaskProgress::new(&app, "读取快照");
    let t = Instant::now(); // 快照整表重解析的耗时口径（替换收尾同步的同一段代码）
    let total = cache_snapshot(&state.snapshot_cache, &path, &mut |done, total| progress.report(done, total))?;
    state.logger.info("scan", format!("读取快照完成：{total} 条映射，耗时 {:.1?}", t.elapsed()));
    progress.finish();
    Ok(SnapshotInfo { total })
}

/// 解析快照并写入快照缓存，返回条目数。两条路径共用：「读取快照」命令与替换收尾的自动同步。
/// 缓存存的是传入路径的字符串形式，前端两处都来自同一套 group_xml_path，查询时按此比对命中。
fn cache_snapshot(
    cache: &Mutex<Option<SnapshotCache>>,
    xml_path: &Path,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<usize, String> {
    let entries = xml_parser::parse_xml_entries_progress(xml_path, progress)?;
    let total = entries.len();
    *cache.lock().unwrap() = Some(SnapshotCache {
        xml_path: xml_path.to_string_lossy().to_string(),
        entries,
    });
    Ok(total)
}

/// 替换收尾：把刚写入的 config.xml 备份到该档快照位置，并把本次映射直填进快照缓存（预览无需再点「读取快照」）。
/// 不回读的理由：write_xml 按传入顺序写、解析按文件顺序读，直填与重解析逐条相同（有单测守着），
/// 省掉一次整表解析（15 万行约 1.5 s）与「刚写完立刻读回来可能未落盘」的不确定性（NAS / 杀软实时扫描）。
/// 拷贝失败只记 WARNING：快照新旧不影响本次替换的结论（config.xml 已写好）。
fn sync_group_snapshot(
    snapshot_path: &Path,
    img_dir: &str,
    mapping: &[MappingEntry],
    cache: &Mutex<Option<SnapshotCache>>,
    logger: &impl LogSink,
) {
    let src = Path::new(img_dir).join("config.xml");
    // 计时现在只覆盖拷贝一步（原来是「拷贝 + 重解析整表」）——这条日志的数值应明显变小。
    let t_sync = Instant::now();
    if !src.is_file() {
        return; // 没有 config.xml（替换成功时必有，这里只做防御）
    }
    if let Some(p) = snapshot_path.parent() {
        if let Err(e) = std::fs::create_dir_all(p) {
            logger.write(log::Level::Warn, "replace", format!("创建快照目录失败：{e}"));
            return;
        }
    }
    // 覆盖旧档景前先轮转备份到同目录（三处覆盖点同此一条路径，见 xml_parser::overwrite_snapshot）。
    if let Err(e) = xml_parser::overwrite_snapshot(&src, snapshot_path) {
        logger.write(log::Level::Warn, "replace", format!("同步存档快照失败：{e}"));
        return;
    }
    // 直填：缓存里的映射就是刚写进快照的那份，路径与「读取快照」命令一致（同一个 group_xml_path）。
    let total = mapping.len();
    *cache.lock().unwrap() = Some(SnapshotCache {
        xml_path: snapshot_path.to_string_lossy().to_string(),
        entries: mapping.to_vec(),
    });
    logger
        .info("replace", format!("同步存档快照完成：{total} 条映射（直填，未重解析），耗时 {:.1?}", t_sync.elapsed()));
}

/// 快照预览取全量条目（uid + 头像相对路径）。imagePath 留空：文件探测按页做，
/// 前端对当前页调 query_snapshot_images 批量解析（整表几万条逐一探测文件开销太大）。
#[tauri::command]
pub fn query_snapshot(state: State<'_, AppState>, xml_path: String) -> Result<Vec<SnapshotEntry>, String> {
    let mut cache = state.snapshot_cache.lock().unwrap();
    // 正常流程「读取快照」已填好缓存且路径一致；路径不符（缓存被换走）时按请求路径重读兜底。
    let hit = cache.as_ref().map(|c| c.xml_path == xml_path).unwrap_or(false);
    if !hit {
        let path = PathBuf::from(&xml_path);
        if !path.is_file() {
            return Ok(Vec::new());
        }
        *cache = Some(SnapshotCache {
            xml_path: xml_path.clone(),
            entries: xml_parser::parse_xml_entries(&path)?,
        });
    }
    Ok(xml_parser::snapshot_entries(&cache.as_ref().unwrap().entries))
}

/// 按前端当前页批量解析快照头像的实际图片路径（与入参顺序一一对应；找不到为 null）。纯路径探测，不碰缓存。
#[tauri::command]
pub fn query_snapshot_images(img_dir: String, rel_paths: Vec<String>) -> Vec<Option<String>> {
    let root = Path::new(&img_dir);
    rel_paths.iter().map(|p| xml_parser::resolve_avatar_path(root, p)).collect()
}

// ---------------- 存档浏览预览（下拉选择存档 = 浏览，不切档） ----------------

/// 预览指定存档（仅供前端下拉浏览，不改变后端当前档/激活状态）；currentProfile 返回被浏览的存档名，激活仍需调 switch_profile。
#[tauri::command]
pub fn preview_profile(state: State<'_, AppState>, name: String) -> Result<AppSnapshot, String> {
    let pm = state.pm.lock().unwrap();
    let (prf_cfg, cur_group) = pm.preview_cfg(&name)?;
    // 浏览档的快照路径与存在性（与激活档口径一致）。
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
        current_rtf: cur_group.as_ref().and_then(|g| prf_cfg.groups.get(g)).map(|g| g.rtf.clone()).unwrap_or_default(),
        replacing: state.running.load(Ordering::SeqCst),
        config_xml,
        log_path: state.logger.path().to_string_lossy().to_string(),
    })
}

// ---------------- 目录打开（「当前任务」卡 config.xml 行：读取快照 / 查看位置） ----------------

/// 打开存档 XML 快照所在目录（「查看位置」按钮）；路径由前端传当前展示档的快照全路径（浏览态自洽）。
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

/// 按 name 查规则文件登记表（唯一白名单来源 templates::rule_files()）。
/// 不在此 match：登记表由各标准模块提供「路径 + 校验 + 写回」，命令层只查表转发，新增标准无需改本文件。
fn rule_file(name: &str) -> Result<&'static templates::RuleFile, String> {
    // 表首次调用时构造一次并常驻。
    static TABLE: std::sync::LazyLock<Vec<templates::RuleFile>> = std::sync::LazyLock::new(templates::rule_files);
    TABLE.iter().find(|f| f.name == name).ok_or_else(|| format!("未知规则文件: {name}"))
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

/// 保存标准规则文件：先按该标准结构解析校验，合法才原子写回（json_store 会 pretty 格式化）。
#[tauri::command]
pub fn save_rule_file(state: State<'_, AppState>, name: String, content: String) -> Result<(), String> {
    let rf = rule_file(&name)?;
    let path = (rf.path)(&state.data_dir);
    (rf.save)(&path, &content)?;
    state.logger.info("rules", format!("已保存标准规则：{name}"));
    Ok(())
}

/// 名单预览「头像」列预览：把列原文解析成当前头像包内的实际图片路径（找不到返回 null）。
/// 先走确定性路径探测（xml_parser::resolve_avatar_path）；裸词干在包根没命中时，
/// 再借「扫描头像」的递归索引在全部子/孙目录里找同名图（缓存缺失/切组则不做递归，避免现场扫盘卡顿）。
#[tauri::command]
pub fn resolve_avatar(state: State<'_, AppState>, avatar: String) -> Result<Option<String>, String> {
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

/// 读取预览图片字节：WebView 无权读本地磁盘，故由 Rust 读出并以二进制响应（ArrayBuffer）返回，前端转 blob URL 显示。
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
        let (summary, data, _) = scan_rtf_core(rtf.to_str().unwrap(), &logger, &mut |_, _| {}).unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.newgan, 2); // 两条 UID 均带 r- 前缀
        assert!(!summary.detailed); // 基础名单无拓展信息
        assert_eq!(summary.format, "基础名单");
        assert_eq!(summary.language, "English");
        assert_eq!(data.len(), 2);
        // 非法 RTF → Err，文案带「RTF 格式不合法」
        let bad = dir.path().join("bad.rtf");
        std::fs::write(&bad, "乱七八糟没有数据行").unwrap();
        let err = scan_rtf_core(bad.to_str().unwrap(), &logger, &mut |_, _| {}).err().unwrap();
        assert!(err.contains("RTF 格式不合法"), "{err}");
    }

    #[test]
    fn progress_dedup_keys_on_status_and_value_only() {
        // 替换流水线阶段内部逐项上报：计数每项都在变，但同一百分比只该发一条事件。
        // 这个用例正是为了防住「把 current/total 放进去重键」——那样等于没去重。
        let mut d = ProgressDedup { last: None };
        assert!(d.should_emit("映射球员头像…", 51));
        assert!(!d.should_emit("映射球员头像…", 51)); // 同百分比：计数变了也不重复发
        assert!(!d.should_emit("映射球员头像…", 51));
        assert!(d.should_emit("映射球员头像…", 52));
        assert!(d.should_emit("读取现有映射…", 52)); // 百分比相同但阶段名变了：要发
    }

    #[test]
    fn sync_group_snapshot_fills_cache_from_mapping_not_by_reparsing() {
        // 自动同步：把阶段4写下的 config.xml 备份到该档快照位置，并立即刷新快照缓存。
        // 缓存内容必须来自**传入的映射**，而不是回读刚写出的文件。
        // 这里故意让盘上的 config.xml 与传入映射不一致：若实现退回「重解析」，缓存里会是盘上那份。
        let tmp = tempfile::tempdir().unwrap();
        let img = tmp.path().join("pack");
        std::fs::create_dir_all(&img).unwrap();
        xml_parser::write_xml(
            &[MappingEntry {
                uid: "r-9999".into(),
                ethnicity: "Asian".into(),
                image: "stale".into(),
            }],
            &img,
            false,
        )
        .unwrap();
        let mapping = vec![MappingEntry {
            uid: "r-1001".into(),
            ethnicity: "SpanMed".into(),
            image: "img1".into(),
        }];
        let snap_path = tmp.path().join("档1").join("snap.xml");
        let cache = Mutex::new(None);
        let (logger, _rx) = ChannelLogger::channel();
        sync_group_snapshot(&snap_path, img.to_str().unwrap(), &mapping, &cache, &logger);

        let got = cache.lock().unwrap();
        let c = got.as_ref().expect("快照缓存应被填充");
        assert_eq!(c.entries, mapping, "缓存必须直填传入映射，而不是回读文件");
        assert_eq!(c.xml_path, snap_path.to_str().unwrap(), "缓存路径即快照预览查询的比对键");
        // 快照文件本身仍是「盘上 config.xml 的副本」（拷贝语义不变，与缓存内容无关），
        // 且写入会自动补建缺失的父目录。
        assert!(snap_path.is_file(), "快照文件应已生成（父目录一并补建）");
        let copied = xml_parser::parse_xml_entries(&snap_path).unwrap();
        assert_eq!(copied[0].uid, "r-9999", "快照文件是 config.xml 的拷贝");
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
        sync_group_snapshot(&snap_path, img.to_str().unwrap(), &[], &cache, &logger);

        assert!(!snap_path.is_file());
        let got = cache.lock().unwrap();
        assert_eq!(got.as_ref().unwrap().xml_path, "old_snapshot.xml");
    }

    #[test]
    fn running_guard_resets_when_the_thread_panics() {
        // start_replace 的线程体 panic 时，running 也必须复位：否则界面全局禁用，
        // 也无法再发起新的替换。
        let flag = Arc::new(AtomicBool::new(true));
        let guard = RunningGuard(Arc::clone(&flag));
        let joined = std::thread::spawn(move || {
            let _guard = guard; // 与生产线程同形：守卫只挂在栈上
            panic!("模拟流水线 panic");
        })
        .join();
        assert!(joined.is_err());
        assert!(!flag.load(Ordering::SeqCst), "panic 后 running 未复位");
    }

    #[test]
    fn wipe_if_poisoned_keeps_healthy_cache_and_unpoisons_the_broken_one() {
        // 未中毒的缓存原样保留：panic 若发生在不持锁的流水线里，「扫描RTF」的成果不该跟着一起丢。
        let healthy = Mutex::new(vec![1, 2, 3]);
        assert!(!wipe_if_poisoned(&healthy));
        assert_eq!(*healthy.lock().unwrap(), vec![1, 2, 3]);

        // 中毒的缓存作废，并且必须 clear_poison：中毒会永久粘在 Mutex 上，
        // 不作废的话之后每条命令的 lock().unwrap() 都会 panic（界面解禁了但一调就炸）。
        let broken = Arc::new(Mutex::new(vec![1]));
        let holder = Arc::clone(&broken);
        let _ = std::thread::spawn(move || {
            let _g = holder.lock().unwrap();
            panic!("中毒现场");
        })
        .join();
        assert!(broken.is_poisoned());
        assert!(wipe_if_poisoned(&broken));
        assert!(broken.lock().unwrap().is_empty(), "中毒的缓存应被清空");
        assert!(!broken.is_poisoned(), "清空后应解除中毒");
    }

    // ---------------- save_and_replace（P3-2）的可纯测部分 ----------------
    // 命令本体依赖 State（Tauri 托管，单测不可构造），故把查找/命名/避让做成近纯函数覆盖。

    #[test]
    fn expand_naming_supports_uid_and_index_placeholders() {
        // 默认模板
        assert_eq!(expand_naming("{uid}_{index}", "p123", 1), "p123_1");
        // 字面量与占位符混排
        assert_eq!(expand_naming("gen-{uid}__{index}.v2", "p1", 12), "gen-p1__12.v2");
        // 未知占位符按字面量保留（不猜、不报错）
        assert_eq!(expand_naming("{uid}-{seq}-{index}", "p1", 2), "p1-{seq}-2");
        // 没有配对右括号：剩余内容原样
        assert_eq!(expand_naming("{uid}x{", "p1", 3), "p1x{");
        // 无占位符：纯字面量
        assert_eq!(expand_naming("static", "p1", 5), "static");
    }

    #[test]
    fn count_existing_only_counts_uid_prefixed_files() {
        let tmp = tempfile::tempdir().unwrap(); // TempDir 必须先绑定变量
        let dir = tmp.path().join("_generated");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("p123_1.png"), b"x").unwrap();
        std::fs::write(dir.join("p123_2.png"), b"x").unwrap();
        std::fs::write(dir.join("p123.png"), b"x").unwrap(); // 无 `_` 前缀：不算
        std::fs::write(dir.join("p12_3.png"), b"x").unwrap(); // 其他 uid 的前缀：不算
        std::fs::create_dir_all(dir.join("p123_9")).unwrap(); // 同名目录：不算
        assert_eq!(count_existing(&dir, "p123"), 2);
        assert_eq!(count_existing(&dir, "nobody"), 0);
    }

    #[test]
    fn next_free_target_counts_then_skips_collisions() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("_generated");
        std::fs::create_dir_all(&dir).unwrap();
        // 空目录：从 1 起
        assert_eq!(next_free_target(&dir, "{uid}_{index}", "p1").unwrap(), dir.join("p1_1.png"));
        // 已有 p1_1 与 p1_3（计数=2 → index=3 撞上 p1_3）：递增避让到 p1_4，
        // 计数与避让两道保障在同一条路径上都生效。
        std::fs::write(dir.join("p1_1.png"), b"x").unwrap();
        std::fs::write(dir.join("p1_3.png"), b"x").unwrap();
        assert_eq!(next_free_target(&dir, "{uid}_{index}", "p1").unwrap(), dir.join("p1_4.png"));
        // 模板不含 {index}：目标名恒定，已存在时递增也腾不出新名字 → 显式报错，绝不静默覆盖旧图
        std::fs::write(dir.join("fixed.png"), b"x").unwrap();
        let err = next_free_target(&dir, "fixed", "p1").unwrap_err();
        assert!(err.contains("命名模板"), "报错应指引补 {{index}}：{err}");
    }

    #[test]
    fn find_tmp_image_matches_by_stem_regardless_of_extension() {
        let tmp = tempfile::tempdir().unwrap(); // TempDir 必须先绑定变量
        let imgtmp = tmp.path().join(".imgtmp");
        std::fs::create_dir_all(&imgtmp).unwrap();
        // 扩展名不写死：按 file_stem 匹配（服务商响应决定扩展名）
        std::fs::write(imgtmp.join("abc.webp"), b"x").unwrap();
        std::fs::write(imgtmp.join("other.png"), b"x").unwrap();
        assert_eq!(find_tmp_image(&imgtmp, "abc").unwrap(), imgtmp.join("abc.webp"));
        // 不存在的 id：报错含「生成图中不存在」
        let err = find_tmp_image(&imgtmp, "missing").unwrap_err();
        assert!(err.contains("生成图中不存在"), "实际报错：{err}");
        // 临时目录整个不存在：读目录失败也给出可懂的中文报错
        let gone = tempfile::tempdir().unwrap();
        let empty = gone.path().join(".imgtmp");
        let err2 = find_tmp_image(&empty, "abc").unwrap_err();
        assert!(err2.contains("临时目录"), "实际报错：{err2}");
    }
}

// ===================== AI 文生图命令（实现统一在 image_ai；与既有命令同居本文件）=====================

#[tauri::command]
pub fn get_image_ai_state(state: State<'_, AppState>) -> Result<Config, String> {
    // public_view：抹掉 Key 密文——前端只认 keysSet 布尔，不需要也不该拿到密文。
    Ok(state.image.lock().unwrap().public_view())
}

/// 渲染 `{{变量|默认值}}` 模板文本（无数据源时全取默认值）；`missing` 非空表示存在既无默认值又无数据源的占位符。
/// 入参用文本而非模板 ID：选中模板渲染一次、生成前对用户编辑后的文本再渲染一次，避免手工编辑被模板原文覆盖。
#[tauri::command]
pub fn render_prompt(body: String, vars: Option<HashMap<String, String>>) -> Result<image_ai::RenderOutcome, String> {
    Ok(image_ai::render(&body, &vars.unwrap_or_default()))
}

/// 按球员数据源渲染模板：uid 定位 rtf_cache 里的球员 → build_vars → render 模板 body。
/// 两把锁各自「锁内只做查找/clone」即释放，锁后都是纯内存计算。
#[tauri::command]
pub fn preview_prompt(
    state: State<'_, AppState>,
    uid: String,
    template_id: String,
) -> Result<image_ai::RenderOutcome, String> {
    let rec = {
        let cache = state.rtf_cache.lock().unwrap();
        cache.iter().find(|p| p.uid == uid).cloned()
    };
    let rec = rec.ok_or_else(|| format!("名单中没有该球员（{uid}），请先扫描RTF名单后再预览提示词"))?;
    let cfg = state.image.lock().unwrap().clone();
    let tpl = cfg
        .templates
        .iter()
        .find(|t| t.id == template_id)
        .ok_or_else(|| format!("找不到提示词模板（{template_id}）"))?;
    let vars = image_ai::build_vars(&rec, tpl);
    Ok(image_ai::render(&tpl.body, &vars))
}

#[tauri::command]
pub fn save_provider(state: State<'_, AppState>, provider: image_ai::ProviderConfig) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::upsert_provider(&mut cfg, provider)?;
    cfg.save_providers(&state.data_dir)?;
    Ok(cfg.public_view())
}

#[tauri::command]
pub fn delete_provider(state: State<'_, AppState>, id: String) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    // 删除前摘显示名供日志；名字为空时只记 id。
    let label = match cfg.provider(&id) {
        Some(p) if !p.name.trim().is_empty() => format!("{}（{id}）", p.name),
        _ => id.clone(),
    };
    image_ai::remove_provider(&mut cfg, &id)?;
    cfg.save_providers(&state.data_dir)?;
    let view = cfg.public_view();
    drop(cfg);
    // 服务商已删，会话内存里那把 Key 也一并清掉（它随配置行的 apiKey 字段一起消失）。
    state.image_keys.lock().unwrap().remove(&id);
    state.logger.info("image_ai", format!("已删除服务商：{label}"));
    Ok(view)
}

/// 保存/清除某服务商的 Key：密文写进该服务商行的 `apiKey` 字段并落盘，明文只留在会话内存。
/// 【锁序】image → image_keys；两段各自「锁内只做一件事」即释放，不嵌套。
#[tauri::command]
pub fn set_provider_key(state: State<'_, AppState>, id: String, key: String) -> Result<(), String> {
    let cleared = key.trim().is_empty();
    let sealed = if cleared {
        None
    } else {
        Some(crate::image_ai::seal_key(&key)?)
    };
    {
        let mut cfg = state.image.lock().unwrap();
        let Some(provider) = cfg.providers.iter_mut().find(|p| p.id == id) else {
            return Err(format!("找不到服务商（{id}）"));
        };
        provider.api_key = sealed;
        cfg.save_providers(&state.data_dir)?;
    }
    {
        let mut keys = state.image_keys.lock().unwrap();
        if cleared {
            keys.remove(&id);
        } else {
            keys.insert(id.clone(), key);
        }
    }
    // 只记「设/清」这个动作，绝不记 Key 内容。
    state.logger.info(
        "image_ai",
        if cleared {
            format!("已清除服务商 Key：{id}")
        } else {
            format!("已保存服务商 Key（加密保存在本机）：{id}")
        },
    );
    Ok(())
}

/// 已配置 Key 的服务商 id 列表（前端重启后靠它认回哪些已配置；只回 id，绝不回密文）。
#[tauri::command]
pub fn get_provider_key_state(state: State<'_, AppState>) -> Vec<String> {
    let mut ids: Vec<String> = state.image_keys.lock().unwrap().keys().cloned().collect();
    ids.sort();
    ids
}

#[tauri::command]
pub fn save_prompt_template(state: State<'_, AppState>, template: image_ai::PromptTemplate) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::upsert_template(&mut cfg, template)?;
    cfg.save_templates(&state.data_dir)?;
    Ok(cfg.public_view())
}

#[tauri::command]
pub fn delete_prompt_template(state: State<'_, AppState>, id: String) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    // 删除前摘显示名供日志；名字为空时只记 id。
    let label = match cfg.templates.iter().find(|t| t.id == id) {
        Some(t) if !t.name.trim().is_empty() => format!("{}（{id}）", t.name),
        _ => id.clone(),
    };
    image_ai::remove_template(&mut cfg, &id)?;
    cfg.save_templates(&state.data_dir)?;
    state.logger.info("image_ai", format!("已删除提示词模板：{label}"));
    Ok(cfg.public_view())
}

#[tauri::command]
pub async fn generate_image(
    state: State<'_, AppState>,
    req: image_ai::GenRequest,
) -> Result<image_ai::GenResult, image_ai::ImageGenError> {
    // await 前把配置 clone 出锁；image 锁即取即放，与 image_keys 保持全仓单一锁序（image → image_keys → queue），不嵌套持有。
    let cfg = state.image.lock().unwrap().clone();
    // 仅校验请求自身的完整性；厂商约束（尺寸/张数/提示词长度）由 provider 层解释，见 generate。
    image_ai::validate_complete(&cfg, &req)?;
    // 免 Key 协议（本地 ComfyUI）传空串占位，适配器不使用（见 ProviderKind::needs_key）。
    let needs_key = cfg.provider(&req.provider_id).map(|p| p.kind.needs_key()).unwrap_or(true);
    let key = if needs_key {
        state
            .image_keys
            .lock()
            .unwrap()
            .get(&req.provider_id)
            .cloned()
            .ok_or_else(|| image_ai::ImageGenError::not_configured("请先输入 API Key（加密保存在本机）"))?
    } else {
        String::new()
    };
    // 日志标签先摘出来：req 随后按值交给 generate，之后取不到。
    let provider = req.provider_id.clone();
    let model = req.model.clone();
    let label = match req.uid.as_deref().filter(|u| !u.is_empty()) {
        Some(uid) => format!("{uid}｜{provider}/{model}"),
        None => format!("{provider}/{model}"),
    };
    let started = Instant::now();
    // 单张直发不展示阶段：空回调（阶段进度只有队列批处理会呈现在界面上）。
    match image_ai::generate(&state.data_dir, &cfg, &key, req, &|_| {}).await {
        Ok(result) => {
            state.logger.info(
                "image_ai",
                format!("生成完成：{label}，{} 张图，耗时 {:.1?}", result.images.len(), started.elapsed()),
            );
            Ok(result)
        }
        Err(mut e) => {
            // 厂商可能把 Key 抄回错误文案，日志与前端展示前统一抹掉。
            e.scrub(&key);
            state.logger.error("image_ai", format!("生成失败：{label}：{}", e.message));
            Err(e)
        }
    }
}

/// 读取持久画廊记录，并逐条现算图片是否仍在会话临时目录（存在性只在查询期计算，绝不写回 gallery.json）。
#[tauri::command]
pub fn get_gallery(state: State<'_, AppState>) -> Result<Vec<image_ai::GalleryItem>, String> {
    Ok(image_ai::load_gallery_items(&state.data_dir))
}

#[tauri::command]
pub fn save_generated_image(
    state: State<'_, AppState>,
    image_id: String,
    target_path: String,
) -> Result<String, String> {
    let src = image_ai::tmp_dir(&state.data_dir).join(format!("{image_id}.png"));
    if !src.is_file() {
        return Err("源图片已失效（临时目录已清理）".into());
    }
    json_store::copy_atomic(&src, Path::new(&target_path))?;
    state.logger.info("image_ai", format!("生成图另存：{target_path}"));
    Ok(target_path)
}

/// 在 `.imgtmp` 里按 image_id 找源文件。生成图扩展名由服务商响应决定，故按 file_stem 遍历比对（不写死 .png）。
/// 找不到＝图片已被会话清理或 id 传错。
fn find_tmp_image(tmp_dir: &Path, image_id: &str) -> Result<PathBuf, String> {
    let entries =
        std::fs::read_dir(tmp_dir).map_err(|e| format!("读取生成图临时目录失败（{}）: {e}", tmp_dir.display()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.file_stem().and_then(|s| s.to_str()) == Some(image_id) {
            return Ok(p);
        }
    }
    Err(format!("生成图中不存在该图片（{image_id}）：可能已被会话清理，请重新生成后再保存并替换"))
}

/// 展开命名模板：`{uid}` → 球员 UID，`{index}` → 序号；其余内容（含未知占位符）按字面量原样保留。
/// 只认这两个占位符：未知的不猜、不报错，原样落进文件名，用户一眼能改掉。
fn expand_naming(pattern: &str, uid: &str, index: usize) -> String {
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                match &after[..close] {
                    "uid" => out.push_str(uid),
                    "index" => out.push_str(&index.to_string()),
                    // 未知占位符（含配对花括号）按字面量回填
                    other => out.push_str(&format!("{{{other}}}")),
                }
                rest = &after[close + 1..];
            }
            // 没有配对的右括号：剩余内容全部原样
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// 统计 `_generated` 里属于该 uid 的已有文件数（file_stem 以 `{uid}_` 开头）；只数文件，同名目录与读目录失败一律跳过。
fn count_existing(dir: &Path, uid: &str) -> usize {
    let prefix = format!("{uid}_");
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let p = e.path();
                    p.is_file() && p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with(&prefix))
                })
                .count()
        })
        .unwrap_or(0)
}

/// 计算落盘目标：index 从「同 uid 已有数 + 1」起步，目标名已存在则继续递增。
/// 不能指望 copy_atomic 拒绝覆盖（它是覆盖写）；模板不含 {index} 时递增也腾不出新名字，有限次后报错，而非静默覆盖旧图。
fn next_free_target(gen_dir: &Path, pattern: &str, uid: &str) -> Result<PathBuf, String> {
    let mut index = count_existing(gen_dir, uid) + 1;
    let mut target = gen_dir.join(format!("{}.png", expand_naming(pattern, uid, index)));
    let mut tries = 0;
    while target.is_file() {
        if tries >= 32 {
            return Err(format!(
                "目标文件已存在且命名模板无法自动避让（{}）：请给命名模板加上 {{index}}",
                target.display()
            ));
        }
        index += 1;
        target = gen_dir.join(format!("{}.png", expand_naming(pattern, uid, index)));
        tries += 1;
    }
    Ok(target)
}

/// 「保存并替换」的回执。`replaced=false` = 图片已落进 `_generated/`，但该 UID 在当前档的
/// config.xml 里没有映射行，替换链路没改动任何东西 —— 前端据此提示「未替换」，不能报成功。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveReplaceResult {
    pub image: image_ai::GenImage,
    pub replaced: bool,
}

/// 「保存并替换」：把 .imgtmp 里的生成图落盘到 `<当前组>/_generated/`，再复用单人换脸链
/// （互斥检查/快照归位/改 config.xml/缓存同步）；映射表没有该 UID 时追加一条（insert_missing）。
/// 【为什么是同步 fn】：内部无 await 点，重活是同步 XML 链路，Tauri 同步命令跑在命令线程池不卡 UI。
#[tauri::command]
pub fn save_and_replace(
    state: State<'_, AppState>,
    image_id: String,
    uid: String,
    save_backup: bool,
) -> Result<SaveReplaceResult, String> {
    let uid = uid.trim().to_string();
    // ① 批量互斥前置检查：把失败挡在复制文件之前，磁盘上不会留下半成品。
    //    replace_single_face 内部还会再查一次 running（双保险，拒绝文案与其对齐）。
    if state.running.load(Ordering::SeqCst) {
        state.logger.warn("image_ai", "保存并替换被拒绝：批量替换正在进行");
        return Err("批量替换正在进行，请等它完成或先取消，再做单人微调".into());
    }
    // ② 短锁取生效配置里的命名模板（clone 一份字符串即放锁，锁内不做任何重活）。
    let pattern = state.image.lock().unwrap().settings.naming_pattern.clone();
    // ③ 源文件：.imgtmp 按 image_id 找（扩展名不写死，见 find_tmp_image）。
    let src = find_tmp_image(&image_ai::tmp_dir(&state.data_dir), &image_id)?;
    // ④ 当前组目录：生成图必须落进头像包，替换链路才能按「包内图片」解析
    //    （parse_image_path_in_pack 会 canonicalize 后剥离组目录前缀，包外路径会被拒）。
    let img_dir = {
        let pm = state.pm.lock().unwrap();
        pm.cur_group.clone().ok_or("请先关联头像包（当前没有选中组），再保存并替换")?
    };
    let gen_dir = Path::new(&img_dir).join("_generated");
    std::fs::create_dir_all(&gen_dir).map_err(|e| format!("创建生成图目录失败（{}）: {e}", gen_dir.display()))?;
    // ⑤ 目标路径：命名模板展开 + 同名避让（见 next_free_target）；生成图统一 png。
    let target = next_free_target(&gen_dir, &pattern, &uid)?;
    json_store::copy_atomic(&src, &target)?;
    // ⑥ 复用单人换脸全链：互斥/快照归位/改 config.xml/同步 mapping_cache 与存档快照全部继承。
    //    命中与否必须上报：图片已安全落盘，但没命中＝映射一行都没改，报「成功」会让用户
    //    以为换上了，回头在名单/快照预览里找不到新头像（replaced=false 由前端换成警告文案）。
    let target_str = target.to_string_lossy().to_string();
    let logger = state.logger.clone();
    // 生成图允许「当前包还没有这名球员的映射」：insert_missing=true 时追加一行。
    let replaced = replace_single_face_impl(&state, uid.clone(), target_str.clone(), save_backup, true)?;
    let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    if replaced {
        logger.info("image_ai", format!("保存并替换完成：{uid} → {target_str}（{bytes} 字节）"));
    } else {
        logger.warn("image_ai", format!("图片已保存但未能写入映射（config.xml 缺失或结构异常）：{uid}（{target_str}）"));
    }
    Ok(SaveReplaceResult {
        image: image_ai::GenImage {
            id: image_id,
            path: target_str,
            bytes,
        },
        replaced,
    })
}

// ---------------- 批量生成队列（image_ai_queue::run_queue 承担实际调度） ----------------

/// add_to_queue 的纯主体：先整批校验（不合规则原样返回 Err、队列不变），通过后才并入队列。
/// 抽成 pub(crate) 是为了让「整批拒绝时队列不变」这条契约可被单测直接钉住（命令层的 State 无法构造）。
pub(crate) fn add_to_queue_inner(
    cfg: &image_ai::Config,
    queue: &Mutex<image_ai_queue::QueueState>,
    entries: Vec<image_ai_queue::QueueEntry>,
) -> Result<Vec<image_ai_queue::QueueEntry>, image_ai::ImageGenError> {
    image_ai_queue::validate_entries(cfg, &entries)?;
    let mut q = queue.lock().unwrap();
    q.add(entries);
    Ok(q.snapshot())
}

/// 追加一批生成任务，返回追加后的全量快照（条目 id 由后端统一生成，前端以此为准）。
/// 任一条目不满足完整性/厂商约束即整批拒绝（防呆：避免跑一半才失败），返回该条目的结构化错误，队列不变。
#[tauri::command]
pub fn add_to_queue(
    state: State<'_, AppState>,
    entries: Vec<image_ai_queue::QueueEntry>,
) -> Result<Vec<image_ai_queue::QueueEntry>, image_ai::ImageGenError> {
    // 先 clone 出 cfg（语句末即释放）再锁 queue，保持全仓单一锁序 image → queue，避免反序加锁。
    let cfg = state.image.lock().unwrap().clone();
    add_to_queue_inner(&cfg, &state.queue, entries)
}

/// 按 id 批量移除条目，返回移除后的全量快照。
#[tauri::command]
pub fn remove_from_queue(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<image_ai_queue::QueueEntry>, String> {
    let mut q = state.queue.lock().unwrap();
    q.remove(&ids);
    Ok(q.snapshot())
}

/// 清空队列，返回空快照。
#[tauri::command]
pub fn clear_queue(state: State<'_, AppState>) -> Result<Vec<image_ai_queue::QueueEntry>, String> {
    let mut q = state.queue.lock().unwrap();
    q.clear();
    Ok(q.snapshot())
}

/// 启动批量生成：校验通过后把队列交给后台异步任务，命令本身立即返回。
/// 进度经事件推送（gen:queue:progress / gen:queue:finished），前端另行 query_queue 拉快照。
#[tauri::command]
pub async fn start_queue(app: AppHandle, state: State<'_, AppState>) -> Result<(), image_ai::ImageGenError> {
    // 出锁前把后台任务需要的全部数据 clone 下来：std Mutex 绝不跨 await 持有。
    // 锁序固定为 image → image_keys → queue（与 generate_image 的 image → image_keys 同序）；
    // 每把锁都在各自语句末尾即释放，不与下一把嵌套持有——全仓单序可排除交叉加锁的死锁面。
    let cfg = state.image.lock().unwrap().clone();
    let concurrency = cfg.settings.concurrency;
    let keys = state.image_keys.lock().unwrap().clone();
    // queue 最后锁：重入检查与逐条 Key 校验都在这一层内完成（与在跑状态同源）。
    {
        let q = state.queue.lock().unwrap();
        // 已有任务在跑（存在 Running 条目）：拒绝重入。
        if q.entries.iter().any(|e| e.status == image_ai_queue::QueueStatus::Running) {
            return Err(image_ai::ImageGenError::internal("队列正在执行中"));
        }
        // 逐条校验 Queued 条目的 Key，任一缺失就整批不启动（跑一半才失败会白白烧掉前几条配额）；
        // 免 Key 协议（本地 ComfyUI）直接放行，口径与 generate_image 一致。
        for e in q.entries.iter().filter(|e| e.status == image_ai_queue::QueueStatus::Queued) {
            let needs_key = cfg.provider(&e.req.provider_id).map(|p| p.kind.needs_key()).unwrap_or(true);
            if !needs_key {
                continue;
            }
            let ok = keys.get(&e.req.provider_id).is_some_and(|k| !k.trim().is_empty());
            if !ok {
                return Err(image_ai::ImageGenError::not_configured("请先输入该服务商 Key"));
            }
        }
    }
    let sink: Arc<dyn image_ai_queue::EventSink> = Arc::new(image_ai_queue::AppEventSink(app));
    let queue = Arc::clone(&state.queue);
    let data_dir = state.data_dir.clone();
    let executor: Arc<dyn image_ai_queue::GenExecutor> = Arc::new(image_ai_queue::RealExecutor);
    // 真实睡眠：重试退避经 spawn_blocking 睡（不阻塞异步运行时）。
    let sleeper: Arc<dyn image_ai_queue::Sleeper> = Arc::new(image_ai_queue::RealSleeper);
    tauri::async_runtime::spawn(async move {
        let (finished, failed, total) =
            image_ai_queue::run_queue(queue, data_dir, cfg, keys, concurrency, executor, sleeper, sink).await;
        log::info!(
            target: "image_ai",
            "批量生成队列结束：成功 {finished}，失败 {failed}，共 {total}"
        );
    });
    Ok(())
}

/// 请求取消：尚未启动的条目将被判「已取消」，在跑的等它自然收尾（见 run_queue）。
#[tauri::command]
pub fn cancel_queue(state: State<'_, AppState>) -> Result<(), String> {
    state.queue.lock().unwrap().cancel = true;
    state.logger.info("image_ai", "用户请求取消批量生成");
    Ok(())
}

/// 查询队列全量快照（条目状态/图片/错误都从这里拉）。
#[tauri::command]
pub fn query_queue(state: State<'_, AppState>) -> Result<Vec<image_ai_queue::QueueEntry>, String> {
    Ok(state.queue.lock().unwrap().snapshot())
}
