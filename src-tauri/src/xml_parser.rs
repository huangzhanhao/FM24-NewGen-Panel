//! config.xml 读写：模板 + 行级正则替换（自旧版 core/XmlParser.py 移植）。
//!
//! config.xml 是 FM 认的「头像映射表」，每行形如：
//!   <record from="种族目录/图片名" to="graphics/pictures/person/<UID>/portrait"/>
//! 即「把某张图片用于某个 UID 的球员头像」。本模块负责全量生成、整体解析、单行替换与滚动备份。
//!
//! 【日志】本模块全是无状态的纯函数，不自带 Logger，故经全局 log 门面直接记录（模块名 xml，
//! 由官方插件分发给 stdout / 滚动文件 / 内存缓冲 → 前端日志面板）。
//! 口径与 face_mapper 一致：**只记上层看不到的内部信息**，不与调用方的阶段日志重复
//! （「写入 N 条映射」「读取快照完成 N 条」等已由 replace_flow / commands 记录，此处不再复述）——
//! 于是这里只留三类线索：畸形映射行的丢弃数、备份的创建与轮转清理、单人替换落空的具体原因。
use crate::assets::CONFIG_TEMPLATE;
use crate::paging::PageResult;
use crate::rtf_scanner::NEWGAN_UID_PREFIX;
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
    if ethnicity.is_empty() {
        image.to_string()
    } else {
        format!("{ethnicity}/{image}")
    }
}

/// 解析 config.xml → 有序条目列表（保留文件里的出现顺序，不做去重）。
/// 【何时用】「快照预览」要展示快照的完整映射清单；按 UID 查找的场景用 parse_xml。
pub fn parse_xml_entries(path: &Path) -> Result<Vec<MappingEntry>, String> {
    parse_xml_entries_progress(path, &mut |_, _| {})
}

/// 映射行的行首标记：既是进度分母的计数口径，也是 `RECORD_RE` 的必需前缀。
///
/// 【为什么要单独给分母】`captures_iter` 是懒迭代器，不先拿到总数就没有分母。原先写作
/// `RECORD_RE.find_iter(&text).count()` —— 那是**第二趟完整正则匹配**，15 万行的大表约占总解析
/// 耗时的三成。本标记是 `RECORD_RE` 里的硬性字面片段（`from="`），所以 `text.matches(...)` 的计数
/// **≥ 真实匹配数**：分母偏大只会让进度条提前停在区间内（紧随其后就是下一阶段的锚点，仍单调），
/// 绝不会因为分母偏小而让计数越界。`matches` 走 memchr，比跑一遍正则快得多。
/// 【口径同步】它与 `RECORD_RE` 必须一致；`parse_xml_entries_progress_reports_exact_total` 测试守着这条。
const RECORD_MARK: &str = "<record from=\"";

