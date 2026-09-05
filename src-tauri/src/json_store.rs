//! 【新手导读】JSON 配置读写：原子写入（tmp + rename），损坏时备份 .corrupt 后按缺失处理。
//! 【为什么用原子写入】直接覆写目标文件时，如果写到一半断电/崩溃，会留下一个「半截」的损坏文件。
//! 先写临时文件、再一次性 rename 覆盖，可以保证目标文件要么是旧的完整内容、要么是新的完整内容。
use serde::de::DeserializeOwned; // 能「从任意生命周期反序列化」的类型约束（泛型 T 需要）
use serde::Serialize;            // 能「序列化为 JSON」的类型约束
use std::fs;
use std::path::Path;

/// 加载配置时可能遇到的两种错误，分开处理：文件不存在 vs 文件损坏。
pub enum LoadError {
    Missing,          // 文件不存在（首次运行），调用方可回退到默认配置
    Corrupt(String),  // 文件存在但无法解析，携带错误描述
}

/// 读取并解析 JSON 文件为泛型类型 T。
/// 【泛型】<T: DeserializeOwned> 表示「任何可反序列化的类型」，因此这个函数能复用于各种配置结构体。
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<T, LoadError> {
    // 先读成字符串。match 对读取结果分三种情况处理：
    let text = match fs::read_to_string(path) {
        Ok(t) => t,                                              // 读取成功，拿到内容
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing), // 文件不存在
        Err(e) => return Err(LoadError::Corrupt(e.to_string())), // 其他 IO 错误（如权限），当作损坏
    };
    // 再把字符串解析为 T：
    match serde_json::from_str(&text) {
        Ok(v) => Ok(v), // 解析成功
        Err(_) => {
            // 与 Python 版一致：损坏 → 备份 .corrupt → 调用方回退默认。
            // let _ = ... 表示「忽略这个操作的返回值」（备份失败也不影响主流程）。
            let _ = fs::copy(path, format!("{}.corrupt", path.display()));
            Err(LoadError::Corrupt(format!(
                "配置文件损坏，已备份为 {}.corrupt", path.display()
            )))
        }
    }
}

/// 把数据序列化为 JSON 并原子写入目标文件。
/// 【泛型】<T: Serialize> 表示「任何可序列化的类型」。
pub fn save<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    // with_extension("json.tmp") 把目标路径的扩展名换成 .tmp，得到临时文件路径（如 cfg.json.tmp）。
    let tmp = path.with_extension("json.tmp");
    // to_string_pretty 生成带缩进、人类可读的 JSON（比紧凑格式方便用户查看/手改）。
    let text = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    // 先把内容写到临时文件（此时目标文件仍是旧的完整内容）。
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    // Windows 上 std::fs::rename 会覆盖已存在的目标文件（等价 os.replace），一步完成原子替换。
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}
