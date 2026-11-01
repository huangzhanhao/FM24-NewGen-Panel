//! AI 文生图批量生成队列：条目状态机 + 并发调度执行器。
//!
//! 【与既有模块的复用关系】生成动作直接复用 `image_ai::generate`（写盘 + 画廊记录），
//! 本模块只负责「把一批请求按并发上限跑完」：条目在 Queued → Running → Success/Failed
//! 之间流转，全程通过事件把进度推给前端（仿 replace 流水线的 emit 先例）。
//!
//! 【两个可测性 Seam】单测拿不到 `AppHandle` 也不能触网，故：
//! - 事件出口抽成 [`EventSink`] trait：真实实现 [`AppEventSink`] emit 到 Tauri 事件总线，
//!   测试用收集器替身（`run_queue` 的签名因此收 sink 而非 AppHandle）；
//! - 生成动作抽成 [`GenExecutor`] trait（对象安全，`Pin<Box<dyn Future>>` 形式，
//!   仿 image_ai_providers 的 `ImageProvider`）：真实实现 [`RealExecutor`] 调
//!   `image_ai::generate`，测试用返回预置结果的替身。
//!
//! 【锁纪律】队列状态是 `Arc<Mutex<QueueState>>`（std Mutex）：所有读写都是
//! 「短锁」（lock → 改 → 立即 unlock），锁内绝不 await，仿 commands.rs 的既有纪律。
//! 【并发控制】不引新依赖：用 `Arc<Mutex<u8>>` 占位计数 + 主循环 50ms 轮询空位
//! （等待用 `spawn_blocking + thread::sleep`，与 dashscope 轮询同款）。

use crate::image_ai;
use super::types::{CancelToken, GenStage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// 生成一批前的占位轮询间隔：50ms 足够细（单次生成秒级耗时），又不至于空转烧 CPU。
const SLOT_POLL: Duration = Duration::from_millis(50);

/// 单条任务的最大尝试次数（1 次首发 + 2 次重试）。仅对 `retryable` 错误重试。
/// 【单条目最坏墙钟口径】= 尝试数 × 单次尝试耗时 + 退避之和；单次尝试耗时随厂商不同
/// （异步厂商含 submit + 轮询预算 + 在途 GET，见 dashscope 轮询处注释），退避由
/// [`backoff_delay`] 给出（2s + 4.25s，若带 `Retry-After` 则各档可到 30s 上限）。
const MAX_ATTEMPTS: u32 = 3;

/// 重试退避纯函数：`2s × 2^n`（n = 已重试次数，从 0 起）+ 确定性抖动，单次上限 30s；
/// 服务端给了 `Retry-After`（秒）时优先取它（同样受 30s 上限钳制）。
/// 【为什么抖动是确定性的】零新增依赖下无随机源；确定性数值才可被单测精确断言。
/// n=0 抖动为 0，故首次退避恰为 2s（无 Retry-After 时）。
pub fn backoff_delay(attempt: u32, retry_after: Option<u64>) -> Duration {
    const BASE_MS: u64 = 2000;
    const CAP: Duration = Duration::from_secs(30);
    if let Some(secs) = retry_after {
        return Duration::from_secs(secs).min(CAP);
    }
    let exp = BASE_MS.saturating_mul(1u64 << attempt.min(20));
    let jitter_ms = (attempt as u64 * 250).min(1000);
    Duration::from_millis(exp.saturating_add(jitter_ms)).min(CAP)
}

/// 进度事件名（前端 listen("gen:queue:progress")）。
pub const GEN_QUEUE_PROGRESS_EVENT: &str = "gen:queue:progress";
/// 收尾事件名（前端 listen("gen:queue:finished")）。
pub const GEN_QUEUE_FINISHED_EVENT: &str = "gen:queue:finished";

/// 队列条目状态。小写序列化与前端字面量（"queued"/"running"/…）一致。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QueueStatus {
    Queued,
    Running,
    Success,
    Failed,
}

/// 一条批量生成任务。经 IPC 往返（前端提交 / 查询快照返回），故 Serialize + Deserialize。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueEntry {
    /// 队列条目 id。**由后端 `add` 时统一用 `image_ai::new_id()` 生成**：唯一性由单一来源
    /// 负责，前端不需要（也无法可靠地）保证不撞 id；提交后的 snapshot 带回真实 id 供
    /// 移除/取消定位。
    pub id: String,
    /// 球员 UID（P3 数据源用，本任务仅透传；进度事件带出供前端定位行）。
    pub uid: String,
    /// 球员名（展示用）。
    pub name: String,
    /// 生成请求（透传给 `image_ai::generate`）。
    pub req: image_ai::GenRequest,
    pub status: QueueStatus,
    /// 成功后回填的临时图片元信息（`skip_serializing_if` 让空态不出现在 JSON；
    /// `default` 让前端提交的新条目可以不带这两个字段）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<image_ai::GenImage>,
    /// 失败原因（中文，直接展示）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// 失败类别：`error` 展示串的结构化标注，供前端决定是否展示「重试本条」并给处置建议。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<image_ai::GenErrorKind>,
    /// 已尝试次数（口径：执行前 +1，成功/失败都反映真实值；入队未跑时为 0）。
    /// 前端据此在 `> 1` 时显示「已重试 N 次」。
    #[serde(default)]
    pub attempts: u32,
}

/// 队列共享状态：条目列表 + 取消令牌。
/// `cancel` 用 [`CancelToken`]（原子布尔 + Arc）而非裸 bool：取消要在**多个派发决策点**
/// 读到同一个值（主循环派发前、重试退避后、generate 发 HTTP 前），裸 bool 只能在锁内读；
/// 它与 `entries` 各管一件事，不需要同一把锁。
///
/// 【语义】取消只拦「尚未提交」的请求：未派发的条目直接判失败，已发出的那一次等它返回、
/// 结果照常落盘记画廊（已计费，中途丢弃等于白花钱）。
#[derive(Default)]
pub struct QueueState {
    pub entries: Vec<QueueEntry>,
    pub cancel: CancelToken,
    /// 本批次实际派发的条目 id（run_queue 开跑时写入、收尾清空）。
    /// 取消只落这些条目：未参与本批次的排队条目必须原样留在队列里。
    pub active: Vec<String>,
}

impl QueueState {
    /// 追加一批条目：id 一律重生成、状态一律复位为 Queued。
    ///
    /// 同 uid 只保留一条排队任务：新入列**替换**此前入列的那条排队条目（占其位，队列顺序稳定），
    /// 无需提示。在跑/终态条目不受影响——「重复入队」针对的是还没跑的任务，历史结果不该被抹掉；
    /// 同一批内的重复 uid 同样后者胜（后入列的逐个顶掉先入列的）。
    pub fn add(&mut self, mut entries: Vec<QueueEntry>) {
        for e in entries.iter_mut() {
            e.id = image_ai::new_id();
            e.status = QueueStatus::Queued;
            e.error = String::new();
            e.error_kind = None;
            e.attempts = 0; // 尚未尝试（口径见 QueueEntry::attempts）
        }
        for e in entries {
            // uid 为空的条目（理论上不会出现）没有「同一个球员」的语义，不参与去重。
            let slot = if e.uid.is_empty() {
                None
            } else {
                self.entries
                    .iter()
                    .position(|old| old.status == QueueStatus::Queued && old.uid == e.uid)
            };
            match slot {
                Some(i) => self.entries[i] = e,
                None => self.entries.push(e),
            }
        }
    }

    /// 按 id 批量移除（在跑的条目被移除后，其任务收尾的状态回写会因找不到 id 而自然失效）。
    pub fn remove(&mut self, ids: &[String]) {
        self.entries.retain(|e| !ids.iter().any(|id| id == &e.id));
    }

    /// 清空队列（不动 cancel 标志：run_queue 开跑时会自行复位它）。
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// 快照：整份克隆给 IPC（条目量级是「一批要生成的球员」，克隆代价可忽略）。
    pub fn snapshot(&self) -> Vec<QueueEntry> {
        self.entries.clone()
    }

    /// 取消本批次里「尚未派发」的条目：只碰 `active` 中的 Queued 条目，
    /// 未参与本批次的排队条目不受影响。返回标记条数。
    ///
    /// 【为什么不能扫全部 Queued】队列里可能留着上一轮没勾选的排队条目（`start_queue(ids)`
    /// 只跑选中项），它们被一起标成「已取消」就等于用户没参与本批的任务凭空消失了。
    /// 幂等：已标记的条目不再是 Queued，重复调用返回 0。
    pub fn cancel_pending(&mut self) -> usize {
        let err = image_ai::ImageGenError::cancelled();
        let mut marked = 0usize;
        for e in self.entries.iter_mut() {
            if e.status != QueueStatus::Queued || !self.active.iter().any(|id| id == &e.id) {
                continue;
            }
            // attempts 不动：这些条目一次都没执行过（口径见 QueueEntry::attempts）。
            e.status = QueueStatus::Failed;
            e.error = err.message.clone();
            e.error_kind = Some(err.kind);
            marked += 1;
        }
        marked
    }
}

/// 进度事件负载（camelCase 与前端契约一致）。
/// status 为 "running" | "success" | "failed"；current 是**已完成数**（终态条目的累计数，
/// 不是派发序号）；stage 仅运行中的阶段事件带值（submitting/polling/downloading）；
/// entryId 是条目唯一 id（uid 可能因重复入队而重复，故阶段定位用它）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueProgress {
    pub status: String,
    pub current: u32,
    pub total: u32,
    pub uid: String,
    /// 事件所属队列条目的唯一 id（同一球员重复入队时 uid 会重复、id 不会），前端据此定位阶段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

