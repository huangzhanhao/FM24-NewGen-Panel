//! Tauri 命令层（对等旧版 services/* + app.py）：前端 invoke 的全部入口。
//!
//! 【设计】数据留在 Rust，IPC 只传摘要/分页：大名单与日志在缓存里按页查询，
//! 每次状态变更后回传 AppSnapshot 让前端整体刷新。命令可能在不同线程被调用、后台替换
//! 也跑在单独线程，因此 AppState 的共享字段用 Mutex 保护、跨线程引用用 Arc、
//! 运行/取消标志用 AtomicBool。返回 Result<T, String> 时，Err 的中文描述由前端直接弹窗展示。
use crate::face_mapper::Mode;
use crate::facepack_scanner::{self, IntegrityResult, PackScan, PackSig};
use crate::image_ai::{self, Config};
use crate::image_ai_queue; // 批量生成队列（条目/状态机/执行器；6 个命令在本文件尾部）
use crate::json_store; // save_generated_image 用其 copy_atomic（既有代码亦用全限定写法）
use crate::logger::{LogPage, LogSink, Logger};
use crate::profile_manager::ProfileManager;
use crate::replace_flow::{self, Outcome, ReplaceParams};
use crate::rtf_scanner::{PlayerRecord, RosterCounts, RosterSummary, RtfScanner};
use crate::templates::{self, RuleConfig};
use crate::xml_parser::{self, MappingEntry, SnapshotEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap; // AppState.image_keys；commands.rs 原本没有这条 use
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
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
    pub rtf_cache: Arc<Mutex<Vec<PlayerRecord>>>, // 最近一次解析的球员缓存（作用域见 clear_caches_if_scope_changed）
    pub mapping_cache: Arc<Mutex<Vec<MappingEntry>>>, // 最近一次生成的映射缓存（同上；两者切档/切组/换名单一律清空）
    pub faces_cache: Arc<Mutex<Option<FaceIndex>>>,   // 「扫描头像」的包索引缓存（替换可复用）
    pub snapshot_cache: Arc<Mutex<Option<SnapshotCache>>>, // 快照解析缓存（快照预览分页复用；「读取快照」与替换收尾都会写）
    pub image: Arc<Mutex<Config>>, // 生图生效配置（image_ai.json + 默认双来源）
    pub image_keys: Arc<Mutex<HashMap<String, String>>>, // 会话内 API Key（不落盘）
    pub queue: Arc<Mutex<image_ai_queue::QueueState>>, // 批量生成队列（条目列表 + cancel 标志）
    pub data_dir: PathBuf,         // 便携数据目录（标准规则 data/rules/ 在其下）
}

/// 「扫描头像」产出的索引缓存：目录 + 完整扫描结果 + 目录签名。
/// 【命中条件】`img_dir` 相同**且**签名一致（见 facepack_scanner::PackSig）。
/// 只比路径的话，用户在两次替换之间删掉包里某张图时，我们会拿过期索引写出**指向不存在图片**的
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
/// 【为什么需要它】这两个缓存是「最近一次解析/生成的」语义，**没有路径键**（不像 `snapshot_cache`
/// 按快照路径命中、`faces_cache` 按目录 + 目录签名），它们的归属完全由这三项决定。
fn cache_scope(pm: &ProfileManager) -> (String, Option<String>, String) {
    (pm.cur_prf.clone(), pm.cur_group.clone(), pm.get_cur_rtf())
}

/// 命令改完存档 / 组 / 名单之后调用：作用域变了就清掉那两个缓存（宁缺勿旧）。
/// 【不清会怎样】它们会继续按「上一次」的内容作答：名单预览显示**上一个档/上一个组**的名单，
/// 单人微调与单人预览拿**别档**的映射行展示抽图结果——两档共用同一头像包时尤其明显。
/// 清空后前端据快照里的作用域变化把 `rosterReady` 复位，各预览回到引导空态，
/// 等下一次「扫描RTF」或替换重新填充（替换必然重填，扫描按需）。
fn clear_caches_if_scope_changed(
    state: &State<'_, AppState>,
    before: (String, Option<String>, String),
) {
    // 锁只在这一句里持有：先取出改后的作用域，随后的两把锁与 pm 无嵌套关系。
    let after = cache_scope(&state.pm.lock().unwrap());
    if after == before {
        return; // 作用域没变（例如删的是别的组）：手上这份缓存仍然对应当前档当前组
    }
    state.rtf_cache.lock().unwrap().clear();
    state.mapping_cache.lock().unwrap().clear();
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
/// current/total = 阶段内的「第 n 个 / 共 total 个」（纯阶段锚点或该阶段无可数单位时为 0/0，
/// 前端据此只显示百分比、不显示计数）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPayload {
    pub status: String,
    pub value: u8,
    pub current: usize,
    pub total: usize,
}

