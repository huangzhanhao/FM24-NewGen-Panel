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
    // 【为什么连点号也要管】本函数的产物会被当作**路径段**使用（存档子目录名、快照文件名）。
    // `.` 与 `..` 是 Windows/POSIX 都认的当前/上级目录段，放行后「删存档」会拿着
    // `user_dir.join("..")` 去 remove_dir_all 整个数据目录；尾部的点号/空格则会被 Windows 吃掉，
    // 让「档A.」与「档A」归一成同一个文件、互相覆盖档景。两种都收敛掉。
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']);
    if trimmed.is_empty() { "default".into() } else { trimmed.to_string() }
}

/// 存档名能否直接当目录名用——比 sanitize 更严，只做判定不改写。
/// 新建/删除存档这两个入口用它**拒绝**畸形名，而不是靠 sanitize 静默收敛：
/// `..` 被收敛成 "default" 后，删这个档就会去删「default」那个档的目录，误伤无辜存档。
/// 注：名里的路径分隔符已由 sanitize 转成 `_`，不会形成穿越，故此处不必重复拦截。
pub fn is_safe_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().any(|c| c != '.') // 排除 . / .. 这类全点号名
        && !name.ends_with('.')            // 尾点会被 Windows 丢弃 → 与其他名撞车
        && !name.ends_with(' ')
}

