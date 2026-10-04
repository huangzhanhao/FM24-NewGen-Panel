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
use crate::image_ai_providers::GenStage;
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

/// 队列共享状态：条目列表 + 「请求取消」标志。
/// cancel 是 bool 而非 AtomicBool：它只被短锁内的读改写触碰（见 run_queue / cancel_queue），
/// 与 entries 同锁保护，避免两套同步原语各管一半。
#[derive(Default)]
pub struct QueueState {
    pub entries: Vec<QueueEntry>,
    pub cancel: bool,
}

impl QueueState {
    /// 追加一批条目：id 一律重生成、状态一律复位为 Queued——重复提交同一条（例如
    /// 前端把旧快照原样传回）也能得到一条全新的排队任务。
    pub fn add(&mut self, mut entries: Vec<QueueEntry>) {
        for e in entries.iter_mut() {
            e.id = image_ai::new_id();
            e.status = QueueStatus::Queued;
            e.error = String::new();
            e.error_kind = None;
            e.attempts = 0; // 尚未尝试（口径见 QueueEntry::attempts）
        }
        self.entries.append(&mut entries);
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
    ) -> Pin<Box<dyn Future<Output = Result<image_ai::GenResult, image_ai::ImageGenError>> + Send + 'a>>;
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
    ) -> Pin<Box<dyn Future<Output = Result<image_ai::GenResult, image_ai::ImageGenError>> + Send + 'a>>
    {
        // generate 按值收请求：克隆一份（一次生成的请求体就几行提示词，克隆可忽略）。
        Box::pin(image_ai::generate(data_dir, cfg, key, req.clone(), progress))
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
pub fn validate_entries(
    cfg: &image_ai::Config,
    entries: &[QueueEntry],
) -> Result<(), image_ai::ImageGenError> {
    for e in entries {
        image_ai::validate_complete(cfg, &e.req)?;
        let provider = cfg
            .provider(&e.req.provider_id)
            .ok_or_else(|| image_ai::ImageGenError::not_configured("请先选择服务商"))?;
        let model = cfg
            .model(&e.req.provider_id, &e.req.model)
            .ok_or_else(|| image_ai::ImageGenError::not_configured("请先选择模型"))?;
        // base_url 与能力快照都只在服务商配置里，用副本补齐，使探针与 generate 实跑的请求同形。
        // （真正读 req.caps() 的是适配器的请求体组装；validate 的 caps 走形参，同形可避免两套输入。）
        let mut req = e.req.clone();
        req.base_url = provider.base_url.clone();
        req.caps = Some(model.capabilities.clone());
        crate::image_ai_providers::for_kind(provider.kind).validate(&req, &model.capabilities)?;
    }
    Ok(())
}

/// 跑完队列里所有 Queued 条目，返回 (finished, failed, total)。
///
/// - 并发上限 `concurrency`（0 视为 1）：主循环派发即占位（`Arc<Mutex<u8>>` 计数），
///   任务收尾释放；空位不足时 50ms 轮询等待。占位在派发侧而非任务开头——spawn 到任务
///   真正起跑之间有调度窗口，若等任务自己占位，主循环会在窗口里超发。
/// - cancel==true 时：尚未派发的条目直接置 Failed(「已取消」)，在跑的等它自然收尾。
///   开跑时先把 cancel 复位 false（上一次运行遗留的标志不该取消这一次）。取消同样对
///   「执行期重试」生效：退避结束后不再发起新一次重试（见任务体内重试循环）。
/// - 每条任务开始/成功/失败都发 `gen:queue:progress`；全部收尾后发 `gen:queue:finished`。
///   事件里的 `current` 是**已完成数**（终态条目累计，非派发序号）；运行中另有阶段事件
///   （`stage` = submitting/polling/downloading），由适配器回调触发、按「阶段」去重。
/// - 执行期重试在**任务体内部**做（最多 `MAX_ATTEMPTS` 次，仅对 `retryable` 错误），
///   重试期间条目保持 Running；退避经 `sleeper` 睡（测试传替身，不真睡）。
pub async fn run_queue(
    state: Arc<Mutex<QueueState>>,
    data_dir: PathBuf,
    cfg: image_ai::Config,
    keys: HashMap<String, String>,
    concurrency: u32,
    executor: Arc<dyn GenExecutor>,
    sleeper: Arc<dyn Sleeper>,
    sink: Arc<dyn EventSink>,
) -> (u32, u32, u32) {
    // 短锁：复位 cancel + 摘出本次要跑的 Queued 条目 id（总量即 total，事件分母固定）。
    let (ids, total) = {
        let mut st = state.lock().unwrap();
        st.cancel = false;
        let ids: Vec<String> = st
            .entries
            .iter()
            .filter(|e| e.status == QueueStatus::Queued)
            .map(|e| e.id.clone())
            .collect();
        let total = ids.len() as u32;
        (ids, total)
    };
    // 0 视为 1；占位计数器是 u8（单机同时跑 255 张图不现实），更高配置一并截断。
    let concurrency = concurrency.clamp(1, u8::MAX as u32) as u8;
    let in_flight = Arc::new(Mutex::new(0u8));
    // 已完成计数：终态条目（成功/失败/取消）每收一条 +1，作为进度事件的 `current`。
    // 为什么另设计数器而不复用派发序号：完成后才递增，才能表达「已完成 X / 总数 Y」。
    let completed = Arc::new(Mutex::new(0u32));
    // cfg/keys 只读共享：Arc 包一次，任务侧只涨引用计数不深拷贝。
    let cfg = Arc::new(cfg);
    let keys = Arc::new(keys);
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
            if state.lock().unwrap().cancel {
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

        // 派发即占位（见函数注释：防 spawn→起跑窗口超发）；任务收尾负责释放。
        *in_flight.lock().unwrap() += 1;
        // handle 收集时连同本条 id：任务 panic 时 JoinHandle 收不到返回值，兜底需靠这份 id 定位。
        let handle_id = id.clone();
        let task_state = Arc::clone(&state);
        let task_in_flight = Arc::clone(&in_flight);
        let task_completed = Arc::clone(&completed);
        let task_executor = Arc::clone(&executor);
        let task_sleeper = Arc::clone(&sleeper);
        let task_sink = Arc::clone(&sink);
        let task_data_dir = data_dir.clone();
        let task_cfg = Arc::clone(&cfg);
        let task_keys = Arc::clone(&keys);
        handles.push((handle_id, tauri::async_runtime::spawn(async move {
            // 短锁置 Running，随后 emit（锁绝不跨 await）。
            {
                let mut st = task_state.lock().unwrap();
                if let Some(e) = st.entries.iter_mut().find(|e| e.id == id) {
                    e.status = QueueStatus::Running;
                }
            }
            // 本条的「已完成数」快照：本条尚未完成，故取当前累计值（终态事件再递增）。
            let running_current = *task_completed.lock().unwrap();
            task_sink.progress(QueueProgress {
                status: "running".into(),
                current: running_current,
                total,
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
                let now = *task_completed.lock().unwrap();
                task_sink.progress(QueueProgress {
                    status: "running".into(),
                    current: now,
                    total,
                    uid: uid.clone(),
                    entry_id: Some(id.clone()),
                    stage: Some(stage.as_str().to_string()),
                });
            };

            let key = task_keys
                .get(&req.provider_id)
                .map(|s| s.as_str())
                .unwrap_or("");
            // 执行期重试：仅 `retryable` 错误重试，最多 MAX_ATTEMPTS 次（1 首发 + 2 重试）。
            // 【为什么留在此处】重试属于「跑一张图」的执行细节，主派发循环只管并发占位；
            // 重试期间条目保持 Running（不新增状态），前端据 attempts>1 显示「已重试 N 次」。
            // attempts 每次执行前自增并即刻回写（短锁），故运行中也能反映当前是第几次。
            let mut attempts = 0u32;
            let outcome = loop {
                attempts += 1;
                // 短锁回写 attempts；条目中途被移除时找不到 id，静默跳过（终态回写同款）。
                {
                    let mut st = task_state.lock().unwrap();
                    if let Some(e) = st.entries.iter_mut().find(|e| e.id == id) {
                        e.attempts = attempts;
                    }
                }
                match task_executor
                    .execute(&task_data_dir, &task_cfg, key, &req, &progress_cb)
                    .await
                {
                    // 还可重试且服务端判定可重试：退避后重来。attempt-1 是「已重试次数」，
                    // 首次失败为 0 → backoff_delay 给出 2s（见其文档）。
                    Err(err) if err.retryable && attempts < MAX_ATTEMPTS => {
                        let delay = backoff_delay(attempts - 1, err.retry_after);
                        task_sleeper.sleep(delay).await;
                        // 【取消对「尚未开始的重试」生效】一次重试即一次新的派发，故退避结束、
                        // 再次 execute 之前再读一次 cancel（短锁读，锁内不 await）。已请求取消
                        // 就不再发起这次重试，定稿为与主派发循环取消分支一致的终态（Failed +
                        // 「已取消」+ Cancelled）；正在执行中的那一次仍不中断（无中断通道）。
                        // attempts 保留已真实发生的次数，不回零也不虚增。
                        if task_state.lock().unwrap().cancel {
                            break Err(image_ai::ImageGenError::cancelled());
                        }
                    }
                    // 成功、不可重试、或已用尽次数：定稿退出循环。
                    other => break other,
                }
            };

            let success = outcome.is_ok();
            // 短锁回写终态；条目中途被移除时找不到 id，静默跳过。attempts 上一步已写，不动它。
            {
                let mut st = task_state.lock().unwrap();
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
                let mut c = task_completed.lock().unwrap();
                *c += 1;
                *c
            };
            task_sink.progress(QueueProgress {
                status: if success { "success" } else { "failed" }.into(),
                current: done,
                total,
                uid,
                entry_id: Some(id),
                stage: None,
            });
            *task_in_flight.lock().unwrap() -= 1;
            success
        })));
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
            }
        }
    }
    let failed = failed_ran + canceled;
    sink.finished(QueueFinished {
        finished,
        failed,
        total,
    });
    (finished, failed, total)
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
                negative_prompt: String::new(),
                size: "1024x1024".into(),
                n: 1,
                quality: None,
                seed: None,
                steps: None,
                uid: Some(uid.into()),
                name: Some(name.into()),
                template_id: None,
                call_mode: None,
                prompt_extend: None,
                watermark: None,
                prompt_extend_mode: None,
                enable_thinking: None,
                base_url: String::new(),
                // 前端提交的请求不带能力快照（由 validate_entries / generate 用服务商配置回填）。
                caps: None,
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

    /// 收集器替身：只攒事件不断言时序（进度语义已由条目状态覆盖，finished 顺带抽查）。
    #[derive(Default)]
    struct CollectSink {
        progress: Mutex<Vec<QueueProgress>>,
        finished: Mutex<Vec<QueueFinished>>,
    }

    impl EventSink for CollectSink {
        fn progress(&self, payload: QueueProgress) {
            self.progress.lock().unwrap().push(payload);
        }
        fn finished(&self, payload: QueueFinished) {
            self.finished.lock().unwrap().push(payload);
        }
    }

    /// 生成替身：按脚本弹出预置结果（弹尽后默认成功），每次调用前先执行 hook 与阶段上报。
    /// hook 供 cancel 测试在首条任务执行期间置位取消标志；report 供阶段测试注入阶段序列
    /// （可含重复项，用于验证「同阶段去重」）。
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
        ) -> Pin<Box<dyn Future<Output = Result<image_ai::GenResult, image_ai::ImageGenError>> + Send + 'a>>
        {
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

    /// 生成替身：execute 返回的 future 直接 panic，用于覆盖 run_queue 的 join-Err 兜底分支
    /// （`h.await` 收 Err 时把仍在 Running 的条目判 Internal）。
    struct PanicExecutor;

    impl GenExecutor for PanicExecutor {
        fn execute<'a>(
            &'a self,
            _data_dir: &'a Path,
            _cfg: &'a image_ai::Config,
            _key: &'a str,
            _req: &'a image_ai::GenRequest,
            _progress: &'a (dyn Fn(GenStage) + Sync),
        ) -> Pin<Box<dyn Future<Output = Result<image_ai::GenResult, image_ai::ImageGenError>> + Send + 'a>>
        {
            Box::pin(async move {
                panic!("模拟执行器 panic");
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

    /// 收集进度并观测首个终态事件（M1 回归用）：一旦收到 success 事件即置位本 sink 的
    /// `saw_success`，供 slow 替身「等兄弟收尾后再上报阶段」编排，避免依赖真实耗时。
    struct SignalSink {
        progress: Mutex<Vec<QueueProgress>>,
        finished: Mutex<Vec<QueueFinished>>,
        saw_success: Arc<AtomicBool>,
    }

    impl SignalSink {
        fn new() -> Self {
            Self {
                progress: Mutex::new(Vec::new()),
                finished: Mutex::new(Vec::new()),
                saw_success: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl EventSink for SignalSink {
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
        ) -> Pin<Box<dyn Future<Output = Result<image_ai::GenResult, image_ai::ImageGenError>> + Send + 'a>>
        {
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
        ) -> Pin<Box<dyn Future<Output = Result<image_ai::GenResult, image_ai::ImageGenError>> + Send + 'a>>
        {
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

    /// 跑一遍 run_queue 的公共骨架（退避走记录型替身，不真睡）。TempDir 先绑定再取 path
    /// （防止临时目录被提前回收）。
    async fn drive(
        state: Arc<Mutex<QueueState>>,
        concurrency: u32,
        executor: Arc<dyn GenExecutor>,
        sink: Arc<dyn EventSink>,
    ) -> (u32, u32, u32) {
        drive_with_sleeper(
            state,
            concurrency,
            executor,
            Arc::new(RecordingSleeper::default()),
            sink,
        )
        .await
    }

    /// 同上，但由调用方提供 sleeper：重试测试要断言退避序列，故需拿到替身本体。
    async fn drive_with_sleeper(
        state: Arc<Mutex<QueueState>>,
        concurrency: u32,
        executor: Arc<dyn GenExecutor>,
        sleeper: Arc<dyn Sleeper>,
        sink: Arc<dyn EventSink>,
    ) -> (u32, u32, u32) {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        run_queue(
            state,
            data_dir,
            image_ai::Config::builtin(),
            HashMap::new(),
            concurrency,
            executor,
            sleeper,
            sink,
        )
        .await
    }

    fn statuses(state: &Arc<Mutex<QueueState>>) -> Vec<QueueStatus> {
        state
            .lock()
            .unwrap()
            .entries
            .iter()
            .map(|e| e.status.clone())
            .collect()
    }

    /// 建队列的统一入口：必须经 add 走真实路径补发条目 id（直接构造会让 id 全空，
    /// run_queue 按 id 定位条目的回写就会全部命中第一条）。
    fn state_with(entries: Vec<QueueEntry>) -> Arc<Mutex<QueueState>> {
        let mut q = QueueState::default();
        q.add(entries);
        Arc::new(Mutex::new(q))
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
        assert_eq!(q.entries[0].id, q.entries[1 - 1].id); // 剩下的是 u2
        assert_eq!(q.entries[0].uid, "u2");
        // 移除不存在的 id：静默无副作用。
        q.remove(&[format!("{id}x")]);
        assert_eq!(q.entries.len(), 1);

        q.clear();
        assert!(q.snapshot().is_empty());
    }

    // ===================== run_queue（替身执行器，不触网） =====================

    #[test]
    fn run_queue_all_success_sequential_and_backfills_images() {
        let state = state_with(vec![
            entry("u1", "甲"),
            entry("u2", "乙"),
            entry("u3", "丙"),
        ]);
        // 上一次运行遗留的取消标志：run 开始时必须复位，否则这批全被误杀。
        state.lock().unwrap().cancel = true;
        let stub = Arc::new(StubExecutor::new(vec![]));
        let sink = Arc::new(CollectSink::default());
        // tokio 不是本 crate 直接依赖（不引新依赖）：async 测试经 tauri 的 async_runtime 驱动。
        let (finished, failed, total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (3, 0, 3));
        assert_eq!(
            statuses(&state),
            vec![
                QueueStatus::Success,
                QueueStatus::Success,
                QueueStatus::Success
            ]
        );
        // 成功条目回填 images；error 清空后不出现在序列化里（String::is_empty 跳过）。
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].images.len(), 1);
        assert!(snap.iter().all(|e| e.error.is_empty()));
        // 并发 1：严格按入队顺序执行。
        assert_eq!(*stub.calls.lock().unwrap(), vec!["u1", "u2", "u3"]);
        // running/success 各 3 条 + finished 1 条。
        assert_eq!(sink.progress.lock().unwrap().len(), 6);
        assert_eq!(
            *sink.finished.lock().unwrap(),
            vec![QueueFinished {
                finished: 3,
                failed: 0,
                total: 3
            }]
        );
    }

    #[test]
    fn run_queue_partial_failure_marks_entry_failed() {
        let state = state_with(vec![
            entry("u1", "甲"),
            entry("u2", "乙"),
            entry("u3", "丙"),
        ]);
        let stub = Arc::new(StubExecutor::new(vec![
            Ok(blank_result()),
            Err(image_ai::ImageGenError::new(
                image_ai::GenErrorKind::Provider,
                "模拟厂商返回错误",
            )),
            Ok(blank_result()),
        ]));
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (2, 1, 3));
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].status, QueueStatus::Success);
        assert_eq!(snap[1].status, QueueStatus::Failed);
        assert_eq!(snap[1].error, "模拟厂商返回错误");
        // 失败类别随执行器错误原样落入条目（前端据此判断是否展示「重试本条」）。
        assert_eq!(snap[1].error_kind, Some(image_ai::GenErrorKind::Provider));
        assert_eq!(snap[2].status, QueueStatus::Success);
        assert_eq!(snap[2].error_kind, None);
        // 失败条目不回填图片。
        assert!(snap[1].images.is_empty());
    }

    #[test]
    fn run_queue_maps_executor_internal_error_to_entry() {
        // 执行器以普通 Err(Internal) 收尾（走 outcome 的 Err 分支，非 panic 兜底）时，
        // 条目的类别必须落到 Internal，前端据此仍允许重试（Internal 在可重试白名单内）。
        let state = state_with(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![Err(image_ai::ImageGenError::internal(
            "任务异常退出",
        ))]));
        let sink = Arc::new(CollectSink::default());
        let (_finished, failed, _total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!(failed, 1);
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(snap[0].error, "任务异常退出");
        assert_eq!(snap[0].error_kind, Some(image_ai::GenErrorKind::Internal));
    }

    #[test]
    fn run_queue_join_err_marks_running_entry_internal_on_panic() {
        // 执行器 future 真 panic → spawn 的 JoinHandle.await 收 Err → 兜底把仍在 Running 的条目
        // 判 Failed(Internal)，避免它永远停在 Running 卡住下一次启动（这才是 panic 兜底分支）。
        let state = state_with(vec![entry("u1", "甲")]);
        let stub = Arc::new(PanicExecutor);
        let sink = Arc::new(CollectSink::default());
        let (_finished, failed, _total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!(failed, 1);
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(snap[0].error, "任务异常退出");
        assert_eq!(snap[0].error_kind, Some(image_ai::GenErrorKind::Internal));
    }

    #[test]
    fn run_queue_cancel_marks_unstarted_entries_failed() {
        let state = state_with(vec![
            entry("u1", "甲"),
            entry("u2", "乙"),
            entry("u3", "丙"),
        ]);
        let stub = Arc::new(StubExecutor::new(vec![]));
        // 首条任务执行期间置位取消：并发 1 下 u1 跑完才轮到 u2，此刻 cancel 必已生效，
        // 断言因此确定（在跑的 u1 等它自然收尾，未启动的 u2/u3 判「已取消」）。
        {
            let st = Arc::clone(&state);
            *stub.hook.lock().unwrap() = Some(Box::new(move || {
                st.lock().unwrap().cancel = true;
            }));
        }
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (1, 2, 3));
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].status, QueueStatus::Success);
        assert_eq!(snap[1].status, QueueStatus::Failed);
        assert_eq!(snap[1].error, "已取消");
        // 取消语义不变：既是 Failed，又带 Cancelled 类别（前端据此隐藏「重试本条」）。
        assert_eq!(snap[1].error_kind, Some(image_ai::GenErrorKind::Cancelled));
        assert_eq!(snap[2].status, QueueStatus::Failed);
        assert_eq!(snap[2].error, "已取消");
        assert_eq!(snap[2].error_kind, Some(image_ai::GenErrorKind::Cancelled));
        // 从未执行过的取消条目：attempts 保持 0（口径见 QueueEntry::attempts）。
        assert_eq!(snap[1].attempts, 0);
        assert_eq!(snap[2].attempts, 0);
        // 只有 u1 真正执行过。
        assert_eq!(*stub.calls.lock().unwrap(), vec!["u1"]);
    }

    #[test]
    fn run_queue_zero_concurrency_means_one() {
        let state = state_with(vec![entry("u1", "甲"), entry("u2", "乙")]);
        let stub = Arc::new(StubExecutor::new(vec![]));
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            0,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (2, 0, 2));
        assert_eq!(
            statuses(&state),
            vec![QueueStatus::Success, QueueStatus::Success]
        );
        assert_eq!(*stub.calls.lock().unwrap(), vec!["u1", "u2"]);
    }

    // ===================== 执行期重试（退避纯函数 + 任务体循环） =====================

    /// 可重试错误：Network 类别默认 retryable=true（模拟网络中断）。
    fn retryable_err(msg: &str) -> image_ai::ImageGenError {
        image_ai::ImageGenError::network(msg)
    }

    /// 不可重试错误：Provider 类别默认 retryable=false（模拟内容审核拦截）。
    fn fatal_err(msg: &str) -> image_ai::ImageGenError {
        image_ai::ImageGenError::new(image_ai::GenErrorKind::Provider, msg)
    }

    #[test]
    fn backoff_delay_follows_exponential_cap_and_retry_after() {
        // n=0：2s + 0 抖动 = 2s（首次退避）。
        assert_eq!(backoff_delay(0, None), Duration::from_millis(2000));
        // n=1：2s×2 + 250ms = 4.25s。
        assert_eq!(backoff_delay(1, None), Duration::from_millis(4250));
        // n=2：2s×4 + 500ms = 8.5s。
        assert_eq!(backoff_delay(2, None), Duration::from_millis(8500));
        // 指数增长终被 30s 上限钳制。
        assert_eq!(backoff_delay(10, None), Duration::from_secs(30));
        // Retry-After 优先于公式，且同样受 30s 上限。
        assert_eq!(backoff_delay(0, Some(7)), Duration::from_secs(7));
        assert_eq!(backoff_delay(0, Some(99)), Duration::from_secs(30));
    }

    #[test]
    fn run_queue_retries_retryable_then_succeeds() {
        let state = state_with(vec![entry("u1", "甲")]);
        // 首次可重试失败 → 重试后成功。
        let stub = Arc::new(StubExecutor::new(vec![
            Err(retryable_err("网络中断")),
            Ok(blank_result()),
        ]));
        // hook 在每次 execute 前记录条目状态：重试期间必须仍是 Running（重试不新增状态）。
        let seen: Arc<Mutex<Vec<QueueStatus>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let st = Arc::clone(&state);
            let seen = Arc::clone(&seen);
            *stub.hook.lock().unwrap() = Some(Box::new(move || {
                let cur = st.lock().unwrap().entries[0].status.clone();
                seen.lock().unwrap().push(cur);
            }));
        }
        let sleeper = Arc::new(RecordingSleeper::default());
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive_with_sleeper(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (1, 0, 1));
        // 执行两次：首发 + 一次重试。
        assert_eq!(stub.calls.lock().unwrap().len(), 2);
        // 重试前只睡一次，退避为首次档 2s（无 Retry-After）。
        assert_eq!(
            *sleeper.slept.lock().unwrap(),
            vec![Duration::from_millis(2000)]
        );
        // 两次 execute 前条目都处于 Running —— 重试不改状态。
        assert_eq!(
            *seen.lock().unwrap(),
            vec![QueueStatus::Running, QueueStatus::Running]
        );
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].status, QueueStatus::Success);
        assert_eq!(snap[0].attempts, 2, "attempts 反映真实执行次数");
        // 进度事件里不应出现「重试」专属状态：仍是 running + success 各一条。
        let statuses: Vec<String> = sink
            .progress
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.status.clone())
            .collect();
        assert_eq!(statuses, vec!["running".to_string(), "success".to_string()]);
    }

    #[test]
    fn run_queue_does_not_retry_non_retryable() {
        let state = state_with(vec![entry("u1", "甲")]);
        // 第二个结果永不消费：不可重试错误必须一次定稿。
        let stub = Arc::new(StubExecutor::new(vec![
            Err(fatal_err("内容审核未通过")),
            Ok(blank_result()),
        ]));
        let sleeper = Arc::new(RecordingSleeper::default());
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive_with_sleeper(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (0, 1, 1));
        assert_eq!(stub.calls.lock().unwrap().len(), 1, "不可重试错误只跑一次");
        assert!(sleeper.slept.lock().unwrap().is_empty(), "不重试就不退避");
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(snap[0].error, "内容审核未通过");
        assert_eq!(snap[0].attempts, 1);
    }

    #[test]
    fn run_queue_gives_up_after_max_attempts() {
        let state = state_with(vec![entry("u1", "甲")]);
        // 三连可重试失败：跑满 MAX_ATTEMPTS（3）即放弃，不再有第 4 次、不再多睡。
        let stub = Arc::new(StubExecutor::new(vec![
            Err(retryable_err("网络中断")),
            Err(retryable_err("网络中断")),
            Err(retryable_err("网络中断")),
        ]));
        let sleeper = Arc::new(RecordingSleeper::default());
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive_with_sleeper(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (0, 1, 1));
        assert_eq!(
            stub.calls.lock().unwrap().len(),
            MAX_ATTEMPTS as usize,
            "尝试次数封顶为 MAX_ATTEMPTS"
        );
        // 只在第 1、2 次失败后睡：n=0 → 2s，n=1 → 4.25s。
        assert_eq!(
            *sleeper.slept.lock().unwrap(),
            vec![Duration::from_millis(2000), Duration::from_millis(4250)]
        );
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(snap[0].error, "网络中断");
        assert_eq!(snap[0].attempts, MAX_ATTEMPTS);
    }

    #[test]
    fn run_queue_honors_retry_after_header() {
        let state = state_with(vec![entry("u1", "甲")]);
        // 限流错误带 Retry-After=7 → 退避应取 7s 而非 2s。
        let stub = Arc::new(StubExecutor::new(vec![
            Err(retryable_err("触发限流").with_retry_after(Some(7))),
            Ok(blank_result()),
        ]));
        let sleeper = Arc::new(RecordingSleeper::default());
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive_with_sleeper(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (1, 0, 1));
        assert_eq!(
            *sleeper.slept.lock().unwrap(),
            vec![Duration::from_secs(7)],
            "Retry-After 优先于退避公式"
        );
        assert_eq!(
            state.lock().unwrap().snapshot()[0].status,
            QueueStatus::Success
        );
    }

    #[test]
    fn run_queue_success_first_try_records_one_attempt() {
        let state = state_with(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![Ok(blank_result())]));
        let sleeper = Arc::new(RecordingSleeper::default());
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive_with_sleeper(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (1, 0, 1));
        assert_eq!(stub.calls.lock().unwrap().len(), 1);
        assert!(sleeper.slept.lock().unwrap().is_empty(), "一次成则无退避");
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].status, QueueStatus::Success);
        // 一次成功 attempts=1；前端「>1 才显示已重试」据此不误报。
        assert_eq!(snap[0].attempts, 1);
    }

    #[test]
    fn run_queue_cancel_during_backoff_skips_retry() {
        // 首发可重试失败、退避期间用户请求取消 ⇒ 不再发起这次重试（重试是一次新的派发），
        // 终态与主派发循环的取消分支完全一致；attempts 保留已真实发生的 1 次。
        let state = state_with(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![
            Err(retryable_err("网络中断")),
            Ok(blank_result()), // 取消后不得被消费
        ]));
        let sleeper = Arc::new(RecordingSleeper::default());
        // 在退避（sleep）期间置位取消：这正是「取消发生在退避期间」的时序。
        {
            let st = Arc::clone(&state);
            *sleeper.hook.lock().unwrap() = Some(Box::new(move || {
                st.lock().unwrap().cancel = true;
            }));
        }
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive_with_sleeper(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sleeper) as Arc<dyn Sleeper>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (0, 1, 1));
        // 执行器只被调用首发那 1 次：取消拦下了重试。
        assert_eq!(stub.calls.lock().unwrap().len(), 1, "取消后不得再发起重试");
        // 退避确实发生过一次（首发失败后），只是睡完就因取消而收手。
        assert_eq!(
            *sleeper.slept.lock().unwrap(),
            vec![Duration::from_millis(2000)]
        );
        let snap = state.lock().unwrap().snapshot();
        assert_eq!(snap[0].status, QueueStatus::Failed);
        assert_eq!(snap[0].error, "已取消");
        assert_eq!(snap[0].error_kind, Some(image_ai::GenErrorKind::Cancelled));
        // attempts 记已发生的真实执行次数（首发 1 次），不回零也不虚增。
        assert_eq!(snap[0].attempts, 1);
    }

    #[test]
    fn run_queue_reports_stage_sequence_and_completed_count() {
        // 队列层对阶段的透传契约：替身按序上报 submitting→polling→downloading，收集器应收到
        // 同序 stage；且每条事件的 current 是**已完成数**（本条未完成时为 0，终态才为 1），
        // 而非派发序号——单条队列下二者数值恰好都从 1 起，故必须锚定「运行期恒 0」才能验出区别。
        let state = state_with(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![Ok(blank_result())]));
        *stub.report.lock().unwrap() = vec![
            GenStage::Submitting,
            GenStage::Polling,
            GenStage::Downloading,
        ];
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (1, 0, 1));
        let events = sink.progress.lock().unwrap();
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
        let entry_id = state.lock().unwrap().snapshot()[0].id.clone();
        assert!(!entry_id.is_empty());
        assert!(
            events
                .iter()
                .all(|p| p.entry_id.as_deref() == Some(entry_id.as_str())),
            "进度事件必须携带所属条目的 entry_id"
        );
    }

    #[test]
    fn run_queue_dedupes_repeated_stage() {
        // 适配器可能重复上报同一阶段（如轮询期间多次自检）；队列层按阶段去重，相邻重复只发一次。
        let state = state_with(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![Ok(blank_result())]));
        *stub.report.lock().unwrap() = vec![
            GenStage::Submitting,
            GenStage::Submitting, // 相邻重复 → 丢弃
            GenStage::Polling,
            GenStage::Downloading,
            GenStage::Downloading, // 相邻重复 → 丢弃
        ];
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (1, 0, 1));
        let events = sink.progress.lock().unwrap();
        let reported: Vec<Option<String>> = events
            .iter()
            .map(|p| p.stage.clone())
            .filter(|s| s.is_some())
            .collect();
        assert_eq!(
            reported,
            vec![
                Some("submitting".into()),
                Some("polling".into()),
                Some("downloading".into()),
            ],
            "同一阶段相邻重复上报必须被去重"
        );
    }

    #[test]
    fn run_queue_reports_non_adjacent_repeat_stage() {
        // 去重只比较「上一个阶段」：非相邻的重复（Submitting→Polling→Submitting）应全部放行。
        let state = state_with(vec![entry("u1", "甲")]);
        let stub = Arc::new(StubExecutor::new(vec![Ok(blank_result())]));
        *stub.report.lock().unwrap() = vec![
            GenStage::Submitting,
            GenStage::Polling,
            GenStage::Submitting,
        ];
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            1,
            Arc::clone(&stub) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (1, 0, 1));
        let reported: Vec<Option<String>> = sink
            .progress
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.stage.clone())
            .filter(|s| s.is_some())
            .collect();
        assert_eq!(
            reported,
            vec![
                Some("submitting".into()),
                Some("polling".into()),
                Some("submitting".into()),
            ],
            "去重只比较相邻阶段，非相邻重复不得被吞"
        );
    }

    #[test]
    fn run_queue_concurrent_stage_current_never_goes_backwards() {
        // M1 回归：并发 2 下 slow 先起跑但不收尾，fast 先成功（completed=1）；slow 随后上报
        // 阶段时 current 必须即时读最新已完成数（1），不得复用启动快照（0）造成进度回跳。
        let state = state_with(vec![entry("slow", "甲"), entry("fast", "乙")]);
        let sink = Arc::new(SignalSink::new());
        let exec = Arc::new(SlowStageExecutor {
            slow_started: Arc::new(AtomicBool::new(false)),
            fast_done: Arc::clone(&sink.saw_success),
        });
        let (finished, failed, total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            2,
            Arc::clone(&exec) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        assert_eq!((finished, failed, total), (2, 0, 2));
        let events = sink.progress.lock().unwrap();
        let currents: Vec<u32> = events.iter().map(|p| p.current).collect();
        // current 单调不减：slow 的阶段事件若复用启动快照会从 1 回跳到 0。
        let mut prev = 0u32;
        for &c in &currents {
            assert!(c >= prev, "current 回跳：{:?}", currents);
            prev = c;
        }
        assert!(currents.iter().all(|&c| c <= total));
    }

    #[test]
    fn run_queue_panic_does_not_mislabel_or_overcount_siblings() {
        // M2 回归：并发 2 下一条 panic 时，仍在跑的兄弟条目不得被兜底误标 Failed(Internal)，
        // 也不得把兄弟计入 completed（否则进度会显示「已完成 > 总数」）。
        let state = state_with(vec![entry("panic", "甲"), entry("slow", "乙")]);
        let exec = Arc::new(PanicWithSibling {
            state: Arc::clone(&state),
            s_started: Arc::new(AtomicBool::new(false)),
        });
        let sink = Arc::new(CollectSink::default());
        let (finished, failed, total) = tauri::async_runtime::block_on(drive(
            Arc::clone(&state),
            2,
            Arc::clone(&exec) as Arc<dyn GenExecutor>,
            Arc::clone(&sink) as Arc<dyn EventSink>,
        ));

        // ① 一成功一失败、总数 2（兄弟未被算作失败）。
        assert_eq!((finished, failed, total), (1, 1, 2));
        // ④ 兄弟条目最终 Success，未被误标 Internal；panic 条目按 Internal 收尾。
        let snap = state.lock().unwrap().snapshot();
        let slow = snap.iter().find(|e| e.uid == "slow").unwrap();
        assert_eq!(slow.status, QueueStatus::Success);
        assert!(slow.error_kind.is_none(), "兄弟条目不得被 panic 兜底误标");
        let panicked = snap.iter().find(|e| e.uid == "panic").unwrap();
        assert_eq!(panicked.status, QueueStatus::Failed);
        assert_eq!(panicked.error_kind, Some(image_ai::GenErrorKind::Internal));

        let events = sink.progress.lock().unwrap();
        let currents: Vec<u32> = events.iter().map(|p| p.current).collect();
        // ② 任意事件 current 不得超过总数（多计会越界）。
        assert!(
            currents.iter().all(|&c| c <= total),
            "current 不得大于 total：{:?}",
            currents
        );
        // ③ current 单调不减（不回跳）。
        let mut prev = 0u32;
        for &c in &currents {
            assert!(c >= prev, "current 回跳：{:?}", currents);
            prev = c;
        }
    }

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

    #[test]
    fn add_to_queue_inner_rejects_whole_batch_and_leaves_queue_unchanged() {
        // 命令层契约（add_to_queue_inner）：整批校验失败时队列必须原样不动——合法条目也不得并入。
        let cfg = image_ai::Config::builtin();
        let queue = Mutex::new(QueueState::default());
        // 预置一条既有条目，证明失败不会动到既有内容。
        queue.lock().unwrap().add(vec![entry("u0", "已有")]);
        let before = queue.lock().unwrap().snapshot();

        let mut bad = entry("u2", "乙");
        bad.req.n = 99;
        let batch = vec![entry("u1", "甲"), bad];
        let err = crate::commands::add_to_queue_inner(&cfg, &queue, batch).unwrap_err();
        assert_eq!(err.kind, image_ai::GenErrorKind::InvalidParam);

        let after = queue.lock().unwrap().snapshot();
        assert_eq!(after.len(), before.len(), "整批拒绝时队列条目数不变");
        assert_eq!(after[0].uid, "u0", "既有条目原样保留");
        assert!(
            after.iter().all(|e| e.uid != "u1"),
            "坏条目之外的合法条目也不得被并入队列"
        );
    }
}