/// 收尾事件负载：finished=成功数、failed=失败数（含「已取消」）、total=本次启动的条目总数。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueFinished {
    pub finished: u32,
    pub failed: u32,
    pub total: u32,
}

/// 事件出口 Seam：run_queue 不直接持有 AppHandle，经此 trait 发事件——
/// 单测用收集器替身即可断言/丢弃事件（真实实现见 [`AppEventSink`]）。
pub trait EventSink: Send + Sync {
    fn progress(&self, payload: QueueProgress);
    fn finished(&self, payload: QueueFinished);
}

/// 真实实现：emit 到 Tauri 事件总线。emit 失败（前端窗口已关等）只静默丢弃，
/// 不能让「通知界面」反过来弄死生成任务本身。
pub struct AppEventSink(pub AppHandle);

impl EventSink for AppEventSink {
    fn progress(&self, payload: QueueProgress) {
        let _ = self.0.emit(GEN_QUEUE_PROGRESS_EVENT, payload);
    }
    fn finished(&self, payload: QueueFinished) {
        let _ = self.0.emit(GEN_QUEUE_FINISHED_EVENT, payload);
    }
}

/// 执行器返回的 future 形态：真实实现与各测试替身共用，省掉每处重抄 6 行 `Pin<Box<dyn Future…>>`。
type ExecFuture<'a> =
    Pin<Box<dyn Future<Output = Result<image_ai::GenResult, image_ai::ImageGenError>> + Send + 'a>>;

/// 生成执行器 Seam：对象安全（`Pin<Box<dyn Future>>` 返回形式，仿 ImageProvider），
/// 让 run_queue 的测试用替身而不触网。`progress` 为适配器阶段回调，原样透传给 `generate`。
pub trait GenExecutor: Send + Sync {
    fn execute<'a>(
        &'a self,
        data_dir: &'a Path,
        cfg: &'a image_ai::Config,
        key: &'a str,
        req: &'a image_ai::GenRequest,
        progress: &'a (dyn Fn(GenStage) + Sync),
        cancel: &'a CancelToken,
    ) -> ExecFuture<'a>;
}

/// 真实执行器：直接调 `image_ai::generate`（写盘 + 画廊记录都在那一条链上）。
pub struct RealExecutor;

impl GenExecutor for RealExecutor {
    fn execute<'a>(
        &'a self,
        data_dir: &'a Path,
        cfg: &'a image_ai::Config,
        key: &'a str,
        req: &'a image_ai::GenRequest,
        progress: &'a (dyn Fn(GenStage) + Sync),
        cancel: &'a CancelToken,
    ) -> ExecFuture<'a> {
        // generate 按值收请求：克隆一份（一次生成的请求体就几行提示词，克隆可忽略）。
        Box::pin(image_ai::generate(
            data_dir,
            cfg,
            key,
            req.clone(),
            progress,
            cancel,
        ))
    }
}

/// 睡眠 Seam：让重试退避在测试里不真睡（仿 [`EventSink`]/[`GenExecutor`]）。
pub trait Sleeper: Send + Sync {
    fn sleep<'a>(&'a self, d: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// 真实实现：async_runtime 未直接导出 sleep（零新增依赖前提），故在 spawn_blocking 里睡。
pub struct RealSleeper;

impl Sleeper for RealSleeper {
    fn sleep<'a>(&'a self, d: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d))
                .await
                .ok();
        })
    }
}

/// 入队前的整批校验：逐条做厂商无关完整性 + 协议约束校验，任一条不合规则整批拒绝。
/// 【为什么放这里】「不合规入队即失败、从不发出 HTTP」是队列的准入契约；add_to_queue 在取
/// queue 锁之前调用本函数，保证不合规时队列内容完全不变。
/// 【为什么只调两个既有函数】两层校验的口径（完整性 + 协议约束）已由 `validate_complete` 与
/// `validate_for_generate` 各自收口；此处重复查询服务商/模型并自组探针等于把同一份判据抄第二份，
/// 一旦 `generate` 调整探针形状就会与入队校验悄悄分叉。
pub fn validate_entries(
    cfg: &image_ai::Config,
    entries: &[QueueEntry],
) -> Result<(), image_ai::ImageGenError> {
    entries.iter().try_for_each(|e| {
        image_ai::validate_complete(cfg, &e.req)?;
        image_ai::validate_for_generate(cfg, &e.req)
    })
}

/// 手动确认清单写回一条条目参数的入参：条目 id + 新的生成请求。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct QueueEditArg {
    pub id: String,
    pub req: image_ai::GenRequest,
}

/// 只覆盖 Queued 条目的 req；非 Queued 或不存在的 id 原样跳过。返回实际写入条数。
///
/// 【为什么这里不校验参数合法性】写回的只是「用户在原有表单里核对过的那份参数」，一份坏参数
/// 不会悄悄发出去：`start_queue` 之前的 [`validate_entries`] 会整批拦下不合规条目，执行期
/// `validate_for_generate` 再按厂商能力位判一次。在这里另抄一套硬性下限等于养第二份判据——
/// 前端放行而后端拒绝只会变成「确认点不动、又看不出哪项错」的死路。
/// 【整批一次性写入】调用方（`update_queue_entries`）持队列锁跑完整个批次，对本进程其他 IPC
/// 而言这批写回是原子的：不会出现「改了一半」被 `query_queue` 读到的中间态。
/// 【身份字段】`uid`/`name`/`template_id` 以入队时的值为准（前端传空即保留队列原值）：
/// 它们决定画廊归属，不属于这次可改的生成参数。
pub fn apply_queue_edits(entries: &mut [QueueEntry], edits: &[QueueEditArg]) -> usize {
    let is_blank = |v: &Option<String>| v.as_deref().unwrap_or("").trim().is_empty();
    let mut applied = 0usize;
    for edit in edits {
        // 只写排队中的条目：在跑/已收尾的条目参数改了也不会被采用，静默跳过。
        let Some(target) = entries
            .iter_mut()
            .find(|e| e.id == edit.id && e.status == QueueStatus::Queued)
        else {
            continue;
        };
        let mut req = edit.req.clone();
        if is_blank(&req.uid) {
            req.uid = target.req.uid.clone();
        }
        if is_blank(&req.name) {
            req.name = target.req.name.clone();
        }
        if is_blank(&req.template_id) {
            req.template_id = target.req.template_id.clone();
        }
        target.req = req;
        applied += 1;
    }
    applied
}

/// 并发占位：派发侧 [`SlotGuard::acquire`] 即 +1，任务收尾由 `Drop` 释放。
///
/// 【为什么是 RAII】任务体若在收尾前 panic，末尾的减计数不会执行，占位就永久泄漏；
/// 条目数大于并发数时，派发循环会卡在等空位处再也不会前进（连取消标志都轮不到读）。
/// `Drop` 在栈展开时照样执行，故 panic 也不会漏。
struct SlotGuard(Arc<Mutex<u8>>);

impl SlotGuard {
    fn acquire(in_flight: &Arc<Mutex<u8>>) -> Self {
        *in_flight.lock().unwrap() += 1;
        Self(Arc::clone(in_flight))
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        // 不再 unwrap：任务 panic 已让锁中毒时，Drop 里二次 panic 会直接 abort 进程。
        if let Ok(mut n) = self.0.lock() {
            *n = n.saturating_sub(1);
        }
    }
}

/// 单条任务共享的环境：`run_queue` 构造一次，各任务按 `Arc` 共享，
/// 免得每个任务各抄 9 行 `Arc::clone`。
struct EntryCtx {
    state: Arc<Mutex<QueueState>>,
    completed: Arc<Mutex<u32>>,
    /// 事件分母（本次启动的条目总数），全程不变。
    total: u32,
    data_dir: PathBuf,
    cfg: Arc<image_ai::Config>,
    keys: Arc<HashMap<String, String>>,
    /// 取消令牌：与 `QueueState.cancel` 是同一个（Arc 共享），供「退避结束后是否还发起
    /// 这次重试」的判断读（真实执行链另在 generate 发 HTTP 前读它，见 `CancelToken` 文档）。
    cancel: CancelToken,
    executor: Arc<dyn GenExecutor>,
    sleeper: Arc<dyn Sleeper>,
    sink: Arc<dyn EventSink>,
}

