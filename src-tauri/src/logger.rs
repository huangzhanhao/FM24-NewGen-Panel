//! 【新手导读】应用日志：环形缓冲（1 万条）+ 10MB×3 滚动文件 + mpsc 批量事件。
//! 对等旧版 NewGanLogManager，但 UI 侧改为经 "log:batch" 事件批量推送（100ms）。
//!
//! 【三个存储层次】
//!   1) 内存环形缓冲：保留最近 1 万条，供前端随时分页回查（快）；
//!   2) 磁盘日志文件：永久保留，过大时滚动备份（newgan.log → .1 → .2 → .3）；
//!   3) mpsc 通道：新日志通过它传给后台线程，再按批发事件给前端（避免逐条 IPC 卡顿）。
use serde::Serialize;
use std::collections::VecDeque;              // 双端队列：可在两头增删，实现「环形缓冲」（满时从头部丢弃）
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};     // mpsc = 多生产者单消费者通道（线程间传递消息）
use std::sync::{LazyLock, Mutex};
use tauri::Emitter;                          // 引入 Emitter trait 才能用 app.emit() 向前端发事件

const RING_MAX: usize = 10_000;              // 内存环形缓冲最大条数（_ 是数字分隔符，仅为易读）
const MAX_BYTES: u64 = 10 * 1024 * 1024;     // 单个日志文件上限 10MB，超过则滚动
const BACKUP_COUNT: usize = 3;               // 最多保留 3 份历史备份

// #[derive(...)] 自动为结构体生成常用能力：
//   Serialize → 可转 JSON 发给前端；Clone → 可复制；PartialEq → 可用 == 比较（测试用）。
// #[serde(rename_all = "camelCase")] → Rust 的蛇形字段（如 is_newgan）序列化为前端的驼峰名（isNewgan）。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub ts: String,      // timestamp 时间戳
    pub level: String,   // 日志级别（DEBUG/INFO/...）
    pub module: String,  // 来源模块（如 "app"/"replace"）
    pub message: String, // 日志正文
}

// 日志级别从低到高的固定顺序（用于「阈值过滤」：选 WARNING 则只看 WARNING 及以上）。
pub const LEVELS: [&str; 5] = ["DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"];

// 把「级别名 → 序号」预先算好存入字典，过滤时直接查序号比大小，避免重复计算。
static LEVEL_ORDER: LazyLock<std::collections::HashMap<&'static str, i32>> = LazyLock::new(|| {
    LEVELS.iter().enumerate().map(|(i, l)| (*l, i as i32)).collect()
});

/// 分页查询的返回体：总条数 + 当前页的条目（前端日志面板按需拉取）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    pub total: usize,       // 过滤后总条数（用于前端分页）
    pub items: Vec<LogLine>, // 当前页实际返回的条目
}

/// 日志器本体。字段都是线程安全的：Mutex 保护缓冲，Sender 可跨线程克隆发送。
pub struct Logger {
    buf: Mutex<VecDeque<LogLine>>, // 内存环形缓冲（Mutex 保证多线程读写安全）
    path: PathBuf,                 // 日志文件路径
    tx: Sender<LogLine>,           // 发送端：新日志投入这里，后台线程取走发事件
}

impl Logger {
    /// 创建日志器。返回（Logger, Receiver）元组：Logger 给应用用，Receiver 交给后台发事件的线程。
    /// mpsc::channel() 创建一对「发送端 tx / 接收端 rx」，一端写入、另一端取出。
    pub fn new(data_dir: &Path) -> Result<(Self, Receiver<LogLine>), String> {
        fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let path = data_dir.join("newgan.log");
        let (tx, rx) = std::sync::mpsc::channel();
        Ok((Self { buf: Mutex::new(VecDeque::new()), path, tx }, rx))
    }

    /// 写一条日志（核心方法）：同时落盘、入内存环形缓冲、推入事件通道。
    /// msg: impl Into<String> 表示「任何能转成 String 的类型」（&str、String 都行），调用更方便。
    pub fn log(&self, level: &str, module: &str, msg: impl Into<String>) {
        // 组装一条日志，时间戳用 chrono 库格式化为本地时间。
        let line = LogLine {
            ts: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            level: level.into(),
            module: module.into(),
            message: msg.into(),
        };
        self.write_file(&line);            // 1) 写入磁盘文件
        let mut buf = self.buf.lock().unwrap(); // 2) 加锁后写入内存缓冲（lock() 获取互斥锁）
        buf.push_back(line.clone());       // 从尾部入队（clone 因为后面还要把原件发给通道）
        if buf.len() > RING_MAX {
            buf.pop_front();               // 超过上限就从头部丢弃最旧的一条（环形缓冲）
        }
        let _ = self.tx.send(line);        // 3) 推入通道，交给后台线程发事件给前端
    }

    // 下面三个是常用级别的快捷方法，内部都转调 log()。
    pub fn info(&self, module: &str, msg: impl Into<String>) { self.log("INFO", module, msg) }
    pub fn warn(&self, module: &str, msg: impl Into<String>) { self.log("WARNING", module, msg) }
    pub fn error(&self, module: &str, msg: impl Into<String>) { self.log("ERROR", module, msg) }