/// 同 `parse_xml_entries`，额外把「已解析第 n 条 / 共 total 条」上报给进度回调（「读取快照」状态栏进度用）。
pub fn parse_xml_entries_progress(
    path: &Path,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<MappingEntry>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读取 config.xml 失败: {e}"))?;
    let total = text.matches(RECORD_MARK).count(); // 进度分母（计数口径见 RECORD_MARK 的说明）
    let mut done = 0usize;
    let mut entries = Vec::new();
    for cap in RECORD_RE.captures_iter(&text) {
        done += 1;
        progress(done, total); // 逐条上报（订阅方按百分比去重后才 emit，不会刷爆 IPC）

        // from 的内容为相对路径："SpanMed/img1"（标准二）、"img1"（标准一平铺）、
        // "子目录/孙目录/图名"（Unknown 嵌套）。最后一段是图名，其余整体为相对目录。
        // 【为什么用 rsplit_once 而不是 split('/').collect() + join("/")】后者每行先建一个 Vec、
        // 再建一个新 String（15 万行 = 30 万次额外分配）；前者只从右切一次、只建两个 String，
        // 语义完全等价（含 "a/" → ("a","")、"/x" → ("","x") 这些边界）。
        let from = &cap[1];
        let (eth, img) = match from.rsplit_once('/') {
            Some((dir, name)) => (dir.to_string(), name.to_string()),
            None => (String::new(), from.to_string()),
        };
        entries.push(MappingEntry {
            uid: cap[2].to_string(),
            ethnicity: eth,
            image: img.to_string(),
        });
    }
    // 【分母 vs 实得】RECORD_MARK 是 RECORD_RE 里的字面片段，故恒有 total ≥ entries.len()；差值就是
    // 「行首形似映射行、却没被正则接受」的行（to 不是 person/<UID>/portrait、UID 不足 4 位、缺右引号等），
    // 它们被静默跳过。上层只拿到「N 条映射」这个数字，看不出文件里混着不合法内容——
    // 这条 WARNING 是唯一线索，用户可据此定位并手工修正 config.xml（而非「少了几个头像」无从下手）。
    // 等于时不记：那才是正常文件（否则「读取快照完成：N 条」刚打完又来一条，纯噪声）。
    if total > entries.len() {
        log::warn!(
            target: "xml",
            "config.xml 有 {} 行行首是 <record from=\" 但未匹配到头像映射格式（共 {} 行候选），已跳过：{}",
            total - entries.len(), total, path.display()
        );
    }
    Ok(entries)
}

/// 解析 config.xml → {uid: entry}（用字典按 UID 快速查找；同 UID 重复行保留最后一条）。
pub fn parse_xml(path: &Path) -> Result<HashMap<String, MappingEntry>, String> {
    parse_xml_progress(path, &mut |_, _| {})
}

