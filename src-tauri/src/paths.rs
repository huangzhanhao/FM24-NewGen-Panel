//! 路径规范化工具（对等旧版 ProfileManager 的 norm_dir/sanitize/group_xml_path）。
//!
//! 【为什么需要归一化】同一个头像包目录可能写成 `C:/a/b`、`C:\a\b`、`C:\a\b\` 多种形式，
//! 必须先归一化才能当作「同一个组」的唯一键（避免重复建组）；快照文件名则要把路径安全地
//! 压缩成合法文件名。
use std::path::{Path, PathBuf};

/// 统一分隔符并保证尾部斜杠：同一头像包目录只对应一个组 key。
/// 例：`C:/a/b` → `C:\a\b\`（Windows 上）。
pub fn norm_dir(img_dir: &str) -> String {
    if img_dir.is_empty() {
        return String::new();
    }
    let sep = std::path::MAIN_SEPARATOR;
    let p = img_dir.replace('/', &sep.to_string());
    let trimmed = p.trim_end_matches(sep);
    if trimmed.is_empty() {
        // 输入只有分隔符（`/` 或 `\`）：去尾后为空，回退为单个根分隔符。
        return sep.to_string();
    }
    // 去掉尾斜杠后统一补一个，保证不同写法归一成同一格式。
    format!("{trimmed}{sep}")
}

/// 路径非法字符 → 下划线（用于把路径拼进文件名），空串回退 "default"。
/// Windows 文件名不允许 \ / : * ? " < > |，而头像包路径要当作快照文件名的一部分，
/// 所以先替换非法字符；结果全空白时用 "default" 兜底，避免生成空文件名。
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

/// 快照文件词干：`graphics_<graphics 目录后的路径段>_snapshot`。
/// 取头像包路径中 "graphics" 段之后的所有路径段（分隔符转 _，段匹配不区分
/// 大小写），加 `_snapshot` 后缀——如 `...\FM2024\graphics\NG_Regens`
/// → `graphics_NG_Regens_snapshot`。
/// 前提：头像包必然位于 graphics 目录下（FM 规范，否则头像包本身不生效），
/// 因此不设目录末段回退；graphics 段缺失仅作防御——用整路径转义兜底（正常不触发）。
fn snapshot_stem(img_dir: &str) -> String {
    let sep = std::path::MAIN_SEPARATOR;
    let norm = img_dir.replace('/', &sep.to_string());
    let segs: Vec<&str> = norm.split(sep).filter(|s| !s.is_empty()).collect();
    if let Some(pos) = segs.iter().rposition(|s| s.eq_ignore_ascii_case("graphics")) {
        let tail = &segs[pos + 1..];
        if !tail.is_empty() {
            return format!("graphics_{}_snapshot", sanitize(&tail.join("_")));
        }
    }
    format!("{}_snapshot", sanitize(&norm))
}

/// 组映射快照路径（短命名）：<user_dir>/<profile>/<快照词干>.xml
/// profile 名作子目录 + 「graphics 后路径段_snapshot」作文件名——
/// 替代旧版「整个路径转义」的超长命名，两端都保持可辨识。
/// 边界：同一存档下若两个组的快照词干同名（不同盘的 graphics\NG_Regens），将共用同一快照文件。
pub fn group_xml_path(user_dir: &Path, profile: &str, img_dir: &str) -> PathBuf {
    user_dir
        .join(sanitize(profile))                     // 存档名子目录
        .join(format!("{}.xml", snapshot_stem(img_dir)))
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
    fn group_xml_path_short_and_identifiable() {
        // 主用例：头像包在 graphics 目录下 → 快照名 = graphics_<后段>_snapshot。
        let dir = r"C:\Users\57552\Documents\Sports Interactive\Football Manager 2024\graphics\NG_Regens";
        let p = group_xml_path(Path::new("d"), "档1", dir);
        assert_eq!(p, Path::new("d").join("档1").join("graphics_NG_Regens_snapshot.xml"));
        // 多级后段：分隔符转 _；大小写不敏感。
        let p2 = group_xml_path(Path::new("d"), "档1", r"C:\fm\Graphics\faces\NG_Regens");
        assert_eq!(
            p2.file_name().unwrap().to_string_lossy(),
            "graphics_faces_NG_Regens_snapshot.xml"
        );
        // 防御分支（正常不触发）：无 graphics 段 → 整路径转义兜底。
        let p3 = group_xml_path(Path::new("d"), "档1", r"C:\pack\");
        assert_eq!(p3.file_name().unwrap().to_string_lossy(), "C__pack__snapshot.xml");
    }
}
