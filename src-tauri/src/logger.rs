//! 应用日志：官方 tauri-plugin-log 插件承担管道，本模块只补「官方没有」的内存缓冲。
//!
//! 【分工】（重构前全部手写；重构后同功能用官方、缺口自己补——功能无损）
//!   官方插件（tauri-plugin-log）负责：
//!     ① log 门面：整个 Rust 生态（tauri / tao / wry 等依赖库）的日志统一汇入一处；
//!     ② Stdout target：控制台输出（开发调试直读）；
//!     ③ Folder target：滚动文件 newgan.log（10MB，滚动保留最近 3 份、文件名带日期）；
//!        行格式经 Target::format 保持 v1 兼容（见 v1_line）。
//!   官方生态没有、本模块自研补齐的：
//!     ④ LogMemory 内存环形缓冲（1 万条）供前端 query_logs 分页回查——官方没有「回查历史」能力；
//!     ⑤ mpsc 批量事件（100ms / 单批上限 500）推给前端 log:batch，避免逐条 IPC 卡顿。
//!
//! 【数据流】任何 log::info!(target: "replace", ...) → 插件分发给三个 target：
//!   stdout 打印 → 滚动文件落盘 → fern 闭包把原始记录组装成 LogLine 入缓冲+通道
//!   → 后台线程聚批 emit("log:batch") → 前端日志面板。
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, LazyLock, Mutex};
use tauri::Emitter;

const RING_MAX: usize = 10_000; // 内存环形缓冲最大条数

// 级别口径对齐 Rust log 门面（Trace/Debug/Info/Warn/Error），
// 显示串沿用 v1 拼写（WARN→WARNING）；旧表的 CRITICAL 无产生源，由 TRACE 接替。
// 日志级别从低到高的固定顺序（用于「阈值过滤」：选 WARNING 则只看 WARNING 及以上）。
pub const LEVELS: [&str; 5] = ["TRACE", "DEBUG", "INFO", "WARNING", "ERROR"];

// 把「级别名 → 序号」预先算好存入字典，过滤时直接查序号比大小，避免重复计算。
static LEVEL_ORDER: LazyLock<std::collections::HashMap<&'static str, i32>> = LazyLock::new(|| {
    LEVELS.iter().enumerate().map(|(i, l)| (*l, i as i32)).collect()
});

/// log 门面级别 → 显示串（大写、v1 拼写：WARN 拼成 WARNING 保持旧日志格式）。
pub fn level_label(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "ERROR",
        log::Level::Warn => "WARNING",
        log::Level::Info => "INFO",
        log::Level::Debug => "DEBUG",
        log::Level::Trace => "TRACE",
    }
}

/// v1 兼容的文件行格式。格式是「对等旧版 Python 日志」的契约，勿随意改动。
pub fn v1_line(ts: &str, level: &str, module: &str, message: &str) -> String {
    format!("| {ts} | NewGAN App | {level} | {module} | - {message}")
}

// camelCase 序列化与前端 types.ts 对齐。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub ts: String,      // timestamp 时间戳（毫秒精度，便于区分同秒内的先后）
    pub level: String,   // 日志级别（level_label 产出的大写名）
    pub module: String,  // 来源模块 = log 门面的 target（如 "app"/"replace"；依赖库日志则是其 target 名）
    pub message: String, // 日志正文
}

/// 分页查询的返回体：复用通用分页契约 `PageResult<LogLine>`（前端日志面板按需拉取）。
pub type LogPage = crate::paging::PageResult<LogLine>;

/// 内存环形缓冲 + 事件通道（官方生态没有的部分，自研补齐）。
/// 字段线程安全：Mutex 保护缓冲，Sender 可跨线程克隆发送。
pub struct LogMemory {
    buf: Mutex<VecDeque<LogLine>>, // 内存环形缓冲（Mutex 保证多线程读写安全）
    tx: Sender<LogLine>,           // 发送端：新日志投入这里，后台线程取走发事件
}

