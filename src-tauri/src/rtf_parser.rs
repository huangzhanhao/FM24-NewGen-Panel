//! 【新手导读】RTF 球员名单解析（自旧版 core/RtfParser.py 移植，行为对等）。
//! FM（Football Manager）可导出一种用竖线 | 分隔列的 RTF 表格，里面是球员信息。
//! 本模块把这些文本行解析成结构化的 PlayerRecord（包含 UID、国籍、姓名、种族码等）。
//! 关键差异：不再逐行打日志（v1 卡顿根因），仅返回结构化结果。
//!
//! 已知的导出格式有三类（见 `RtfFormat`）：经典布局的基础/详细名单共用同一列序，
//! 新版扩展名单（test_UntitledPlayer_ext_new 同款）列序完全不同，check_rtf_valid
//! 自动识别后由 parse_line 按布局取列。
use regex::Regex;                 // 正则表达式库（用于判断一行是否为合法球员行）
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;          // 正则编译开销大，用 LazyLock 全局只编译一次

pub const BASE_FIELD_COUNT: usize = 7;      // 基础名单每行的字段数（UID/国籍/二国籍/姓名/发长/发色/种族码）
pub const DETAILED_FIELD_COUNT: usize = 14; // 详细名单每行的字段数（额外含身高/体重/是否随机人等）
pub const EXT_NEW_FIELD_COUNT: usize = 17;  // 新版扩展名单每行的字段数（随机人标志前置 + 能力/潜力）
const RANDOM_FLAG_INDEX: usize = 13;        // 「是否随机人」标志下标
const NON_NEWGAN_FLAGS: [&str; 2] = ["No", "否"]; // 标志包含这些词则不是随机人（中英各一种）
pub const NEWGAN_UID_PREFIX: &str = "r-";   // 随机人（NewGAN）UID 的固定前缀
const ZERO_WIDTH_SPACE: &str = "\u{200b}";  // 零宽空格（FM 导出时可能混入的不可见字符）
const VALIDATE_LINE_LIMIT: usize = 20;      // 校验格式时只看前 20 行（足够判断，避免读全文）
const HEAD_BYTES: u64 = 64 * 1024;          // 校验时最多读入的字节数（大名单可达上百 MB）

// 新版扩展名单（ext_new）的列下标。列序与经典布局完全不同：
// 随机人标志从最后一列挪到第 2 列，种族码挪到第 12 列，末尾是能力/潜力（本模块不使用）。
const XN_UID: usize = 0;             // 编号（6-10 位数字）
const XN_NEWGAN_FLAG: usize = 1;     // 「Newgen」随机人标志（是/否、Yes/No）
const XN_NAME: usize = 4;            // 姓名（未命名的 regen 显示为 "- -"）
const XN_PRIMARY_NAT: usize = 5;     // 国籍/地区籍
const XN_SEC_NAT: usize = 6;         // 第二国籍/地区籍
const XN_AGE: usize = 7;             // 年龄
const XN_HEIGHT: usize = 8;          // 身高（如 "181厘米" / "181 cm"）
const XN_CLUB: usize = 9;            // 俱乐部
const XN_ETHNICITY: usize = 11;      // 种族码（0-10）
const XN_SKIN: usize = 12;           // 肤色（实测 0-19）
const XN_HAIR_LENGTH: usize = 13;    // 发长（实测 0-4）
const XN_HAIR_COLOR: usize = 14;     // 发色（实测 1-18）

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
// 新版扩展名单数据行：
// | UID | 随机人标志(是/否/Yes/No) | 现状 | 空列 | 姓名 | 国籍 | 二国籍(可空) | 年龄 | 身高 | 俱乐部 | 位置 | 种族码 | 肤色 | 发长 | 发色 | 能力 | 潜力 |
// 国籍列兼容三字码与中文全名（随游戏界面语言而定）；现状/身高/俱乐部/位置为自由文本。
// 实测对 ext_new 测试文件 15.8 万行全量命中，且不会误伤经典布局的数据行。
static RE_NEW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\|\s*[0-9]{4,}\s*\|\s*(?:是|否|Yes|No)\s*\|[^|]*\|[^|]*\|[^|]*\|\s*(?:[A-Z]{3}|[\x{4e00}-\x{9fff}]+)\s*\|\s*(?:[A-Z]{3}|[\x{4e00}-\x{9fff}]+)?\s*\|\s*\d+\s*\|[^|]*\|[^|]*\|[^|]*\|\s*\d{1,2}\s*\|\s*\d+\s*\|\s*\d+\s*\|\s*\d+\s*\|\s*\d+\s*\|\s*\d+\s*\|",
    ).unwrap()
});

