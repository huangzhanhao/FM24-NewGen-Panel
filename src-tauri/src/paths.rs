//! 路径规范化工具（对等 ProfileManager 的 norm_dir/_sanitize/group_xml_path）
use std::path::{Path, PathBuf};

/// 统一分隔符并保证尾部斜杠：同一头像包目录只对应一个组 key
pub fn norm_dir(img_dir: &str) -> String {
    if img_dir.is_empty() {
        return String::new();
    }
    let sep = std::path::MAIN_SEPARATOR;
    let p = img_dir.replace('/', &sep.to_string());
    let trimmed = p.trim_end_matches(sep);
    if trimmed.is_empty() {
        return sep.to_string();
    }
    format!("{trimmed}{sep}")
}

/// 路径非法字符 → 下划线（用于把路径拼进文件名），空串回退 "default"
pub fn sanitize(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let trimmed = cleaned.trim().to_string();
    if trimmed.is_empty() { "default".into() } else { trimmed }
}

/// 组映射快照路径：<user_dir>/<profile>/<sanitized_img_dir>.xml
pub fn group_xml_path(user_dir: &Path, profile: &str, img_dir: &str) -> PathBuf {
    user_dir
        .join(sanitize(profile))
        .join(format!("{}.xml", sanitize(img_dir)))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn group_xml_path_layout() {
        let p = group_xml_path(Path::new("d"), "档1", r"C:\pack\");
        assert_eq!(p, Path::new("d").join("档1").join("C__pack_.xml"));
    }
}