    /// 把一条日志追写到磁盘文件（写前先看文件是否过大，过大先滚动）。
    fn write_file(&self, line: &LogLine) {
        // metadata 拿到文件信息；若已存在且超过 10MB，先执行滚动备份。
        if let Ok(meta) = fs::metadata(&self.path) {
            if meta.len() > MAX_BYTES {
                self.rotate();
            }
        }
        // OpenOptions 以「不存在则创建 + 追写」模式打开， writeln! 写入一行（格式化同 v1 日志）。
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "| {} | NewGAN App | {} | {} | - {}", line.ts, line.level, line.module, line.message);
        }
    }

    /// 日志文件滚动：newgan.log → .1 → .2 → .3（最多 3 份备份，对等 v1）。
    fn rotate(&self) {
        // 从后往前重命名（.2→.3、.1→.2），避免覆盖。rev() 把区间倒过来遍历。
        for i in (1..BACKUP_COUNT).rev() {
            let from = self.path.with_extension(format!("log.{i}"));
            let to = self.path.with_extension(format!("log.{}", i + 1));
            if from.is_file() {
                let _ = fs::rename(&from, &to);
            }
        }
        // 最后把当前 newgan.log 重命名为 .1，下次写入会重新创建一个空的新文件。
        let _ = fs::rename(&self.path, self.path.with_extension("log.1"));
    }

    /// 按级别阈值（或仅该级别）过滤后分页（对等旧版 LogTab._filter_log）。
    /// only=true 只看该级别；only=false 看该级别及以上。offset/limit 实现分页。
    pub fn query(&self, level: &str, only: bool, offset: usize, limit: usize) -> LogPage {
        // want = 目标级别的序号（未知级别兑底为 1=INFO）。
        let want = LEVEL_ORDER.get(level).copied().unwrap_or(1);
        let buf = self.buf.lock().unwrap(); // 加锁读取内存缓冲
        // filter 逐条判断是否保留：取该条级别序号 cur，按 only 决定「等于」还是「大于等于」want。
        let all: Vec<&LogLine> = buf
            .iter()
            .filter(|l| {
                let cur = LEVEL_ORDER.get(l.level.as_str()).copied().unwrap_or(1);
                if only { cur == want } else { cur >= want }
            })
            .collect();
        let total = all.len(); // 先记下总数，再切片取当前页
        LogPage {
            total,
            // skip(offset) 跳过前面的，take(limit) 取 limit 条，cloned() 把 &LogLine 变为 LogLine。
            items: all.into_iter().skip(offset).take(limit).cloned().collect(),
        }
    }

    /// 对外暴露日志文件路径（前端「打开日志文件」按钮用）。
    pub fn path(&self) -> &Path { &self.path }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 启动行必须同时落在日志文件与可查询的环形缓冲里：
    /// 它在 WebView 订阅 `log:batch` 之前就已 emit（Tauri 不重放事件），
    /// 前端只能靠 `query_logs` 回填看到它 —— 本测试守住这条回填路径。
    #[test]
    fn startup_line_survives_for_history_query() {
        let dir = tempfile::tempdir().unwrap();
        let (logger, rx) = Logger::new(dir.path()).unwrap();
        logger.info("app", "Starting Application v2");

        assert_eq!(rx.try_iter().count(), 1, "启动行应已进入事件通道");

        let page = logger.query("INFO", false, 0, 10_000);
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].message, "Starting Application v2");

        let body = std::fs::read_to_string(dir.path().join("newgan.log")).unwrap();
        assert!(body.contains("Starting Application v2"), "文件里也应有启动行: {body}");

        // 阈值过滤：WARNING 以上不包含 INFO 启动行
        assert_eq!(logger.query("WARNING", false, 0, 10_000).total, 0);
    }
}

/// 每 100ms 批量把日志推给前端（避免逐条事件冲击 IPC）。
/// 【机制】开一个后台线程死循环：阻塞等待最多 100ms 收到第一条，再把通道里剩余的一起取走（上限 500），
/// 一次性 emit("log:batch", batch) 发给前端。move 闭包把 app、rx 的所有权移进线程。
pub fn spawn_emitter(app: tauri::AppHandle, rx: Receiver<LogLine>) {
    std::thread::spawn(move || loop {
        let mut batch = Vec::new();
        // recv_timeout：最多等 100ms。收到第一条就放进 batch；超时无消息则 continue 继续下一轮。
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(first) => batch.push(first),
            Err(_) => continue,
        }
        // try_recv 非阻塞地把通道里已有的全部取出（最多 500 条，防止单次批量过大）。
        while let Ok(line) = rx.try_recv() {
            batch.push(line);
            if batch.len() >= 500 {
                break;
            }
        }
        // app.emit 把整批日志以事件形式发给所有前端监听者（前端在 stores 里 listen("log:batch")）。
        let _ = app.emit("log:batch", batch);
    });
}