impl LogMemory {
    /// 创建（缓冲, 接收端）对：缓冲经 Arc 共享给 fern 闭包与 Logger，接收端交给后台发事件的线程。
    pub fn channel() -> (Self, Receiver<LogLine>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { buf: Mutex::new(VecDeque::new()), tx }, rx)
    }

    /// fern 自定义 target 的入口：把一条 log 门面记录转成 LogLine，入环形缓冲 + 事件通道。
    /// 由 lib.rs 里 TargetKind::Dispatch 的 fern 闭包回调（官方插件分发的第三路）。
    pub fn record(&self, record: &log::Record<'_>) {
        self.push(LogLine {
            ts: chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            level: level_label(record.level()).into(),
            module: record.target().into(),
            message: record.args().to_string(),
        });
    }

    fn push(&self, line: LogLine) {
        // unwrap_or_else(into_inner)：锁被污染（持有线程 panic 过）时取回数据继续用——
        // 日志器不能把应用搞挂。
        let mut buf = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        buf.push_back(line.clone());       // 从尾部入队（clone 因为后面还要把原件发给通道）
        if buf.len() > RING_MAX {
            buf.pop_front();               // 超过上限就从头部丢弃最旧的一条（环形缓冲）
        }
        let _ = self.tx.send(line);        // 推入通道，交给后台线程发事件给前端
    }

    /// 按级别阈值（或仅该级别）过滤后分页（对等旧版 LogTab._filter_log）。
    /// only=true 只看该级别；only=false 看该级别及以上。offset/limit 实现分页。
    pub fn query(&self, level: &str, only: bool, offset: usize, limit: usize) -> LogPage {
        // want = 目标级别的序号（未知级别兜底为 INFO）。
        let want = LEVEL_ORDER.get(level).copied().unwrap_or(LEVEL_ORDER["INFO"]);
        let buf = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        let all: Vec<&LogLine> = buf
            .iter()
            .filter(|l| {
                let cur = LEVEL_ORDER.get(l.level.as_str()).copied().unwrap_or(LEVEL_ORDER["INFO"]);
                if only { cur == want } else { cur >= want }
            })
            .collect();
        let total = all.len();
        LogPage {
            total,
            items: all.into_iter().skip(offset).take(limit).cloned().collect(),
        }
    }
}

/// 应用日志门面：级别 API（trace/debug/info/warn/error）全部由 `LogSink` trait 提供，
/// 统一经 `write` 转发给 log 门面宏，再由官方插件三个 target（stdout / 滚动文件 / 内存缓冲）消化。
/// Logger 自身只随身携带「打开日志文件」的路径与「回查历史」的内存缓冲。
pub struct Logger {
    path: PathBuf,          // 日志文件路径（前端「打开日志文件」按钮用）
    memory: Arc<LogMemory>, // 内存环形缓冲（query 回查用）
}

impl Logger {
    /// 创建日志门面：`data_dir` 决定日志文件路径（`<data_dir>/newgan.log`），
    /// `memory` 为自研内存环形缓冲（`query` 回查历史用），须与 lib.rs 里那份 Arc 指向同一实例。
    pub fn new(data_dir: &Path, memory: Arc<LogMemory>) -> Self {
        Self { path: data_dir.join("newgan.log"), memory }
    }

    /// 对外暴露日志文件路径。
    pub fn path(&self) -> &Path { &self.path }

    /// 回查历史（代理到内存缓冲；对等旧版 LogTab._filter_log）。
    pub fn query(&self, level: &str, only: bool, offset: usize, limit: usize) -> LogPage {
        self.memory.query(level, only, offset, limit)
    }
}