/// 跑完队列里所有 Queued 条目，返回 (finished, failed, total)。
///
/// - 并发上限 `concurrency`（0 视为 1）：主循环派发即占位（[`SlotGuard`] 计数），任务收尾由
///   `Drop` 释放（panic 也照样释放）；空位不足时 50ms 轮询等待。占位在派发侧而非任务开头——
///   spawn 到任务真正起跑之间有调度窗口，若等任务自己占位，主循环会在窗口里超发。
/// - cancel==true 时：尚未派发的条目直接置 Failed(「已取消」)，在跑的等它自然收尾。
///   开跑时先把 cancel 复位 false（上一次运行遗留的标志不该取消这一次）。取消同样对
///   「执行期重试」生效：退避结束后不再发起新一次重试（见任务体内重试循环）。
/// - 每条任务开始/成功/失败都发 `gen:queue:progress`；全部收尾后发 `gen:queue:finished`。
///   事件里的 `current` 是**已完成数**（终态条目累计，非派发序号）；运行中另有阶段事件
///   （`stage` = submitting/polling/downloading），由适配器回调触发、按「阶段」去重。
/// - 执行期重试在**任务体内部**做（最多 `MAX_ATTEMPTS` 次，仅对 `retryable` 错误），
///   重试期间条目保持 Running；退避经 `sleeper` 睡（测试传替身，不真睡）。
/// - `only`：只跑这些 id（按勾选启动）。`None` = 全部 Queued；非 Queued 的 id 自然跳过，
///   未选中的条目保持 Queued 留在队列里。
pub async fn run_queue(
    state: Arc<Mutex<QueueState>>,
    data_dir: PathBuf,
    cfg: image_ai::Config,
    keys: HashMap<String, String>,
    concurrency: u32,
    only: Option<Vec<String>>,
    executor: Arc<dyn GenExecutor>,
    sleeper: Arc<dyn Sleeper>,
    sink: Arc<dyn EventSink>,
) -> (u32, u32, u32) {
    // 短锁：复位 cancel + 摘出本次要跑的 Queued 条目 id（总量即 total，事件分母固定）。
    let (ids, total, cancel) = {
        let mut st = state.lock().unwrap();
        st.cancel.reset();
        let cancel = st.cancel.clone();
        let allow: Option<std::collections::HashSet<&str>> =
            only.as_ref().map(|v| v.iter().map(String::as_str).collect());
        let ids: Vec<String> = st
            .entries
            .iter()
            .filter(|e| e.status == QueueStatus::Queued)
            .filter(|e| allow.as_ref().is_none_or(|a| a.contains(e.id.as_str())))
            .map(|e| e.id.clone())
            .collect();
        let total = ids.len() as u32;
        // 记下本批次真正派发的条目：取消只落这些，未参与本批次的排队条目原样留在队列里。
        st.active = ids.clone();
        (ids, total, cancel)
    };
    // 0 视为 1；占位计数器是 u8（单机同时跑 255 张图不现实），更高配置一并截断。
    let concurrency = concurrency.clamp(1, u8::MAX as u32) as u8;
    // 执行叙事的起点（收尾汇总在命令层 spawn 包装里）；并发记 clamp 后的实际生效值。
    log::info!(target: "image_ai", "批量生成开始：{total} 条任务（并发 {concurrency}）");
    let in_flight = Arc::new(Mutex::new(0u8));
    // 已完成计数：终态条目（成功/失败/取消）每收一条 +1，作为进度事件的 `current`。
    // 为什么另设计数器而不复用派发序号：完成后才递增，才能表达「已完成 X / 总数 Y」。
    let completed = Arc::new(Mutex::new(0u32));
    // 只读共享：Arc 包一次，任务侧只涨引用计数不深拷贝。
    let ctx = Arc::new(EntryCtx {
        state: Arc::clone(&state),
        completed: Arc::clone(&completed),
        total,
        data_dir,
        cfg: Arc::new(cfg),
        keys: Arc::new(keys),
        cancel: cancel.clone(),
        executor,
        sleeper,
        sink,
    });
    let mut handles = Vec::new();
    let mut canceled = 0u32;

    for id in ids {
        // 短锁取本条数据；条目可能已被用户移除（remove 不区分状态），找不到就跳过。
        let (uid, req) = {
            let st = state.lock().unwrap();
            match st.entries.iter().find(|e| e.id == id) {
                Some(e) => (e.uid.clone(), e.req.clone()),
                None => continue,
            }
        };
        // 等待循环：cancel 优先于空位——请求过取消就不再派发新任务。
        let mut was_canceled = false;
        loop {
            if cancel.is_cancelled() {
                // 短锁回写：尚未启动的条目直接判「已取消」。
                {
                    let mut st = state.lock().unwrap();
                    if let Some(e) = st.entries.iter_mut().find(|e| e.id == id) {
                        let err = image_ai::ImageGenError::cancelled();
                        e.status = QueueStatus::Failed;
                        e.error = err.message;
                        e.error_kind = Some(err.kind);
                    }
                }
                // 取消也是终态：计入已完成，保证后续事件的 current 不落拍。
                *completed.lock().unwrap() += 1;
                was_canceled = true;
                break;
            }
            if *in_flight.lock().unwrap() < concurrency {
                break;
            }
            // async_runtime 未直接再导出 sleep（无新依赖前提下）：spawn_blocking 里睡 50ms。
            tauri::async_runtime::spawn_blocking(|| std::thread::sleep(SLOT_POLL))
                .await
                .ok();
        }
        if was_canceled {
            canceled += 1;
            continue;
        }

        // 派发即占位（见函数注释：防 spawn→起跑窗口超发），由 SlotGuard 的 Drop 释放。
        // handle 收集时连同本条 id：任务 panic 时 JoinHandle 收不到返回值，兜底需靠这份 id 定位。
        let handle_id = id.clone();
        let task_ctx = Arc::clone(&ctx);
        let slot = SlotGuard::acquire(&in_flight);
        handles.push((
            handle_id,
            tauri::async_runtime::spawn(async move { run_entry(&task_ctx, id, uid, req, slot).await }),
        ));
    }

    // 等所有在跑任务收尾（cancel 只影响「尚未派发」的条目，在跑的必须等完）。
    let mut finished = 0u32;
    let mut failed_ran = 0u32;
    for (hid, h) in handles {
        match h.await {
            Ok(true) => finished += 1,
            Ok(false) => failed_ran += 1,
            Err(_) => {
                // 任务 panic：兜底只针对**本次 Err 对应的那一条 id**（handle 收集时已带上 id）——
                // 任务 panic 时 JoinHandle 收不到返回值，无法从结果里拿 id。并发 > 1 时兄弟条目
                // 仍在正常跑，若扫全部 Running 会把它们一并误标 Failed(Internal) 并多计 completed
                // （进度可显示「已完成 > 总数」）。找不到或已非 Running（兄弟已自行收尾）则不计数。
                let marked = {
                    let mut st = state.lock().unwrap();
                    match st.entries.iter_mut().find(|e| e.id == hid) {
                        Some(e) if e.status == QueueStatus::Running => {
                            e.status = QueueStatus::Failed;
                            e.error = "任务异常退出".into();
                            e.error_kind = Some(image_ai::GenErrorKind::Internal);
                            1u32
                        }
                        _ => 0u32,
                    }
                };
                // 兜底判定的条目同样是终态：计入已完成，避免后续事件的 current 落拍。
                *completed.lock().unwrap() += marked;
                failed_ran += 1;
                log::error!(
                    target: "image_ai",
                    "生成任务异常退出（panic）：{}",
                    if marked > 0 { "条目已标记为失败" } else { "条目已非运行态，未重复标记" }
                );
            }
        }
    }
    let failed = failed_ran + canceled;
    // 本批次收尾：清空 active，避免下一次「取消」把上一批的 id 当成自己的。
    state.lock().unwrap().active.clear();
    ctx.sink.finished(QueueFinished {
        finished,
        failed,
        total,
    });
    (finished, failed, total)
}