/// 只读文件前 `max` 字节并按 UTF-8 解码（截断处残缺的多字节字符容忍处理）。
/// 校验只用文件开头几行，这样上百 MB 的名单也不必整份载入内存。
fn read_head(path: &Path, max: u64) -> Result<String, String> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|e| format!("读取 RTF 失败: {e}"))?;
    let mut buf = Vec::new();
    file.take(max).read_to_end(&mut buf).map_err(|e| format!("读取 RTF 失败: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

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

/// RTF 名单的列布局。已知的三种导出格式归为两类：
/// 基础名单与详细名单（ext）共用经典列序，只是列数不同；
/// 新版扩展名单（ext_new）列序完全不同，单独一类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RtfFormat {
    /// 经典布局：| UID | 国籍 | 二国籍 | 姓名 | 发长 | 发色 | 种族码 |（详细名单再续 7 列）。
    #[default]
    Classic,
    /// 新版扩展布局（test_UntitledPlayer_ext_new 同款）：
    /// | UID | 随机人标志 | 现状 | 空列 | 姓名 | 国籍 | 二国籍 | 年龄 | 身高 | 俱乐部 | 位置
    /// | 种族码 | 肤色 | 发长 | 发色 | 能力 | 潜力 |。
    ExtNew,
}

/// 解析器本体：记录当前 RTF 是否合法、是什么语言、属于哪种列布局。
pub struct RtfParser {
    pub is_rtf_valid: bool,
    pub rtf_language: String, // "English" / "简体中文"
    pub rtf_format: RtfFormat,
}

impl RtfParser {
    /// 创建一个新解析器，默认不合法、语言 English、经典布局。
    pub fn new() -> Self {
        Self { is_rtf_valid: false, rtf_language: "English".into(), rtf_format: RtfFormat::default() }
    }

    /// 读文件头部判定格式与语言（对等旧版 check_rtf_valid，并支持新版扩展布局）。
    /// 【为何只读头部】名单可能上百 MB，但判断格式只需看开头几行；
    /// read_head 最多读 HEAD_BYTES 字节，不再为了看 20 行而把整份文件载入内存。
    /// &mut self 表示会修改自身状态（is_rtf_valid / rtf_language / rtf_format）。
    pub fn check_rtf_valid(&mut self, path: &Path) -> Result<bool, String> {
        self.is_rtf_valid = false;
        self.rtf_language = "Unknown".into();
        self.rtf_format = RtfFormat::Classic;
        let text = read_head(path, HEAD_BYTES)?;
        // 零宽空格会切断中文字段（如「兴\u{200b}趣」），匹配前整行统一清掉。
        let text = text.replace(ZERO_WIDTH_SPACE, "");
        // take(20) 取前 20 行，collect 成 Vec 再 join 拼回一个字符串，供正则匹配。
        let head: String = text.lines().take(VALIDATE_LINE_LIMIT).collect::<Vec<_>>().join("\n");
        // 先试经典布局（英文/中文），再试新版扩展布局；哪个匹配就定哪种布局且标记合法。
        if RE_EN.is_match(&head) {
            self.rtf_language = "English".into();
        } else if RE_CN.is_match(&head) {
            self.rtf_language = "简体中文".into();
        } else if RE_NEW.is_match(&head) {
            self.rtf_format = RtfFormat::ExtNew;
            // 新版名单的国籍列随游戏语言可能是三字码（无需翻译）或中文全名（需翻译），按数据判定。
            self.rtf_language = Self::ext_new_language(&head).into();
        } else {
            return Ok(false);
        }
        self.is_rtf_valid = true;
        Ok(true)
    }

    /// 新版名单的语言按数据行的国籍列判定：三字码 → English，含 CJK 汉字 → 简体中文。
    fn ext_new_language(head: &str) -> &'static str {
        for line in head.lines() {
            if RE_NEW.is_match(line) {
                // split 首段是行首 | 之前的空串，因此字段 i 在 split 结果中的下标是 i+1。
                let nat = line.split('|').nth(XN_PRIMARY_NAT + 1).map(str::trim).unwrap_or("");
                let has_cjk = nat.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
                return if has_cjk { "简体中文" } else { "English" };
            }
        }
        "English"
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
            if let Some(rec) = Self::parse_line(line, self.rtf_format, filter_newgen) {
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
    fn parse_line(line: &str, format: RtfFormat, filter_newgen: bool) -> Option<PlayerRecord> {
        // 零宽空格（FM 导出时混入的不可见字符）会切断中文字段，先整行清掉再匹配与切分。
        let line = line.replace(ZERO_WIDTH_SPACE, "");
        match format {
            RtfFormat::Classic => Self::parse_classic(&line, filter_newgen),
            RtfFormat::ExtNew => Self::parse_ext_new(&line, filter_newgen),
        }
    }

    /// 经典布局（基础/详细名单）：| UID | 国籍 | 二国籍 | 姓名 | 发长 | 发色 | 种族码 |。
    fn parse_classic(line: &str, filter_newgen: bool) -> Option<PlayerRecord> {
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

    /// 新版扩展布局（ext_new）：列序与经典完全不同——随机人标志在第 2 列、种族码在第 12 列，
    /// 没有体重与 Face 列（detailed 里对应字段留空），末尾的能力/潜力本模块不使用。
    fn parse_ext_new(line: &str, filter_newgen: bool) -> Option<PlayerRecord> {
        if !RE_NEW.is_match(line) {
            return None;
        }
        let mut fields: Vec<String> = line.split('|').map(|f| f.trim().to_string()).collect();
        if fields.len() < EXT_NEW_FIELD_COUNT + 2 {
            return None; // +2：行首行尾各多出一个空段
        }
        fields.pop();
        fields.remove(0);
        // UID 与种族码必须为纯数字，种族码 0-10（口径与经典布局一致）。
        let uid_raw = fields[XN_UID].as_str();
        if uid_raw.is_empty() || !uid_raw.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let eth = fields[XN_ETHNICITY].as_str();
        if eth.is_empty() || !eth.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        if eth.parse::<u32>().ok()? > 10 {
            return None;
        }
        let mut record = PlayerRecord {
            uid: format!("{NEWGAN_UID_PREFIX}{uid_raw}"),
            primary_nat: fields[XN_PRIMARY_NAT].clone(),
            sec_nat: fields[XN_SEC_NAT].clone(),
            name: fields[XN_NAME].clone(),
            hair_length: fields[XN_HAIR_LENGTH].clone(),
            hair_color: fields[XN_HAIR_COLOR].clone(),
            ethnicity_code: eth.to_string(),
            detailed: Some(DetailedInfo {
                skin_code: fields[XN_SKIN].clone(),
                face_id: String::new(), // 新版格式没有 Face 列
                club: fields[XN_CLUB].clone(),
                age: fields[XN_AGE].clone(),
                height: fields[XN_HEIGHT].clone(),
                weight: String::new(), // 新版格式没有体重列
                is_newgan: fields[XN_NEWGAN_FLAG].clone(),
            }),
        };
        // 随机人标志在第 2 列（经典布局在最后一列）：包含「否/No」则不是随机人，口径同经典。
        let flag = fields[XN_NEWGAN_FLAG].as_str();
        if NON_NEWGAN_FLAGS.iter().any(|f| flag.contains(f)) {
            record.uid = record
                .uid
                .strip_prefix(NEWGAN_UID_PREFIX)
                .unwrap_or(&record.uid)
                .to_string();
            if filter_newgen {
                return None;
            }
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

    // ---------------- 新版扩展布局（ext_new） ----------------

    /// 照抄真实 test_UntitledPlayer_ext_new 的表头与数据行形态（含零宽空格、"- -" 占位姓名）。
    const XN_SAMPLE: &str = "\
| 编\u{200b}号        | Newgen | 现\u{200b}状        |     | 姓\u{200b}名  | 国\u{200b}籍/地\u{200b}区籍 | 第\u{200b}二国\u{200b}籍/地\u{200b}区籍 | 年\u{200b}龄 | 身\u{200b}高 | 俱\u{200b}乐部 | 位\u{200b}置 |    |    |    |    | 能\u{200b}力 | 潜\u{200b}力 |
| ----------------------------------------------------------------|
| 2000256231 | 否      | 兴\u{200b}趣        |     | - -          | ESP      | MAR         | 24  | 181厘\u{200b}米 | -          |     | 2   | 10  | 1   | 15  | 199 | 200 |
| 2002245569 | 否      |                   |     | - -          | ENG      |             | 19  | 167厘米     | -          |     | 0   | 2   | 1   | 3   | 139 | 199 |
| 2002097028 | 是      | 业余              |     | Zhang Yu     | CHN      |             | 22  | 182厘米     | ShanX      | D (C) | 10  | 8   | 2   | 15  | 2   | 2   |
";

    #[test]
    fn validate_and_parse_ext_new() {
        let path = write_rtf(XN_SAMPLE);
        let mut p = RtfParser::new();
        assert!(p.check_rtf_valid(&path).unwrap());
        assert_eq!(p.rtf_format, RtfFormat::ExtNew);
        // 国籍列是三字码：即便表头是中文也无需翻译。
        assert_eq!(p.rtf_language, "English");
        // 过滤随机人：只剩「是」那行。
        let data = p.parse_rtf(&path, true, &HashMap::new()).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-2002097028");
        assert_eq!(data[0].primary_nat, "CHN");
        assert_eq!(data[0].ethnicity_code, "10");
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(d.club, "ShanX");
        assert_eq!(d.age, "22");
        assert_eq!(d.height, "182厘米");
        assert_eq!(d.is_newgan, "是");
        // 不过滤：3 行全保留，「否」行无 r- 前缀，列映射按新布局取值。
        let data = p.parse_rtf(&path, false, &HashMap::new()).unwrap();
        assert_eq!(data.len(), 3);
        let first = &data[0];
        assert_eq!(first.uid, "2000256231"); // 否（非随机人）：无前缀
        assert_eq!(first.sec_nat, "MAR");
        assert_eq!(first.name, "- -");
        assert_eq!(first.hair_length, "1");    // 发长在姓名后第 4 个无名列（经典布局是第 1 个）
        assert_eq!(first.hair_color, "15");
        assert_eq!(first.ethnicity_code, "2"); // 种族码在姓名后第 7 列（经典布局是第 4 列）
        let d = first.detailed.as_ref().unwrap();
        assert_eq!(d.skin_code, "10");
        assert_eq!(d.is_newgan, "否");
        assert_eq!(d.weight, "");              // 新版格式没有体重列
    }

    #[test]
    fn ext_new_chinese_nationalities_translated() {
        // 新版名单 + 中文全名国籍（游戏界面为中文时的另一种导出形态）→ 需走翻译表。
        let sample = "\
| 2002075172 | 是 | 兴趣 |  | 扎克·布劳顿 | 美国 |  | 20 | 180厘米 | - |  | 3 | 5 | 1 | 9 | 100 | 120 |
";
        let path = write_rtf(sample);
        let mut p = RtfParser::new();
        assert!(p.check_rtf_valid(&path).unwrap());
        assert_eq!(p.rtf_format, RtfFormat::ExtNew);
        assert_eq!(p.rtf_language, "简体中文");
        let mut tr = HashMap::new();
        tr.insert("简体中文".to_string(), {
            let mut m = HashMap::new();
            m.insert("美国".to_string(), "USA".to_string());
            m
        });
        let data = p.parse_rtf(&path, true, &tr).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-2002075172");
        assert_eq!(data[0].primary_nat, "USA");
    }

    // ---------------- 真实导出样本（tests/testing_data） ----------------

    /// 真实导出样本路径（仓库根的 tests/testing_data）；文件缺失时跳过对应断言。
    fn real_rtf(name: &str) -> Option<std::path::PathBuf> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/testing_data").join(name);
        p.is_file().then_some(p)
    }

    #[test]
    fn real_test_files_recognized() {
        // 三种真实导出格式都要能被 check_rtf_valid 自动识别。
        let mut p = RtfParser::new();
        if let Some(f) = real_rtf("test_UntitledPlayer_500.rtf") {
            assert!(p.check_rtf_valid(&f).unwrap());
            assert_eq!(p.rtf_format, RtfFormat::Classic);
            assert_eq!(p.rtf_language, "English"); // 表头中文但国籍是三字码
        }
        if let Some(f) = real_rtf("test_UntitledPlayer_ext.rtf") {
            assert!(p.check_rtf_valid(&f).unwrap());
            assert_eq!(p.rtf_format, RtfFormat::Classic);
            assert_eq!(p.rtf_language, "English");
        }
        if let Some(f) = real_rtf("test_UntitledPlayer_ext_new.rtf") {
            assert!(p.check_rtf_valid(&f).unwrap());
            assert_eq!(p.rtf_format, RtfFormat::ExtNew);
            assert_eq!(p.rtf_language, "English");
        }
    }

    #[test]
    fn real_ext_new_full_parse_smoke() {
        // 冒烟：真实 ext_new 名单（73MB、15.8 万行）整份解析，验证列映射在大文件上成立。
        let Some(f) = real_rtf("test_UntitledPlayer_ext_new.rtf") else { return; };
        let mut p = RtfParser::new();
        assert!(p.check_rtf_valid(&f).unwrap());
        let data = p.parse_rtf(&f, false, &HashMap::new()).unwrap();
        assert!(data.len() >= 150_000, "应解析出全部数据行，实际 {}", data.len());
        let newgan = data.iter().filter(|r| r.uid.starts_with(NEWGAN_UID_PREFIX)).count();
        assert!(newgan > 0 && newgan < data.len(), "随机人/非随机人应同时存在：{newgan}/{}", data.len());
        let first = &data[0];
        assert_eq!(first.uid, "2000256231");
        assert_eq!(first.primary_nat, "ESP");
        assert_eq!(first.sec_nat, "MAR");
        assert_eq!(first.ethnicity_code, "2");
        assert_eq!(first.hair_length, "1");
        assert_eq!(first.hair_color, "15");
        let d = first.detailed.as_ref().unwrap();
        assert_eq!(d.skin_code, "10");
        assert_eq!(d.age, "24");
        assert_eq!(d.height, "181厘米");
        assert_eq!(d.is_newgan, "否");
        assert_eq!(d.weight, "");
    }
}