/// 同 `parse_xml`，额外把「已解析第 n 条 / 共 total 条」上报给进度回调
/// （替换流水线按 Preserve/Overwrite 读现有 config.xml 时用）。
/// 【为何委托给 entries 版】两者本就是同一趟解析，只是最后收成字典还是保留顺序；
/// 摊开写第二遍会把「先数一遍拿分母」的口径复制一份，容易两处走偏。
pub fn parse_xml_progress(
    path: &Path,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<HashMap<String, MappingEntry>, String> {
    Ok(parse_xml_entries_progress(path, progress)?
        .into_iter()
        .map(|e| (e.uid.clone(), e))
        .collect())
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
                record_from(&m.ethnicity, &m.image),
                m.uid
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
    pack: &str,  // 新图片所在种族目录
    image: &str, // 新图片名
    img_dir: &Path,
    save_backup: bool,
) -> Result<bool, String> {
    let config_path = img_dir.join("config.xml");
    if !config_path.is_file() {
        // 上层拿到的只有 false，与「UID 不存在」同形；把「文件压根没有」这个不同原因单独留痕，
        // 否则用户只看到「单人微调没反应」，而命令层那条 INFO 日志会误报成「换脸成功」。
        log::warn!(
            target: "xml",
            "单人替换跳过：头像包内没有 config.xml，无从改起（{}）",
            config_path.display()
        );
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
    if !replaced {
        // 文件在、但整表没有该 UID 的映射行（多为 UID 填错，或该球员本就不在本次映射里）。
        // 命令层无论如何都会打一条「单人换脸 {uid} → …」的 INFO，此处补齐「其实没改成」的反证。
        log::warn!(
            target: "xml",
            "单人替换未命中：config.xml 中没有 UID {uid} 的映射行，文件内容未改动（{}）",
            config_path.display()
        );
    }
    Ok(replaced)
}

/// 备份为 config备份_YYYYMMDD-HHMMSS.xml，仅保留最近 10 份。
fn backup_rotation(config_path: &Path, img_dir: &Path) -> Result<(), String> {
    // 用当前时间戳生成唯一备份名，先拷贝一份。
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup = img_dir.join(format!("config_backup_{ts}.xml"));
    fs::copy(config_path, &backup).map_err(|e| e.to_string())?;
    // 【上层只看到「写前备份=是」】具体落在哪个文件、又清理了哪些旧备份，都只有这里知道；
    // 用户复查「上次替换前那份 config.xml」时全靠这条日志给出确切的文件名。
    log::info!(
        target: "xml",
        "已备份现有 config.xml → {}",
        backup.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    );
    // 列出目录下所有历史备份。
    let mut backups: Vec<_> = fs::read_dir(img_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("config_backup_") && n.ends_with(".xml")
        })
        .map(|e| e.path())
        .collect();
    backups.sort(); // 文件名含时间戳，字典序 = 时间序
                    // 超过 10 份就删除最旧的（切片 [..len-10] 是前面的旧备份）。
    if backups.len() > BACKUP_KEEP {
        // 被清理的是「第 n 份更早的备份」，用户发现某份历史备份不见了时，这条日志能解释是轮转删的
        // （而不是杀软/同步盘动的）。删除失败只 `let _ =` 吞掉（旧备份留着不影响正确性），故不逐份记名。
        let pruned = backups.len() - BACKUP_KEEP;
        for old in &backups[..backups.len() - BACKUP_KEEP] {
            let _ = fs::remove_file(old);
        }
        log::info!(
            target: "xml",
            "备份已超过 {} 份，清理最旧的 {} 份（现有备份 {} 份）",
            BACKUP_KEEP, pruned, BACKUP_KEEP
        );
    }
    Ok(())
}

/// 从用户选中的图片绝对路径反推 (相对目录路径, 图名)。只认头像包内真实存在的图片：
/// 包根图片 → 目录为空串；子/孙目录图片 → 相对包根的目录路径，多级用 "/" 分隔（如 "MENA/sub"）。
/// 包外路径与不存在的文件一律返回 None——旧版对取不到相对路径的情况会退化成最后一级目录名，
/// 把包外图片也写进 config.xml，FM 读不到那张图还污染映射表，故按「单人微调只认包内图片」收紧。
/// 词干走 facepack_scanner::image_stem 唯一口径，与 scan_pack 的 root_images 同规则（"a.1.png" → "a.1"）。
pub fn parse_image_path_in_pack(img_dir: &str, image_path: &str) -> Option<(String, String)> {
    let p = Path::new(image_path);
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
        return None;
    }
    let image = crate::facepack_scanner::image_stem(image_path)?;
    if image.starts_with('.') {
        return None;
    } // 拒收 ._x.png 等隐藏名，与池侧口径一致
    if !p.is_file() {
        return None;
    } // 手输路径可能打错：不存在的文件写进 config.xml 会替换「成功」但 FM 显示空
      // 两边 canonicalize 后做前缀剥离（Windows 大小写/分隔符不敏感的权威比较）；
      // 剥离失败 = 图片在包外（含不同盘），直接拒绝，不再退化取目录名。
    let parent = p.parent()?;
    let rel = parent
        .canonicalize()
        .ok()?
        .strip_prefix(&Path::new(img_dir).canonicalize().ok()?)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    Some((rel, image))
}

/// 探测实际图片文件（.png/.jpg/.jpeg，对等旧版 PlayerService.resolve_image_file）。
/// config.xml 里只存图片名不存扩展名，需按常见扩展名依次试找出真实文件。
/// ethnicity 为映射里的相对目录路径（"" = 包根，多级 "a/b" = 子/孙目录）。
pub fn resolve_image_file(img_dir: &Path, ethnicity: &str, image: &str) -> Option<String> {
    let mut base = img_dir.to_path_buf();
    if !ethnicity.is_empty() {
        for seg in ethnicity.split('/') {
            // 相对路径逐级拼接（"/" 统一分隔符）
            base = base.join(seg);
        }
    }
    for ext in ["png", "jpg", "jpeg"] {
        let p = base.join(format!("{image}.{ext}"));
        if p.is_file() {
            return Some(p.to_string_lossy().to_string());
        }
    }
    None // 三种扩展名都没找到
}