/// FNV-1a 64 位：快照词干里那段短哈希用它。
/// 【为什么不用 std 的 DefaultHasher】它明确不保证跨版本稳定——换一次工具链就可能让全部
/// 档景的文件名变样，等于把「本档本组的现有映射」整体作废（切档 restore 静默跳过 → 串档）。
/// FNV-1a 是写死的算式，永远给同一个输入同一个值。
fn fnv1a64(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 快照命名的键：`norm_dir` 归一 + ASCII 小写。
/// Windows 路径不区分大小写，`graphics\NG` 与 `graphics\ng` 是同一个头像包，必须落到同一个
/// 快照文件；小写化同时喂给可读段与哈希，名字才完全由「哪个目录」决定。
fn path_key(img_dir: &str) -> String {
    norm_dir(img_dir).to_ascii_lowercase()
}

/// 可读段：取头像包路径中 "graphics" 段之后的所有路径段（分隔符转 _）。
/// 前提：头像包必然位于 graphics 目录下（FM 规范，否则头像包本身不生效），因此不设目录末段
/// 回退；graphics 段缺失仅作防御——用整路径转义兜底（正常不触发）。入参须是 `path_key` 的输出。
fn readable_tail(key: &str) -> String {
    let sep = std::path::MAIN_SEPARATOR;
    let segs: Vec<&str> = key.split(sep).filter(|s| !s.is_empty()).collect();
    if let Some(pos) = segs.iter().rposition(|s| *s == "graphics") {
        let tail = &segs[pos + 1..];
        if !tail.is_empty() {
            return format!("graphics_{}", sanitize(&tail.join("_")));
        }
    }
    sanitize(key)
}

/// 两个组键是否指同一个目录（Windows 语义：分隔符写法、大小写、尾斜杠都不算两个目录）。
/// 【为什么需要它】组配置以 `norm_dir` 的结果当键，而它只归一分隔符——`C:\FM\NG` 与
/// `C:\fm\ng` 会登记成两个组，可它们的新快照名是同一个（`path_key` 已折大小写），
/// 于是「两个组共用一份档景」从组键这条路又溜回来了。
pub fn same_dir(a: &str, b: &str) -> bool {
    path_key(a) == path_key(b)
}

/// 快照文件词干：`graphics_<尾部>_snapshot-<8 位十六进制>`。
/// 【为什么要挂哈希】只靠可读段（分隔符与非法字符都转成 `_`）会让 `graphics\NG\Regens` 与
/// `graphics\NG_Regens` 得到同一个词干——同一存档下两个不同的组共用一份档景，正是
/// `source_xml` 设计要消灭的串档模式，而首次成功替换就把错状态持久化。哈希按完整路径键算，
/// 把这种「必然撞」降到 1e-5 量级（32 bit、同一存档上千个组）；可读段照旧留着，
/// 用户在文件管理器里仍能认出它是哪个包。
fn snapshot_stem(img_dir: &str) -> String {
    let key = path_key(img_dir);
    let hash = &format!("{:016x}", fnv1a64(&key))[..8];
    format!("{}_snapshot-{hash}", readable_tail(&key))
}

/// 组映射快照路径：<user_dir>/<profile>/<快照词干>.xml
/// profile 名作子目录 + 「graphics 后路径段_snapshot-哈希」作文件名——
/// 两端都保持可辨识，且文件名对「哪个头像包目录」是单射的（见 `snapshot_stem`）。
pub fn group_xml_path(user_dir: &Path, profile: &str, img_dir: &str) -> PathBuf {
    snap_path(user_dir, profile, &snapshot_stem(img_dir))
}

/// 组快照路径：`<user_dir>/<sanitize(profile)>/<stem>.xml`。
fn snap_path(user_dir: &Path, profile: &str, stem: &str) -> PathBuf {
    user_dir
        .join(sanitize(profile))
        .join(format!("{stem}.xml"))
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
    fn sanitize_never_yields_a_traversal_segment() {
        // sanitize 的结果要能安全当路径段：这些输入曾让「删存档」指向 user_dir 的父目录。
        assert_eq!(sanitize(".."), "default");
        assert_eq!(sanitize("."), "default");
        assert_eq!(sanitize("...."), "default");
        // 尾点被 Windows 丢弃，会让两个名字撞同一个文件 → 一并裁掉；名中间的点号保留。
        assert_eq!(sanitize("档A."), "档A");
        assert_eq!(sanitize("FM24.0"), "FM24.0");
    }

    #[test]
    fn profile_name_gate_accepts_normal_and_rejects_dots() {
        assert!(is_safe_profile_name("FM24.0"));
        assert!(is_safe_profile_name("No Profile"));
        assert!(!is_safe_profile_name(""));
        assert!(!is_safe_profile_name("."));
        assert!(!is_safe_profile_name(".."));
        assert!(!is_safe_profile_name("档A.")); // 与「档A」同目录，必须拒之门外而非静默改写
    }

    #[test]
    fn group_xml_path_short_and_identifiable() {
        // 主用例：头像包在 graphics 目录下 → graphics_<后段，小写>_snapshot-<8 位哈希>。
        let dir = r"C:\Users\57552\Documents\Sports Interactive\Football Manager 2024\graphics\NG_Regens";
        let p = group_xml_path(Path::new("d"), "档1", dir);
        assert_eq!(p.parent().unwrap(), Path::new("d").join("档1"));
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("graphics_ng_regens_snapshot-"), "{name}");
        assert!(name.ends_with(".xml"), "{name}");
        assert_eq!(name.len(), "graphics_ng_regens_snapshot-".len() + 8 + ".xml".len(), "{name}");
        // 多级后段：分隔符转 _；"graphics" 段匹配不区分大小写。
        assert!(group_xml_path(Path::new("d"), "档1", r"C:\fm\Graphics\faces\NG_Regens")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("graphics_faces_ng_regens_snapshot-"));
        // 防御分支（正常不触发）：无 graphics 段 → 整路径转义兜底。
        assert!(group_xml_path(Path::new("d"), "档1", r"C:\pack\")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("c__pack__snapshot-"));
    }

    #[test]
    fn different_dirs_never_share_a_snapshot_file() {
        // 旧命名把路径分隔符与非法字符一起转成 `_`，下面两条得到同一个词干 → 同一存档下两个组
        // 共用一份档景（一份 live config.xml 对两份档景），首次成功替换就把错状态持久化。
        let a = group_xml_path(Path::new("d"), "档1", r"C:\fm\graphics\NG\Regens");
        let b = group_xml_path(Path::new("d"), "档1", r"C:\fm\graphics\NG_Regens");
        assert_ne!(a, b, "反斜杠层级与下划线必须可区分");
        assert_ne!(
            group_xml_path(Path::new("d"), "档1", r"D:\fm\graphics\NG\Regens"),
            group_xml_path(Path::new("d"), "档1", r"E:\fm\graphics\NG\Regens"),
            "不同盘符的同名层级必须可区分"
        );
        // 反过来：同一目录的不同写法（大小写、正/反斜杠、尾斜杠）必须落在同一个文件，
        // 否则 Windows 上同一个包又会摊出两份档景。
        assert_eq!(a, group_xml_path(Path::new("d"), "档1", r"C:\FM\Graphics\NG\Regens\"));
        assert_eq!(a, group_xml_path(Path::new("d"), "档1", "C:/fm/graphics/ng/regens"));
    }
}