/// 轻量任务（检查完整性 / 扫描头像 / 扫描RTF / 读取快照）的进度载荷：
/// 除任务名与百分比外，多带「第 current 个 / 共 total 个」，状态栏据此显示「n/total」原始口径。
/// 【为何不复用 ProgressPayload】替换流水线只报阶段与百分比（没有可数的单位），两者载荷形状与
/// 消费方都不同；拆成两个事件（replace:progress / task:progress）后互不干扰。
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

    /// 收尾上报 100%：命令成功返回前必须调一次——总数为 0（无子目录/无数据行）或小任务跑得太快时，
    /// 进度条也不会停在半路。
    fn finish(&mut self) {
        self.emit(100);
    }

    fn emit(&mut self, value: u8) {
        if self.last == Some(value) {
            return; // 百分比没变：不重复 emit
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
/// 计数每项都在变——把计数放进键等于没去重，照样刷爆 IPC。百分比本就是由 current/total 折算来的，
/// 放行时把当时的计数一并带出去，两者自洽。
struct ProgressDedup {
    last: Option<(String, u8)>,
}

impl ProgressDedup {
    /// 返回 true 表示这次该 emit（并把本次的 (阶段文案, 百分比) 记为上次）。
    fn should_emit(&mut self, status: &str, value: u8) -> bool {
        if self
            .last
            .as_ref()
            .is_some_and(|(s, v)| s == status && *v == value)
        {
            return false;
        }
        self.last = Some((status.to_string(), value));
        true
    }
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
    let mut pm = state.pm.lock().unwrap();
    let before = cache_scope(&pm); // 新建即激活＝切档：改前记作用域，改完比对作废名单/映射缓存
    pm.create_profile(&name)?;
    drop(pm);
    clear_caches_if_scope_changed(&state, before);
    state
        .logger
        .info("profile", format!("创建并激活 Profile: {name}"));
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
        "（当前激活档，已切回 No Profile）" // 删掉当前激活档会连带切档，这个状态变化在文案里点明
    } else {
        ""
    };
    state
        .logger
        .info("profile", format!("删除 Profile: {name}{suffix}"));
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
    let before = cache_scope(&pm); // 当前组会变（新组）
    pm.ensure_group(&path).ok_or("目录不可用")?; // ok_or 把 Option::None 转为 Err
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
pub fn set_current_group(
    state: State<'_, AppState>,
    img_dir: String,
) -> Result<AppSnapshot, String> {
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
    let before = cache_scope(&pm); // 删的若是当前组，当前组会回退到剩下的组 → 作用域变化
    if !pm.delete_group(&img_dir)? {
        return Err("组不存在".into());
    }
    drop(pm);
    clear_caches_if_scope_changed(&state, before);
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
/// 并给出可一键补全的缺失目录与未配置的多余目录。判定逻辑在 facepack_scanner::check_integrity_progress，
/// 命令层只负责加载规则配置、把结果回传前端（IPC），并把核对进度发给状态栏。
/// 【async】与另外三个扫描命令一致：标准一平铺包的顶层条目可达数万，同步命令跑在主线程上，
/// 会把 task:progress 事件和界面重绘一起堵住，进度条反而看不出实时变化。
#[tauri::command]
pub async fn check_integrity(
    app: AppHandle,
    state: State<'_, AppState>,
    img_dir: String,
) -> Result<IntegrityResult, String> {
    let config = RuleConfig::load(&state.data_dir)?;
    // 进度以「头像包顶层条目」为单位（先列目录拿到分母再逐条核对），状态栏显示 n/total。
    let mut progress = TaskProgress::new(&app, "检查完整性");
    let result = facepack_scanner::check_integrity_progress(
        &img_dir,
        &config.folder_lists(),
        &mut |done, total| progress.report(done, total),
    );
    progress.finish();
    Ok(result)
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
    // filter=false：扫描展示全量名单（Replace 流程才按「仅随机人」过滤）。
    // 「扫描RTF」按钮没有取消入口（状态栏进度只报不做），所以这里给一个恒为 false 的标志。
    let (data, counts) = scanner.parse_roster_progress(
        Path::new(rtf_path),
        false,
        logger,
        &AtomicBool::new(false),
        progress,
    )?;
    // 摘要口径统一收在 RosterSummary（随机人前缀判定、拓展信息判定都不在这里重复一遍）。
    Ok((RosterSummary::from_records(&scanner, &data), data, counts))
}

/// 「扫描头像」：扫描当前组头像包、缓存索引（供替换复用），结果写日志。
/// 【async】重扫描（递归遍历包内全部目录）跑在异步运行时线程，避免阻塞主线程导致界面卡住。
#[tauri::command]
pub async fn scan_images(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ImageScanResult, String> {
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

    // 进度以「一级子目录」为单位（见 scan_pack_progress），扫完一个大包能明显看到进度在走。
    let mut progress = TaskProgress::new(&app, "扫描头像");
    let t = Instant::now(); // 扫盘是替换流水线的最大单项开销，这里单独记一条供对比
    let (total, standard, scan) = facepack_scanner::scan_and_total_progress(
        &img_dir,
        &config.folder_lists(),
        &mut |done, total| progress.report(done, total),
    )?;
    let scan_elapsed = t.elapsed();
    // 目录签名随索引一起存：替换复用前先比对（包内文件被增删时重扫，避免拿过期索引写错映射）。
    // 【计时必须与扫盘分开】pack_sig 不递归，但要读完包根并**逐个一级目录数一遍直接子条目**
    // （标准三 28 万图实测 0.15–0.19 s，与扫盘本身同量级）。混在一起记会把扫盘说得比实际慢一倍多
    //（实测：扫盘 128 ms 的包，这条日志会显示成 317 ms）。
    let t_sig = Instant::now();
    let sig = facepack_scanner::pack_sig(&img_dir);
    let sig_elapsed = t_sig.elapsed();
    // Arc 包一层入缓存：替换复用时只涨引用计数，不复制整份图片索引。
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
    progress.finish(); // 收尾 100%（空包/平铺包没有子目录可报，进度条也得走满）
    Ok(ImageScanResult { total, standard })
}

/// 「扫描RTF」：解析当前组 RTF 并填充名单预览缓存（无需先 Replace），结果写日志。
/// 【async】大名单解析跑在异步运行时线程，避免阻塞主线程导致界面卡住。
#[tauri::command]
pub async fn scan_rtf(app: AppHandle, state: State<'_, AppState>) -> Result<RosterSummary, String> {
    let rtf = { state.pm.lock().unwrap().get_cur_rtf() };
    if rtf.is_empty() {
        return Err("请先选择 RTF 名单".into());
    }
    // 进度以「数据行」为单位（解析前先数一遍行数拿到分母，见 parse_roster_progress）。
    let mut progress = TaskProgress::new(&app, "扫描RTF");
    // &*state.logger：&Arc<Logger> 显式解引用成 &Logger（泛型参数处不会自动解强制引用）。
    let t = Instant::now(); // 解析名单的耗时口径（识别 + 全量解析），与替换流水线里的同一段同口径
    let (summary, data, counts) = scan_rtf_core(&rtf, &*state.logger, &mut |done, total| {
        progress.report(done, total)
    })?;
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
    progress.finish(); // 收尾 100%（空名单/无数据行时进度条也得走满）
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

/// 「正在替换」标志的 RAII 守卫：Drop 时复位，让「校验提前 return」「线程正常收尾」「线程 panic」
/// 三条路径共用同一个出口。
/// 【为什么不能只靠 store(false)】`swap(true)`（本命令开头）与 `store(false)`（线程末尾）
/// 相隔约 180 行，中间任何一次 panic 都会跳过复位：running 永久为 true，界面全局禁用，
/// 也无法再发起新的替换。
struct RunningGuard(Arc<AtomicBool>);

impl RunningGuard {
    /// 主动复位。**两处补发 `replace:finished` 的调用方都必须先调它、再 emit。**
    /// 【为什么不能只靠 Drop】Drop 要等线程闭包结束，而它前面那句 `app.emit` 是把事件投递给主线程
    /// 再等主线程接手；主线程那一刻若正忙在别的事情上（例如 tauri-plugin-dialog 那个原生弹窗的嵌套
    /// 消息循环），这次 emit 就会把本线程按在原地震不动——running 跟着一起被按住，界面持续全局禁用
    ///（2026-09-25 实测到就是这个卡法）。复位挪到 emit 之前，emit 卡多久都不再影响解锁。
    /// store(false) 幂等，Drop 再走一遍无害，于是 panic、参数校验失败这些没走到 release
    /// 的路径仍然由 Drop 兜住。
    fn release(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.release();
    }
}

/// 后台替换线程的入参。之所以收成一个结构体而不是就地留在闭包里：
/// 线程体要能被 `catch_unwind` 包住，而 panic 之后还需要一份能再次借用的句柄来发补救事件。
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

/// 缓存被 panic 毒化时才作废（未中毒的一律原样保留）。
/// 【为什么先判中毒】`replace_flow::run` 内部不持任何锁，最常见的 panic 根本不会毒化谁；
/// 这时若顺手清缓存，就把「扫描RTF」载入的名单一起丢了——与失败/取消不写回空表是同一个口径。
/// 【为什么要 clear_poison】中毒会永久粘在 Mutex 上，之后每个 `lock().unwrap()` 都 panic，
/// 等于界面解禁了但任何命令一调就炸。
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
        // 【签名比对为什么挪进线程】pack_sig 不递归，但要读完包根**并逐个一级目录数一遍直接子条目**
        // （标准三 28 万图实测 0.15–0.19 s）。同步命令跑在主线程上，留在那里会让点击「替换」后
        // 先有约 0.2 s 的无响应；挪进来既不阻塞界面，日志顺序也自然（签名 → 开始替换 → 索引完成）。
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
            &*self.logger, // &Arc<Logger> 显式解引用成 &Logger（泛型参数处不会自动解强制引用）
        );
        // 【收尾计时】下面这段（把两份缓存写回去 + 同步存档快照 + 发完成事件）全在 run() 返回之后，
        // 不属于流水线自己的「总耗时」，却卡着 running 复位（界面全局禁用）——必须单独计时才看得见。
        let t_tail = Instant::now();
        // 把解析结果与映射写入缓存（供后续预览/分页查询使用）。
        // 【按值搬走，不再 clone】result 后面只用到 outcome / roster / scan，rtf_data 与 mapping
        // 都能直接移动；15 万条实测一次整表 clone 约 0.1 s。mapping 仍需留一份给下面的快照直填，
        // 所以只 clone 一次（与改造前的量相同），另一处改为移动。
        let mapping = result.mapping;
        let mapping_empty = mapping.is_empty();
        // 【失败/取消不动这两份缓存】`failed()` / `cancelled()` 带回来的是空表，无条件写回就等于
        // 把「扫描RTF」载入的名单一起清掉：在 96% 处因 config.xml 被编辑器占用而失败的一轮，
        // 作用域（当前档/当前组/当前名单）根本没有变化，用户却得重扫一遍并丢掉筛选、排序与页码。
        if matches!(result.outcome, Outcome::Finished) {
            *self.rtf_cache.lock().unwrap() = result.rtf_data;
            *self.mapping_cache.lock().unwrap() = mapping.clone();
        }
        // 【头像包索引回填】本轮是现场扫盘的话，把索引连同目录签名写回「扫描头像」缓存：
        // 下次替换同一目录只需比对目录签名（实测 28 万图约 0.15 s）即可复用，省掉整包重扫
        //（实测：旧口径单线程 14.6–39 s，改并行 + `file_type()` 后约 0.13 s）。
        // 【签名直接复用线程开头那一份，不再重算】替换自己只写 config.xml 与 config_backup_*.xml，
        // 两者都不入签名（见 PackSig 的文档），所以 want_sig 此刻仍然代表磁盘现状；
        // 若包在本轮被外部改动，这份签名会与磁盘不符 → 下次替换签不上 → 重扫（失败方向是安全的）。
        // 命中缓存的一轮 fresh_scan 为 None，不动缓存；失败/取消没有写出 config.xml，同样不动。
        if matches!(result.outcome, Outcome::Finished) {
            if let Some(scan) = result.scan {
                *self.faces_cache.lock().unwrap() = Some(FaceIndex {
                    img_dir: self.params.img_dir.clone(),
                    scan,
                    sig: want_sig,
                });
                self.logger
                    .info("replace", "头像包索引已回填缓存：下次替换同一目录可免重扫");
            }
        }
        // 快照预览的数据源也在收尾处刷新（与「读取快照」按钮写的是同一份缓存）：
        // 替换成功 → 把刚写好的 config.xml 备份到该档快照位置，并**直接用本次映射**填缓存，
        // 让快照预览无需再点「读取快照」就能看到本次结果；失败/取消不改 config.xml，快照保持原样
        //（已读过的那份仍然有效，所以这里只写不清）。
        if matches!(result.outcome, Outcome::Finished) {
            if let Some(snap_path) = self.snapshot_path.as_deref() {
                sync_group_snapshot(
                    snap_path,
                    &self.params.img_dir,
                    &mapping,
                    &self.snapshot_cache,
                    &*self.logger,
                );
            }
        }
        if matches!(result.outcome, Outcome::Finished) && mapping_empty {
            // 跑完却一条映射也没有：config.xml 会被写成空表，必须留 WARNING
            self.logger.warn("replace", "映射结果为空：本次写入的 config.xml 不含任何映射（多为「仅随机人」过滤掉了全部名单，或头像包内无图片）");
        }
        // 名单摘要（替换时后端已经解析过名单）：随事件回传前端，名单预览不必再点一次「扫描RTF」
        // 就知道名单格式；失败/取消为 None（与 rtf_cache 被写回空表一致）。
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
        // 收尾耗时（写两份缓存 + 同步快照）：这段在流水线「总耗时」之外，
        // 但它是 running 复位（按钮解禁）之前最后一段阻塞，单独记一条方便对比。
        self.logger.info(
            "replace",
            format!(
                "收尾完成：耗时 {:.1?}（写缓存 + 快照同步）",
                t_tail.elapsed()
            ),
        );
        // 【先复位、后 emit】见 RunningGuard::release 的说明：界面解锁只认这次 finished 事件，
        // 而 emit 可能把本线程按在主线程的收件队列上等一会儿——复位必须走在它前面。
        self._running.release();
        // 发「完成」事件（前端收到后刷新快照并弹提示）。
        let _ = self.app.emit(
            "replace:finished",
            serde_json::json!({ "result": result_str, "roster": roster }),
        );
    }

    /// 线程 panic 后的补救：留 ERROR、作废被毒化的缓存、补发 finished{failed} 让前端解锁界面。
    /// 【为什么还发事件】panic 路径上进度条停在最后那个百分比、`replacing` 却已随守卫复位，
    /// 不发完成事件的话前端没有机会把「替换中」的收尾态改成「失败」。
    fn on_panic(&self) {
        // 第一件事就是复位：下面的日志写入与 emit 都可能卡在主线程序列上（同 RunningGuard::release）。
        // panic 现场越复杂，界面越不该被锁着。
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
        let _ = self.app.emit(
            "replace:finished",
            serde_json::json!({ "result": "failed", "roster": null }),
        );
    }
}

/// 发起替换**之前**的预检：本次会不会把本档既有的映射清空，需要的话让前端先弹一句确认。
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
        pm.cur_group
            .as_ref()
            .map(|g| crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, g))
    };
    // 还没有档景 = 本档在这个包从没换过脸，生成覆盖不掉任何东西。
    let Some(path) = snapshot_path.filter(|p| p.is_file()) else {
        return Ok(none());
    };
    // 缓存里正是这份档景（用户点过「读取快照」，或刚替换完自动同步）时条数白拿。
    // 锁在这里就释放：下面那条解析要跑上秒级，抱着锁会把替换收尾的同一把锁一起堵住。
    let cached = {
        let cache = state.snapshot_cache.lock().unwrap();
        cache
            .as_ref()
            .filter(|c| c.xml_path == path.to_string_lossy())
            .map(|c| c.entries.len())
    };
    let existing = match cached {
        Some(n) => n,
        _ => {
            // 解析失败（档景读不出）直接 Err：预检的意义是「动手前把要丢多少说清楚」，
            // 说不清就不发起，而不是把「读不到」当成 0 条照常覆盖。
            let mut progress = TaskProgress::new(&app, "检查既有映射");
            let entries = xml_parser::parse_xml_entries_progress(&path, &mut |done, total| {
                progress.report(done, total)
            })?;
            progress.finish(); // 空档景/解析过快时进度条也得走满，别停在半路
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
pub fn start_replace(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ReplaceArgs,
) -> Result<(), String> {
    // swap(true) 原子地把 running 置 true 并返回旧值；旧值已是 true 说明有任务在跑，拒绝重入。
    if state.running.swap(true, Ordering::SeqCst) {
        return Err("已有替换任务在执行".into());
    }
    // 从这里起下面的每条提前 return、以及后台线程的整个生命周期，都由这个守卫负责复位。
    let running = RunningGuard(Arc::clone(&state.running));
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
        // 保留/覆写的判据＝**本档本组的存档快照**（不是头像包里的 live config.xml）：
        // live 是全档共享的一份，两档关联同一头像包时会残留别档内容（详见 ReplaceParams::source_xml）。
        source_xml: snapshot_path.clone(),
        rule_config,
    };
    // 后台线程需要的句柄交给 ctx（Arc::clone 只涨引用计数，不拷贝真实数据）；
    // running 守卫一起搬进去，线程函数返回（正常或 unwind）时由它复位。
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
    let uid = uid.trim().to_string();
    // 【与批量替换互斥】两边写的是同一个 live config.xml：流水线收尾整体重写，单人微调只改一行。
    // 并发进行就有一方的结果被无声吞掉，而两条路径都会报告成功——这里的拒绝是唯一能让用户看见的机会。
    if state.running.load(Ordering::SeqCst) {
        state
            .logger
            .warn("viewer", "单人换脸被拒绝：批量替换正在进行");
        return Err("批量替换正在进行，请等它完成或先取消，再做单人微调".into());
    }
    // 先取当前组目录（parse_image_path_in_pack 需要它来判定"图片是否在包根"）。
    let img_dir = {
        let pm = state.pm.lock().unwrap();
        pm.cur_group.clone().ok_or("当前没有选中组")?
    };
    let Some((pack, image)) = xml_parser::parse_image_path_in_pack(&img_dir, &image_path) else {
        return Err(
            "所选图片不在当前头像包内（或文件不存在），请从当前头像包目录或其子目录中选择图片"
                .into(),
        );
    };
    // 【先把 live 归位成「本档的档景」，再改】单人微调改的是全档共享的 live config.xml，而它的
    // 归属靠切档 restore 维持——restore 有三条静默跳过分支（该档在此目录无快照／目录不可用／
    // copy 失败），跳过时 live 里可能还是**别档**的内容。先按本档快照把 live 恢复一遍，这一行替换
    // 才确实落在本档的现有映射上，随后写回快照也不会把别档的内容一并带进来。
    // 【快照不存在＝本档在此目录还没有历史】此时 live 归谁无从判断，宁可不改：报错让用户先跑替换。
    let snap = {
        let pm = state.pm.lock().unwrap();
        crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, &img_dir)
    };
    if !snap.is_file() {
        state.logger.warn(
            "viewer",
            "单人换脸已跳过：当前档在此头像包目录下还没有存档快照",
        );
        return Err("当前存档在此头像包目录下还没有存档快照，无法确定本档的现有映射。请先执行一次替换后再微调。".into());
    }
    // 【原子恢复】目标是用户头像包里的 config.xml，直接 copy 是截断覆写：中途失败会让整包无脸，
    // 而这里的失败本就只 WARN 后继续（见下），更不该把用户文件写成半截。
    if let Err(e) = crate::json_store::copy_atomic(&snap, &Path::new(&img_dir).join("config.xml")) {
        // 恢复失败不致命：退化为「在 live 现况上改」（与改造前一致），但必须留下痕迹。
        state.logger.warn(
            "viewer",
            format!("单人换脸前恢复本档现有映射失败，将按头像包当前内容继续：{e}"),
        );
    }
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
        drop(cache); // 下面要锁 snapshot_cache，先放开这把，避免无谓的嵌套持锁
                     // 【把微调写回本档快照】快照才是「本档本组的现有映射」，替换的保留/覆写判据取自它
                     // （见 FaceMapper::existing_xml）。不写回的话下一轮替换仍按微调前的快照判断 →
                     // 刚改的这行会被当成「新球员」重抽，静默覆盖用户的调整。
        sync_live_to_snapshot(&state, &snap, &img_dir, &uid, &pack, &image);
    }
    if hit {
        state.logger.info(
            "viewer",
            format!(
                "单人换脸 {uid} → {}",
                xml_parser::record_from(&pack, &image)
            ),
        );
    }
    // 未命中时 single_replacement 已经 WARN 过「文件未改动」，这里不能再报成功。
    Ok(hit) // 返回是否命中（未命中 = config.xml 里本来就没这个 UID）
}