/// 可替换的日志出口（测试缝）：生产 = 全局 log 门面（官方插件承接）；
/// 测试 = ChannelLogger 把日志送进通道供断言（单元测试装不了全局 logger）。
/// 泛型默认方法保持 run() 等调用点 `logger.info("模块", 消息)` 的写法不变。
pub trait LogSink: Send + Sync {
    /// 唯一实现点：把「一条消息 + 级别 + 模块名」发出去（生产转发全局 log 门面，测试进通道）。
    fn write(&self, level: log::Level, module: &str, msg: String);
    // 五个级别快捷方法：全部转发给 write，级别语义由 log::Level 承载（调用点写法与旧固有方法一致）。
    // 注意生产阈值为 Info（见 lib.rs），trace/debug（现用于 rtf_scanner 的逐行/统计细节）默认不落盘、不进前端。
    fn trace(&self, module: &str, msg: impl Into<String>) { self.write(log::Level::Trace, module, msg.into()) }
    fn debug(&self, module: &str, msg: impl Into<String>) { self.write(log::Level::Debug, module, msg.into()) }
    fn info(&self, module: &str, msg: impl Into<String>) { self.write(log::Level::Info, module, msg.into()) }
    fn warn(&self, module: &str, msg: impl Into<String>) { self.write(log::Level::Warn, module, msg.into()) }
    fn error(&self, module: &str, msg: impl Into<String>) { self.write(log::Level::Error, module, msg.into()) }
}

impl LogSink for Logger {
    fn write(&self, level: log::Level, module: &str, msg: String) {
        log::log!(target: module, level, "{}", msg);
    }
}

/// 对象安全的日志出口：只留唯一那个 `write`，供需要**按 trait 对象持有**日志器的地方使用
/// （如 `ProfileManager` 后置注入的 `Arc<dyn …>` 字段）。
/// 【为什么不直接存 `Arc<dyn LogSink>`】`LogSink` 的五个便捷方法收 `impl Into<String>`
/// （泛型方法）→ 该 trait **不是 dyn 兼容的**，无法组成 trait 对象。
/// 实现由下面的 blanket impl 自动提供（库里任何 `LogSink` 都是 `LogSinkDyn`），
/// 于是「调用点写 `info/warn/…`」与「字段存 trait 对象」两件事都不必将就。
pub trait LogSinkDyn: Send + Sync {
    fn write(&self, level: log::Level, module: &str, msg: String);
}

impl<T: LogSink + ?Sized> LogSinkDyn for T {
    fn write(&self, level: log::Level, module: &str, msg: String) {
        LogSink::write(self, level, module, msg)
    }
}

/// 测试用捕获器：日志进通道，测试端 try_iter 收集断言（仅测试构建参与编译）。
#[cfg(test)]
pub struct ChannelLogger(Sender<LogLine>);

#[cfg(test)]
impl ChannelLogger {
    pub fn channel() -> (Self, Receiver<LogLine>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self(tx), rx)
    }
}