/// 「头像」列原文是否为裸词干（无路径分隔符、无扩展名）：只有裸词干才允许在头像包里递归兜底找同名图；
/// 带路径/带扩展名的输入已按其指定位置探测过，不再递归（避免歧义命中）。
pub fn is_bare_stem(avatar: &str) -> bool {
    !avatar.contains('/') && !avatar.contains('\\') && Path::new(avatar).extension().is_none()
}

/// 解析名单「头像」列原文为头像包内的实际图片文件（名单预览悬停/点击预览用）。
/// FM 导出时该列通常为空文本；非空时按用户可能填入的形态依次尝试：
///   1. 绝对路径（盘上确实存在的图片文件，原样返回）；
///   2. 相对头像包根的路径且自带扩展名（如 `African/face_1.png`），判存在即返回；
///   3. 不带扩展名的相对路径/裸词干（如 `face_kuka`、`MENA/sub/face_2`），
///      按 png/jpg/jpeg 依次探测（复用 resolve_image_file 的配置口径）。
/// 裸词干在包根子目录里的递归兜底由命令层借助头像扫描索引完成，本函数只做确定性路径探测。
pub fn resolve_avatar_path(img_dir: &Path, avatar: &str) -> Option<String> {
    let avatar = avatar.trim();
    if avatar.is_empty() {
        return None;
    }
    let raw = Path::new(avatar);
    // 1) 绝对路径：以文件真实存在为准。
    if raw.is_absolute() {
        return if raw.is_file() {
            Some(raw.to_string_lossy().to_string())
        } else {
            None
        };
    }
    let rel = img_dir.join(raw);
    // 2) 自带扩展名：相对包根判存在。
    if raw.extension().is_some() {
        return if rel.is_file() {
            Some(rel.to_string_lossy().to_string())
        } else {
            None
        };
    }
    // 3) 无扩展名：拆成「相对目录 + 词干」，按三种扩展名探测
    //    （裸词干的相对目录为空串 = 包根；多级用 "/" 分隔，与 resolve_image_file 同口径）。
    let rel_dir = raw
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let stem = raw.file_stem()?.to_string_lossy().to_string();
    if stem.is_empty() || stem == "." {
        return None;
    }
    resolve_image_file(img_dir, &rel_dir, &stem)
}

/// 给 UID 补随机人前缀的辅助（查询缓存时兼容两种形态）。
/// 同一球员 UID 在缓存里可能带 r- 也可能不带（随机人带、原生不带），返回两种形态都查一遍。
pub fn with_newgan_prefix_variants(uid: &str) -> [String; 2] {
    let prefixed = format!("{NEWGAN_UID_PREFIX}{uid}");
    [uid.to_string(), prefixed]
}

/// 快照预览的一行（对应前端 SnapshotEntry）：UID + 头像相对路径（from 原文，平铺包无目录前缀）
/// + 按当前展示组头像包解析出的实际图片路径（包内找不到为 None/null）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEntry {
    pub uid: String,
    pub rel_path: String,
    pub image_path: Option<String>,
}

/// 快照预览分页查询参数（search 按 UID 子串筛选，空串=不筛选）。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotQuery {
    pub offset: usize,
    pub limit: usize,
    pub search: String,
}

