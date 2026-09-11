//! 【新手导读】config.xml 读写：模板 + 行级正则替换（自旧版 core/XmlParser.py 移植）。
//! config.xml 是 FM 认的「头像映射表」，每行形如：
//!   <record from="种族目录/图片名" to="graphics/pictures/person/<UID>/portrait"/>
//! 意思是「把某张图片用于某个 UID 的球员头像」。本模块负责读、写、单行替换、备份这些配置。
use crate::assets::CONFIG_TEMPLATE;        // 复用 assets 里的 config.xml 模板
use crate::rtf_scanner::NEWGAN_UID_PREFIX;  // 复用 r- 前缀常量
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

/// 一条映射：某 UID 的球员 → 某种族目录下的某张图片。同样 camelCase 序列化给前端。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MappingEntry {
    pub uid: String,       // 球员 UID（可能带 r- 前缀）
    pub ethnicity: String, // 种族/国籍目录名（如 SpanMed）
    pub image: String,     // 图片名（不含扩展名）
}

// 从 config.xml 行中提取 from/to 两个值的正则。
// Python 版用 lookbehind（后顾断言）；Rust regex 不支持，改为整行捕获组（匹配语义等价）。
// 捕获组 1 = from 的内容（种族/图片）；捕获组 2 = UID（(?:r-)? 表示 r- 前缀可有可无）。
static RECORD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"from="([^"]+)" to="graphics/pictures/person/((?:r-)?\d{4,})/portrait""#).unwrap()
});

const BACKUP_KEEP: usize = 10; // 最多保留的备份份数

/// config.xml 的 from 值：平铺（ethnicity 空）只写图名，否则 "种族/图名"。
pub fn record_from(ethnicity: &str, image: &str) -> String {
    if ethnicity.is_empty() { image.to_string() } else { format!("{ethnicity}/{image}") }
}

/// 解析 config.xml → {uid: entry}（用字典按 UID 快速查找）。
pub fn parse_xml(path: &Path) -> Result<HashMap<String, MappingEntry>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读取 config.xml 失败: {e}"))?;
    let mut out = HashMap::new();
    // captures_iter 遍历所有匹配项；cap[1]/cap[2] 分别是第 1/2 个捕获组。
    for cap in RECORD_RE.captures_iter(&text) {
        // from 的内容为相对路径："SpanMed/img1"（标准二）、"img1"（标准一平铺）、
        // "子目录/孙目录/图名"（Unknown 嵌套）。最后一段是图名，其余整体为相对目录。
        let parts: Vec<&str> = cap[1].split('/').collect();
        let (eth, img) = if parts.len() >= 2 {
            (parts[..parts.len() - 1].join("/"), parts[parts.len() - 1])
        } else {
            (String::new(), parts[0])
        };
        out.insert(
            cap[2].to_string(), // 以 UID 为键
            MappingEntry {
                uid: cap[2].to_string(),
                ethnicity: eth,
                image: img.to_string(),
            },
        );
    }
    Ok(out)
}

/// 全量覆写 config.xml（对等旧版 write_xml：模板 [players] 占位替换）。
/// 【何时用】批量替换时，把全部映射一次性重新生成 config.xml。
pub fn write_xml(data: &[MappingEntry], img_dir: &Path, save_backup: bool) -> Result<(), String> {
    let config_path = img_dir.join("config.xml");
    // 需要备份且已有旧文件时，先滚动备份。
    if save_backup && config_path.is_file() {
        backup_rotation(&config_path, img_dir)?;
    }
    // 把每条映射拼成一行 <record .../>。record_from 对非空 ethnicity 输出与旧版逐字节一致。
    let lines: Vec<String> = data
        .iter()
        .map(|m| {
            format!(
                r#"<record from="{}" to="graphics/pictures/person/{}/portrait"/>"#,
                record_from(&m.ethnicity, &m.image), m.uid
            )
        })
        .collect();
    // 与 v1 输出逐字节一致：行间 16 空格缩进。把模板里的 [players] 占位符换成所有行。
    let xml = CONFIG_TEMPLATE.replace("[players]", &lines.join("\n                "));
    fs::write(&config_path, xml).map_err(|e| format!("写入 config.xml 失败: {e}"))?;
    Ok(())
}

