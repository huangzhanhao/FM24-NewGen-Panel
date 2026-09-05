//! 【新手导读】RTF 球员名单解析（自旧版 core/RtfParser.py 移植，行为对等）。
//! FM（Football Manager）可导出一种用竖线 | 分隔列的 RTF 表格，里面是球员信息。
//! 本模块把这些文本行解析成结构化的 PlayerRecord（包含 UID、国籍、姓名、种族码等）。
//! 关键差异：不再逐行打日志（v1 卡顿根因），仅返回结构化结果。
use regex::Regex;                 // 正则表达式库（用于判断一行是否为合法球员行）
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;          // 正则编译开销大，用 LazyLock 全局只编译一次

pub const BASE_FIELD_COUNT: usize = 7;      // 基础名单每行的字段数（UID/国籍/二国籍/姓名/发长/发色/种族码）
pub const DETAILED_FIELD_COUNT: usize = 14; // 详细名单每行的字段数（额外含身高/体重/是否随机人等）
const HEIGHT_INDEX: usize = 11;             // 身高在详细字段中的下标
const WEIGHT_INDEX: usize = 12;             // 体重下标
const RANDOM_FLAG_INDEX: usize = 13;        // 「是否随机人」标志下标
const NON_NEWGAN_FLAGS: [&str; 2] = ["No", "否"]; // 标志包含这些词则不是随机人（中英各一种）
pub const NEWGAN_UID_PREFIX: &str = "r-";   // 随机人（NewGAN）UID 的固定前缀
const ZERO_WIDTH_SPACE: &str = "\u{200b}";  // 零宽空格（FM 导出时可能混入的不可见字符）
const VALIDATE_LINE_LIMIT: usize = 20;      // 校验格式时只读前 20 行（足够判断，避免读全文）

// 英文名单：国籍为三字码（如 ESP）；中文名单：国籍为 CJK 汉字（如 西班牙）。
// 【为什么用正则】需要快速判断一行是否符合「| 数字UID | 国籍 | ... |」的结构，并区分中英文。
// [\x{4e00}-\x{9fff}] 是 Unicode 中的汉字区段。static + LazyLock 保证正则只编译一次、全局复用。
static RE_EN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\|\s*[0-9]{4,}\s*\|\s*[A-Z]{3}\s*\|(\s*([A-Z]{3})?\s*\|)?\s*[^|]+\s*\|\s*\d+\s*\|\s*\d+\s*\|\s*\d+\s*\|",
    ).unwrap()
});
static RE_CN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\|\s*[0-9]{4,}\s*\|\s*[\x{4e00}-\x{9fff}]+\s*\|\s*[\x{4e00}-\x{9fff}]*\s*\|\s*[^|]+\s*\|\s*\d+\s*\|\s*\d+\s*\|\s*\d+\s*\|",
    ).unwrap()
});

/// 详细名单才有的额外信息（身高/体重/俱乐部等）。同样用 camelCase 序列化为前端字段。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetailedInfo {
    pub skin_code: String,
    pub face_id: String,
    pub club: String,
    pub age: String,
    pub height: String,
    pub weight: String,
    pub is_newgan: String,
}

/// 一个球员的解析结果（基础名单）。这是整个模块输出的核心数据结构。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRecord {
    pub uid: String,          // 随机人带 r- 前缀
    pub primary_nat: String,  // 解析后统一为英文三字码
    pub sec_nat: String,
    pub name: String,
    pub hair_length: String,
    pub hair_color: String,
    pub ethnicity_code: String, // "0".."10"
    // Option 表示「可能有也可能没有」；详细名单才有 detailed。
    // skip_serializing_if 让前端在无详细数据时不会看到这个字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detailed: Option<DetailedInfo>,
}

/// 解析器本体：记录当前 RTF 是否合法、是什么语言。
pub struct RtfParser {
    pub is_rtf_valid: bool,
    pub rtf_language: String, // "English" / "简体中文"
}

impl RtfParser {
    /// 创建一个新解析器，默认不合法、语言 English。
    pub fn new() -> Self {
        Self { is_rtf_valid: false, rtf_language: "English".into() }
    }

    /// 读前 20 行判定格式与语言（对等旧版 check_rtf_valid）。
    /// 【为何只读前 20 行】名单可能很大，但判断格式只需看开头几行，避免全文扫描拖慢速度。
    /// &mut self 表示会修改自身状态（is_rtf_valid / rtf_language）。
    pub fn check_rtf_valid(&mut self, path: &Path) -> Result<bool, String> {
        self.is_rtf_valid = false;
        self.rtf_language = "Unknown".into();
        let text = fs::read_to_string(path).map_err(|e| format!("读取 RTF 失败: {e}"))?;
        // take(20) 取前 20 行，collect 成 Vec 再 join 拼回一个字符串，供正则匹配。
        let head: String = text.lines().take(VALIDATE_LINE_LIMIT).collect::<Vec<_>>().join("\n");
        // 先试英文正则，再试中文正则；哪个匹配就定为哪种语言且标记合法。
        if RE_EN.is_match(&head) {
            self.rtf_language = "English".into();
            self.is_rtf_valid = true;
        } else if RE_CN.is_match(&head) {
            self.rtf_language = "简体中文".into();
            self.is_rtf_valid = true;
        }
        Ok(self.is_rtf_valid)
    }

