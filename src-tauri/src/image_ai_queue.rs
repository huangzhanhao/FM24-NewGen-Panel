//! AI 文生图批量生成队列：条目状态机 + 并发调度执行器。
//!
//! 【与既有模块的复用关系】生成动作直接复用 `image_ai::generate`（写盘 + 会话历史），
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
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// 生成一批前的占位轮询间隔：50ms 足够细（单次生成秒级耗时），又不至于空转烧 CPU。
const SLOT_POLL: Duration = Duration::from_millis(50);

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
/// status 为 "running" | "success" | "failed"；current 是已派发序号（1 起）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueProgress {
    pub status: String,
    pub current: u32,
    pub total: u32,
    pub uid: String,
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
/// 让 run_queue 的测试用替身而不触网。
pub trait GenExecutor: Send + Sync {
    fn execute<'a>(
        &'a self,
        data_dir: &'a Path,
        cfg: &'a image_ai::Config,
        key: &'a str,
        req: &'a image_ai::GenRequest,
    ) -> Pin<Box<dyn Future<Output = Result<image_ai::GenResult, String>> + Send + 'a>>;
}

/// 真实执行器：直接调 `image_ai::generate`（写盘 + 会话历史都在那一条链上）。
pub struct RealExecutor;

impl GenExecutor for RealExecutor {
    fn execute<'a>(
        &'a self,
        data_dir: &'a Path,
        cfg: &'a image_ai::Config,
        key: &'a str,
        req: &'a image_ai::GenRequest,
    ) -> Pin<Box<dyn Future<Output = Result<image_ai::GenResult, String>> + Send + 'a>> {
        // generate 按值收请求：克隆一份（一次生成的请求体就几行提示词，克隆可忽略）。
        Box::pin(image_ai::generate(data_dir, cfg, key, req.clone()))
    }
}

