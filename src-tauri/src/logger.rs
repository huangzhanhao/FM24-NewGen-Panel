//! 应用日志：环形缓冲（1 万条）+ 10MB×3 滚动文件 + mpsc 批量事件
//! 对等 NewGanLogManager，但 UI 侧改为经 "log:batch" 事件批量推送（100ms）
use serde::Serialize;
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{LazyLock, Mutex};
use tauri::Emitter;

const RING_MAX: usize = 10_000;
const MAX_BYTES: u64 = 10 * 1024 * 1024;
const BACKUP_COUNT: usize = 3;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub ts: String,
    pub level: String,
    pub module: String,
    pub message: String,
}

pub const LEVELS: [&str; 5] = ["DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL"];

static LEVEL_ORDER: LazyLock<std::collections::HashMap<&'static str, i32>> = LazyLock::new(|| {
    LEVELS.iter().enumerate().map(|(i, l)| (*l, i as i32)).collect()
});

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    pub total: usize,
    pub items: Vec<LogLine>,
}

pub struct Logger {
    buf: Mutex<VecDeque<LogLine>>,
    path: PathBuf,
    tx: Sender<LogLine>,
}

impl Logger {
    pub fn new(data_dir: &Path) -> Result<(Self, Receiver<LogLine>), String> {
        fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let path = data_dir.join("newgan.log");
        let (tx, rx) = std::sync::mpsc::channel();
        Ok((Self { buf: Mutex::new(VecDeque::new()), path, tx }, rx))
    }

    pub fn log(&self, level: &str, module: &str, msg: impl Into<String>) {
        let line = LogLine {
            ts: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            level: level.into(),
            module: module.into(),
            message: msg.into(),
        };
        self.write_file(&line);
        let mut buf = self.buf.lock().unwrap();
        buf.push_back(line.clone());
        if buf.len() > RING_MAX {
            buf.pop_front();
        }
        let _ = self.tx.send(line);
    }

    pub fn info(&self, module: &str, msg: impl Into<String>) { self.log("INFO", module, msg) }
    pub fn warn(&self, module: &str, msg: impl Into<String>) { self.log("WARNING", module, msg) }
    pub fn error(&self, module: &str, msg: impl Into<String>) { self.log("ERROR", module, msg) }

    fn write_file(&self, line: &LogLine) {
        if let Ok(meta) = fs::metadata(&self.path) {
            if meta.len() > MAX_BYTES {
                self.rotate();
            }
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "| {} | NewGAN App | {} | {} | - {}", line.ts, line.level, line.module, line.message);
        }
    }

    fn rotate(&self) {
        // newgan.log → .1 → .2 → .3（最多 3 份备份，对等 v1）
        for i in (1..BACKUP_COUNT).rev() {
            let from = self.path.with_extension(format!("log.{i}"));
            let to = self.path.with_extension(format!("log.{}", i + 1));
            if from.is_file() {
                let _ = fs::rename(&from, &to);
            }
        }
        let _ = fs::rename(&self.path, self.path.with_extension("log.1"));
    }

    /// 按级别阈值（或仅该级别）过滤后分页（对等 LogTab._filter_log）
    pub fn query(&self, level: &str, only: bool, offset: usize, limit: usize) -> LogPage {
        let want = LEVEL_ORDER.get(level).copied().unwrap_or(1);
        let buf = self.buf.lock().unwrap();
        let all: Vec<&LogLine> = buf
            .iter()
            .filter(|l| {
                let cur = LEVEL_ORDER.get(l.level.as_str()).copied().unwrap_or(1);
                if only { cur == want } else { cur >= want }
            })
            .collect();
        let total = all.len();
        LogPage {
            total,
            items: all.into_iter().skip(offset).take(limit).cloned().collect(),
        }
    }

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

/// 每 100ms 批量把日志推给前端（避免逐条事件冲击 IPC）
pub fn spawn_emitter(app: tauri::AppHandle, rx: Receiver<LogLine>) {
    std::thread::spawn(move || loop {
        let mut batch = Vec::new();
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(first) => batch.push(first),
            Err(_) => continue,
        }
        while let Ok(line) = rx.try_recv() {
            batch.push(line);
            if batch.len() >= 500 {
                break;
            }
        }
        let _ = app.emit("log:batch", batch);
    });
}