/// 单人微调后，把刚写出的 live `config.xml` 同步到 `snap`（当前档该组的存档快照），
/// 并就地更新快照预览缓存里那一条（缓存路径与 `snap` 一致才改）。
/// 【路径键】`snap` 必须来自 `paths::group_xml_path`，与快照预览、收尾同步共用同一口径。
fn sync_live_to_snapshot(
    state: &State<'_, AppState>,
    snap: &Path,
    img_dir: &str,
    uid: &str,
    pack: &str,
    image: &str,
) {
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
                    if let Some(e) = c.entries.iter_mut().find(|e| e.uid == uid) {
                        e.ethnicity = pack.to_string();
                        e.image = image.to_string();
                    }
                }
            }
        }
        Err(e) => state
            .logger
            .warn("viewer", format!("单人换脸同步存档快照失败：{e}")),
    }
}

/// 查询/预览（对等旧版 PlayerService 查询链）。按优先级从多个数据源拼出单人预览：
/// ① 最近一次替换的映射缓存 → ② 本档本组的存档快照（快照预览缓存命中同一路径时免整表解析）→
/// ③ 按映射在头像包目录里解析实际图片文件 → ④ 名单缓存补球员资料。
/// 【为什么是快照而不是 live config.xml】替换的保留/覆写判据已改读本档快照（见 ReplaceParams::source_xml），
/// 显示侧同源才不会出现「按别档的映射显示本档抽图结果」。
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
    // 2) 缓存没命中，就读「当前档该组的存档快照」找——与替换判据同一出处（ReplaceParams::source_xml）。
    //    【为什么不读 img_dir/config.xml】那份 live 是全档共享的：两档关联同一头像包时它可能还留着
    //    上一个档的内容（切档 restore 有静默跳过分支），显示侧按它取映射＝把别档的抽图结果显示成本档的。
    //    快照不存在＝本档在此目录没有历史 → 不猜（保持 None，与替换的「按无历史处理」一致）。
    if mapping.is_none() {
        if let Some(g) = &pm.cur_group {
            let snap = crate::paths::group_xml_path(&pm.user_cfg_path(), &pm.cur_prf, g);
            let snap_str = snap.to_string_lossy().to_string();
            // 快照预览缓存里正好是这份快照时直接查（路径命中）：大快照整表解析要 1 s 以上，
            // 而本命令是同步命令、跑在主线程上，能省则省。路径不符/缓存为空时再读文件兜底。
            mapping = {
                let cache = state.snapshot_cache.lock().unwrap();
                cache
                    .as_ref()
                    .filter(|c| c.xml_path == snap_str)
                    .and_then(|c| {
                        c.entries
                            .iter()
                            .find(|e| variants.contains(&e.uid))
                            .cloned()
                    })
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

/// 名单预览「头像」列：按一页的 UID 批量取头像图片的绝对路径（一次 IPC，避免逐行往返）。
/// 【出处与单人预览同源】复用 preview_player 的查找口径：映射缓存 → 本档本组的存档快照
/// （快照预览缓存命中同一路径时免整表解析）→ 读快照文件兜底；再按当前组解析出实际图片文件。
/// 【为什么是批量】RTF 名单里的「头像」列原文导出时恒为空（见 rtf_scanner 的 EXT_AVATAR_COL），
/// 头像只能按 UID 反查映射；一页 8 行逐行调用要走 8 次 IPC，且缓存未命中时会把整表解析重复 8 遍
/// （15 万行实测 1 s 以上）。这里把「读文件兜底」限制为整批最多解析一次。
/// 映射缺失 / 图片不在包内 / 未选组 → 该 UID 对应 null（前端留白）。
/// 【async】与三个扫描命令同款：未命中缓存时会读整表兜底（15 万行实测 1 s 以上），
/// 同步命令跑在主线程上会把界面一起堵住。函数内不跨 await 持锁，future 仍是 Send。
#[tauri::command]
pub async fn query_avatars(
    state: State<'_, AppState>,
    uids: Vec<String>,
) -> Result<std::collections::HashMap<String, Option<String>>, String> {
    // 先按「全部无头像」铺底；下面只覆盖命中的（未选组、没读过快照时就是这个结果）。
    let mut out: std::collections::HashMap<String, Option<String>> =
        uids.iter().map(|u| (u.clone(), None)).collect();
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
    let mut found: std::collections::HashMap<String, MappingEntry> =
        std::collections::HashMap::new();
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
                    if let Some(m) = xml_parser::with_newgan_prefix_variants(uid.trim())
                        .iter()
                        .find_map(|v| xml.get(v))
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
        out.insert(
            uid,
            xml_parser::resolve_image_file(root, &entry.ethnicity, &entry.image),
        );
    }
    Ok(out)
}

/// 名单预览取全量：rtf_scanner 解析一次进缓存，这里整份交给前端，分页/筛选/排序都在前端做。
/// 小工具口径：一次全量 IPC 比按页往返简单直接；名单至多数千条，开销可忽略。
#[tauri::command]
pub fn query_players(state: State<'_, AppState>) -> Vec<PlayerRecord> {
    state.rtf_cache.lock().unwrap().clone()
}

/// 按 uid 批量取球员记录（队列坞条目行的身份信息：年龄/所在地帮认出同名同姓者）。
/// 【为什么批量】队列坞原先对每个 uid 单独发一次 query_players(search=uid, limit=1)：
/// N 个条目 = N 次 IPC + N×M 线扫；批量一次 IPC、一遍哈希命中。语义与逐条等价——
/// 队列条目的 uid 来自名单记录本身（规范形），精确相等是逐条「全名包含」的子集。
/// 在锁内过滤、只克隆命中行（N 条），避免 clone 整份大名单；未命中的 uid 直接缺席，
/// 缓存未填上时返回空表（前端降级为仅姓名/UID 展示）。
#[tauri::command]
pub fn query_players_by_uids(state: State<'_, AppState>, uids: Vec<String>) -> Vec<PlayerRecord> {
    let want: std::collections::HashSet<&str> = uids.iter().map(|s| s.as_str()).collect();
    state
        .rtf_cache
        .lock()
        .unwrap()
        .iter()
        .filter(|r| want.contains(r.uid.as_str()))
        .cloned()
        .collect()
}

// ---------------- 快照预览（「读取快照」按钮 → 结果区「快照预览」Tab） ----------------

/// 「读取快照」：解析存档 XML 快照（.user/<存档>/<组>_snapshot.xml，config.xml 的存档副本）
/// 并缓存有序条目，供「快照预览」分页查询。xml_path 由前端传当前展示档的快照全路径（浏览态自洽）；
/// 每次点击都重读文件，展示的始终是快照文件的最新内容。
/// 【async】大快照（可达数万行）的正则解析跑在异步运行时线程，避免阻塞主线程导致界面卡住。
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
    let total = cache_snapshot(&state.snapshot_cache, &path, &mut |done, total| {
        progress.report(done, total)
    })?;
    state.logger.info(
        "scan",
        format!("读取快照完成：{total} 条映射，耗时 {:.1?}", t.elapsed()),
    );
    progress.finish(); // 收尾 100%（空 config.xml / 无映射时进度条也得走满）
    Ok(SnapshotInfo { total })
}

/// 解析快照文件并写入快照缓存，返回条目数。两条路径共用：「读取快照」命令（用户显式读，带进度上报），
/// 以及替换收尾的自动同步（sync_group_snapshot，传无操作回调）。缓存里存的路径即传入路径的字符串形式——
/// 前端两处拿到的都是同一套 `group_xml_path` 结果，快照预览查询时按此路径比对命中。
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

/// 替换收尾时同步该档的存档 XML 快照：把刚写入的 config.xml 备份到该档快照位置
/// （等价于切档时 save_group_xml 做的备份，保证「存档资料」里那份也是最新的），
/// 再把**本次映射直填**进快照缓存——快照预览因此无需再点「读取快照」就能看到本次结果。
///
/// 【为什么不回读刚写出的文件】`write_xml` 按**传入顺序**逐行写出、`parse_xml_entries` 又按
/// **文件顺序**返回，所以直填与「重解析一遍」逐条相同（`written_mapping_reparses_identically_in_order`
/// 守着这条等价性）。省掉的是一次整表正则解析（15 万行实测 1.4–1.5 s，而且卡住 running 复位＝按钮
/// 解禁）；更关键的是在 SMB/NFS（FM 玩家常把头像包放 NAS）或杀软实时扫描介入的目录上，
/// 「刚写完立刻读回来」还可能读到尚未完全落盘的内容——直填连这个不确定性一并去掉。
/// 【失败路径也少了一条】不再有解析，于是「解析失败 → 清空缓存」那段宁缺勿旧的分支自然消失。
/// 拷贝失败仍只记 WARNING、不返回 Err：快照新旧不影响本次替换的结论（config.xml 已写好）。
/// 【只写不清】失败/取消时调用方根本不会调它（config.xml 未被改写，已读的那份仍有效）。
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
            logger.write(
                log::Level::Warn,
                "replace",
                format!("创建快照目录失败：{e}"),
            );
            return;
        }
    }
    // 覆盖旧档景前先轮转备份到同目录（三处覆盖点同此一条路径，见 xml_parser::overwrite_snapshot）。
    if let Err(e) = xml_parser::overwrite_snapshot(&src, snapshot_path) {
        logger.write(
            log::Level::Warn,
            "replace",
            format!("同步存档快照失败：{e}"),
        );
        return;
    }
    // 直填：缓存里存的就是刚写进快照的那份映射，路径与「读取快照」命令写的一致（同一个 group_xml_path）。
    let total = mapping.len();
    *cache.lock().unwrap() = Some(SnapshotCache {
        xml_path: snapshot_path.to_string_lossy().to_string(),
        entries: mapping.to_vec(),
    });
    logger.info(
        "replace",
        format!(
            "同步存档快照完成：{total} 条映射（直填，未重解析），耗时 {:.1?}",
            t_sync.elapsed()
        ),
    );
}