/// 只替换匹配 UID 的那一行（对等旧版 single_replacement_in_xml）。返回是否命中。
/// 【何时用】「单人微调」功能：不改其他行，只把某个 UID 换成新图片。
pub fn single_replacement(
    uid: &str,
    pack: &str,     // 新图片所在种族目录
    image: &str,    // 新图片名
    img_dir: &Path,
    save_backup: bool,
) -> Result<bool, String> {
    let config_path = img_dir.join("config.xml");
    if !config_path.is_file() {
        return Ok(false); // 没有 config.xml，无从替换，返回未命中
    }
    if save_backup {
        backup_rotation(&config_path, img_dir)?;
    }
    let text = fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
    // target 是要定位的片段（根据 UID 拼出）；new_line 是替换后的完整行（record_from 兼容平铺 pack=""）。
    let target = format!(r#"to="graphics/pictures/person/{uid}/portrait""#);
    let new_line = format!(
        r#"<record from="{}" to="graphics/pictures/person/{uid}/portrait"/>"#,
        record_from(pack, image),
    );
    let mut replaced = false;
    let mut out: Vec<String> = Vec::new();
    // 逐行扫：命中目标且尚未替换过，就换成新行（只换第一条）；其余行原样保留。
    for line in text.lines() {
        if !replaced && line.contains(&target) {
            out.push(new_line.clone());
            replaced = true;
        } else {
            out.push(line.to_string());
        }
    }
    fs::write(&config_path, out.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(replaced)
}

/// 备份为 config备份_YYYYMMDD-HHMMSS.xml，仅保留最近 10 份。
fn backup_rotation(config_path: &Path, img_dir: &Path) -> Result<(), String> {
    // 用当前时间戳生成唯一备份名，先拷贝一份。
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup = img_dir.join(format!("config备份_{ts}.xml"));
    fs::copy(config_path, &backup).map_err(|e| e.to_string())?;
    // 列出目录下所有备份文件（filter_map(|e| e.ok()) 跳过读取出错的项）。
    let mut backups: Vec<_> = fs::read_dir(img_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("config备份_") && n.ends_with(".xml")
        })
        .map(|e| e.path())
        .collect();
    backups.sort(); // 文件名含时间戳，字典序 = 时间序
    // 超过 10 份就删除最旧的（切片 [..len-10] 是前面的旧备份）。
    if backups.len() > BACKUP_KEEP {
        for old in &backups[..backups.len() - BACKUP_KEEP] {
            let _ = fs::remove_file(old);
        }
    }
    Ok(())
}

/// 从用户选中的图片绝对路径反推 (相对目录路径, 图名)。平铺包（图片在 img_dir 根）→ 目录为空串；
/// 子/孙目录图片 → 相对 img_dir 的目录路径，多级用 "/" 分隔（如 "MENA/sub"）。
/// 词干用 file_stem，与 facepack_scanner::scan_pack 的 root_images 同规则（"a.1.png" → "a.1"）。
pub fn parse_image_path_in_pack(img_dir: &str, image_path: &str) -> Option<(String, String)> {
    let p = Path::new(image_path);
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg") { return None; }
    let image = p.file_stem()?.to_string_lossy().to_string();
    if image.starts_with('.') { return None; }   // 拒收 ._x.png 等隐藏名，与池侧口径一致
    let parent = p.parent()?;
    let root = Path::new(img_dir);
    // 优先 canonicalize（Windows 大小写/分隔符不敏感的权威比较），取父目录相对包根的路径。
    let rel_dir = if let (Ok(p_abs), Ok(r_abs)) = (parent.canonicalize(), root.canonicalize()) {
        match p_abs.strip_prefix(&r_abs) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            // 不同盘等无法取相对路径：退化为最后一级目录名（与旧版行为一致）。
            Err(_) => match parent.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => return None,
            },
        }
    } else if parent == root {
        String::new()
    } else {
        match parent.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => return None, // 裸相对文件名（无父目录）→ 不合法
        }
    };
    Some((rel_dir, image))
}

