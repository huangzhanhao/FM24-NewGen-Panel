//! Tauri 命令层（对等旧版 services/* + app.py）：前端 invoke 的全部入口。
//!
//! 【设计】数据留在 Rust，IPC 只传摘要/分页：大名单与日志在缓存里按页查询，
//! 每次状态变更后回传 AppSnapshot 让前端整体刷新。命令可能在不同线程被调用、后台替换
//! 也跑在单独线程，因此 AppState 的共享字段用 Mutex 保护、跨线程引用用 Arc、
//! 运行/取消标志用 AtomicBool。返回 Result<T, String> 时，Err 的中文描述由前端直接弹窗展示。
use crate::face_mapper::Mode;
use crate::facepack_scanner::{self, IntegrityResult, PackScan, PackSig};
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
    pub faces_cache: Arc<Mutex<Option<FaceIndex>>>, // 「扫描头像」的包索引缓存（替换可复用）
    pub snapshot_cache: Arc<Mutex<Option<SnapshotCache>>>, // 快照解析缓存（快照预览分页复用；「读取快照」与替换收尾都会写）
    pub data_dir: PathBuf, // 便携数据目录（标准规则 data/rules/ 在其下）
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

/// 「扫描RTF」核心：格式识别 + 全量解析名单（全字段入库），返回 (摘要, 记录列表)。
/// 识别失败 → Err("RTF 格式不合法：…")，文案带首个不合法字段定位，前端弹窗直接展示。
fn scan_rtf_core(
    rtf_path: &str,
    logger: &impl LogSink,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(RosterSummary, Vec<PlayerRecord>), String> {
    let mut scanner = RtfScanner::new();
    scanner.detect_format(Path::new(rtf_path), logger)?;
    // filter=false：扫描展示全量名单（Replace 流程才按「仅随机人」过滤）。
    let data = scanner.parse_roster_progress(Path::new(rtf_path), false, logger, progress)?;
    // 摘要口径统一收在 RosterSummary（随机人前缀判定、拓展信息判定都不在这里重复一遍）。
    Ok((RosterSummary::from_records(&scanner, &data), data))
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
    let (summary, data) = scan_rtf_core(&rtf, &*state.logger, &mut |done, total| {
        progress.report(done, total)
    })?;
    *state.rtf_cache.lock().unwrap() = data; // 名单预览（query_players）直接读这份缓存（全字段）
    state.logger.info(
        "scan",
        format!(
            "扫描 RTF 完成：{} 条（随机人 {}，{}，{}），耗时 {:.1?}",
            summary.total,
            summary.newgan,
            summary.format,
            summary.language,
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
        pre_scan: None, // 由后台线程按「缓存命中 + 目录签名」填，见下
        // 保留/覆写的判据＝**本档本组的存档快照**（不是头像包里的 live config.xml）：
        // live 是全档共享的一份，两档关联同一头像包时会残留别档内容（详见 ReplaceParams::source_xml）。
        source_xml: snapshot_path.clone(),
        rule_config,
    };
    // 后台线程需要的句柄先 clone 出来（Arc::clone 只涨引用计数，不拷贝真实数据）。
    let (cancel, running) = (state.cancel.clone(), state.running.clone());
    let (rtf_cache, mapping_cache) = (state.rtf_cache.clone(), state.mapping_cache.clone());
    let snapshot_cache = state.snapshot_cache.clone(); // 替换成功时同步刷新（快照预览的数据源）
    let faces_cache = state.faces_cache.clone(); // 复用前比对 + 现场扫盘后回填，都在这份缓存上
    let logger = state.logger.clone();
    std::thread::spawn(move || {
        // 【签名比对为什么挪进线程】pack_sig 不递归，但要读完包根**并逐个一级目录数一遍直接子条目**
        // （标准三 28 万图实测 0.15–0.19 s）。同步命令跑在主线程上，留在那里会让点击「替换」后
        // 先有约 0.2 s 的无响应；挪进来既不阻塞界面，日志顺序也自然（签名 → 开始替换 → 索引完成）。
        let mut params = params;
        let t_sig = Instant::now();
        let want_sig = facepack_scanner::pack_sig(&params.img_dir);
        let reuse = faces_cache
            .lock()
            .unwrap()
            .as_ref()
            .filter(|c| c.img_dir == params.img_dir)
            .filter(|c| c.sig.is_some() && c.sig == want_sig)
            .map(|c| c.scan.clone());
        logger.info(
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
        params.pre_scan = reuse;
        let emit_app = app.clone();
        // 跑流水线；progress 回调里通过 emit 把进度发给前端（事件名 replace:progress）。
        // 流水线关键步骤的日志经 logger 记录，由 log:batch 事件实时回显前端日志面板。
        // 阶段内部按项上报（逐球员抽图可达数万次），去重后才 emit——见 ProgressDedup。
        let mut dedup = ProgressDedup { last: None };
        let result = replace_flow::run(
            &params,
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
            &cancel,
            &*logger, // &Arc<Logger> 显式解引用成 &Logger（泛型参数处不会自动解强制引用）
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
        *rtf_cache.lock().unwrap() = result.rtf_data;
        *mapping_cache.lock().unwrap() = mapping.clone();
        // 【头像包索引回填】本轮是现场扫盘的话，把索引连同目录签名写回「扫描头像」缓存：
        // 下次替换同一目录只需比对目录签名（实测 28 万图约 0.15 s）即可复用，省掉整包重扫
        //（实测：旧口径单线程 14.6–39 s，改并行 + `file_type()` 后约 0.13 s）。
        // 【签名直接复用线程开头那一份，不再重算】替换自己只写 config.xml 与 config备份_*.xml，
        // 两者都不入签名（见 PackSig 的文档），所以 want_sig 此刻仍然代表磁盘现状；
        // 若包在本轮被外部改动，这份签名会与磁盘不符 → 下次替换签不上 → 重扫（失败方向是安全的）。
        // 命中缓存的一轮 fresh_scan 为 None，不动缓存；失败/取消没有写出 config.xml，同样不动。
        if matches!(result.outcome, Outcome::Finished) {
            if let Some(scan) = result.scan {
                *faces_cache.lock().unwrap() = Some(FaceIndex {
                    img_dir: params.img_dir.clone(),
                    scan,
                    sig: want_sig,
                });
                logger.info("replace", "头像包索引已回填缓存：下次替换同一目录可免重扫");
            }
        }
        // 快照预览的数据源也在收尾处刷新（与「读取快照」按钮写的是同一份缓存）：
        // 替换成功 → 把刚写好的 config.xml 备份到该档快照位置，并**直接用本次映射**填缓存，
        // 让快照预览无需再点「读取快照」就能看到本次结果；失败/取消不改 config.xml，快照保持原样
        //（已读过的那份仍然有效，所以这里只写不清）。
        if matches!(result.outcome, Outcome::Finished) {
            if let Some(snap_path) = snapshot_path.as_deref() {
                sync_group_snapshot(
                    snap_path,
                    &params.img_dir,
                    &mapping,
                    &snapshot_cache,
                    &*logger,
                );
            }
        }
        if matches!(result.outcome, Outcome::Finished) && mapping_empty {
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
        // 收尾耗时（写两份缓存 + 同步快照）：这段在流水线「总耗时」之外，
        // 但它是 running 复位（按钮解禁）之前最后一段阻塞，单独记一条方便对比。
        logger.info("replace", format!(
            "收尾完成：耗时 {:.1?}（写缓存 + 快照同步）", t_tail.elapsed()));
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
        state
            .logger
            .warn("viewer", "单人换脸已跳过：当前档在此头像包目录下还没有存档快照");
        return Err("当前存档在此头像包目录下还没有存档快照，无法确定本档的现有映射。请先执行一次替换后再微调。".into());
    }
    if let Err(e) = std::fs::copy(&snap, Path::new(&img_dir).join("config.xml")) {
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
    state.logger.info(
        "viewer",
        format!(
            "单人换脸 {uid} → {}",
            xml_parser::record_from(&pack, &image)
        ),
    );
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
    match std::fs::copy(Path::new(img_dir).join("config.xml"), snap) {
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
    state
        .logger
        .info("scan", format!("读取快照完成：{total} 条映射，耗时 {:.1?}", t.elapsed()));
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
    if let Err(e) = std::fs::copy(&src, snapshot_path) {
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
    logger.info("replace", format!(
        "同步存档快照完成：{total} 条映射（直填，未重解析），耗时 {:.1?}", t_sync.elapsed()));
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
        let (summary, data) =
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
        sync_group_snapshot(&snap_path, img.to_str().unwrap(), &entries, &cache, &logger);

        assert!(snap_path.is_file()); // 快照文件已生成（父目录一并补建）
        let got = cache.lock().unwrap();
        let c = got.as_ref().expect("快照缓存应被填充");
        assert_eq!(c.xml_path, snap_path.to_str().unwrap()); // 路径即快照预览查询的比对键
        assert_eq!(c.entries.len(), 1);
        assert_eq!(c.entries[0].uid, "r-1001");
    }

    #[test]
    fn sync_group_snapshot_fills_cache_from_mapping_not_by_reparsing() {
        // 缓存内容必须来自**传入的映射**，而不是回读刚写出的文件。
        // 这里故意让盘上的 config.xml 与传入映射不一致：若实现退回「重解析」，缓存里会是盘上那份。
        let tmp = tempfile::tempdir().unwrap();
        let img = tmp.path().join("pack");
        std::fs::create_dir_all(&img).unwrap();
        xml_parser::write_xml(
            &[MappingEntry { uid: "r-9999".into(), ethnicity: "Asian".into(), image: "stale".into() }],
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
        // 快照文件本身仍是「盘上 config.xml 的副本」（拷贝语义不变，与缓存内容无关）。
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
}