/// 快照预览取全量条目（uid + 头像相对路径）。imagePath 留空——文件探测按页做，
/// 前端对当前页调 query_snapshot_images 批量解析（整表几万条逐一探测文件开销太大）。
#[tauri::command]
pub fn query_snapshot(
    state: State<'_, AppState>,
    xml_path: String,
) -> Result<Vec<SnapshotEntry>, String> {
    let mut cache = state.snapshot_cache.lock().unwrap();
    // 正常流程「读取快照」已填好缓存且路径一致；路径不符（缓存被换走）时按请求路径重读兜底。
    let hit = cache
        .as_ref()
        .map(|c| c.xml_path == xml_path)
        .unwrap_or(false);
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
    Ok(xml_parser::snapshot_entries(
        &cache.as_ref().unwrap().entries,
    ))
}

/// 按前端当前页批量解析快照头像的实际图片路径（与入参顺序一一对应；找不到为 null）。
/// 纯路径探测不碰缓存：resolve_avatar_path 覆盖 绝对路径/带扩展名/裸词干补扩展名 三种形态。
#[tauri::command]
pub fn query_snapshot_images(img_dir: String, rel_paths: Vec<String>) -> Vec<Option<String>> {
    let root = Path::new(&img_dir);
    rel_paths
        .iter()
        .map(|p| xml_parser::resolve_avatar_path(root, p))
        .collect()
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
        let (summary, data, _) =
            scan_rtf_core(rtf.to_str().unwrap(), &logger, &mut |_, _| {}).unwrap();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.newgan, 2); // 两条 UID 均带 r- 前缀
        assert!(!summary.detailed); // 基础名单无拓展信息
        assert_eq!(summary.format, "基础名单");
        assert_eq!(summary.language, "English");
        assert_eq!(data.len(), 2);
        // 非法 RTF → Err，文案带「RTF 格式不合法」
        let bad = dir.path().join("bad.rtf");
        std::fs::write(&bad, "乱七八糟没有数据行").unwrap();
        let err = scan_rtf_core(bad.to_str().unwrap(), &logger, &mut |_, _| {})
            .err()
            .unwrap();
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
        assert_eq!(
            c.xml_path,
            snap_path.to_str().unwrap(),
            "缓存路径即快照预览查询的比对键"
        );
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
        assert_eq!(
            expand_naming("gen-{uid}__{index}.v2", "p1", 12),
            "gen-p1__12.v2"
        );
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
        assert_eq!(
            next_free_target(&dir, "{uid}_{index}", "p1").unwrap(),
            dir.join("p1_1.png")
        );
        // 已有 p1_1 与 p1_3（计数=2 → index=3 撞上 p1_3）：递增避让到 p1_4，
        // 计数与避让两道保障在同一条路径上都生效。
        std::fs::write(dir.join("p1_1.png"), b"x").unwrap();
        std::fs::write(dir.join("p1_3.png"), b"x").unwrap();
        assert_eq!(
            next_free_target(&dir, "{uid}_{index}", "p1").unwrap(),
            dir.join("p1_4.png")
        );
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
        assert_eq!(
            find_tmp_image(&imgtmp, "abc").unwrap(),
            imgtmp.join("abc.webp")
        );
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
    Ok(state.image.lock().unwrap().clone())
}