    /// 逐行解析（须先 check_rtf_valid 通过）。返回英文三字码数据。
    /// filter_newgen：是否只保留随机人；translations：中文国籍翻译表。
    pub fn parse_rtf(
        &self,
        path: &Path,
        filter_newgen: bool,
        translations: &HashMap<String, HashMap<String, String>>,
    ) -> Result<Vec<PlayerRecord>, String> {
        // 防御性校验：未先调用 check_rtf_valid 就直接解析时报错。
        if !self.is_rtf_valid {
            return Err("RTF 格式无效，请先校验".into());
        }
        let text = fs::read_to_string(path).map_err(|e| format!("读取 RTF 失败: {e}"))?;
        let mut data = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue; // 跳过空行
            }
            // 逐行解析；能解析出球员就加入结果（表头/分隔线等非数据行会返回 None 被跳过）。
            if let Some(rec) = Self::parse_line(line, filter_newgen) {
                data.push(rec);
            }
        }
        // 非英文名单：把中文国籍翻译成英文三字码（方便后续统一查种族表）。
        if self.rtf_language != "English" {
            data = Self::translate(data, translations, &self.rtf_language);
        }
        Ok(data)
    }

    /// 解析单行。返回 Some(球员) 表示成功，None 表示该行不是合法球员行（跳过）。
    /// 这是一个「关联函数」（无 self 参数），通过 Self::parse_line 调用。
    fn parse_line(line: &str, filter_newgen: bool) -> Option<PlayerRecord> {
        // 先用正则快速排除非数据行（表头、分隔线等）。
        if !(RE_EN.is_match(line) || RE_CN.is_match(line)) {
            return None;
        }
        // 按 | 切分列，每列 trim 去空白。首尾会各多出一个空段（因为行以 | 开头结尾）。
        let mut fields: Vec<String> = line.split('|').map(|f| f.trim().to_string()).collect();
        if fields.len() < 2 {
            return None;
        }
        fields.pop();       // 去掉行尾空段
        fields.remove(0);   // 去掉行首空段
        if fields.len() < BASE_FIELD_COUNT {
            return None; // 字段不够（至少需 7 列）
        }
        // UID 必须是纯数字（逐字节判断都是 ASCII 数字），否则跳过。
        let uid_raw = fields[0].as_str();
        if uid_raw.is_empty() || !uid_raw.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // 种族码也必须是纯数字。
        let eth = fields[6].as_str();
        if eth.is_empty() || !eth.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // 种族码必须在 0..10 范围（ok()? 把解析失败也当作 None 返回）。
        if eth.parse::<u32>().ok()? > 10 {
            return None;
        }
        // 组装基础球员记录，UID 加上 r- 前缀（后面可能因「非随机人」再去掉）。
        let mut record = PlayerRecord {
            uid: format!("{NEWGAN_UID_PREFIX}{uid_raw}"),
            primary_nat: fields[1].clone(),
            sec_nat: fields[2].clone(),
            name: fields[3].clone(),
            hair_length: fields[4].clone(),
            hair_color: fields[5].clone(),
            ethnicity_code: eth.to_string(),
            detailed: None,
        };
        // 字段数 ≥ 14 说明是「详细名单」，额外解析身高/体重/是否随机人等。
        if fields.len() >= DETAILED_FIELD_COUNT {
            // 身高/体重可能混入零宽空格 U+200B，先清掉（否则 "18\u{200b}0" 会误判）。
            for idx in [HEIGHT_INDEX, WEIGHT_INDEX] {
                fields[idx] = fields[idx].replace(ZERO_WIDTH_SPACE, "");
            }
            let flag = fields[RANDOM_FLAG_INDEX].as_str();
            if NON_NEWGAN_FLAGS.iter().any(|f| flag.contains(f)) {
                // 非随机人：去掉 r- 前缀；filter_newgen=true 时整条剔除。
                record.uid = record
                    .uid
                    .strip_prefix(NEWGAN_UID_PREFIX)
                    .unwrap_or(&record.uid)
                    .to_string();
                if filter_newgen {
                    return None;
                }
            }
            record.detailed = Some(DetailedInfo {
                skin_code: fields[7].clone(),
                face_id: fields[8].clone(),
                club: fields[9].clone(),
                age: fields[10].clone(),
                height: fields[11].clone(),
                weight: fields[12].clone(),
                is_newgan: fields[13].clone(),
            });
        }
        Some(record)
    }

    /// 中文名籍 → 英文三字码（找不到时保留原值，对等 Python 行为）。
    /// data 按值传入（拿走所有权），map 逐条修改国籍后 collect 重新收集为 Vec。
    fn translate(
        data: Vec<PlayerRecord>,
        translations: &HashMap<String, HashMap<String, String>>,
        language: &str,
    ) -> Vec<PlayerRecord> {
        let empty = HashMap::new();
        // 取出当前语言的翻译表；没有就用空表（unwrap_or 兑底）。
        let map = translations.get(language).unwrap_or(&empty);
        data.into_iter()
            .map(|mut r| {
                // 主国籍：查表命中就替换为三字码，否则保留原值。
                if let Some(t) = map.get(&r.primary_nat) {
                    r.primary_nat = t.clone();
                }
                // 第二国籍：非空且命中才替换。
                if !r.sec_nat.is_empty() {
                    if let Some(t) = map.get(&r.sec_nat) {
                        r.sec_nat = t.clone();
                    }
                }
                r
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_rtf(content: &str) -> std::path::PathBuf {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.keep().unwrap().1
    }

    const EN_SAMPLE: &str = "\
| UID       | Nat  | 2nd Nat | Name          |    |    |   |
| ----------------------------------------------------------------|
| 2000472008| ESP  | USA     | Pepe Sáenz    | 1  | 12 | 1 |
| 2002161479| BRA  |         | José·Milhomem | 1  | 16 | 3 |
";

    const CN_SAMPLE: &str = "\
| 编号      | 国籍/地区籍 | 第二国籍 | 姓名             |    |    |   |
| 2002075172| 美国     |        | 扎克·布劳顿      | 3  | 9  | 0 |
";

    #[test]
    fn validate_english_and_chinese() {
        let mut p = RtfParser::new();
        assert!(p.check_rtf_valid(&write_rtf(EN_SAMPLE)).unwrap());
        assert_eq!(p.rtf_language, "English");
        let mut p = RtfParser::new();
        assert!(p.check_rtf_valid(&write_rtf(CN_SAMPLE)).unwrap());
        assert_eq!(p.rtf_language, "简体中文");
        let mut p = RtfParser::new();
        assert!(!p.check_rtf_valid(&write_rtf("乱七八糟没有数据行")).unwrap());
    }

    #[test]
    fn parse_english_basic() {
        let path = write_rtf(EN_SAMPLE);
        let mut p = RtfParser::new();
        p.check_rtf_valid(&path).unwrap();
        let data = p.parse_rtf(&path, true, &HashMap::new()).unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].uid, "r-2000472008");
        assert_eq!(data[0].primary_nat, "ESP");
        assert_eq!(data[0].sec_nat, "USA");
        assert_eq!(data[0].name, "Pepe Sáenz");
        assert_eq!(data[0].ethnicity_code, "1");
        assert!(data[0].detailed.is_none());
    }

    #[test]
    fn parse_detailed_and_newgan_filter() {
        let sample = "\
| 2001234567| ESP  | USA     | Newgen Kid    | 1  | 12 | 1  | 2 | 1 | FC Test | 19 | 180 | 75 | Yes |
| 2001234568| ESP  |         | Real Legend   | 1  | 12 | 1  | 2 | 1 | FC Test | 31 | 18\u{200b}0 | 80 | No  |
";
        let path = write_rtf(sample);
        let mut p = RtfParser::new();
        p.check_rtf_valid(&path).unwrap();
        // 过滤随机人：只剩 Yes 那条
        let data = p.parse_rtf(&path, true, &HashMap::new()).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-2001234567");
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(d.club, "FC Test");
        assert_eq!(d.height, "180");
        // 不过滤：No 那条保留且无 r- 前缀，零宽空格被清理
        let data = p.parse_rtf(&path, false, &HashMap::new()).unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[1].uid, "2001234568");
        assert_eq!(data[1].detailed.as_ref().unwrap().height, "180");
    }

    #[test]
    fn invalid_lines_skipped() {
        // UID 非纯数字 / 种族码越界 → 跳过
        let sample = "\
| 2001234567| ESP  |         | Ok Player     | 1  | 12 | 1 |
| abc123    | ESP  |         | Bad Uid       | 1  | 12 | 1 |
| 2001234569| ESP  |         | Bad Eth       | 1  | 12 | 99 |
";
        let path = write_rtf(sample);
        let mut p = RtfParser::new();
        p.check_rtf_valid(&path).unwrap();
        let data = p.parse_rtf(&path, true, &HashMap::new()).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-2001234567");
    }

    #[test]
    fn translate_chinese_nationality() {
        let path = write_rtf(CN_SAMPLE);
        let mut p = RtfParser::new();
        p.check_rtf_valid(&path).unwrap();
        let mut tr = HashMap::new();
        tr.insert("简体中文".to_string(), {
            let mut m = HashMap::new();
            m.insert("美国".to_string(), "USA".to_string());
            m
        });
        let data = p.parse_rtf(&path, true, &tr).unwrap();
        assert_eq!(data[0].primary_nat, "USA");
    }
}