/// 快照预览的筛选/分页/图片解析（纯逻辑，不碰后端状态）：每条图片解析最多探测 png/jpg/jpeg
/// 三种扩展名，且仅对当前页的条目执行——整表几万条逐一探测文件开销太大，按页解析才现实。
pub fn query_snapshot_page(
    entries: &[MappingEntry],
    img_dir: &str,
    query: &SnapshotQuery,
) -> PageResult<SnapshotEntry> {
    let search = query.search.trim().to_lowercase();
    let matched: Vec<&MappingEntry> = entries
        .iter()
        .filter(|e| search.is_empty() || e.uid.to_lowercase().contains(&search))
        .collect();
    let total = matched.len();
    let root = Path::new(img_dir);
    let items = matched
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .map(|e| SnapshotEntry {
            uid: e.uid.clone(),
            rel_path: record_from(&e.ethnicity, &e.image),
            image_path: resolve_image_file(root, &e.ethnicity, &e.image),
        })
        .collect();
    PageResult { total, items }
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
            MappingEntry {
                uid: "r-2000472008".into(),
                ethnicity: "SpanMed".into(),
                image: "img1".into(),
            },
            MappingEntry {
                uid: "2001234568".into(),
                ethnicity: "African".into(),
                image: "img2".into(),
            },
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
    fn parse_xml_entries_keeps_order_and_matches_dict() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        write_xml(
            &[
                MappingEntry {
                    uid: "2000000001".into(),
                    ethnicity: "Asian".into(),
                    image: "a1".into(),
                },
                MappingEntry {
                    uid: "r-2000000002".into(),
                    ethnicity: "MENA/sub".into(),
                    image: "m9".into(),
                },
            ],
            &pack,
            false,
        )
        .unwrap();
        // 有序版：保留文件顺序与多级目录拆分。
        let list = parse_xml_entries(&pack.join("config.xml")).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].uid, "2000000001");
        assert_eq!(list[1].ethnicity, "MENA/sub");
        assert_eq!(list[1].image, "m9");
        // 字典版与有序版内容一致（同 UID 保留最后一条的语义不变）。
        let map = parse_xml(&pack.join("config.xml")).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map["r-2000000002"].image, "m9");
    }

    #[test]
    fn written_mapping_reparses_identically_in_order() {
        // 【P0-3 的等价性依据】write_xml 按**传入顺序**逐行写出，parse_xml_entries 又按**文件顺序**
        // 返回，所以「拿本次 mapping 直填快照缓存」与「回读刚写出的文件」结果逐条相同（顺序也相同）。
        // 替换收尾省掉的正是那次回读（15 万行实测 1.4–1.5 s，且卡住按钮解禁）——
        // 这条断言就是那次省略的安全网：一旦 write/parse 的口径不再互逆，它会立刻失败。
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        let data = vec![
            MappingEntry {
                uid: "r-1001".into(),
                ethnicity: "".into(),
                image: "flat1".into(),
            },
            MappingEntry {
                uid: "r-1002".into(),
                ethnicity: "SpanMed".into(),
                image: "s.1".into(),
            },
            MappingEntry {
                uid: "2000000003".into(),
                ethnicity: "MENA/sub".into(),
                image: "deep".into(),
            },
        ];
        write_xml(&data, &pack, false).unwrap();
        let parsed = parse_xml_entries(&pack.join("config.xml")).unwrap();
        assert_eq!(parsed, data, "直填（用写入的 Vec）必须与回读文件逐条相同");
    }

    #[test]
    fn parse_xml_entries_progress_reports_exact_total() {
        // 进度分母（RECORD_MARK 的字面计数）必须与真实解析出的条数一致。
        // 分母偏大只会让进度条在区间内提前停住（后面还有下一阶段的锚点，仍单调），不会越界；
        // 但若 RECORD_RE 改了而 RECORD_MARK 忘了同步，这条断言会立刻失败——两份口径的唯一守卫。
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        write_xml(
            &[
                MappingEntry {
                    uid: "2000000001".into(),
                    ethnicity: "Asian".into(),
                    image: "a1".into(),
                },
                MappingEntry {
                    uid: "r-2000000002".into(),
                    ethnicity: "MENA/sub".into(),
                    image: "m9".into(),
                },
                MappingEntry {
                    uid: "2000000003".into(),
                    ethnicity: "".into(),
                    image: "flat".into(),
                },
            ],
            &pack,
            false,
        )
        .unwrap();
        let mut seen: Vec<(usize, usize)> = Vec::new();
        let entries =
            parse_xml_entries_progress(&pack.join("config.xml"), &mut |d, t| seen.push((d, t)))
                .unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(
            seen.last().copied(),
            Some((3, 3)),
            "进度应报到 (条数, 条数)：{seen:?}"
        );
        // 模板自身（<record> 根元素、注释、<list>）不含行首标记，所以计数恰好等于映射行数。
        let text = std::fs::read_to_string(pack.join("config.xml")).unwrap();
        assert_eq!(text.matches(RECORD_MARK).count(), 3);
    }

    #[test]
    fn single_replacement_hits_only_target_line() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("pack");
        std::fs::create_dir_all(&pack).unwrap();
        write_xml(
            &[
                MappingEntry {
                    uid: "r-1001".into(),
                    ethnicity: "Asian".into(),
                    image: "a1".into(),
                },
                MappingEntry {
                    uid: "r-1002".into(),
                    ethnicity: "Asian".into(),
                    image: "a2".into(),
                },
            ],
            &pack,
            false,
        )
        .unwrap();
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
        write_xml(
            &[MappingEntry {
                uid: "r-1".into(),
                ethnicity: "Asian".into(),
                image: "x".into(),
            }],
            &pack,
            false,
        )
        .unwrap();
        // 手动造 12 份历史备份（时间戳递增）
        for i in 0..12 {
            let name = format!("config备份_20260101-0000{:02}.xml", i);
            std::fs::copy(pack.join("config.xml"), pack.join(&name)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100)); // 秒级时间戳递增
        }
        write_xml(
            &[MappingEntry {
                uid: "r-1".into(),
                ethnicity: "Asian".into(),
                image: "x".into(),
            }],
            &pack,
            true,
        )
        .unwrap();
        let count = std::fs::read_dir(&pack)
            .unwrap()
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
            &[MappingEntry {
                uid: "r-1001".into(),
                ethnicity: "".into(),
                image: "img1".into(),
            }],
            &pack,
            false,
        )
        .unwrap();
        let text = std::fs::read_to_string(pack.join("config.xml")).unwrap();
        assert!(text.contains(r#"from="img1""#));
        let parsed = parse_xml(&pack.join("config.xml")).unwrap();
        assert_eq!(parsed["r-1001"].ethnicity, "");
        assert_eq!(parsed["r-1001"].image, "img1");
        // 单人替换 pack="" → from="image"（平铺形态）
        assert!(single_replacement("r-1001", "", "img2", &pack, false).unwrap());
        let parsed2 = parse_xml(&pack.join("config.xml")).unwrap();
        assert_eq!(
            (
                parsed2["r-1001"].ethnicity.as_str(),
                parsed2["r-1001"].image.as_str()
            ),
            ("", "img2")
        );
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
        assert_eq!(
            parse_image_path_in_pack(&root_s, &dir.path().join("x.txt").to_string_lossy()),
            None
        );
        assert_eq!(
            parse_image_path_in_pack(&root_s, &dir.path().join("._h.png").to_string_lossy()),
            None
        );
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

    #[test]
    fn parse_image_path_in_pack_rejects_outside_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root_s = dir.path().to_string_lossy().to_string();
        // 包根合法图作对照（能反推出空目录）
        std::fs::write(dir.path().join("in.png"), b"png").unwrap();
        assert_eq!(
            parse_image_path_in_pack(&root_s, &dir.path().join("in.png").to_string_lossy()),
            Some(("".into(), "in".into()))
        );
        // 包外同盘目录（tempdir 的兄弟目录）：旧版会退化取最后一级目录名放行，现在必须拒
        let out = dir.path().parent().unwrap().join("ngm_out_of_pack");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("o1.png"), b"png").unwrap();
        assert_eq!(
            parse_image_path_in_pack(&root_s, &out.join("o1.png").to_string_lossy()),
            None
        );
        // 路径在包内但文件不存在（手输打错字）
        assert_eq!(
            parse_image_path_in_pack(&root_s, &dir.path().join("ghost.png").to_string_lossy()),
            None
        );
    }

    #[test]
    fn resolve_avatar_path_variants() {
        let dir = tempfile::tempdir().unwrap();
        // 包根图（裸词干，无扩展名 → png/jpg/jpeg 探测）。
        std::fs::write(dir.path().join("face_kuka.png"), b"png").unwrap();
        assert_eq!(
            resolve_avatar_path(dir.path(), "face_kuka"),
            Some(
                dir.path()
                    .join("face_kuka.png")
                    .to_string_lossy()
                    .to_string()
            )
        );
        // 自带扩展名的相对路径。
        assert_eq!(
            resolve_avatar_path(dir.path(), "face_kuka.png"),
            Some(
                dir.path()
                    .join("face_kuka.png")
                    .to_string_lossy()
                    .to_string()
            )
        );
        // 子目录词干（多级相对路径，补扩展名）。
        let sub = dir.path().join("MENA").join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("face_2.jpg"), b"jpg").unwrap();
        assert_eq!(
            resolve_avatar_path(dir.path(), "MENA/sub/face_2"),
            Some(sub.join("face_2.jpg").to_string_lossy().to_string())
        );
        // 反斜杠分隔同样成立（Windows 用户手写路径；Linux 下反斜杠不是路径分隔符，故仅 Windows 断言）。
        #[cfg(windows)]
        assert_eq!(
            resolve_avatar_path(dir.path(), r"MENA\sub\face_2"),
            Some(sub.join("face_2.jpg").to_string_lossy().to_string())
        );
        // 绝对路径：存在才命中。
        let abs = dir.path().join("abs_face.png");
        std::fs::write(&abs, b"png").unwrap();
        assert_eq!(
            resolve_avatar_path(dir.path(), &abs.to_string_lossy()),
            Some(abs.to_string_lossy().to_string())
        );
        // 空白 / 不存在的词干 / 不存在的绝对路径 → None（子目录递归查找不在本函数职责内）。
        assert_eq!(resolve_avatar_path(dir.path(), "   "), None);
        assert_eq!(resolve_avatar_path(dir.path(), "no_such_face"), None);
        assert_eq!(
            resolve_avatar_path(
                dir.path(),
                &dir.path().join("missing.png").to_string_lossy()
            ),
            None
        );
    }

    #[test]
    fn query_snapshot_page_filters_paginates_resolves() {
        let tmp = tempfile::tempdir().unwrap();
        let eth = tmp.path().join("SpanMed");
        std::fs::create_dir_all(&eth).unwrap();
        std::fs::write(eth.join("img1.png"), b"png").unwrap();
        let entries = vec![
            MappingEntry {
                uid: "r-1001".into(),
                ethnicity: "SpanMed".into(),
                image: "img1".into(),
            },
            MappingEntry {
                uid: "2000472008".into(),
                ethnicity: String::new(),
                image: "ghost".into(),
            },
        ];
        let root = tmp.path().to_str().unwrap();
        let q = |offset: usize, limit: usize, search: &str| SnapshotQuery {
            offset,
            limit,
            search: search.into(),
        };
        // 全量：rel_path 复原 from 原文；能找到的图片解析出绝对路径，找不到的为 None。
        let all = query_snapshot_page(&entries, root, &q(0, 10, ""));
        assert_eq!(all.total, 2);
        assert_eq!(all.items[0].rel_path, "SpanMed/img1");
        assert!(all.items[0]
            .image_path
            .as_deref()
            .unwrap()
            .ends_with("img1.png"));
        assert_eq!(all.items[1].rel_path, "ghost"); // 平铺条目无目录前缀
        assert_eq!(all.items[1].image_path, None);
        // UID 子串筛选。
        let hit = query_snapshot_page(&entries, root, &q(0, 10, "1001"));
        assert_eq!(hit.total, 1);
        assert_eq!(hit.items[0].uid, "r-1001");
        // 翻页：limit=1 时第 2 页恰好是第二条。
        let page2 = query_snapshot_page(&entries, root, &q(1, 1, ""));
        assert_eq!(page2.items.len(), 1);
        assert_eq!(page2.items[0].uid, "2000472008");
    }
}