/// 跑完队列里所有 Queued 条目，返回 (finished, failed, total)。
///
/// - 并发上限 `concurrency`（0 视为 1）：主循环派发即占位（`Arc<Mutex<u8>>` 计数），
///   任务收尾释放；空位不足时 50ms 轮询等待。占位在派发侧而非任务开头——spawn 到任务
///   真正起跑之间有调度窗口，若等任务自己占位，主循环会在窗口里超发。
/// - cancel==true 时：尚未派发的条目直接置 Failed(「已取消」)，在跑的等它自然收尾。
///   开跑时先把 cancel 复位 false（上一次运行遗留的标志不该取消这一次）。
/// - 每条任务开始/成功/失败都发 `gen:queue:progress`；全部收尾后发 `gen:queue:finished`。
pub async fn run_queue(
    state: Arc<Mutex<QueueState>>,
    data_dir: PathBuf,
    cfg: image_ai::Config,
    keys: HashMap<String, String>,
    concurrency: u32,
    executor: Arc<dyn GenExecutor>,
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
    // cfg/keys 只读共享：Arc 包一次，任务侧只涨引用计数不深拷贝。
    let cfg = Arc::new(cfg);
    let keys = Arc::new(keys);
    let mut handles = Vec::new();
    let mut canceled = 0u32;

    for (i, id) in ids.into_iter().enumerate() {
        let current = i as u32 + 1;
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
                let mut st = state.lock().unwrap();
                if let Some(e) = st.entries.iter_mut().find(|e| e.id == id) {
                    e.status = QueueStatus::Failed;
                    e.error = "已取消".into();
                }
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
        let task_state = Arc::clone(&state);
        let task_in_flight = Arc::clone(&in_flight);
        let task_executor = Arc::clone(&executor);
        let task_sink = Arc::clone(&sink);
        let task_data_dir = data_dir.clone();
        let task_cfg = Arc::clone(&cfg);
        let task_keys = Arc::clone(&keys);
        handles.push(tauri::async_runtime::spawn(async move {
            // 短锁置 Running，随后 emit（锁绝不跨 await）。
            {
                let mut st = task_state.lock().unwrap();
                if let Some(e) = st.entries.iter_mut().find(|e| e.id == id) {
                    e.status = QueueStatus::Running;
                }
            }
            task_sink.progress(QueueProgress {
                status: "running".into(),
                current,
                total,
                uid: uid.clone(),
            });

            let key = task_keys
                .get(&req.provider_id)
                .map(|s| s.as_str())
                .unwrap_or("");
            let outcome = task_executor
                .execute(&task_data_dir, &task_cfg, key, &req)
                .await;

            let success = outcome.is_ok();
            // 短锁回写终态；条目中途被移除时找不到 id，静默跳过。
            {
                let mut st = task_state.lock().unwrap();
                if let Some(e) = st.entries.iter_mut().find(|e| e.id == id) {
                    match outcome {
                        Ok(res) => {
                            e.status = QueueStatus::Success;
                            e.images = res.images;
                            e.error = String::new();
                        }
                        Err(msg) => {
                            e.status = QueueStatus::Failed;
                            e.error = msg;
                        }
                    }
                }
            }
            task_sink.progress(QueueProgress {
                status: if success { "success" } else { "failed" }.into(),
                current,
                total,
                uid,
            });
            *task_in_flight.lock().unwrap() -= 1;
            (id, success)
        }));
    }

    // 等所有在跑任务收尾（cancel 只影响「尚未派发」的条目，在跑的必须等完）。
    let mut finished = 0u32;
    let mut failed_ran = 0u32;
    for h in handles {
        match h.await {
            Ok((_, true)) => finished += 1,
            Ok((_, false)) => failed_ran += 1,
            Err(_) => {
                // 任务 panic：兜底把条目判失败，避免它永远停在 Running 卡住下一次启动。
                // （handle 不带结果，id 由任务正常返回时才有；panic 场景扫 Running 条目收尾。）
                let mut st = state.lock().unwrap();
                for e in st
                    .entries
                    .iter_mut()
                    .filter(|e| e.status == QueueStatus::Running)
                {
                    e.status = QueueStatus::Failed;
                    e.error = "任务异常退出".into();
                }
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
                prompt_extend: None,
                watermark: None,
                prompt_extend_mode: None,
                enable_thinking: None,
                base_url: String::new(),
                supports_response_format: false,
                supports_negative_prompt: false,
                supports_prompt_extend: false,
                supports_watermark: false,
                supports_seed: false,
                supports_n: true,
                supports_prompt_extend_mode: false,
                prompt_extend_mode_default: "direct".into(),
                supports_enable_thinking: false,
                enable_thinking_default: true,
                watermark_default: false,
            },
            status: QueueStatus::Queued,
            images: vec![],
            error: String::new(),
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

    /// 生成替身：按脚本弹出预置结果（弹尽后默认成功），每次调用前先执行 hook。
    /// hook 供 cancel 测试在首条任务执行期间置位取消标志。
    struct StubExecutor {
        results: Mutex<VecDeque<Result<image_ai::GenResult, String>>>,
        calls: Mutex<Vec<String>>,
        hook: Mutex<Option<Box<dyn Fn() + Send>>>,
    }

    impl StubExecutor {
        fn new(results: Vec<Result<image_ai::GenResult, String>>) -> Self {
            Self {
                results: Mutex::new(results.into()),
                calls: Mutex::new(Vec::new()),
                hook: Mutex::new(None),
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
        ) -> Pin<Box<dyn Future<Output = Result<image_ai::GenResult, String>> + Send + 'a>>
        {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(req.uid.clone().unwrap_or_default());
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

    /// 跑一遍 run_queue 的公共骨架。TempDir 先绑定再取 path（防止临时目录被提前回收）。
    async fn drive(
        state: Arc<Mutex<QueueState>>,
        concurrency: u32,
        executor: Arc<dyn GenExecutor>,
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
            Err("模拟厂商返回错误".into()),
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
        assert_eq!(snap[2].status, QueueStatus::Success);
        // 失败条目不回填图片。
        assert!(snap[1].images.is_empty());
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
        assert_eq!(snap[2].status, QueueStatus::Failed);
        assert_eq!(snap[2].error, "已取消");
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
}