/// 单条任务的执行体：置 Running → 阶段回调（相邻去重）→ 重试循环 → 终态回写 → 进度事件。
/// 返回本次是否成功，供 [`run_queue`] 汇总；占位由 `_slot` 的 Drop 释放（任务 panic 也不漏）。
///
/// 【为什么独立成函数】`run_queue` 只管派发、等空位与汇总，单条的重试与状态回写自成一段；
/// 两者揉在同一个 spawn 闭包里时，「并发」与「一次尝试」的边界只能靠缩进辨认。
async fn run_entry(
    ctx: &EntryCtx,
    id: String,
    uid: String,
    req: image_ai::GenRequest,
    _slot: SlotGuard,
) -> bool {
    // 短锁置 Running，随后 emit（锁绝不跨 await）。
    {
        let mut st = ctx.state.lock().unwrap();
        if let Some(e) = st.entries.iter_mut().find(|e| e.id == id) {
            e.status = QueueStatus::Running;
        }
    }
    // 本条的「已完成数」快照：本条尚未完成，故取当前累计值（终态事件再递增）。
    let running_current = *ctx.completed.lock().unwrap();
    ctx.sink.progress(QueueProgress {
        status: "running".into(),
        current: running_current,
        total: ctx.total,
        uid: uid.clone(),
        entry_id: Some(id.clone()),
        stage: None,
    });
    // 阶段进度回调：适配器上报阶段，这里按「阶段」去重——原子记录上一阶段（无锁无 await，
    // 故可在回调内安全调用）。单值足够：一条任务内阶段单向推进，只需比较相邻是否相同。
    let last_stage = AtomicU8::new(0);
    let progress_cb = |stage: GenStage| {
        let code = stage as u8 + 1; // 0 保留给「尚无阶段」
        if last_stage.swap(code, Ordering::Relaxed) == code {
            return; // 同一阶段重复上报 → 丢弃
        }
        // current 在发射前即时短锁读一次，而非复用本条启动时的快照：并发下兄弟任务可能
        // 已先收尾，复用旧快照会让「已完成」与进度条回跳。锁内只取值（不 await、不 emit）。
        let now = *ctx.completed.lock().unwrap();
        ctx.sink.progress(QueueProgress {
            status: "running".into(),
            current: now,
            total: ctx.total,
            uid: uid.clone(),
            entry_id: Some(id.clone()),
            stage: Some(stage.as_str().to_string()),
        });
    };

    let key = ctx.keys.get(&req.provider_id).map(|s| s.as_str()).unwrap_or("");
    // 失败行标签：有球员名带名（日志里一眼认出是谁），没有则只记 uid。
    let label = match req.name.as_deref().filter(|n| !n.trim().is_empty()) {
        Some(name) => format!("{name}（{uid}）"),
        None => uid.clone(),
    };
    // 提交前留痕：人是谁 + 这条要发的参数（与单张的「提交生图」同格式，便于按关键字筛日志）。
    log::info!(
        target: "image_ai",
        "队列任务开始：{} | {}",
        image_ai::person_summary(&req),
        image_ai::request_summary(&req)
    );
    // 执行期重试：仅 `retryable` 错误重试，最多 MAX_ATTEMPTS 次（1 首发 + 2 重试）。
    // 【为什么留在此处】重试属于「跑一张图」的执行细节，主派发循环只管并发占位；
    // 重试期间条目保持 Running（不新增状态），前端据 attempts>1 显示「已重试 N 次」。
    // attempts 每次执行前自增并即刻回写（短锁），故运行中也能反映当前是第几次。
    let mut attempts = 0u32;
    let outcome = loop {
        attempts += 1;
        // 短锁回写 attempts；条目中途被移除时找不到 id，静默跳过（终态回写同款）。
        {
            let mut st = ctx.state.lock().unwrap();
            if let Some(e) = st.entries.iter_mut().find(|e| e.id == id) {
                e.attempts = attempts;
            }
        }
        let executed = ctx
            .executor
            .execute(&ctx.data_dir, &ctx.cfg, key, &req, &progress_cb, &ctx.cancel)
            .await
            // 厂商 401 常把被拒的 Key 抄进 message，而这条文案会进日志（用户贴日志
            // 求助时等于公开）：在留痕/展示之前统一抹掉，单点覆盖全部适配器。
            .map_err(|mut err| {
                err.scrub(key);
                err
            });
        match executed {
            // 还可重试且服务端判定可重试：退避后重来。attempt-1 是「已重试次数」，
            // 首次失败为 0 → backoff_delay 给出 2s（见其文档）。
            Err(err) if err.retryable && attempts < MAX_ATTEMPTS => {
                let delay = backoff_delay(attempts - 1, err.retry_after);
                // 重试期间条目保持 Running、界面看不出重试在发生，日志是唯一留痕。
                log::warn!(
                    target: "image_ai",
                    "生成失败（第 {attempts} 次尝试，将重试）：{label}——{}；{:.1?} 后重试",
                    err.message,
                    delay
                );
                ctx.sleeper.sleep(delay).await;
                // 【取消对「尚未开始的重试」生效】一次重试即一次新的派发，故退避结束、
                // 再次 execute 之前再读一次 cancel（短锁读，锁内不 await）。已请求取消
                // 就不再发起这次重试，定稿为与主派发循环取消分支一致的终态（Failed +
                // 「已取消」+ Cancelled）；正在执行中的那一次仍不中断（无中断通道）。
                // attempts 保留已真实发生的次数，不回零也不虚增。
                if ctx.cancel.is_cancelled() {
                    break Err(image_ai::ImageGenError::cancelled());
                }
            }
            // 成功、不可重试、或已用尽次数：定稿退出循环。
            other => break other,
        }
    };

    let success = outcome.is_ok();
    // 终态失败留痕：队列是会话态、重启即失，日志是唯一能回查「谁失败了、为什么」的通道。
    // 取消类不记——那是用户显式动作，取消入口与收尾汇总已有专门行。
    if let Err(err) = &outcome {
        if !matches!(err.kind, image_ai::GenErrorKind::Cancelled) {
            let attempts_note = if attempts > 1 {
                format!("（共尝试 {attempts} 次）")
            } else {
                String::new()
            };
            log::warn!(
                target: "image_ai",
                "生成失败：{label}——{}{attempts_note}",
                err.message
            );
        }
    }
    // 短锁回写终态；条目中途被移除时找不到 id，静默跳过。attempts 上一步已写，不动它。
    {
        let mut st = ctx.state.lock().unwrap();
        if let Some(e) = st.entries.iter_mut().find(|e| e.id == id) {
            match outcome {
                Ok(res) => {
                    e.status = QueueStatus::Success;
                    e.images = res.images;
                    e.error = String::new();
                    e.error_kind = None;
                }
                Err(err) => {
                    e.status = QueueStatus::Failed;
                    e.error = err.message;
                    e.error_kind = Some(err.kind);
                }
            }
        }
    }
    // 终态即一条已完成：递增后再带上新值，进度条的「已完成 X」随之 +1。
    let done = {
        let mut c = ctx.completed.lock().unwrap();
        *c += 1;
        *c
    };
    ctx.sink.progress(QueueProgress {
        status: if success { "success" } else { "failed" }.into(),
        current: done,
        total: ctx.total,
        uid,
        entry_id: Some(id),
        stage: None,
    });
    success
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicBool;

    /// 造一条干净的排队条目（id 留空：由 add 统一补发，见 QueueEntry::id 注释）。
    fn entry(uid: &str, name: &str) -> QueueEntry {
        QueueEntry {
            id: String::new(),
            uid: uid.into(),
            name: name.into(),
            req: image_ai::GenRequest {
                provider_id: "openai".into(),
                model: "gpt-image-1".into(),
                prompt: "一张脸".into(),
                size: "1024x1024".into(),
                uid: Some(uid.into()),
                name: Some(name.into()),
                // 前端提交的请求不带能力快照（由 validate_entries / generate 用服务商配置回填）。
                ..image_ai::test_req_base()
            },
            status: QueueStatus::Queued,
            images: vec![],
            error: String::new(),
            error_kind: None,
            attempts: 0,
        }
    }

    /// 预置成功结果：带一张图，用于断言 images 回填。
    fn blank_result() -> image_ai::GenResult {
        image_ai::GenResult {
            images: vec![image_ai::GenImage {
                id: "img1".into(),
                path: "p.png".into(),
                bytes: 1,
            }],
        }
    }

    /// 可重试错误：Network 类别默认 retryable=true（模拟网络中断）。
    fn retryable_err(msg: &str) -> image_ai::ImageGenError {
        image_ai::ImageGenError::network(msg)
    }

    /// 不可重试错误：Provider 类别默认 retryable=false（模拟内容审核拦截）。
    fn fatal_err(msg: &str) -> image_ai::ImageGenError {
        image_ai::ImageGenError::new(image_ai::GenErrorKind::Provider, msg)
    }

    /// 收集器替身：攒事件供断言，并观测首个终态成功事件（M1 回归的编排信号）。
    /// `saw_success` 让 slow 替身「等兄弟收尾后再上报阶段」，避免依赖真实耗时。
    #[derive(Default)]
    struct CollectSink {
        progress: Mutex<Vec<QueueProgress>>,
        finished: Mutex<Vec<QueueFinished>>,
        saw_success: Arc<AtomicBool>,
    }

    impl EventSink for CollectSink {
        fn progress(&self, payload: QueueProgress) {
            if payload.status == "success" {
                self.saw_success.store(true, Ordering::Release);
            }
            self.progress.lock().unwrap().push(payload);
        }
        fn finished(&self, payload: QueueFinished) {
            self.finished.lock().unwrap().push(payload);
        }
    }

    /// 生成替身：按脚本弹出预置结果（弹尽后默认成功），每次调用前先执行 hook 与阶段上报。
    /// hook 供 cancel 测试在首条任务执行期间置位取消标志；report 供阶段测试注入阶段序列
    /// （可含重复项，用于验证「同阶段去重」）。
    /// 【panic 场景不在此】`h.await` 收 Err 的 join-Err 兜底由 [`PanicWithSibling`] 覆盖：
    /// 那条用例要求同批两个条目行为不同（一条 panic、一条收尾），单一替身表达不了。
    struct StubExecutor {
        results: Mutex<VecDeque<Result<image_ai::GenResult, image_ai::ImageGenError>>>,
        calls: Mutex<Vec<String>>,
        hook: Mutex<Option<Box<dyn Fn() + Send>>>,
        report: Mutex<Vec<GenStage>>,
    }

    impl StubExecutor {
        fn new(results: Vec<Result<image_ai::GenResult, image_ai::ImageGenError>>) -> Self {
            Self {
                results: Mutex::new(results.into()),
                calls: Mutex::new(Vec::new()),
                hook: Mutex::new(None),
                report: Mutex::new(Vec::new()),
            }
        }
    }

    impl GenExecutor for StubExecutor {
        fn execute<'a>(
            &'a self,
            _data_dir: &'a Path,
            _cfg: &'a image_ai::Config,
            _key: &'a str,
            req: &'a image_ai::GenRequest,
            progress: &'a (dyn Fn(GenStage) + Sync),
            _cancel: &'a CancelToken,
        ) -> ExecFuture<'a> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(req.uid.clone().unwrap_or_default());
                // 先按脚本上报阶段（先取值再调用回调，回调内不持本替身的锁）。
                let stages: Vec<GenStage> = self.report.lock().unwrap().drain(..).collect();
                for s in stages {
                    progress(s);
                }
                if let Some(h) = self.hook.lock().unwrap().as_ref() {
                    h();
                }
                match self.results.lock().unwrap().pop_front() {
                    Some(r) => r,
                    None => Ok(blank_result()),
                }
            })
        }
    }

    /// 异步等待一个原子布尔被置位：每次让出后小睡，绝不阻塞 runtime 线程（并发时序测试编排用）。
    async fn wait_flag(flag: &AtomicBool) {
        while !flag.load(Ordering::Acquire) {
            tauri::async_runtime::spawn_blocking(|| std::thread::sleep(Duration::from_millis(1)))
                .await
                .ok();
        }
    }

    /// 阶段回跳替身（M1）：uid=="slow" 先置位 slow_started（此刻它的启动快照已读毕），再等
    /// fast 终态到达后连报两阶段再成功；fast 等 slow_started 才收尾。时序因此确定：slow 的
    /// 启动快照必为 0、fast 的完成必在 slow 上报阶段之前，故 current 复用快照即会被抓到回跳。
    struct SlowStageExecutor {
        slow_started: Arc<AtomicBool>,
        fast_done: Arc<AtomicBool>,
    }

    impl GenExecutor for SlowStageExecutor {
        fn execute<'a>(
            &'a self,
            _data_dir: &'a Path,
            _cfg: &'a image_ai::Config,
            _key: &'a str,
            req: &'a image_ai::GenRequest,
            progress: &'a (dyn Fn(GenStage) + Sync),
            _cancel: &'a CancelToken,
        ) -> ExecFuture<'a> {
            Box::pin(async move {
                if req.uid.as_deref() == Some("slow") {
                    self.slow_started.store(true, Ordering::Release);
                    wait_flag(&self.fast_done).await;
                    progress(GenStage::Submitting);
                    progress(GenStage::Polling);
                } else {
                    wait_flag(&self.slow_started).await;
                }
                Ok(blank_result())
            })
        }
    }

    /// panic + 兄弟替身（M2）：uid=="panic" 等 slow 起跑后 panic；uid=="slow" 先置位、再等
    /// panic 条目**被兜底判 Failed** 后才收尾。这样兜底发生时 slow 必仍在 Running，否则 bug
    /// （扫全部 Running）会因 slow 已 Success 而漏报、测试失去判别力。
    struct PanicWithSibling {
        state: Arc<Mutex<QueueState>>,
        s_started: Arc<AtomicBool>,
    }

    impl GenExecutor for PanicWithSibling {
        fn execute<'a>(
            &'a self,
            _data_dir: &'a Path,
            _cfg: &'a image_ai::Config,
            _key: &'a str,
            req: &'a image_ai::GenRequest,
            _progress: &'a (dyn Fn(GenStage) + Sync),
            _cancel: &'a CancelToken,
        ) -> ExecFuture<'a> {
            Box::pin(async move {
                if req.uid.as_deref() == Some("panic") {
                    wait_flag(&self.s_started).await;
                    panic!("模拟执行器 panic");
                } else {
                    self.s_started.store(true, Ordering::Release);
                    loop {
                        let done = {
                            let st = self.state.lock().unwrap();
                            st.entries
                                .iter()
                                .any(|e| e.uid == "panic" && e.status == QueueStatus::Failed)
                        };
                        if done {
                            break;
                        }
                        tauri::async_runtime::spawn_blocking(|| {
                            std::thread::sleep(Duration::from_millis(1))
                        })
                        .await
                        .ok();
                    }
                    Ok(blank_result())
                }
            })
        }
    }

    /// 睡眠替身：记录每次退避时长、绝不真睡（否则重试测试会真等数秒到分钟）。
    /// 记录序列同时用于断言「退避公式」与「是否根本不重试（序列为空）」。
    /// hook 在每次「睡觉」时执行，供「退避期间触发取消」这类时序测试注入副作用。
    #[derive(Default)]
    struct RecordingSleeper {
        slept: Mutex<Vec<Duration>>,
        hook: Mutex<Option<Box<dyn Fn() + Send>>>,
    }

    impl Sleeper for RecordingSleeper {
        fn sleep<'a>(&'a self, d: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                self.slept.lock().unwrap().push(d);
                if let Some(h) = self.hook.lock().unwrap().as_ref() {
                    h();
                }
            })
        }
    }

    /// 队列夹具：状态 + 事件收集器 + 记录型睡眠替身三者打包，用例开头 2–3 行即可开跑。
    /// 条目一律经 `QueueState::add` 入队——它负责统一补发 id（直接构造会让 id 全空，
    /// run_queue 按 id 定位条目的回写就会全部命中第一条）。
    struct Harness {
        state: Arc<Mutex<QueueState>>,
        sink: Arc<CollectSink>,
        sleeper: Arc<RecordingSleeper>,
    }

    impl Harness {
        fn new(entries: Vec<QueueEntry>) -> Self {
            let mut q = QueueState::default();
            q.add(entries);
            Self {
                state: Arc::new(Mutex::new(q)),
                sink: Arc::new(CollectSink::default()),
                sleeper: Arc::new(RecordingSleeper::default()),
            }
        }

        /// 跑一遍 run_queue：本次调用的临时目录在内部建好并保活到收尾；退避走记录型替身不真睡。
        /// tokio 不是本 crate 直接依赖（不引新依赖）：async 测试经 tauri 的 async_runtime 驱动。
        fn run(&self, executor: Arc<dyn GenExecutor>, concurrency: u32) -> (u32, u32, u32) {
            let dir = tempfile::tempdir().unwrap();
            let data_dir = dir.path().to_path_buf();
            tauri::async_runtime::block_on(run_queue(
                Arc::clone(&self.state),
                data_dir,
                image_ai::Config::builtin(),
                HashMap::new(),
                concurrency,
                None,
                executor,
                Arc::clone(&self.sleeper) as Arc<dyn Sleeper>,
                Arc::clone(&self.sink) as Arc<dyn EventSink>,
            ))
        }

        /// 只跑指定 id 的变体：按勾选启动的回归用（其余排队条目应原样留在队列里）。
        fn run_only(&self, executor: Arc<dyn GenExecutor>, ids: Vec<String>) -> (u32, u32, u32) {
            let dir = tempfile::tempdir().unwrap();
            tauri::async_runtime::block_on(run_queue(
                Arc::clone(&self.state),
                dir.path().to_path_buf(),
                image_ai::Config::builtin(),
                HashMap::new(),
                1,
                Some(ids),
                executor,
                Arc::clone(&self.sleeper) as Arc<dyn Sleeper>,
                Arc::clone(&self.sink) as Arc<dyn EventSink>,
            ))
        }

        fn snapshot(&self) -> Vec<QueueEntry> {
            self.state.lock().unwrap().snapshot()
        }

        fn statuses(&self) -> Vec<QueueStatus> {
            self.snapshot().iter().map(|e| e.status.clone()).collect()
        }

        fn progress(&self) -> Vec<QueueProgress> {
            self.sink.progress.lock().unwrap().clone()
        }

        fn finished(&self) -> Vec<QueueFinished> {
            self.sink.finished.lock().unwrap().clone()
        }

        /// 本批次的退避序列（断言「退避公式」与「根本不重试」）。
        fn slept(&self) -> Vec<Duration> {
            self.sleeper.slept.lock().unwrap().clone()
        }
    }

    // ===================== QueueState 状态机 =====================

    #[test]
    fn queue_state_add_remove_clear_snapshot() {
        let mut q = QueueState::default();
        q.add(vec![entry("u1", "甲"), entry("u2", "乙")]);
        // id 由后端统一补发且唯一；状态一律复位为 Queued。
        assert_eq!(q.entries.len(), 2);
        assert!(q.entries.iter().all(|e| e.status == QueueStatus::Queued));
        assert!(!q.entries[0].id.is_empty());
        assert_ne!(q.entries[0].id, q.entries[1].id);
        assert_eq!(q.snapshot().len(), 2);

        let id = q.entries[0].id.clone();
        q.remove(&[id.clone()]);
        assert_eq!(q.entries.len(), 1);
        assert_eq!(q.entries[0].uid, "u2");
        // 移除不存在的 id：静默无副作用。
        q.remove(&[format!("{id}x")]);
        assert_eq!(q.entries.len(), 1);

        // 同 uid 去重：新入列替换此前入列的排队条目（占其位），只入其一、无提示。
        // 终态条目不受影响——历史结果不该被同 uid 的新任务抹掉。
        q.add(vec![entry("u9", "旧")]);
        let old_id = q.entries[0].id.clone();
        q.entries[0].status = QueueStatus::Success; // 旧条目先跑完
        q.add(vec![entry("u9", "新")]);
        assert_eq!(q.entries.len(), 2, "成功的旧条目保留，新条目追加");
        q.add(vec![entry("u9", "再新")]);
        assert_eq!(q.entries.len(), 2, "排队中的同 uid 条目被替换而非追加");
        assert!(q.entries.iter().any(|e| e.status == QueueStatus::Success));
        let queued: Vec<&QueueEntry> = q
            .entries
            .iter()
            .filter(|e| e.status == QueueStatus::Queued)
            .collect();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].name, "再新");
        assert_ne!(queued[0].id, old_id, "替换后的条目是新 id");

        // 同一批内的重复 uid 同样后者胜（同一条去重路径，逐条顶掉先入列的）。
        q.add(vec![entry("u20", "批内甲"), entry("u20", "批内乙")]);
        let same_batch: Vec<&QueueEntry> = q
            .entries
            .iter()
            .filter(|e| e.status == QueueStatus::Queued && e.uid == "u20")
            .collect();
        assert_eq!(same_batch.len(), 1);
        assert_eq!(same_batch[0].name, "批内乙");

        q.clear();
        assert!(q.snapshot().is_empty());
    }

    // ===================== run_queue（替身执行器，不触网） =====================

    #[test]
    fn run_queue_only_runs_selected_entries() {
        // 按勾选启动：只跑传进来的 id，其余排队条目必须原样留在队列里（下一批还能跑）。
        let h = Harness::new(vec![entry("u1", "甲"), entry("u2", "乙"), entry("u3", "丙")]);
        let ids: Vec<String> = h.snapshot().iter().map(|e| e.id.clone()).collect();
        let stub = Arc::new(StubExecutor::new(vec![]));
        let (finished, failed, total) =
            h.run_only(Arc::clone(&stub) as Arc<dyn GenExecutor>, vec![ids[1].clone()]);

        assert_eq!((finished, failed, total), (1, 0, 1), "总数只含本次提交的条目");
        assert_eq!(
            h.statuses(),
            vec![QueueStatus::Queued, QueueStatus::Success, QueueStatus::Queued],
            "未选中的条目保持排队"
        );
        assert_eq!(*stub.calls.lock().unwrap(), vec!["u2"], "只有选中的那条被执行");
    }

    #[test]
    fn run_queue_marks_terminal_state_and_backfills_images() {
        // 场景一：全成功。并发 1 与 0（缺省归一为 1，避免除零/挂起）两种输入断言一致。
        for concurrency in [1, 0] {
            let h = Harness::new(vec![entry("u1", "甲"), entry("u2", "乙"), entry("u3", "丙")]);
            // 上一次运行遗留的取消标志：run 开始时必须复位，否则这批全被误杀。
            h.state.lock().unwrap().cancel.cancel();
            let stub = Arc::new(StubExecutor::new(vec![]));
            let (finished, failed, total) =
                h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, concurrency);

            assert_eq!((finished, failed, total), (3, 0, 3));
            assert_eq!(h.statuses(), vec![QueueStatus::Success; 3]);
            let snap = h.snapshot();
            // 成功条目回填 images；error 清空后不出现在序列化里；一次成功 attempts=1
            //（前端「>1 才显示已重试」据此不误报）。
            assert_eq!(snap[0].images.len(), 1);
            assert!(snap.iter().all(|e| e.error.is_empty() && e.attempts == 1));
            assert!(h.slept().is_empty(), "一次成则无退避");
            // 严格按入队顺序执行；running/success 各 3 条 + finished 1 条。
            assert_eq!(*stub.calls.lock().unwrap(), vec!["u1", "u2", "u3"]);
            assert_eq!(h.progress().len(), 6);
            assert_eq!(
                h.finished(),
                vec![QueueFinished {
                    finished: 3,
                    failed: 0,
                    total: 3
                }]
            );
        }

        // 场景二：中间条目以厂商错误收尾，前后条目照常跑完（批次不因单条失败中断）。
        let h = Harness::new(vec![entry("u1", "甲"), entry("u2", "乙"), entry("u3", "丙")]);
        let stub = Arc::new(StubExecutor::new(vec![
            Ok(blank_result()),
            Err(image_ai::ImageGenError::new(
                image_ai::GenErrorKind::Provider,
                "模拟厂商返回错误",
            )),
            Ok(blank_result()),
        ]));
        let (finished, failed, total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);

        assert_eq!((finished, failed, total), (2, 1, 3));
        let snap = h.snapshot();
        assert_eq!(snap[0].status, QueueStatus::Success);
        assert_eq!(snap[1].status, QueueStatus::Failed);
        assert_eq!(snap[1].error, "模拟厂商返回错误");
        // 失败类别随执行器错误原样落入条目（前端据此判断是否展示「重试本条」）。
        assert_eq!(snap[1].error_kind, Some(image_ai::GenErrorKind::Provider));
        assert!(snap[1].images.is_empty(), "失败条目不回填图片");
        assert_eq!(snap[2].status, QueueStatus::Success);
        assert_eq!(snap[2].error_kind, None);

        // 场景三：执行器以普通 Err(Internal) 收尾（同样走 outcome 的 Err 分支，非 panic 兜底），
        // 类别必须落到 Internal——它在可重试白名单内，前端仍给「重试本条」。
        let h = Harness::new(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![Err(
            image_ai::ImageGenError::internal("任务异常退出"),
        )]));
        let (_finished, failed, _total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);
        assert_eq!(failed, 1);
        let snap = h.snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(snap[0].error, "任务异常退出");
        assert_eq!(snap[0].error_kind, Some(image_ai::GenErrorKind::Internal));
    }

    /// 执行中取消的替身：先置位队列共享令牌（等价用户点「取消生成」），再返回**真实厂商
    /// 错误**——请求已提交（已计费），结果必须原样落到条目上，取消不得覆盖它。
    /// 令牌若不共享，主循环就不会停止派发，但本条断言只关心「已提交任务的真实结果不被取消
    /// 改写」，故不依赖共享性。
    struct CancelWhileRunning {
        state: Arc<Mutex<QueueState>>,
    }

    impl GenExecutor for CancelWhileRunning {
        fn execute<'a>(
            &'a self,
            _data_dir: &'a Path,
            _cfg: &'a image_ai::Config,
            _key: &'a str,
            _req: &'a image_ai::GenRequest,
            _progress: &'a (dyn Fn(GenStage) + Sync),
            _cancel: &'a CancelToken,
        ) -> ExecFuture<'a> {
            Box::pin(async move {
                self.state.lock().unwrap().cancel.cancel();
                Err(fatal_err("内容审核未通过"))
            })
        }
    }

    #[test]
    fn run_queue_cancel_stops_dispatch_and_retry_but_keeps_submitted_result() {
        // 场景一：尚未派发的条目直接判「已取消」。首条执行期间置位取消——并发 1 下 u2/u3 必在
        // 派发前看到标志，断言因此确定（在跑的 u1 已提交，等它自然收尾）。
        let h = Harness::new(vec![entry("u1", "甲"), entry("u2", "乙"), entry("u3", "丙")]);
        let stub = Arc::new(StubExecutor::new(vec![]));
        {
            let st = Arc::clone(&h.state);
            *stub.hook.lock().unwrap() = Some(Box::new(move || {
                st.lock().unwrap().cancel.cancel();
            }));
        }
        let (finished, failed, total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);

        assert_eq!((finished, failed, total), (1, 2, 3));
        let snap = h.snapshot();
        assert_eq!(snap[0].status, QueueStatus::Success);
        for e in &snap[1..] {
            assert_eq!(e.status, QueueStatus::Failed);
            assert_eq!(e.error, "已取消");
            // 取消既是 Failed 又带 Cancelled 类别（前端据此隐藏「重试本条」）。
            assert_eq!(e.error_kind, Some(image_ai::GenErrorKind::Cancelled));
            assert_eq!(e.attempts, 0, "从未执行过的取消条目 attempts 保持 0");
        }
        assert_eq!(*stub.calls.lock().unwrap(), vec!["u1"], "只有 u1 真正执行过");

        // 场景二：退避期间取消 ⇒ 不再发起这次重试（一次重试即一次新的派发）；
        // attempts 保留已真实发生的 1 次，不回零也不虚增。
        let h = Harness::new(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![
            Err(retryable_err("网络中断")),
            Ok(blank_result()), // 取消后不得被消费
        ]));
        {
            let st = Arc::clone(&h.state);
            *h.sleeper.hook.lock().unwrap() = Some(Box::new(move || {
                st.lock().unwrap().cancel.cancel();
            }));
        }
        let (finished, failed, total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);

        assert_eq!((finished, failed, total), (0, 1, 1));
        assert_eq!(stub.calls.lock().unwrap().len(), 1, "取消后不得再发起重试");
        assert_eq!(h.slept(), vec![Duration::from_millis(2000)], "退避发生过一次");
        let snap = h.snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(snap[0].error, "已取消");
        assert_eq!(snap[0].error_kind, Some(image_ai::GenErrorKind::Cancelled));
        assert_eq!(snap[0].attempts, 1);

        // 场景三：执行中取消（请求已提交）。取消只拦未提交的请求，不得改写已提交任务的结果：
        // 条目必须保留厂商的真实失败原因（Failed + Provider），而不是被替换成「已取消」
        // （那会让用户以为没花钱，实际上服务端已计费）。attempts 也不得因取消而重试。
        let h = Harness::new(vec![entry("u1", "甲")]);
        let exec = Arc::new(CancelWhileRunning {
            state: Arc::clone(&h.state),
        });
        let (finished, failed, total) = h.run(Arc::clone(&exec) as Arc<dyn GenExecutor>, 1);

        assert_eq!((finished, failed, total), (0, 1, 1));
        let snap = h.snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(
            snap[0].error, "内容审核未通过",
            "已提交任务的真实错误不得被「已取消」覆盖"
        );
        assert_eq!(
            snap[0].error_kind,
            Some(image_ai::GenErrorKind::Provider),
            "类别取厂商错误（Provider），不是 Cancelled"
        );
        assert_eq!(snap[0].attempts, 1, "不可重试错误只跑一次");
    }

    #[test]
    fn run_queue_records_active_batch_and_spares_unselected_queue() {
        // run_queue 必须把「本批次实际派发的条目 id」写进 QueueState.active：cancel_queue 只落
        // 这批 id，未参与本批次的排队条目必须原样留在队列里。这里把该写入做成断言。
        let h = Harness::new(vec![entry("u1", "甲"), entry("u2", "乙"), entry("u3", "丙")]);
        let ids: Vec<String> = h.snapshot().iter().map(|e| e.id.clone()).collect();
        let stub = Arc::new(StubExecutor::new(vec![]));
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let st = Arc::clone(&h.state);
            let seen = Arc::clone(&seen);
            // 首条派发前先记下 active（此刻本批次 id 必须已写入），再模拟用户点「取消生成」。
            *stub.hook.lock().unwrap() = Some(Box::new(move || {
                let st = st.lock().unwrap();
                *seen.lock().unwrap() = st.active.clone();
                st.cancel.cancel();
            }));
        }
        // 只勾选甲、乙（丙未参与本批次）。
        let (finished, failed, total) = h.run_only(
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            vec![ids[0].clone(), ids[1].clone()],
        );

        assert_eq!((finished, failed, total), (1, 1, 2), "只跑本批次两条");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![ids[0].clone(), ids[1].clone()],
            "active 必须记录本批次派发的 id，且不含未勾选的丙"
        );
        let snap = h.snapshot();
        assert_eq!(snap[0].status, QueueStatus::Success, "已派发即已提交，取消不中断");
        assert_eq!(snap[1].status, QueueStatus::Failed);
        assert_eq!(snap[1].error, "已取消");
        assert_eq!(snap[1].error_kind, Some(image_ai::GenErrorKind::Cancelled));
        assert_eq!(snap[1].attempts, 0);
        // 丙：未参与本批次，取消不得误伤。
        assert_eq!(snap[2].status, QueueStatus::Queued);
        assert!(snap[2].error.is_empty());
        assert!(snap[2].error_kind.is_none());
        assert_eq!(snap[2].attempts, 0);
        assert_eq!(
            *stub.calls.lock().unwrap(),
            vec!["u1"],
            "乙在派发前排到了取消"
        );
    }

    // ===================== 执行期重试（退避纯函数 + 任务体循环） =====================

    #[test]
    fn run_queue_retries_with_backoff_and_honors_retry_after() {
        // 场景零：退避公式本身（纯函数）。2s×2ⁿ + 确定性抖动，30s 封顶；Retry-After 优先且同样封顶。
        assert_eq!(backoff_delay(0, None), Duration::from_millis(2000));
        assert_eq!(backoff_delay(1, None), Duration::from_millis(4250));
        assert_eq!(backoff_delay(2, None), Duration::from_millis(8500));
        assert_eq!(backoff_delay(10, None), Duration::from_secs(30));
        assert_eq!(backoff_delay(0, Some(7)), Duration::from_secs(7));
        assert_eq!(backoff_delay(0, Some(99)), Duration::from_secs(30));

        // 场景一：首发可重试失败 → 退避 2s → 重试成功；重试期间条目保持 Running（不新增状态）。
        let h = Harness::new(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![
            Err(retryable_err("网络中断")),
            Ok(blank_result()),
        ]));
        // hook 在每次 execute 前记录条目状态。
        let seen: Arc<Mutex<Vec<QueueStatus>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let st = Arc::clone(&h.state);
            let seen = Arc::clone(&seen);
            *stub.hook.lock().unwrap() = Some(Box::new(move || {
                seen.lock()
                    .unwrap()
                    .push(st.lock().unwrap().entries[0].status.clone());
            }));
        }
        let (finished, failed, total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);

        assert_eq!((finished, failed, total), (1, 0, 1));
        assert_eq!(stub.calls.lock().unwrap().len(), 2, "首发 + 一次重试");
        assert_eq!(h.slept(), vec![Duration::from_millis(2000)], "首次档退避");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![QueueStatus::Running, QueueStatus::Running],
            "重试不改状态"
        );
        let snap = h.snapshot();
        assert_eq!(snap[0].status, QueueStatus::Success);
        assert_eq!(snap[0].attempts, 2, "attempts 反映真实执行次数");
        // 进度事件里不出现「重试」专属状态：仍是 running + success 各一条。
        let statuses: Vec<String> = h.progress().iter().map(|p| p.status.clone()).collect();
        assert_eq!(statuses, vec!["running".to_string(), "success".to_string()]);

        // 场景二：服务端给了 Retry-After=7 → 退避取 7s 而非公式值。
        let h = Harness::new(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![
            Err(retryable_err("触发限流").with_retry_after(Some(7))),
            Ok(blank_result()),
        ]));
        let (finished, failed, total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);

        assert_eq!((finished, failed, total), (1, 0, 1));
        assert_eq!(
            h.slept(),
            vec![Duration::from_secs(7)],
            "Retry-After 优先于退避公式"
        );
        assert_eq!(h.snapshot()[0].status, QueueStatus::Success);
    }

    #[test]
    fn run_queue_retry_stops_on_fatal_or_exhausted() {
        // 场景一：不可重试错误一次定稿（第二个结果永不消费）。
        let h = Harness::new(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![
            Err(fatal_err("内容审核未通过")),
            Ok(blank_result()),
        ]));
        let (finished, failed, total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);

        assert_eq!((finished, failed, total), (0, 1, 1));
        assert_eq!(stub.calls.lock().unwrap().len(), 1, "不可重试错误只跑一次");
        assert!(h.slept().is_empty(), "不重试就不退避");
        let snap = h.snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(snap[0].error, "内容审核未通过");
        assert_eq!(snap[0].attempts, 1);

        // 场景二：三连可重试失败 → 跑满 MAX_ATTEMPTS（3）即放弃，不再有第 4 次、不再多睡。
        let h = Harness::new(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![
            Err(retryable_err("网络中断")),
            Err(retryable_err("网络中断")),
            Err(retryable_err("网络中断")),
        ]));
        let (finished, failed, total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);

        assert_eq!((finished, failed, total), (0, 1, 1));
        assert_eq!(
            stub.calls.lock().unwrap().len(),
            MAX_ATTEMPTS as usize,
            "尝试次数封顶为 MAX_ATTEMPTS"
        );
        // 只在第 1、2 次失败后睡：n=0 → 2s，n=1 → 4.25s。
        assert_eq!(
            h.slept(),
            vec![Duration::from_millis(2000), Duration::from_millis(4250)]
        );
        let snap = h.snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(snap[0].error, "网络中断");
        assert_eq!(snap[0].attempts, MAX_ATTEMPTS);
    }

    // ===================== 阶段事件的透传与去重 =====================

    #[test]
    fn run_queue_forwards_stages_with_dedupe() {
        // ① 队列层对阶段的透传契约：替身按序上报 submitting→polling→downloading，收集器应收到
        // 同序 stage；且每条事件的 current 是**已完成数**（本条未完成时为 0，终态才为 1），
        // 而非派发序号——单条队列下二者数值恰好都从 1 起，故必须锚定「运行期恒 0」才能验出区别。
        let h = Harness::new(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![Ok(blank_result())]));
        *stub.report.lock().unwrap() = vec![
            GenStage::Submitting,
            GenStage::Polling,
            GenStage::Downloading,
        ];
        let (finished, failed, total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);

        assert_eq!((finished, failed, total), (1, 0, 1));
        let events = h.progress();
        // 事件序列：running(None) → 三个阶段 → 终态 success(None)。
        let stages: Vec<Option<String>> = events.iter().map(|p| p.stage.clone()).collect();
        assert_eq!(
            stages,
            vec![
                None,
                Some("submitting".into()),
                Some("polling".into()),
                Some("downloading".into()),
                None,
            ]
        );
        // 运行期（含全部阶段事件）current 恒为「已完成数」= 0；终态递增为 1。
        assert!(
            events[..events.len() - 1].iter().all(|p| p.current == 0),
            "运行期的 current 必须是已完成数（本条未完成前恒 0），而非派发序号"
        );
        let last = events.last().unwrap();
        assert_eq!(last.current, 1);
        assert_eq!(last.status, "success");
        assert_eq!(last.stage, None, "终态事件不带阶段");
        // 每条事件都带条目唯一 id（前端据此定位阶段；uid 在重复入队时会重复）。
        let entry_id = h.snapshot()[0].id.clone();
        assert!(!entry_id.is_empty());
        assert!(
            events
                .iter()
                .all(|p| p.entry_id.as_deref() == Some(entry_id.as_str())),
            "进度事件必须携带所属条目的 entry_id"
        );

        // ② 适配器可能重复上报同一阶段（如轮询期间多次自检）；队列层按「上一个阶段」去重：
        // 相邻重复只发一次；非相邻重复（Submitting→Polling→Submitting）必须放行。
        let cases = [
            (
                vec![
                    GenStage::Submitting,
                    GenStage::Submitting, // 相邻重复 → 丢弃
                    GenStage::Polling,
                    GenStage::Downloading,
                    GenStage::Downloading, // 相邻重复 → 丢弃
                ],
                vec!["submitting", "polling", "downloading"],
            ),
            (
                vec![
                    GenStage::Submitting,
                    GenStage::Polling,
                    GenStage::Submitting,
                ],
                vec!["submitting", "polling", "submitting"],
            ),
        ];
        for (report, expected) in cases {
            let h = Harness::new(vec![entry("u1", "甲")]);
            let stub = Arc::new(StubExecutor::new(vec![Ok(blank_result())]));
            *stub.report.lock().unwrap() = report;
            let (finished, failed, total) = h.run(Arc::clone(&stub) as Arc<dyn GenExecutor>, 1);

            assert_eq!((finished, failed, total), (1, 0, 1));
            let reported: Vec<Option<String>> = h
                .progress()
                .iter()
                .map(|p| p.stage.clone())
                .filter(|s| s.is_some())
                .collect();
            assert_eq!(
                reported,
                expected
                    .into_iter()
                    .map(|s| Some(s.to_string()))
                    .collect::<Vec<_>>(),
                "相邻重复去重、非相邻重复放行"
            );
        }
    }

    #[test]
    fn run_queue_concurrent_stage_current_never_goes_backwards() {
        // M1 回归：并发 2 下 slow 先起跑但不收尾，fast 先成功（completed=1）；slow 随后上报
        // 阶段时 current 必须即时读最新已完成数（1），不得复用启动快照（0）造成进度回跳。
        let h = Harness::new(vec![entry("slow", "甲"), entry("fast", "乙")]);
        let exec = Arc::new(SlowStageExecutor {
            slow_started: Arc::new(AtomicBool::new(false)),
            fast_done: Arc::clone(&h.sink.saw_success),
        });
        let (finished, failed, total) = h.run(Arc::clone(&exec) as Arc<dyn GenExecutor>, 2);

        assert_eq!((finished, failed, total), (2, 0, 2));
        let currents: Vec<u32> = h.progress().iter().map(|p| p.current).collect();
        // current 单调不减：slow 的阶段事件若复用启动快照会从 1 回跳到 0。
        let mut prev = 0u32;
        for &c in &currents {
            assert!(c >= prev, "current 回跳：{:?}", currents);
            prev = c;
        }
        assert!(currents.iter().all(|&c| c <= total));
    }

    #[test]
    fn run_queue_survives_entry_panic() {
        // 场景一（M2）：并发 2 下一条 panic 时，仍在跑的兄弟不得被兜底误标 Failed(Internal)，
        // 也不得把兄弟计入 completed（否则进度会显示「已完成 > 总数」）。
        let h = Harness::new(vec![entry("panic", "甲"), entry("slow", "乙")]);
        let exec = Arc::new(PanicWithSibling {
            state: Arc::clone(&h.state),
            s_started: Arc::new(AtomicBool::new(false)),
        });
        let (finished, failed, total) = h.run(Arc::clone(&exec) as Arc<dyn GenExecutor>, 2);

        assert_eq!((finished, failed, total), (1, 1, 2), "兄弟未被算作失败");
        let snap = h.snapshot();
        let slow = snap.iter().find(|e| e.uid == "slow").unwrap();
        assert_eq!(slow.status, QueueStatus::Success);
        assert!(slow.error_kind.is_none(), "兄弟条目不得被 panic 兜底误标");
        let panicked = snap.iter().find(|e| e.uid == "panic").unwrap();
        assert_eq!(panicked.status, QueueStatus::Failed);
        assert_eq!(panicked.error, "任务异常退出", "panic 兜底文案");
        assert_eq!(panicked.error_kind, Some(image_ai::GenErrorKind::Internal));

        // 进度不回跳 / 不多计由 run_queue_concurrent_stage_current_never_goes_backwards 覆盖，
        // 此处不重复断言。

        // 场景二：并发 1 + 两条。panic 那条若不释放占位，第二条永远等不到空位（等空位的循环里
        // 也读不到 cancel），整批挂死；超时护栏把「挂死」判成失败，而不是卡住测试进程。
        let h = Harness::new(vec![entry("panic", "甲"), entry("ok", "乙")]);
        let (finished, failed, total) =
            run_with_timeout(&h, Arc::new(PanicFirst), 1, Duration::from_secs(15));
        assert_eq!((finished, failed, total), (1, 1, 2), "panic 计失败，兄弟仍应跑完");
        assert_eq!(h.statuses(), [QueueStatus::Failed, QueueStatus::Success]);
        // panic 兜底文案只落在本条；兄弟的回填与计数不受影响。
        assert_eq!(h.snapshot()[0].error, "任务异常退出");
        assert_eq!(h.snapshot()[1].images.len(), 1);
    }

    /// 带超时护栏地跑一遍队列：结果经通道回传，超时即判「占位未释放」。
    /// 【为什么不用 `Harness::run`】那个是同步 block_on，占位一旦泄漏就永久挂住整个测试进程；
    /// 本组用例正是要抓这种泄漏，必须能把「挂死」变成可判定的失败。
    fn run_with_timeout(
        h: &Harness,
        executor: Arc<dyn GenExecutor>,
        concurrency: u32,
        wait: Duration,
    ) -> (u32, u32, u32) {
        let state = Arc::clone(&h.state);
        let sleeper = Arc::clone(&h.sleeper) as Arc<dyn Sleeper>;
        let sink = Arc::clone(&h.sink) as Arc<dyn EventSink>;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let dir = tempfile::tempdir().unwrap();
            let out = tauri::async_runtime::block_on(run_queue(
                state,
                dir.path().to_path_buf(),
                image_ai::Config::builtin(),
                HashMap::new(),
                concurrency,
                None,
                executor,
                sleeper,
                sink,
            ));
            let _ = tx.send(out);
        });
        rx.recv_timeout(wait)
            .unwrap_or_else(|_| panic!("队列未在 {wait:?} 内收尾：并发占位疑似未释放"))
    }

    /// 首位 panic、其余成功的执行器：并发 1 下「兄弟能否起跑」就是占位是否释放的判据。
    struct PanicFirst;

    impl GenExecutor for PanicFirst {
        fn execute<'a>(
            &'a self,
            _data_dir: &'a Path,
            _cfg: &'a image_ai::Config,
            _key: &'a str,
            req: &'a image_ai::GenRequest,
            _progress: &'a (dyn Fn(GenStage) + Sync),
            _cancel: &'a CancelToken,
        ) -> ExecFuture<'a> {
            Box::pin(async move {
                if req.uid.as_deref() == Some("panic") {
                    panic!("模拟执行器 panic");
                }
                Ok(blank_result())
            })
        }
    }

    // ===================== 入队整批校验（不合规即整批拒绝） =====================

    // ===================== 入队整批校验（不合规即整批拒绝） =====================

    #[test]
    fn validate_entries_rejects_whole_batch_on_one_bad_entry() {
        let cfg = image_ai::Config::builtin();
        // 全合规：openai/gpt-image-1、n=1（上限 10）、size 在枚举内 → 放行。
        assert!(validate_entries(&cfg, &[entry("u1", "甲")]).is_ok());
        // 混入一条 n 超上限（maxN=10）→ 整批拒绝，且错误带类别。
        let good = entry("u1", "甲");
        let mut bad = entry("u2", "乙");
        bad.req.n = 99;
        let batch = vec![good, bad];
        let err = validate_entries(&cfg, &batch).unwrap_err();
        assert_eq!(err.kind, image_ai::GenErrorKind::InvalidParam);
        // 入参切片不被修改：校验只读，不给调用方留下「被就地改过」的假象。
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[1].req.n, 99);
    }

    // ===================== 取消只作用于本批次（active） =====================

    #[test]
    fn cancel_pending_only_touches_active_batch() {
        // 队列里 3 条排队，但本批次只派发了「甲」：取消只能落甲。乙丙是用户没勾选的排队条目，
        // 被一起标成「已取消」就等于凭空弄丢了他们没参与本批的任务。
        let mut q = QueueState::default();
        q.add(vec![entry("u1", "甲"), entry("u2", "乙"), entry("u3", "丙")]);
        let ids: Vec<String> = q.entries.iter().map(|e| e.id.clone()).collect();
        q.active = vec![ids[0].clone()];

        assert_eq!(q.cancel_pending(), 1);
        assert_eq!(q.entries[0].status, QueueStatus::Failed);
        assert_eq!(q.entries[0].error, "已取消");
        assert_eq!(
            q.entries[0].error_kind,
            Some(image_ai::GenErrorKind::Cancelled),
            "已取消带 Cancelled 类别（前端据此隐藏「重试本条」）"
        );
        assert_eq!(q.entries[0].attempts, 0, "从未执行过的取消条目 attempts 保持 0");
        for e in &q.entries[1..] {
            assert_eq!(e.status, QueueStatus::Queued, "未参与本批次的排队条目照常排队");
            assert!(e.error.is_empty());
            assert!(e.error_kind.is_none());
        }

        // 幂等：已标记的条目不再是 Queued，再取消一次没有可标的。
        assert_eq!(q.cancel_pending(), 0);
        assert_eq!(q.entries[1].status, QueueStatus::Queued);
    }

    // ===================== 手动确认的参数写回 =====================

    /// 一条合法的待写回参数（沿用队列夹具那份请求：尺寸 1024x1024、n=1）。
    fn edit_arg(id: &str) -> QueueEditArg {
        QueueEditArg {
            id: id.into(),
            req: entry("u1", "甲").req,
        }
    }

    #[test]
    fn apply_queue_edits_writes_queued_only() {
        let mut q = QueueState::default();
        q.add(vec![entry("u1", "甲"), entry("u2", "乙"), entry("u3", "丙")]);
        // 场景构造：甲排队（应被改）、乙在跑、丙已成功；另有一条不存在的 id。
        q.entries[1].status = QueueStatus::Running;
        q.entries[2].status = QueueStatus::Success;
        let ids: Vec<String> = q.entries.iter().map(|e| e.id.clone()).collect();

        let mut queued_edit = edit_arg(&ids[0]);
        queued_edit.req.prompt = "新提示词".into();
        queued_edit.req.size = "512x512".into();
        queued_edit.req.n = 2;
        // 身份字段留空 = 保留队列原值（决定画廊归属，不是这次可改的参数）。
        queued_edit.req.uid = None;
        queued_edit.req.name = None;
        queued_edit.req.template_id = None;
        let mut running_edit = edit_arg(&ids[1]);
        running_edit.req.prompt = "不该生效".into();
        let mut success_edit = edit_arg(&ids[2]);
        success_edit.req.prompt = "也不该生效".into();
        let missing_edit = edit_arg("no-such-id");

        let applied = apply_queue_edits(
            &mut q.entries,
            &[queued_edit, running_edit, success_edit, missing_edit],
        );
        assert_eq!(applied, 1, "只有排队条目被写入");
        assert_eq!(q.entries[0].req.prompt, "新提示词");
        assert_eq!(q.entries[0].req.size, "512x512");
        assert_eq!(q.entries[0].req.n, 2);
        assert_eq!(q.entries[0].req.uid.as_deref(), Some("u1"), "uid 保留入队时的值");
        assert_eq!(q.entries[0].req.name.as_deref(), Some("甲"));
        assert_eq!(q.entries[0].status, QueueStatus::Queued, "写参数不动状态");
        assert_eq!(q.entries[1].req.prompt, "一张脸", "在跑条目原样");
        assert_eq!(q.entries[2].req.prompt, "一张脸", "已成功条目原样");

        // 写回**不做参数合法性校验**（合法性由 start_queue 前的 validate_entries 兜底）：
        // 一份「不合规」的参数在这里照样落库，队列照样不变形——这是刻意口径，不是漏判。
        let mut odd_edit = edit_arg(&ids[0]);
        odd_edit.req.n = 9;
        assert_eq!(apply_queue_edits(&mut q.entries, &[odd_edit]), 1);
        assert_eq!(q.entries[0].req.n, 9, "写回层不拦参数，交由启动前校验");
        assert_eq!(q.entries[0].status, QueueStatus::Queued);
    }
}
