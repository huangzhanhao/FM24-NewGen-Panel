//! JSON 配置的原子读写：写走 tmp + rename，读按「缺失 / 损坏」区分错误。
//!
//! 【为什么原子写入】直接覆写目标文件时，写到一半断电/崩溃会留下半截损坏文件；先写临时文件、
//! 再一次性 rename 覆盖，目标文件要么是旧的完整内容、要么是新的完整内容。
//!
//! 【错误语义】Missing = 文件不存在（首次运行），调用方回退默认配置；Corrupt = 存在但解析失败，
//! 读取侧先把原文件备份为 <路径>.corrupt 再报错（对齐 v1 行为），调用方同样回退默认，
//! 用户手改过的内容可从 .corrupt 找回。
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::Path;

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
    // 临时文件与目标同目录只差扩展名，保证 rename 是同一卷内的原子操作。
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    // Windows 上 rename 覆盖已存在的目标文件，一步完成原子替换。
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}
