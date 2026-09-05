//! JSON 配置读写：原子写入（tmp + rename），损坏时备份 .corrupt 后按缺失处理
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::Path;

pub enum LoadError {
    Missing,
    Corrupt(String),
}

pub fn load<T: DeserializeOwned>(path: &Path) -> Result<T, LoadError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing),
        Err(e) => return Err(LoadError::Corrupt(e.to_string())),
    };
    match serde_json::from_str(&text) {
        Ok(v) => Ok(v),
        Err(_) => {
            // 与 Python 版一致：损坏 → 备份 .corrupt → 调用方回退默认
            let _ = fs::copy(path, format!("{}.corrupt", path.display()));
            Err(LoadError::Corrupt(format!(
                "配置文件损坏，已备份为 {}.corrupt", path.display()
            )))
        }
    }
}

pub fn save<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    // Windows 上 std::fs::rename 会覆盖已存在的目标文件（等价 os.replace）
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}