/// 探测实际图片文件（.png/.jpg/.jpeg，对等旧版 PlayerService.resolve_image_file）。
/// 【为何】config.xml 里只存图片名不存扩展名，需要按常见扩展名依次试找出真实文件。
/// ethnicity 为映射里的相对目录路径（"" = 包根，多级 "a/b" = 子/孙目录）。
pub fn resolve_image_file(img_dir: &Path, ethnicity: &str, image: &str) -> Option<String> {
    let mut base = img_dir.to_path_buf();
    if !ethnicity.is_empty() {
        for seg in ethnicity.split('/') { // 相对路径逐级拼接（"/" 统一分隔符）
            base = base.join(seg);
        }
    }
    for ext in ["png", "jpg", "jpeg"] {
        let p = base.join(format!("{image}.{ext}"));
        if p.is_file() {
            return Some(p.to_string_lossy().to_string()); // to_string_lossy 把路径安全转为 String
        }
    }
    None // 三种扩展名都没找到
}

/// 给 UID 补随机人前缀的辅助（查询缓存时兼容两种形态）。
/// 【为何】同一球员 UID 在缓存里可能带 r- 也可能不带，返回两种形态方便都去查一下。
pub fn with_newgan_prefix_variants(uid: &str) -> [String; 2] {
    let prefixed = format!("{NEWGAN_UID_PREFIX}{uid}");
    [uid.to_string(), prefixed]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facepack() -> std::path::PathBuf {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("pack");
        std::fs::create_dir_all(&p).unwrap();
        // keep dir alive: 借 path 但由调用方保证在同函数内使用
        p
    }

    #[test]
    fn write_then_parse_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        let data = vec![
            MappingEntry { uid: "r-2000472008".into(), ethnicity: "SpanMed".into(), image: "img1".into() },
            MappingEntry { uid: "2001234568".into(), ethnicity: "African".into(), image: "img2".into() },
        ];
        write_xml(&data, &pack, false).unwrap();
        let parsed = parse_xml(&pack.join("config.xml")).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["r-2000472008"].image, "img1");
        // 产物含 FM 需要的头尾结构
        let text = std::fs::read_to_string(pack.join("config.xml")).unwrap();
        assert!(text.contains(r#"<list id="maps">"#));
        assert!(text.contains(r#"from="SpanMed/img1""#));
        let _ = facepack(); // 占位调用避免未使用告警
    }

    #[test]
    fn single_replacement_hits_only_target_line() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        write_xml(
            &[
                MappingEntry { uid: "r-1001".into(), ethnicity: "Asian".into(), image: "a1".into() },
                MappingEntry { uid: "r-1002".into(), ethnicity: "Asian".into(), image: "a2".into() },
            ],
            &pack, false,
        ).unwrap();
        assert!(single_replacement("r-1001", "MENA", "m9", &pack, false).unwrap());
        let parsed = parse_xml(&pack.join("config.xml")).unwrap();
        assert_eq!(parsed["r-1001"].ethnicity, "MENA");
        assert_eq!(parsed["r-1001"].image, "m9");
        assert_eq!(parsed["r-1002"].image, "a2"); // 未受影响
        assert!(!single_replacement("r-9999", "MENA", "m9", &pack, false).unwrap());
    }

    #[test]
    fn backup_rotation_keeps_latest_ten() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        write_xml(&[MappingEntry { uid: "r-1".into(), ethnicity: "Asian".into(), image: "x".into() }], &pack, false).unwrap();
        // 手动造 12 份历史备份（时间戳递增）
        for i in 0..12 {
            let name = format!("config备份_20260101-0000{:02}.xml", i);
            std::fs::copy(pack.join("config.xml"), pack.join(&name)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100)); // 秒级时间戳递增
        }
        write_xml(&[MappingEntry { uid: "r-1".into(), ethnicity: "Asian".into(), image: "x".into() }], &pack, true).unwrap();
        let count = std::fs::read_dir(&pack).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("config备份_"))
            .count();
        assert_eq!(count, 10);
    }

    #[test]
    fn record_from_flat_and_nested() {
        assert_eq!(record_from("", "img1"), "img1");
        assert_eq!(record_from("MENA", "m9"), "MENA/m9");
    }

    #[test]
    fn flat_roundtrip_and_single_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        // 平铺条目：write 出 from="img1"（无目录前缀），parse 回 ("", "img1")
        write_xml(
            &[MappingEntry { uid: "r-1001".into(), ethnicity: "".into(), image: "img1".into() }],
            &pack, false,
        ).unwrap();
        let text = std::fs::read_to_string(pack.join("config.xml")).unwrap();
        assert!(text.contains(r#"from="img1""#));
        let parsed = parse_xml(&pack.join("config.xml")).unwrap();
        assert_eq!(parsed["r-1001"].ethnicity, "");
        assert_eq!(parsed["r-1001"].image, "img1");
        // 单人替换 pack="" → from="image"（平铺形态）
        assert!(single_replacement("r-1001", "", "img2", &pack, false).unwrap());
        let parsed2 = parse_xml(&pack.join("config.xml")).unwrap();
        assert_eq!((parsed2["r-1001"].ethnicity.as_str(), parsed2["r-1001"].image.as_str()), ("", "img2"));
    }

    #[test]
    fn parse_image_path_in_pack_and_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let root_s = dir.path().to_string_lossy().to_string();
        let eth = dir.path().join("MENA");
        std::fs::create_dir_all(&eth).unwrap();
        std::fs::write(eth.join("m9.png"), b"png").unwrap();
        // 嵌套：取父目录名为种族（对等旧 parse_image_path 行为）
        assert_eq!(
            parse_image_path_in_pack(&root_s, &eth.join("m9.png").to_string_lossy()),
            Some(("MENA".into(), "m9".into()))
        );
        // 孙目录：相对路径多级（"/" 分隔），resolve 同口径命中
        let deep = dir.path().join("MENA").join("sub");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("d2.png"), b"png").unwrap();
        assert_eq!(
            parse_image_path_in_pack(&root_s, &deep.join("d2.png").to_string_lossy()),
            Some(("MENA/sub".into(), "d2".into()))
        );
        assert_eq!(
            resolve_image_file(dir.path(), "MENA/sub", "d2"),
            Some(deep.join("d2.png").to_string_lossy().to_string())
        );
        // 平铺：父 == 根 → 种族为空串
        std::fs::write(dir.path().join("r1.png"), b"png").unwrap();
        assert_eq!(
            parse_image_path_in_pack(&root_s, &dir.path().join("r1.png").to_string_lossy()),
            Some(("".into(), "r1".into()))
        );
        // 点号文件名：file_stem 规则（与 scan_pack 的 root_images 一致）
        std::fs::write(dir.path().join("a.1.png"), b"png").unwrap();
        assert_eq!(
            parse_image_path_in_pack(&root_s, &dir.path().join("a.1.png").to_string_lossy()),
            Some(("".into(), "a.1".into()))
        );
        // 非图片扩展名 → None；隐藏名 → None；裸相对文件名（无父目录）→ None
        assert_eq!(parse_image_path_in_pack(&root_s, &dir.path().join("x.txt").to_string_lossy()), None);
        assert_eq!(parse_image_path_in_pack(&root_s, &dir.path().join("._h.png").to_string_lossy()), None);
        assert_eq!(parse_image_path_in_pack(&root_s, "r9.png"), None);
        // resolve：嵌套命中种族目录；空 ethnicity 命中包根；找不到 → None
        assert_eq!(
            resolve_image_file(dir.path(), "MENA", "m9"),
            Some(eth.join("m9.png").to_string_lossy().to_string())
        );
        assert_eq!(
            resolve_image_file(dir.path(), "", "r1"),
            Some(dir.path().join("r1.png").to_string_lossy().to_string())
        );
        assert_eq!(resolve_image_file(dir.path(), "MENA", "nope"), None);
    }
}