#[cfg(test)]
impl LogSink for ChannelLogger {
    fn write(&self, level: log::Level, module: &str, msg: String) {
        // 【刻意不按级别过滤】rtf_scanner / profile_manager 的一批测试就是靠断言 debug 行来校
        // 「丢了多少行、为什么丢」这类诊断是否真的发出；生产侧的 Info 阈值在 log 门面上，不在这儿。
        let _ = self.0.send(LogLine {
            ts: chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
            level: level_label(level).into(),
            module: module.into(),
            message: msg,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_line_format_is_stable() {
        // 文件行格式是对等 v1 的契约：| 时间 | NewGAN App | 级别 | 模块 | - 正文
        let line = v1_line("2026-09-10 12:00:00", "WARNING", "replace", "测试消息");
        assert_eq!(line, "| 2026-09-10 12:00:00 | NewGAN App | WARNING | replace | - 测试消息");
    }

    #[test]
    fn level_label_uses_v1_spelling() {
        assert_eq!(level_label(log::Level::Warn), "WARNING");
        assert_eq!(level_label(log::Level::Trace), "TRACE");
    }

    /// 官方插件分发的第三路（fern 自定义输出）：Record 经 LogMemory 必须同时
    /// 落进环形缓冲（可 query 回填）与事件通道（前端增量）。此路径守住「启动行
    /// 在 WebView 订阅 log:batch 之前就已产生，前端只能靠 query_logs 回填」的约定。
    #[test]
    fn record_reaches_ring_buffer_and_channel() {
        let (memory, rx) = LogMemory::channel();
        let record = log::Record::builder()
            .level(log::Level::Info)
            .target("app")
            .args(format_args!("Starting Application v2"))
            .build();
        memory.record(&record);

        assert_eq!(rx.try_iter().count(), 1, "启动行应已进入事件通道");

        let page = memory.query("INFO", false, 0, 10_000);
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].module, "app");
        assert_eq!(page.items[0].message, "Starting Application v2");
        assert_eq!(page.items[0].level, "INFO");
        // 时间戳含毫秒（.3f 格式固定产出 3 位小数）。
        let ts = &page.items[0].ts;
        assert_eq!(ts.len(), "2026-09-10 12:00:00.000".len(), "时间戳应含毫秒：{ts}");

        // 阈值过滤：WARNING 以上不包含 INFO 启动行
        assert_eq!(memory.query("WARNING", false, 0, 10_000).total, 0);
    }

    #[test]
    fn ring_buffer_trims_to_max() {
        let (memory, _rx) = LogMemory::channel();
        for i in 0..(RING_MAX + 5) {
            memory.push(LogLine {
                ts: String::new(),
                level: "INFO".into(),
                module: "test".into(),
                message: format!("{i}"),
            });
        }
        let page = memory.query("INFO", false, 0, 10_000);
        assert_eq!(page.total, RING_MAX, "应只保留最新 1 万条");
        assert_eq!(page.items[0].message, "5", "最旧的 5 条应被从头部丢弃");
    }

    /// `query` 的两种过滤模式与分页（前端日志面板选级别、翻页都走它）：
    /// only=true 只取该级别（与阈值模式相反，选 WARNING 不应带出 ERROR）；
    /// offset/limit 只切窗口，total 恒为过滤后的总条数（前端据此算页数）。
    #[test]
    fn query_filters_by_level_and_pages() {
        let (memory, _rx) = LogMemory::channel();
        for (level, msg) in [
            ("INFO", "i0"),
            ("WARNING", "w0"),
            ("INFO", "i1"),
            ("ERROR", "e0"),
            ("INFO", "i2"),
        ] {
            memory.push(LogLine {
                ts: String::new(),
                level: level.into(),
                module: "test".into(),
                message: msg.into(),
            });
        }
        // only=true：精确级别，WARNING 不带出 ERROR。
        let page = memory.query("WARNING", true, 0, 10);
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].message, "w0");
        // only=false：阈值模式，WARNING 及以上含 ERROR。
        assert_eq!(memory.query("WARNING", false, 0, 10).total, 2);
        // 分页窗口：total 仍是过滤后的 3 条，items 只给窗口内那 1 条。
        let page = memory.query("INFO", true, 1, 1);
        assert_eq!((page.total, page.items.len()), (3, 1));
        assert_eq!(page.items[0].message, "i1");
        // 未知级别按 INFO 兜底（前端传了脏值时不至于查空）。
        assert_eq!(memory.query("NOPE", true, 0, 10).total, 3);
    }

    #[test]
    fn level_filter_is_monotonic_across_all_levels() {
        // 阈值语义是「该级别及以上」：TRACE 全收，ERROR 只收 ERROR 本身。
        let (memory, _rx) = LogMemory::channel();
        for level in LEVELS {
            memory.push(LogLine {
                ts: String::new(),
                level: level.into(),
                module: "test".into(),
                message: level.into(),
            });
        }
        assert_eq!(memory.query("TRACE", false, 0, 99).total, LEVELS.len());
        assert_eq!(memory.query("ERROR", false, 0, 99).total, 1);
        assert_eq!(memory.query("WARNING", false, 0, 99).total, 2);
    }
}

/// 每 100ms 批量把日志推给前端（避免逐条事件冲击 IPC）。
/// 后台线程死循环：最多等 100ms 收到第一条，再把通道里剩余的一起取走（上限 500），
/// 一次性 emit("log:batch", batch) 发给前端（前端在 stores 里 listen("log:batch")）。
pub fn spawn_emitter(app: tauri::AppHandle, rx: Receiver<LogLine>) {
    std::thread::spawn(move || loop {
        let mut batch = Vec::new();
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(first) => batch.push(first),
            Err(_) => continue, // 超时无消息：下一轮继续等
        }
        while let Ok(line) = rx.try_recv() {
            batch.push(line);
            if batch.len() >= 500 {
                break; // 单批上限，防止一次 emit 载荷过大
            }
        }
        let _ = app.emit("log:batch", batch);
    });
}