/// 渲染 `{{变量|默认值}}` 模板文本（P1 无数据源，vars 恒为空 → 全部取默认值）。
/// 入参刻意用**文本**而非模板 ID：选中模板时对模板 body 渲染一次，生成前对用户编辑后的文本再渲染一次，
/// 这样用户的手工编辑不会被模板原文覆盖。返回 `missing` 非空表示存在「既无默认值、又无数据源」的占位符。
#[tauri::command]
pub fn render_prompt(
    body: String,
    vars: Option<HashMap<String, String>>,
) -> Result<image_ai::RenderOutcome, String> {
    Ok(image_ai::render(&body, &vars.unwrap_or_default()))
}

/// 按球员数据源渲染模板（P3）：uid 定位 rtf_cache 里的球员 → build_vars → render 模板 body。
/// 【短锁】两把锁各自「锁内只做查找/clone」即释放，锁后都是纯内存计算，绝不跨任何风险操作持锁。
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
    let rec =
        rec.ok_or_else(|| format!("名单中没有该球员（{uid}），请先扫描RTF名单后再预览提示词"))?;
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
pub fn save_provider(
    state: State<'_, AppState>,
    provider: image_ai::ProviderConfig,
) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::upsert_provider(&mut cfg, provider)?;
    cfg.save_providers(&state.data_dir)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub fn delete_provider(state: State<'_, AppState>, id: String) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::remove_provider(&mut cfg, &id)?;
    cfg.save_providers(&state.data_dir)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub fn set_provider_key(state: State<'_, AppState>, id: String, key: String) -> Result<(), String> {
    let mut keys = state.image_keys.lock().unwrap();
    if key.trim().is_empty() {
        keys.remove(&id);
    } else {
        keys.insert(id, key);
    }
    Ok(())
}

#[tauri::command]
pub fn save_prompt_template(
    state: State<'_, AppState>,
    template: image_ai::PromptTemplate,
) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::upsert_template(&mut cfg, template)?;
    cfg.save_templates(&state.data_dir)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub fn delete_prompt_template(state: State<'_, AppState>, id: String) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::remove_template(&mut cfg, &id)?;
    cfg.save_templates(&state.data_dir)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub async fn generate_image(
    state: State<'_, AppState>,
    req: image_ai::GenRequest,
) -> Result<image_ai::GenResult, image_ai::ImageGenError> {
    // await 前把配置 clone 出锁，不跨 await 持锁。
    // image 锁「即取即放」：语句末即释放，绝不与其后的 image_keys 嵌套持有（全仓单一锁序 image → image_keys → queue）。
    let cfg = state.image.lock().unwrap().clone();
    // 仅校验请求自身的完整性；厂商约束（尺寸/张数/提示词长度）由 provider 层解释，见 generate。
    image_ai::validate_complete(&cfg, &req)?;
    // 免 Key 协议（本地 ComfyUI）传空串占位：适配器不使用，见 ProviderKind::needs_key。
    let needs_key = cfg
        .provider(&req.provider_id)
        .map(|p| p.kind.needs_key())
        .unwrap_or(true);
    let key = if needs_key {
        state
            .image_keys
            .lock()
            .unwrap()
            .get(&req.provider_id)
            .cloned()
            .ok_or_else(|| {
                image_ai::ImageGenError::not_configured(
                    "请先输入 API Key（仅本会话记忆，不会保存）",
                )
            })?
    } else {
        String::new()
    };
    // 单张直发不展示阶段：空回调（阶段进度只有队列批处理会呈现在界面上）。
    image_ai::generate(&state.data_dir, &cfg, &key, req, &|_| {}).await
}

/// 读取持久画廊记录（data_dir/gallery.json），并逐条**现算**图片是否仍在会话临时目录。
/// 存在性只在查询期计算，绝不写回 gallery.json（写回会在下次启动即刻失真）。
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
    Ok(target_path)
}

/// 在 `.imgtmp` 里按 image_id 找源文件。生成图落盘名为 `{image_id}.{扩展名}`，扩展名由服务商
/// 响应决定，所以按 file_stem 遍历比对而不是写死 .png（对等 save_generated_image 的查找意图，
/// 但放宽到任意扩展名）。找不到＝图片已被会话清理或 id 传错。
fn find_tmp_image(tmp_dir: &Path, image_id: &str) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(tmp_dir)
        .map_err(|e| format!("读取生成图临时目录失败（{}）: {e}", tmp_dir.display()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.file_stem().and_then(|s| s.to_str()) == Some(image_id) {
            return Ok(p);
        }
    }
    Err(format!(
        "生成图中不存在该图片（{image_id}）：可能已被会话清理，请重新生成后再保存并替换"
    ))
}

/// 展开命名模板为文件主干：`{uid}` → 球员 UID，`{index}` → 序号；其余内容（含未知占位符）
/// 一律按字面量原样保留。只认这两个占位符是为了让「用户改模板」的行为可预期——
/// 未知的占位符不猜、不报错，原样落进文件名里，用户一眼就能看到并改掉。
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

/// 统计 `_generated` 目录里「属于该 uid」的已有文件数：file_stem 以 `{uid}_` 开头才算
/// （默认模板 `{uid}_{index}` 的产物都带这个前缀）。只数文件，同名目录与读目录失败一律跳过。
fn count_existing(dir: &Path, uid: &str) -> usize {
    let prefix = format!("{uid}_");
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let p = e.path();
                    p.is_file()
                        && p.file_stem()
                            .and_then(|s| s.to_str())
                            .is_some_and(|s| s.starts_with(&prefix))
                })
                .count()
        })
        .unwrap_or(0)
}

