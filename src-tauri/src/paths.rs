//! 【新手导读】路径规范化工具（对等旧版 ProfileManager 的 norm_dir/_sanitize/group_xml_path）。
//! 为什么需要它：同一个头像包目录可能写成 `C:/a/b`、`C:\a\b`、`C:\a\b\` 多种形式，
//! 必须先归一化才能当作「同一个组」的唯一键（避免重复建组）。同时要能把路径安全地拼进文件名。
use std::path::{Path, PathBuf}; // Path=借用的路径（只读）；PathBuf=拥有所有权的路径（可修改）

/// 统一分隔符并保证尾部斜杠：同一头像包目录只对应一个组 key。
/// 例：`C:/a/b` → `C:\a\b\`（Windows 上）。pub 表示对外公开，其他模块可调用。
pub fn norm_dir(img_dir: &str) -> String {
    if img_dir.is_empty() {
        return String::new(); // 空路径直接返回空串，不做处理
    }
    // MAIN_SEPARATOR 是当前系统的目录分隔符（Windows 为 `\`，Linux/macOS 为 `/`）。
    let sep = std::path::MAIN_SEPARATOR;
    // 先把所有正斜杠 `/` 统一替换为系统分隔符（to_string() 是因为 replace 需要 &str 而 sep 是 char）。
    let p = img_dir.replace('/', &sep.to_string());
    // 去掉尾部所有分隔符，避免 `C:\a\b\\\` 这种多余的尾斜杠。
    let trimmed = p.trim_end_matches(sep);
    if trimmed.is_empty() {
        // 例如输入只有 `/` 或 `\`，去尾后为空，回退为单个根分隔符。
        return sep.to_string();
    }
    // 正常情况下：去掉尾斜杠后再统一补上一个尾斜杠，保证格式一致。
    format!("{trimmed}{sep}")
}

/// 路径非法字符 → 下划线（用于把路径拼进文件名），空串回退 "default"。
/// 【为什么】Windows 文件名不允许 \ / : * ? " < > | 这些字符；而我们需要把
/// 「头像包路径」当作快照文件名的一部分，所以先把非法字符换成下划线。
pub fn sanitize(text: &str) -> String {
    // chars() 逐字符遍历，map 把非法字符换成 '_'，其余保留，最后 collect 拼回 String。
    let cleaned: String = text
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let trimmed = cleaned.trim().to_string(); // 去掉首尾空白
    // 若结果为空（比如输入全是空白），用 "default" 兑底，避免生成空文件名。
    if trimmed.is_empty() { "default".into() } else { trimmed }
}

/// 组映射快照路径（短命名）：<user_dir>/<profile>/<目录末段>.xml
/// 【命名】profile 名作子目录 + 头像包目录末段作文件名——替代旧版「整个路径转义」
/// 的超长命名，两端都保持可辨识。边界：同一存档下若两个组的目录末段同名
/// （如 C:\a\GUNZO 与 D:\b\GUNZO），将共用同一快照文件。
pub fn group_xml_path(user_dir: &Path, profile: &str, img_dir: &str) -> PathBuf {
    let tail = Path::new(img_dir)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    user_dir
        .join(sanitize(profile))                        // 存档名子目录
        .join(format!("{}.xml", sanitize(&tail)))       // 目录末段作文件名
}

// 【测试模块】#[cfg(test)] 表示这些代码只在 `cargo test` 时才编译，不会进入正式程序。
// mod tests 里写单元测试，验证上面的函数行为是否符合预期。
#[cfg(test)]
mod tests {
    use super::*; // 引入父模块（即本文件）的所有内容，以便测试里直接用 norm_dir 等函数

    #[test]
    fn norm_dir_trims_and_appends_sep() {
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(norm_dir("C:/a/b/"), format!("C:{sep}a{sep}b{sep}"));
        assert_eq!(norm_dir(""), "");
    }

    #[test]
    fn sanitize_replaces_illegal_chars() {
        assert_eq!(sanitize(r"C:\x:y"), "C__x_y");
        assert_eq!(sanitize("  "), "default");
    }

    #[test]
    fn group_xml_path_short_and_identifiable() {
        // 短命名：profile 子目录 + 目录末段.xml（替代旧版「整路径转义」的超长命名）。
        let long_dir = r"C:\Users\57552\Documents\Sports Interactive\Football Manager 2024\GUNZO 风格头像包";
        let p = group_xml_path(Path::new("d"), "档1", long_dir);
        assert_eq!(p, Path::new("d").join("档1").join("GUNZO 风格头像包.xml"));
    }
}
