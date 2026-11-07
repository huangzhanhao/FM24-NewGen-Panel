//! 原子写盘 + JSON 配置的读写。
//!
//! 【原子写】先写同目录临时文件，再一次性 rename 覆盖：目标文件要么是旧的完整内容、
//! 要么是新的完整内容，不会留下半截损坏文件。`write_atomic`/`copy_atomic` 也服务
//! `config.xml` 与档景快照——全项目只保留这一份 tmp+rename 实现。
//!
//! 【JSON 错误语义】Missing = 文件不存在（首次运行），调用方回退默认配置；Corrupt = 存在但解析失败，
//! 读取侧先把原文件备份为 <路径>.corrupt 再报错（对齐 v1 行为），调用方同样回退默认，
//! 用户手改过的内容可从 .corrupt 找回。
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 临时名后缀计数：与 pid 一起保证同一进程内并发写盘不撞同一个临时文件
/// （固定名会让两次写互相踩，且失败后残留的名次不可控）。
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 目标同目录下的唯一临时路径。必须同目录，rename 才是同卷内的原子替换。
fn temp_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!("{name}.tmp-{}-{seq}", std::process::id()))
}

/// 原子写字节：先落同目录临时文件，再 rename 覆盖目标。
/// 失败方向：目标文件保持原内容不变；rename 失败时临时文件会被清掉，不在用户目录里留残骸。
pub fn write_atomic(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    let tmp = temp_sibling(path);
    fs::write(&tmp, bytes).map_err(|e| format!("写入临时文件失败（{}）: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("原子替换失败（{} → {}）: {e}", tmp.display(), path.display())
    })
}

/// 原子拷贝：`fs::copy` 覆写已存在的目标同样是截断式写，所以先拷到临时名再 rename。
pub fn copy_atomic(src: &Path, dst: &Path) -> Result<(), String> {
    let tmp = temp_sibling(dst);
    fs::copy(src, &tmp).map_err(|e| format!("拷贝临时文件失败（{}）: {e}", tmp.display()))?;
    fs::rename(&tmp, dst).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("原子替换失败（{} → {}）: {e}", tmp.display(), dst.display())
    })
}

/// 加载配置的两种失败形态。分开定义是因为调用方的应对不同：缺失直接回退默认即可；
/// 损坏要提示用户（原文件已备份为 .corrupt，可找回手改内容）。
pub enum LoadError {
    Missing,         // 文件不存在（首次运行）
    Corrupt(String), // 存在但无法解析，携带错误描述
}

/// 读取并解析 JSON 文件为 `T`。NotFound 之外的 IO 错误（权限等）同样按 Corrupt 处理——
/// 对调用方而言都是「有文件但拿不到合法配置」。
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<T, LoadError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing),
        Err(e) => return Err(LoadError::Corrupt(e.to_string())),
    };
    match serde_json::from_str(&text) {
        Ok(v) => Ok(v),
        Err(_) => {
            // 解析失败 → 先备份原文件再报 Corrupt（对齐 v1）；备份失败不影响主流程。
            let _ = fs::copy(path, format!("{}.corrupt", path.display()));
            Err(LoadError::Corrupt(format!(
                "配置文件损坏，已备份为 {}.corrupt", path.display()
            )))
        }
    }
}

/// 序列化为带缩进的 JSON 并原子写入目标文件（pretty 格式便于用户直接打开手改）。
pub fn save<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    write_atomic(path, text)
}