/// 计算本次落盘的目标路径：index 从「同 uid 已有数 + 1」起步，目标名已存在则继续递增。
/// 【为什么不能指望 copy_atomic 拒绝覆盖】copy_atomic 是覆盖写（rename 语义，见其实现），
/// 「不覆盖已有生成图」必须在这里显式保证；模板不含 {index} 时递增也腾不出新名字，
/// 有限次后报错而非静默覆盖——那种情况下同名即旧图，覆盖会无声丢掉用户之前保存的生成图。
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

/// 「保存并替换」闭环（P3）：把 `.imgtmp` 里的生成图落盘到 `<当前组>/_generated/`，
/// 再复用 `replace_single_face` 的完整单人换脸链（互斥检查/快照归位/config.xml 改行/缓存同步），
/// 不重写任何替换逻辑。返回落盘后的图片信息（前端据此更新结果卡）。
/// 【为什么是同步 fn】内部没有 await 点，重活是 replace_single_face 的同步 XML 链路；
/// 与它保持同形（Tauri 同步命令跑在命令线程池，不会卡 UI），不硬套 async。
#[tauri::command]
pub fn save_and_replace(
    state: State<'_, AppState>,
    image_id: String,
    uid: String,
    save_backup: bool,
) -> Result<image_ai::GenImage, String> {
    let uid = uid.trim().to_string();
    // ① 批量互斥前置检查：把失败挡在复制文件之前，磁盘上不会留下半成品。
    //    replace_single_face 内部还会再查一次 running（双保险，拒绝文案与其对齐）。
    if state.running.load(Ordering::SeqCst) {
        state
            .logger
            .warn("image_ai", "保存并替换被拒绝：批量替换正在进行");
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
        pm.cur_group
            .clone()
            .ok_or("请先关联头像包（当前没有选中组），再保存并替换")?
    };
    let gen_dir = Path::new(&img_dir).join("_generated");
    std::fs::create_dir_all(&gen_dir)
        .map_err(|e| format!("创建生成图目录失败（{}）: {e}", gen_dir.display()))?;
    // ⑤ 目标路径：命名模板展开 + 同名避让（见 next_free_target）；生成图统一 png。
    let target = next_free_target(&gen_dir, &pattern, &uid)?;
    json_store::copy_atomic(&src, &target)?;
    // ⑥ 复用单人换脸全链：互斥/快照归位/改 config.xml/同步 mapping_cache 与存档快照全部继承。
    //    返回的命中 bool 不再上报——未命中时 replace_single_face 内部已 WARN 留痕，
    //    图片本身已安全落盘，算不上本命令的失败。
    let target_str = target.to_string_lossy().to_string();
    let logger = state.logger.clone();
    replace_single_face(state, uid.clone(), target_str.clone(), save_backup)?;
    let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    logger.info(
        "image_ai",
        format!("保存并替换完成：{uid} → {target_str}（{bytes} 字节）"),
    );
    Ok(image_ai::GenImage {
        id: image_id,
        path: target_str,
        bytes,
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
/// 任一待入队条目不满足完整性/厂商约束即整批拒绝（与 start_queue 的「跑一半才失败」同样的
/// 防呆意图），返回该条目的结构化错误，队列保持不变。
#[tauri::command]
pub fn add_to_queue(
    state: State<'_, AppState>,
    entries: Vec<image_ai_queue::QueueEntry>,
) -> Result<Vec<image_ai_queue::QueueEntry>, image_ai::ImageGenError> {
    // 锁序固定为 image → queue（与 generate_image/start_queue 同序）；先 clone 出 cfg（语句末即释放），
    // 再进入 add_to_queue_inner 锁 queue，避免反序加锁把死锁面带进队列路径。
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

/// 启动批量生成：校验通过后把整个队列交给后台异步任务，命令本身立即返回。
/// 【为什么 async】run_queue 是异步的；用 tauri::async_runtime::spawn 让它跑在运行时上，
/// 进度经事件推送（gen:queue:progress / gen:queue:finished），前端另行 query_queue 拉快照。
#[tauri::command]
pub async fn start_queue(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), image_ai::ImageGenError> {
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
        if q.entries
            .iter()
            .any(|e| e.status == image_ai_queue::QueueStatus::Running)
        {
            return Err(image_ai::ImageGenError::internal("队列正在执行中"));
        }
        // 启动前逐条校验 Queued 条目的服务商 Key：任一缺失就整批不启动（跑一半才失败
        // 会白白烧掉前几条的配额）。Key 只在本会话内存（见 set_provider_key）。
        // 免 Key 协议（本地 ComfyUI）直接放行，口径与 generate_image 一致（ProviderKind::needs_key）。
        for e in q
            .entries
            .iter()
            .filter(|e| e.status == image_ai_queue::QueueStatus::Queued)
        {
            let needs_key = cfg
                .provider(&e.req.provider_id)
                .map(|p| p.kind.needs_key())
                .unwrap_or(true);
            if !needs_key {
                continue;
            }
            let ok = keys
                .get(&e.req.provider_id)
                .is_some_and(|k| !k.trim().is_empty());
            if !ok {
                return Err(image_ai::ImageGenError::not_configured(
                    "请先输入该服务商 Key",
                ));
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
        let (finished, failed, total) = image_ai_queue::run_queue(
            queue,
            data_dir,
            cfg,
            keys,
            concurrency,
            executor,
            sleeper,
            sink,
        )
        .await;
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
    Ok(())
}

/// 查询队列全量快照（条目状态/图片/错误都从这里拉）。
#[tauri::command]
pub fn query_queue(state: State<'_, AppState>) -> Result<Vec<image_ai_queue::QueueEntry>, String> {
    Ok(state.queue.lock().unwrap().snapshot())
}
