//! config.xml 读写：模板 + 行级正则替换（自 core/XmlParser.py 移植）
use crate::assets::CONFIG_TEMPLATE;
use crate::rtf_parser::NEWGAN_UID_PREFIX;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MappingEntry {
    pub uid: String,
    pub ethnicity: String,
    pub image: String,
}

// Python 版用 lookbehind；Rust regex 不支持，改为整行捕获组（匹配语义等价）
static RECORD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"from="([^"]+)" to="graphics/pictures/person/((?:r-)?\d{4,})/portrait""#).unwrap()
});

const BACKUP_KEEP: usize = 10;

/// 解析 config.xml → {uid: entry}
pub fn parse_xml(path: &Path) -> Result<HashMap<String, MappingEntry>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读取 config.xml 失败: {e}"))?;
    let mut out = HashMap::new();
    for cap in RECORD_RE.captures_iter(&text) {
        let eth_img: Vec<&str> = cap[1].split('/').collect();
        if eth_img.len() >= 2 {
            out.insert(
                cap[2].to_string(),
                MappingEntry {
                    uid: cap[2].to_string(),
                    ethnicity: eth_img[0].to_string(),
                    image: eth_img[1].to_string(),
                },
            );
        }
    }
    Ok(out)
}

/// 按 UID 取 "种族/图片名" 路径（对等 get_imgpath_from_uid）
pub fn get_imgpath_from_uid(path: &Path, uid: &str) -> Option<String> {
    parse_xml(path).ok()?.get(uid).map(|e| format!("{}/{}", e.ethnicity, e.image))
}

/// 全量覆写 config.xml（对等 write_xml：模板 [players] 占位替换）
pub fn write_xml(data: &[MappingEntry], img_dir: &Path, save_backup: bool) -> Result<(), String> {
    let config_path = img_dir.join("config.xml");
    if save_backup && config_path.is_file() {
        backup_rotation(&config_path, img_dir)?;
    }
    let lines: Vec<String> = data
        .iter()
        .map(|m| {
            format!(
                r#"<record from="{}/{}" to="graphics/pictures/person/{}/portrait"/>"#,
                m.ethnicity, m.image, m.uid
            )
        })
        .collect();
    // 与 v1 输出逐字节一致：行间 16 空格缩进
    let xml = CONFIG_TEMPLATE.replace("[players]", &lines.join("\n                "));
    fs::write(&config_path, xml).map_err(|e| format!("写入 config.xml 失败: {e}"))?;
    Ok(())
}

/// 只替换匹配 UID 的那一行（对等 single_replacement_in_xml）。返回是否命中。
pub fn single_replacement(
    uid: &str,
    pack: &str,
    image: &str,
    img_dir: &Path,
    save_backup: bool,
) -> Result<bool, String> {
    let config_path = img_dir.join("config.xml");
    if !config_path.is_file() {
        return Ok(false);
    }
    if save_backup {
        backup_rotation(&config_path, img_dir)?;
    }
    let text = fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
    let target = format!(r#"to="graphics/pictures/person/{uid}/portrait""#);
    let new_line = format!(
        r#"<record from="{pack}/{image}" to="graphics/pictures/person/{uid}/portrait"/>"#,
    );
    let mut replaced = false;
    let mut out: Vec<String> = Vec::new();
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

/// 备份为 config备份_YYYYMMDD-HHMMSS.xml，仅保留最近 10 份
fn backup_rotation(config_path: &Path, img_dir: &Path) -> Result<(), String> {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup = img_dir.join(format!("config备份_{ts}.xml"));
    fs::copy(config_path, &backup).map_err(|e| e.to_string())?;
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
    if backups.len() > BACKUP_KEEP {
        for old in &backups[..backups.len() - BACKUP_KEEP] {
            let _ = fs::remove_file(old);
        }
    }
    Ok(())
}

/// 反推图片路径中的 (子目录, 图片名)（对等 PlayerService.parse_image_path）
pub fn parse_image_path(image_path: &str) -> Option<(String, String)> {
    let re = Regex::new(r"[\\/]+([^\\/]+)[\\/]+([^\\/]+)\.(?:png|jpg|jpeg)$").unwrap();
    let caps = re.captures(image_path)?;
    Some((caps[1].to_string(), caps[2].to_string()))
}

/// 探测实际图片文件（.png/.jpg/.jpeg，对等 PlayerService.resolve_image_file）
pub fn resolve_image_file(img_dir: &Path, ethnicity: &str, image: &str) -> Option<String> {
    for ext in ["png", "jpg", "jpeg"] {
        let p = img_dir.join(ethnicity).join(format!("{image}.{ext}"));
        if p.is_file() {
            return Some(p.to_string_lossy().to_string());
        }
    }
    None
}

/// 给 UID 补随机人前缀的辅助（查询缓存时兼容两种形态）
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
    fn parse_image_path_and_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let eth = dir.path().join("MENA");
        std::fs::create_dir_all(&eth).unwrap();
        std::fs::write(eth.join("m9.png"), b"png").unwrap();
        assert_eq!(
            parse_image_path(r"C:\pack\MENA\m9.png"),
            Some(("MENA".into(), "m9".into()))
        );
        assert_eq!(parse_image_path(r"C:\m9.png"), None);
        assert_eq!(
            resolve_image_file(dir.path(), "MENA", "m9"),
            Some(eth.join("m9.png").to_string_lossy().to_string())
        );
        assert_eq!(resolve_image_file(dir.path(), "MENA", "nope"), None);
    }
}
