//! 【新手导读】RTF 名单扫描器：判别 FM 导出名单的格式，并把每行全部字段解析入库（缓存）。
//! FM（Football Manager）可把自定义视图「另存为 RTF」，得到用竖线 | 分隔列的表格文本。
//!
//! 【六种格式 = 三种列结构 × 两种数据语言】
//!   Basic     基础名单（球员/职员通用，7 列）——导出自「is newgen」过滤视图，全员随机人；
//!   ExtPlayer 拓展球员（≥17 列，第 17 列 = R.C.A，数字）；
//!   ExtStaff  拓展职员（≥18 列，第 3 列 = 头像，第 17 列 = R.C.A，第 18 列 = 性别）。
//! 语言按数据行的国籍列判定：含汉字 → 简体中文，否则 → English。
//! 国籍有三种形态：三位大写码（short）、英文全名（long，如 "U.S.A."）、中文国名/截断别名；
//! 解析后经翻译表（nat_translation.json，按语言查节点）统一归一化为三字码。
//!
//! 【判别不读表头】表头/分隔线/空行被「第 0 列须为 4-10 位纯数字 UID」天然排除，
//! 只看数据行的列结构：第 1 列是 Newgen 标志 → 拓展，否则 → 基础；
//! 拓展内部球员/职员前 17 列同构，职员多第 18 列性别——第 18 列是性别词
//! （男/女/Male/Female）即职员，否则是球员（其第 18 列起为用户自定义列）。
//! 旧版布局（14 列详细、性别第 3 列的旧职员拓展）不是合法格式，整体判「RTF 格式不合法」。
use crate::logger::LogSink;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub const NEWGAN_UID_PREFIX: &str = "r-"; // 随机人（NewGAN）UID 的固定前缀
const ZERO_WIDTH_SPACE: &str = "\u{200b}"; // 零宽空格（FM 导出时可能混入的不可见字符）
const HEAD_BYTES: u64 = 64 * 1024; // 格式识别最多读入的字节数（大名单可达上百 MB）
const DETECT_LINE_LIMIT: usize = 20; // 格式识别只看前 20 行（足够投票，避免读全文）
const MISS_LOG_MAX: usize = 50; // 翻译缺失逐条记日志的上限，超出合并汇总（避免刷屏）

/// Newgen 标志的合法取值（检测时第 1 列必须完全等于其中之一）。
const NEWGEN_FLAG_VALUES: [&str; 4] = ["是", "否", "Yes", "No"];
/// 标志含这些词 → 随机人（UID 加 r- 前缀）。
const NEWGAN_FLAGS: [&str; 2] = ["是", "Yes"];
/// 第 18 列性别词的合法取值（拓展职员的判别依据；英文不区分大小写）。
const GENDER_VALUES: [&str; 4] = ["男", "女", "male", "female"];

const BASIC_FIELD_COUNT: usize = 7; // 基础名单：UID/国籍/二国籍/姓名/发长/发色/种族码
const EXT_FIELD_COUNT: usize = 17; // 拓展球员：17 个标准列（其后为用户自定义列，照单入库）
const EXT_STAFF_FIELD_COUNT: usize = 18; // 拓展职员：比球员多第 18 列（性别）

// 数值码列的合法区间（实测口径；越界行按非法行跳过）。
const ETHNICITY_RANGE: (u32, u32) = (0, 10);
const SKIN_RANGE: (u32, u32) = (0, 19);
const HAIR_LENGTH_RANGE: (u32, u32) = (0, 5);
const HAIR_COLOR_RANGE: (u32, u32) = (1, 19);

// 基础名单列下标。
const BASIC_UID_COL: usize = 0;
const BASIC_NAT_COL: usize = 1;
const BASIC_SEC_NAT_COL: usize = 2;
const BASIC_NAME_COL: usize = 3;
const BASIC_HAIR_LENGTH_COL: usize = 4;
const BASIC_HAIR_COLOR_COL: usize = 5;
const BASIC_ETHNICITY_COL: usize = 6;

// 拓展名单列下标（球员/职员共用前 17 列，职员多第 18 列性别）。
const EXT_UID_COL: usize = 0;
const EXT_NEWGAN_FLAG_COL: usize = 1;
const EXT_AVATAR_COL: usize = 2; // 头像（FM 图片列，导出为空文本；原文照单入库）
const EXT_NAME_COL: usize = 3;
const EXT_NAT_COL: usize = 4;
const EXT_SEC_NAT_COL: usize = 5;
const EXT_AGE_COL: usize = 6;
const EXT_HEIGHT_COL: usize = 7;
const EXT_CLUB_COL: usize = 8;
const EXT_NINE_COL: usize = 9; // 球员 = 俱乐部赛事级别；职员 = 偏好工作
const EXT_ETHNICITY_COL: usize = 10;
const EXT_SKIN_COL: usize = 11;
const EXT_HAIR_LENGTH_COL: usize = 12;
const EXT_HAIR_COLOR_COL: usize = 13;
const EXT_CA_COL: usize = 14;
const EXT_PA_COL: usize = 15;
const EXT_RCA_COL: usize = 16; // R.C.A（球员/职员都在第 17 列，数字）
const EXT_GENDER_COL: usize = 17; // 性别（第 18 列，仅职员）

/// RTF 名单的列结构。语言不在这里（按国籍列数据另行判定），见 `RtfScanner::language`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RtfFormat {
    /// 基础名单：| UID | 国籍 | 二国籍 | 姓名 | 发长 | 发色 | 种族码 |（球员/职员通用）。
    #[default]
    Basic,
    /// 拓展球员：… | 俱乐部 | 赛事级别 | 种族码 | 肤色 | 发长 | 发色 | CA | PA | R.C.A |。
    ExtPlayer,
    /// 拓展职员：… | 俱乐部 | 偏好工作 | 种族码 | 肤色 | 发长 | 发色 | CA | PA | R.C.A | 性别 |。
    ExtStaff,
}

const FORMATS: [RtfFormat; 3] = [RtfFormat::Basic, RtfFormat::ExtPlayer, RtfFormat::ExtStaff];

impl RtfFormat {
    /// 中文标签（日志/摘要用）。
    pub fn label(self) -> &'static str {
        match self {
            RtfFormat::Basic => "基础名单",
            RtfFormat::ExtPlayer => "拓展球员名单",
            RtfFormat::ExtStaff => "拓展职员名单",
        }
    }
    /// 该格式的国籍列下标（语言判定与翻译都看它）。
    fn nat_col(self) -> usize {
        match self {
            RtfFormat::Basic => BASIC_NAT_COL,
            _ => EXT_NAT_COL,
        }
    }
}

/// 拓展名单（球员/职员）才有的额外信息，承载第 3-18 列及用户自定义列的全部原文。
/// 【瘦身说明】新版导出都没有体重（weight）与 Face 列，字段已删除。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetailedInfo {
    pub skin_code: String,         // 肤色码（第 12 列）
    pub avatar: String,            // 头像列原文（第 3 列；FM 图片列，导出为空文本）
    pub is_newgan: String,         // Newgen 标志原文（是/否/Yes/No）
    pub club: String,              // 俱乐部（第 9 列）
    pub age: String,               // 年龄（第 7 列）
    pub height: String,            // 身高（第 8 列）
    pub based: String,             // 俱乐部赛事级别/所在地（第 10 列，仅球员）
    pub preferred_job: String,     // 偏好工作（第 10 列，仅职员）
    pub ca: String,                // 当前能力 CA（第 15 列）
    pub pa: String,                // 潜力 PA（第 16 列）
    pub rca: String,               // 推荐能力 R.C.A（第 17 列，球员/职员都有）
    pub gender: String,            // 性别（第 18 列，仅职员）
    pub extra_fields: Vec<String>, // 用户自定义列原文（球员第 18 列起 / 职员第 19 列起）
}

/// 一条名单记录（球员或职员）。这是整个模块输出的核心数据结构。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRecord {
    pub uid: String,         // 随机人带 r- 前缀
    pub primary_nat: String, // 解析后统一为英文三字码（翻译缺失时保留原值）
    pub sec_nat: String,
    pub name: String,
    pub hair_length: String,
    pub hair_color: String,
    pub ethnicity_code: String, // "0".."10"
    // Option 表示「可能有也可能没有」；拓展名单才有 detailed。
    // skip_serializing_if 让前端在无详细数据时不会看到这个字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detailed: Option<DetailedInfo>,
}

/// 单行判定的结论：
/// - Data：命中某种格式（可投票、可解析）；
/// - NotDataRow：表头/分隔线/空行等（第 0 列不是纯数字）；
/// - Rejected：像数据行但有字段不合法（带字段定位，用于「RTF 格式不合法」报错）。
enum RowVerdict {
    Data(RtfFormat),
    NotDataRow,
    Rejected { field: &'static str, detail: String },
}

/// 扫描器本体：记录当前 RTF 是否合法、数据语言、属于哪种列结构。
pub struct RtfScanner {
    pub is_valid: bool,
    pub language: String, // "English" / "简体中文" / "Unknown"（未识别时）
    pub format: RtfFormat,
}

impl RtfScanner {
    /// 创建一个新扫描器，默认不合法、语言未知、基础布局。
    pub fn new() -> Self {
        Self {
            is_valid: false,
            language: "Unknown".into(),
            format: RtfFormat::default(),
        }
    }

    /// 读文件头部判别格式与语言（detect = 扫描识别）。
    /// 【为何只读头部】名单可能上百 MB，但判别只需看开头几行；
    /// 最多读 HEAD_BYTES 字节、取前 DETECT_LINE_LIMIT 行，对每条数据行跑判别树并投票。
    /// 成功 → Ok(())；识别不出任何格式 → Err("RTF 格式不合法：第 N 行「字段」…实际「值」")。
    pub fn detect_format(&mut self, path: &Path) -> Result<(), String> {
        self.is_valid = false;
        self.language = "Unknown".into();
        self.format = RtfFormat::default();
        let text = read_head(path, HEAD_BYTES)?;

        let mut votes = [0usize; 3]; // 各格式的命中行数
        let mut languages: [Option<&'static str>; 3] = [None; 3]; // 各格式首个命中行的语言
        let mut first_rejection: Option<String> = None; // 首条不合法行（全部失败时用于报错定位）

        for (idx, line) in text.lines().take(DETECT_LINE_LIMIT).enumerate() {
            let cells = split_row_cells(line);
            match classify_row(&cells) {
                RowVerdict::NotDataRow => {}
                RowVerdict::Rejected { field, detail } => {
                    if first_rejection.is_none() {
                        first_rejection = Some(format!("第 {} 行「{field}」{detail}", idx + 1));
                    }
                }
                RowVerdict::Data(fmt) => {
                    let slot = fmt as usize;
                    votes[slot] += 1;
                    if languages[slot].is_none() {
                        let nat = cells.get(fmt.nat_col()).map(String::as_str).unwrap_or("");
                        languages[slot] = Some(if contains_cjk(nat) {
                            "简体中文"
                        } else {
                            "English"
                        });
                    }
                }
            }
        }

        // 取命中最多的格式（平票取先出现的；三格式互斥，正常不会平票）。
        let mut best: Option<usize> = None;
        for i in 0..FORMATS.len() {
            if votes[i] > 0 && best.map_or(true, |b| votes[i] > votes[b]) {
                best = Some(i);
            }
        }
        let Some(best) = best else {
            return Err(match first_rejection {
                Some(where_) => format!("RTF 格式不合法：{where_}"),
                None => "RTF 格式不合法：未识别到合法的数据行".into(),
            });
        };
        self.format = FORMATS[best];
        self.language = languages[best].unwrap_or("English").into();
        self.is_valid = true;
        Ok(())
    }

    /// 逐行解析整份名单（须先 detect_format 通过），返回全字段入库的记录。
    /// keep_only_newgen：仅按 UID 字段筛选——只保留带 r- 前缀（随机人）的记录。
    /// translations：多语言国籍翻译表（语言 → 显示文本 → 三字码）；当前语言查不到的值保留原值并记警告日志。
    pub fn parse_roster(
        &self,
        path: &Path,
        keep_only_newgen: bool,
        translations: &HashMap<String, HashMap<String, String>>,
        logger: &impl LogSink,
    ) -> Result<Vec<PlayerRecord>, String> {
        // 防御性校验：未先调用 detect_format 就直接解析时报错。
        if !self.is_valid {
            return Err("RTF 格式无效，请先执行格式识别".into());
        }
        let text = fs::read_to_string(path).map_err(|e| format!("读取 RTF 失败: {e}"))?;
        let mut data = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue; // 跳过空行
            }
            // 能按当前格式解析出行就入库（表头/分隔线/坏行/异格式行返回 None 被跳过）。
            if let Some(record) = self.parse_line(line, keep_only_newgen) {
                data.push(record);
            }
        }
        // 国籍归一化：非三字码的值（中文国名/截断别名、英文全名）查当前语言的翻译节点转码，
        // 查不到保留原值，按值去重记警告日志。
        let misses = translate_nationalities(&mut data, translations, &self.language);
        log_translation_misses(&misses, logger);
        Ok(data)
    }

    /// 解析单行。返回 Some(记录) 表示成功；None 表示该行不是当前格式的合法数据行（跳过）。
    fn parse_line(&self, line: &str, keep_only_newgen: bool) -> Option<PlayerRecord> {
        let cells = split_row_cells(line);
        let format = match classify_row(&cells) {
            RowVerdict::Data(format) if format == self.format => format,
            _ => return None,
        };
        let record = match format {
            RtfFormat::Basic => parse_basic_row(&cells),
            RtfFormat::ExtPlayer => parse_ext_row(&cells, RtfFormat::ExtPlayer),
            RtfFormat::ExtStaff => parse_ext_row(&cells, RtfFormat::ExtStaff),
        };
        // 过滤只看 UID 字段：带 r- 前缀的是随机人。
        (record.uid.starts_with(NEWGAN_UID_PREFIX) || !keep_only_newgen).then_some(record)
    }
}

// ---------------- 行管线：清洗、切分、判别 ----------------

/// 只读文件前 `max` 字节并按 UTF-8 解码（截断处残缺的多字节字符容忍处理）。
fn read_head(path: &Path, max: u64) -> Result<String, String> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|e| format!("读取 RTF 失败: {e}"))?;
    let mut buf = Vec::new();
    file.take(max)
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取 RTF 失败: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// 清掉 FM 导出时混入的零宽空格（会切断中文字段）。
fn strip_zero_width_chars(line: &str) -> String {
    line.replace(ZERO_WIDTH_SPACE, "")
}

/// 按 | 切分一行并 trim 每列；行首行尾各有一个空段（行以 | 开头结尾），去掉。
fn split_row_cells(line: &str) -> Vec<String> {
    let mut cells: Vec<String> = strip_zero_width_chars(line)
        .split('|')
        .map(|c| c.trim().to_string())
        .collect();
    if cells.len() >= 2 {
        cells.pop();
        cells.remove(0);
    }
    cells
}

/// 单行判别树（检测与解析共用，单一真源）：
/// ① 第 0 列必须是 4-10 位纯数字 UID（表头/分隔线/空行 → NotDataRow）；
/// ② 第 1 列是 Newgen 标志 → 拓展路径，否则 → 基础路径；
/// ③ 拓展路径依次校验列数、国籍、四个数值码与第 17 列 R.C.A，最后按第 18 列性别词分球员/职员；
///    基础路径要求恰好 7 有效列（第 8 列起非空 = 旧版详细名单 → 拒）。
fn classify_row(cells: &[String]) -> RowVerdict {
    let Some(uid) = cells.first() else {
        return RowVerdict::NotDataRow;
    };
    if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
        return RowVerdict::NotDataRow; // 表头（文字列）、分隔线（---）等非数据行
    }
    if !is_valid_uid(uid) {
        return RowVerdict::Rejected {
            field: "编号(UID)",
            detail: format!("应为 4-10 位数字，实际「{uid}」"),
        };
    }
    let flag = cells
        .get(EXT_NEWGAN_FLAG_COL)
        .map(String::as_str)
        .unwrap_or("");
    if NEWGEN_FLAG_VALUES.contains(&flag) {
        classify_ext_row(cells)
    } else {
        classify_basic_row(cells)
    }
}

/// 基础名单（7 列）判别：列数、第 8 列起仅允许空列、国籍形态、三个数值码。
fn classify_basic_row(cells: &[String]) -> RowVerdict {
    if cells.len() < BASIC_FIELD_COUNT {
        return RowVerdict::Rejected {
            field: "列数",
            detail: format!(
                "基础名单应为 {BASIC_FIELD_COUNT} 列，实际 {} 列",
                cells.len()
            ),
        };
    }
    // 第 8 列起出现非空列：旧版 14 列详细名单等已不支持的格式。
    if cells[BASIC_FIELD_COUNT..].iter().any(|c| !c.is_empty()) {
        return RowVerdict::Rejected {
            field: "第 8 列起",
            detail: "存在非空数据（旧版 14 列详细名单已不再支持，请用新视图重新导出）".into(),
        };
    }
    if let Some(v) = invalid_nationality(cells, BASIC_NAT_COL, "国籍", false) {
        return v;
    }
    if let Some(v) = invalid_nationality(cells, BASIC_SEC_NAT_COL, "第二国籍", true) {
        return v;
    }
    if let Some(v) = invalid_code(
        cells,
        BASIC_HAIR_LENGTH_COL,
        "头发长度码",
        HAIR_LENGTH_RANGE,
    ) {
        return v;
    }
    if let Some(v) = invalid_code(cells, BASIC_HAIR_COLOR_COL, "头发颜色码", HAIR_COLOR_RANGE)
    {
        return v;
    }
    if let Some(v) = invalid_code(cells, BASIC_ETHNICITY_COL, "种族码", ETHNICITY_RANGE) {
        return v;
    }
    RowVerdict::Data(RtfFormat::Basic)
}

/// 拓展名单判别：列数、国籍、四个数值码、第 17 列 R.C.A，最后按第 18 列性别词分球员/职员。
/// 【为何没有第 3 列锚点】职员第 3 列是头像（FM 图片列，导出为空文本），不作判别依据；
/// 旧职员布局（性别第 3 列）的第 10 列是偏好工作文本，会被「种族码」数值校验自然拦下。
fn classify_ext_row(cells: &[String]) -> RowVerdict {
    if cells.len() < EXT_FIELD_COUNT {
        return RowVerdict::Rejected {
            field: "列数",
            detail: format!(
                "拓展名单应至少 {EXT_FIELD_COUNT} 列，实际 {} 列",
                cells.len()
            ),
        };
    }
    if let Some(v) = invalid_nationality(cells, EXT_NAT_COL, "国籍", false) {
        return v;
    }
    if let Some(v) = invalid_nationality(cells, EXT_SEC_NAT_COL, "第二国籍", true) {
        return v;
    }
    if let Some(v) = invalid_code(cells, EXT_ETHNICITY_COL, "种族码", ETHNICITY_RANGE) {
        return v;
    }
    if let Some(v) = invalid_code(cells, EXT_SKIN_COL, "肤色码", SKIN_RANGE) {
        return v;
    }
    if let Some(v) = invalid_code(cells, EXT_HAIR_LENGTH_COL, "头发长度码", HAIR_LENGTH_RANGE)
    {
        return v;
    }
    if let Some(v) = invalid_code(cells, EXT_HAIR_COLOR_COL, "头发颜色码", HAIR_COLOR_RANGE) {
        return v;
    }
    // 第 17 列 R.C.A：球员/职员都是数字（结构锚点，不做区间校验）。
    let rca = cells[EXT_RCA_COL].as_str();
    if rca.is_empty() || !rca.bytes().all(|b| b.is_ascii_digit()) {
        return RowVerdict::Rejected {
            field: "第 17 列",
            detail: format!("R.C.A 应为数字，实际「{rca}」"),
        };
    }
    // 第 18 列是拓展球员/职员唯一的结构差异：职员在此放性别词，
    // 球员的第 18 列起是用户自定义列（性别词精确匹配，避免普通自定义文本被误判成职员）。
    if cells.len() >= EXT_STAFF_FIELD_COUNT && is_gender_value(&cells[EXT_GENDER_COL]) {
        RowVerdict::Data(RtfFormat::ExtStaff)
    } else {
        RowVerdict::Data(RtfFormat::ExtPlayer)
    }
}

/// 国籍列校验：不合法返回 Rejected（带列号与实际值），合法返回 None。
fn invalid_nationality(
    cells: &[String],
    col: usize,
    field: &'static str,
    allow_empty: bool,
) -> Option<RowVerdict> {
    let value = cells.get(col).map(String::as_str).unwrap_or("");
    if is_valid_nationality(value, allow_empty) {
        return None;
    }
    Some(RowVerdict::Rejected {
        field,
        detail: format!(
            "第 {} 列应为三位大写国籍码、国籍英文名或中文国名，实际「{value}」",
            col + 1
        ),
    })
}

/// 数值码列校验：不合法返回 Rejected（带列号与实际值），合法返回 None。
fn invalid_code(
    cells: &[String],
    col: usize,
    field: &'static str,
    (min, max): (u32, u32),
) -> Option<RowVerdict> {
    let value = cells.get(col).map(String::as_str).unwrap_or("");
    if is_number_in_range(value, min, max) {
        return None;
    }
    Some(RowVerdict::Rejected {
        field,
        detail: format!("第 {} 列应为 {min}-{max} 的数字，实际「{value}」", col + 1),
    })
}

// ---------------- 谓词（统一 is_* 风格） ----------------

/// 文本是否含汉字（国籍列中文形态的依据；也用于语言判定）。
fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// UID：4-10 位纯数字（实测 FM UID 的位数范围；11 位以上视为溢出脏数据）。
fn is_valid_uid(uid: &str) -> bool {
    (4..=10).contains(&uid.chars().count()) && uid.bytes().all(|b| b.is_ascii_digit())
}

/// 数值码：非空、纯数字、落在 [min, max]（parse::<u32> 失败的超长串也算不合法）。
fn is_number_in_range(value: &str, min: u32, max: u32) -> bool {
    !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value
            .parse::<u32>()
            .map(|n| (min..=max).contains(&n))
            .unwrap_or(false)
}

/// 三位大写英文国籍码（如 ESP）。
fn is_three_letter_code(nat: &str) -> bool {
    nat.len() == 3 && nat.bytes().all(|b| b.is_ascii_uppercase())
}

/// 国籍英文全名（long 格式）：以字母开头、至少 2 个字符、不含数字，
/// 可含空格与 & . - ' ( ) 等连接符及变音字母（如 "U.S.A."、"Türkiye"、"Hong Kong (China PR)"）。
/// 【为何宽松】只做形态判定不查表：查不到的值走「保留原值 + 警告」兜底，不拒行。
fn is_long_form_name(nat: &str) -> bool {
    nat.chars().next().is_some_and(|c| c.is_alphabetic())
        && nat.chars().count() >= 2
        && nat.chars().all(|c| {
            c.is_alphabetic() || matches!(c, ' ' | '&' | '.' | '-' | '\'' | '’' | '(' | ')')
        })
}

/// 国籍形态校验：三位大写码、英文全名或含汉字（纯形态，不查翻译表——
/// 截断中文名、未收录全名都能通过；翻译查不到时保留原值并记警告日志）。
fn is_valid_nationality(nat: &str, allow_empty: bool) -> bool {
    if nat.is_empty() {
        return allow_empty;
    }
    is_three_letter_code(nat) || contains_cjk(nat) || is_long_form_name(nat)
}

/// Newgen 标志是否为随机人：含 是/Yes → 是（正向判定，UID 加 r- 前缀）。
fn is_newgan(flag: &str) -> bool {
    NEWGAN_FLAGS.iter().any(|f| flag.contains(f))
}

/// 第 18 列是否为性别词（拓展职员判别；英文不区分大小写）。
fn is_gender_value(value: &str) -> bool {
    GENDER_VALUES.contains(&value.to_ascii_lowercase().as_str())
}

// ---------------- 行解析（classify 通过后取列组装，不再重复校验） ----------------

/// 基础名单行：7 列全员随机人（导出自 is newgen 过滤视图），UID 统一加 r- 前缀。
fn parse_basic_row(cells: &[String]) -> PlayerRecord {
    PlayerRecord {
        uid: format!("{NEWGAN_UID_PREFIX}{}", cells[BASIC_UID_COL]),
        primary_nat: cells[BASIC_NAT_COL].clone(),
        sec_nat: cells[BASIC_SEC_NAT_COL].clone(),
        name: cells[BASIC_NAME_COL].clone(),
        hair_length: cells[BASIC_HAIR_LENGTH_COL].clone(),
        hair_color: cells[BASIC_HAIR_COLOR_COL].clone(),
        ethnicity_code: cells[BASIC_ETHNICITY_COL].clone(),
        detailed: None,
    }
}

/// 拓展名单行（球员/职员前 17 列同构，仅第 10/18 列语义不同），全字段入库。
fn parse_ext_row(cells: &[String], format: RtfFormat) -> PlayerRecord {
    let is_player = format == RtfFormat::ExtPlayer;
    // 随机人正向判定：标志含 是/Yes → 加 r- 前缀；否/No → 保留裸 UID。
    let flag = cells[EXT_NEWGAN_FLAG_COL].as_str();
    let uid = if is_newgan(flag) {
        format!("{NEWGAN_UID_PREFIX}{}", cells[EXT_UID_COL])
    } else {
        cells[EXT_UID_COL].clone()
    };
    PlayerRecord {
        uid,
        primary_nat: cells[EXT_NAT_COL].clone(),
        sec_nat: cells[EXT_SEC_NAT_COL].clone(),
        name: cells[EXT_NAME_COL].clone(),
        hair_length: cells[EXT_HAIR_LENGTH_COL].clone(),
        hair_color: cells[EXT_HAIR_COLOR_COL].clone(),
        ethnicity_code: cells[EXT_ETHNICITY_COL].clone(),
        detailed: Some(DetailedInfo {
            skin_code: cells[EXT_SKIN_COL].clone(),
            avatar: cells[EXT_AVATAR_COL].clone(),
            is_newgan: flag.to_string(),
            club: cells[EXT_CLUB_COL].clone(),
            age: cells[EXT_AGE_COL].clone(),
            height: cells[EXT_HEIGHT_COL].clone(),
            based: if is_player {
                cells[EXT_NINE_COL].clone()
            } else {
                String::new()
            },
            preferred_job: if is_player {
                String::new()
            } else {
                cells[EXT_NINE_COL].clone()
            },
            ca: cells[EXT_CA_COL].clone(),
            pa: cells[EXT_PA_COL].clone(),
            rca: cells[EXT_RCA_COL].clone(),
            gender: if is_player {
                String::new()
            } else {
                cells[EXT_GENDER_COL].clone()
            },
            // 用户自定义列：球员从第 18 列起，职员从第 19 列起。
            extra_fields: if is_player {
                cells[EXT_FIELD_COUNT..].to_vec()
            } else {
                cells[EXT_STAFF_FIELD_COUNT..].to_vec()
            },
        }),
    }
}

// ---------------- 国籍翻译 ----------------

/// 国籍显示文本 → 三字码（就地改写）。三字码（short 格式）已是目标形态直接跳过；
/// 查不到保留原值，返回缺失值与出现次数（按行数降序）。
fn translate_nationalities(
    data: &mut [PlayerRecord],
    translations: &HashMap<String, HashMap<String, String>>,
    language: &str,
) -> Vec<(String, usize)> {
    let empty = HashMap::new();
    let map = translations.get(language).unwrap_or(&empty);
    let mut misses: HashMap<String, usize> = HashMap::new();
    for record in data.iter_mut() {
        // 主国籍与第二国籍分别查表；空值与三字码跳过。
        for nat in [&mut record.primary_nat, &mut record.sec_nat] {
            if nat.is_empty() || is_three_letter_code(nat) {
                continue;
            }
            match map.get(nat.as_str()) {
                Some(code) => *nat = code.clone(),
                None => *misses.entry(nat.clone()).or_insert(0) += 1,
            }
        }
    }
    let mut misses: Vec<(String, usize)> = misses.into_iter().collect();
    misses.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    misses
}

/// 翻译缺失 → 警告日志：逐值一条（按行数降序），种类过多时合并汇总，避免大名单刷屏。
fn log_translation_misses(misses: &[(String, usize)], logger: &impl LogSink) {
    for (value, count) in misses.iter().take(MISS_LOG_MAX) {
        logger.write(
            log::Level::Warn,
            "rtf",
            format!("国籍「{value}」不在翻译表，保留原值（{count} 行）"),
        );
    }
    if misses.len() > MISS_LOG_MAX {
        let rest: usize = misses[MISS_LOG_MAX..].iter().map(|(_, c)| c).sum();
        logger.write(
            log::Level::Warn,
            "rtf",
            format!(
                "另有 {} 种国籍不在翻译表，保留原值（共 {rest} 行）",
                misses.len() - MISS_LOG_MAX
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::NAT_TRANSLATION;
    use crate::logger::{ChannelLogger, LogLine};
    use std::io::Write;

    fn write_rtf(content: &str) -> std::path::PathBuf {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.keep().unwrap().1
    }

    /// 真实导出样本路径（仓库根的 tests/testing_data）；文件缺失时跳过对应断言（CI 上天然跳过）。
    fn real_rtf(name: &str) -> Option<std::path::PathBuf> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/testing_data")
            .join(name);
        p.is_file().then_some(p)
    }

    /// 收集测试日志出口里的 warn 消息。
    fn warn_messages(rx: &std::sync::mpsc::Receiver<LogLine>) -> Vec<String> {
        rx.try_iter()
            .filter(|l| l.level == "WARNING")
            .map(|l| l.message)
            .collect()
    }

    // ---------------- 合成样本（CI 可跑） ----------------

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

    /// 拓展球员（新布局）：第 17 列 R.C.A 为数字。
    const EXT_PLAYER_SAMPLE: &str = "\
| UID        | Newgen |     | Name          | Nat | 2nd Nat | Age | Height | Club       | Based                            |     |     |     |     | CA | PA  | R.C.A |
| 12315      | No     |     | Toni Micevski | MKD |         | 54  | 188 cm | Poeševo    | North Macedonia (Fourth League)  | 0   | 5   | 1   | 12  | 27 | 122 | 27    |
| 103607     | Yes    |     | James Henry   | ENG | SCO     | 35  | 185 cm | Shrewsbury | England (Sky Bet League One)     | 0   | 4   | 1   | 7   | 79 | 122 | 78    |
";

    /// 拓展职员（新布局，18 列）：第 3 列头像（FM 图片列导出为空）、第 17 列 R.C.A、第 18 列性别。
    const EXT_STAFF_CN_SAMPLE: &str = "\
| 编号        | Newgen | 头像 | 姓名   | 国籍/地区籍 | 第二国籍/地区籍 | 年龄 | 身高    | 俱乐部    | 偏好工作  |     |     |     |     | 能力 | 潜力 | R.C.A | 性别 |
| 2002243829 | 是     |      | 王晨   | 中国        |                | 62   | 182厘米 |           | 足球总监  | 10  | 8   | 1   | 16  | 144  | 199  | 140   | 男   |
| 306951      | 否     |      | 库卡   | 巴西        |                | 68   | 182厘米 | 梅州客家  | 足球总监  | 0   | 8   | 1   | 11  | 142  | 160  | 138   | 男   |
";

    const EXT_STAFF_EN_SAMPLE: &str = "\
| UID      | Newgen | Avatar | Name      | Nat | 2nd Nat | Age | Height | Club | Preferred Job       |     |     |     |     | CA  | PA  | R.C.A | Gender |
| 67214325 | No     |        | 亚由美    | KOR | ENG     | 26  | 167 cm |      | Performance Analyst | 10  | 8   | 1   | 16  | 200 | 200 | 195   | Female |
";

    #[test]
    fn detect_basic_formats() {
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(EN_SAMPLE)).unwrap();
        assert_eq!(s.format, RtfFormat::Basic);
        assert_eq!(s.language, "English");
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(CN_SAMPLE)).unwrap();
        assert_eq!(s.format, RtfFormat::Basic);
        assert_eq!(s.language, "简体中文");
        // 乱码：没有数据行 → 报「RTF 格式不合法」并说明未识别到数据行。
        let err = RtfScanner::new()
            .detect_format(&write_rtf("乱七八糟没有数据行"))
            .unwrap_err();
        assert!(
            err.contains("RTF 格式不合法") && err.contains("未识别到合法的数据行"),
            "{err}"
        );
    }

    #[test]
    fn parse_basic_english() {
        let path = write_rtf(EN_SAMPLE);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        let logger = ChannelLogger::channel().0;
        // 基础名单全员随机人：keep_only_newgen 过滤不掉任何行。
        let data = s
            .parse_roster(&path, true, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].uid, "r-2000472008");
        assert_eq!(data[0].primary_nat, "ESP");
        assert_eq!(data[0].sec_nat, "USA");
        assert_eq!(data[0].name, "Pepe Sáenz");
        assert_eq!(data[0].hair_length, "1");
        assert_eq!(data[0].hair_color, "12");
        assert_eq!(data[0].ethnicity_code, "1");
        assert!(data[0].detailed.is_none());
    }

    #[test]
    fn basic_rejects_legacy_detailed() {
        // 旧版 14 列详细名单（第 8 列起非空）→ 整份判不合法并给出定位。
        let sample = "| 2001234567| ESP  | USA  | Newgen Kid | 1 | 12 | 1 | 2 | 1 | FC Test | 19 | 180 | 75 | Yes |\n";
        let err = RtfScanner::new()
            .detect_format(&write_rtf(sample))
            .unwrap_err();
        assert!(
            err.contains("RTF 格式不合法") && err.contains("第 8 列起") && err.contains("旧版"),
            "{err}"
        );
    }

    #[test]
    fn invalid_rows_skipped() {
        // UID 非纯数字的行被当非数据行跳过；种族码越界的行按非法行跳过，合法行保留。
        let sample = "\
| 2001234567| ESP  |         | Ok Player | 1  | 12 | 1 |
| abc123    | ESP  |         | Bad Uid   | 1  | 12 | 1 |
| 2001234569| ESP  |         | Bad Eth   | 1  | 12 | 99 |
";
        let path = write_rtf(sample);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        let logger = ChannelLogger::channel().0;
        let data = s
            .parse_roster(&path, true, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-2001234567");
    }

    #[test]
    fn translate_chinese_nationality() {
        let path = write_rtf(CN_SAMPLE);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        let mut tr = HashMap::new();
        tr.insert("简体中文".to_string(), {
            let mut m = HashMap::new();
            m.insert("美国".to_string(), "USA".to_string());
            m
        });
        let logger = ChannelLogger::channel();
        let data = s.parse_roster(&path, true, &tr, &logger.0).unwrap();
        assert_eq!(data[0].primary_nat, "USA");
        // 全部命中：无警告日志。
        assert!(warn_messages(&logger.1).is_empty());
    }

    #[test]
    fn translate_miss_keeps_value_and_logs_warning() {
        // 翻译表查不到的中文值（截断别名、未收录国名都会走这条路）→ 保留原值不丢行，并记 WARNING 日志。
        // 「北马其」这类实测截断别名已陆续收录，这里用表里必然没有的虚构国名验证缺失路径。
        let sample = "\
| 编号      | 国籍/地区籍 | 第二国籍 | 姓名        |    |    |   |
| 2002075172| 西兰公国   | 美国     | 扎克·布劳顿 | 3  | 9  | 0 |
";
        let path = write_rtf(sample);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        let logger = ChannelLogger::channel();
        let data = s
            .parse_roster(&path, false, &NAT_TRANSLATION, &logger.0)
            .unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].primary_nat, "西兰公国"); // 缺失保留原值
        assert_eq!(data[0].sec_nat, "USA"); // 命中的第二国籍正常翻译
        let warns = warn_messages(&logger.1);
        assert!(
            warns
                .iter()
                .any(|m| m.contains("西兰公国") && m.contains("保留原值")),
            "应有翻译缺失警告：{:?}",
            warns.first()
        );
        // 命中的国籍不产生警告。
        assert!(!warns.iter().any(|m| m.contains("美国")));
    }

    #[test]
    fn ext_player_detect_parse_and_full_fields() {
        let path = write_rtf(EXT_PLAYER_SAMPLE);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
        assert_eq!(s.language, "English");
        // keep_only_newgen=true：只剩 Yes 那行（r- 前缀）。
        let logger = ChannelLogger::channel().0;
        let data = s
            .parse_roster(&path, true, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-103607");
        // 不过滤：No 行保留且为裸 UID，全字段按新布局入库。
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 2);
        let first = &data[0];
        assert_eq!(first.uid, "12315"); // 否（非随机人）：无前缀
        assert_eq!(first.primary_nat, "MKD");
        assert_eq!(first.sec_nat, "");
        assert_eq!(first.name, "Toni Micevski");
        assert_eq!(first.hair_length, "1"); // 发长在第 13 列
        assert_eq!(first.hair_color, "12"); // 发色在第 14 列
        assert_eq!(first.ethnicity_code, "0"); // 种族码在第 11 列
        let d = first.detailed.as_ref().unwrap();
        assert_eq!(d.skin_code, "5"); // 肤色在第 12 列
        assert_eq!(d.avatar, ""); // 头像列（第 3 列）为空
        assert_eq!(d.is_newgan, "No");
        assert_eq!(d.club, "Poeševo"); // 第 9 列
        assert_eq!(d.age, "54");
        assert_eq!(d.height, "188 cm");
        assert_eq!(d.based, "North Macedonia (Fourth League)"); // 赛事级别仅球员
        assert_eq!(d.preferred_job, "");
        assert_eq!(d.ca, "27");
        assert_eq!(d.pa, "122");
        assert_eq!(d.rca, "27");
        assert_eq!(d.gender, "");
        assert!(d.extra_fields.is_empty());
    }

    #[test]
    fn ext_player_extra_columns_stored() {
        // 第 17 列之后追加用户自定义列 → 仍合法，原文进入 extra_fields。
        let sample = "\
| 12315 | No | | Toni Micevski | MKD | | 54 | 188 cm | Poeševo | North Macedonia (Fourth League) | 0 | 5 | 1 | 12 | 27 | 122 | 27 | 惯用脚 | 左 |
";
        let path = write_rtf(sample);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
        let logger = ChannelLogger::channel().0;
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(
            data[0].detailed.as_ref().unwrap().extra_fields,
            vec!["惯用脚".to_string(), "左".to_string()]
        );
    }

    #[test]
    fn ext_kind_column_discriminates() {
        // 前 17 列同构（第 17 列 R.C.A 均为数字）；第 18 列性别词 → 职员，否则球员。
        let base = "| 12315 | No | | X | MKD | | 54 | 188 cm | C | B | 0 | 5 | 1 | 12 | 27 | 122 |";
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(&format!("{base} 27 |\n")))
            .unwrap(); // 17 列整
        assert_eq!(s.format, RtfFormat::ExtPlayer);
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(&format!("{base} 27 | 男 |\n")))
            .unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(&format!("{base} 27 | Female |\n")))
            .unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        // 第 18 列是球员的用户自定义列（非性别词）→ 仍是球员。
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(&format!("{base} 27 | 惯用脚 |\n")))
            .unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
        // 第 17 列 R.C.A 为空 → 非法行定位到「第 17 列」。
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{base}  |\n")))
            .unwrap_err();
        assert!(
            err.contains("RTF 格式不合法") && err.contains("第 17 列"),
            "{err}"
        );
    }

    #[test]
    fn ext_staff_detect_parse_and_full_fields() {
        // 中文职员：国籍列是中文国名 → 简体中文。
        let path = write_rtf(EXT_STAFF_CN_SAMPLE);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        assert_eq!(s.language, "简体中文");
        let logger = ChannelLogger::channel().0;
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 2);
        // 是 → r- 前缀（新签职员）；否 → 裸 UID。
        assert_eq!(data[0].uid, "r-2002243829");
        assert_eq!(data[1].uid, "306951");
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(d.preferred_job, "足球总监"); // 偏好工作仅职员
        assert_eq!(d.based, "");
        assert_eq!(d.gender, "男"); // 第 18 列
        assert_eq!(d.ca, "144");
        assert_eq!(d.pa, "199");
        assert_eq!(d.rca, "140"); // R.C.A 第 17 列，职员也有
        assert_eq!(d.avatar, ""); // 头像列（第 3 列）为空
                                  // 英文职员：国籍列是三字码 → English；性别 Male/Female 入库。
        let path = write_rtf(EXT_STAFF_EN_SAMPLE);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        assert_eq!(s.language, "English");
        let logger = ChannelLogger::channel().0;
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(d.gender, "Female");
        assert_eq!(d.rca, "195");
        assert_eq!(d.preferred_job, "Performance Analyst");
        assert_eq!(d.club, "");
    }

    #[test]
    fn ext_staff_avatar_and_extras_stored() {
        // 职员头像列有内容时原文入库；职员自定义列从第 19 列起。
        let sample = "\
| 306951 | 否 | face_kuka | 库卡 | 巴西 | | 68 | 182厘米 | 梅州客家 | 足球总监 | 0 | 8 | 1 | 11 | 142 | 160 | 138 | 男 | 合同年 | 2027 |
";
        let path = write_rtf(sample);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        let logger = ChannelLogger::channel().0;
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(d.avatar, "face_kuka");
        assert_eq!(d.gender, "男");
        assert_eq!(
            d.extra_fields,
            vec!["合同年".to_string(), "2027".to_string()]
        );
    }

    #[test]
    fn legacy_staff_layout_rejected() {
        // 旧职员拓展布局（性别在第 3 列、姓名在第 5 列、无 R.C.A）→ 整份判不合法：
        // 其第 11 列是偏好工作文本，被「种族码」数值校验拦下。
        let sample = "\
| 67214325   | 否 | 女 |  | 亚由美 | KOR | ENG | 26 | 167厘米 |  | 表现分析师 | 10 | 8 | 1 | 16 | 200 | 200 |
";
        let err = RtfScanner::new()
            .detect_format(&write_rtf(sample))
            .unwrap_err();
        assert!(
            err.contains("RTF 格式不合法") && err.contains("种族码"),
            "{err}"
        );
    }

    #[test]
    fn uid_length_boundaries() {
        // 3 位 / 11 位 → 非法行；4 位 / 10 位 → 合法。
        let sample = "\
| 123| ESP  |     | Short Uid  | 1  | 12 | 1 |
| 12345678901| ESP |    | Long Uid   | 1  | 12 | 1 |
| 1234| ESP  |     | Min Uid    | 1  | 12 | 1 |
| 1234567890| ESP  |     | Max Uid    | 1  | 12 | 1 |
";
        let path = write_rtf(sample);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        let logger = ChannelLogger::channel().0;
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].uid, "r-1234");
        assert_eq!(data[1].uid, "r-1234567890");
        // 整份只有越界 UID 行 → 报「编号(UID)」定位。
        let err = RtfScanner::new()
            .detect_format(&write_rtf("| 123| ESP | | X | 1 | 12 | 1 |\n"))
            .unwrap_err();
        assert!(err.contains("编号(UID)"), "{err}");
    }

    #[test]
    fn nationality_form_checks() {
        // 三位大写码 / 含汉字 / 英文全名 → 合法；单字母 / 纯数字 → 不合法。
        // 英文全名查翻译表命中 → 转码；未收录（如第二国籍列的海外华侨 'SC'）→ 保留原值 + 警告。
        let sample = "\
| 2000472008| SGP |         | Ok Code Nat    | 1 | 12 | 0 |
| 2000472009| 北马其 |        | Ok Cjk Nat     | 1 | 12 | 0 |
| 2000472010| SC  |        | Ok Long Nat    | 1 | 12 | 0 |
";
        let path = write_rtf(sample);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        let logger = ChannelLogger::channel();
        let data = s
            .parse_roster(&path, false, &NAT_TRANSLATION, &logger.0)
            .unwrap();
        assert_eq!(data.len(), 3);
        assert_eq!(data[2].primary_nat, "SC"); // 未收录全名保留原值
        let warns = warn_messages(&logger.1);
        assert!(
            warns
                .iter()
                .any(|m| m.contains("SC") && m.contains("保留原值")),
            "应有翻译缺失警告：{:?}",
            warns.first()
        );
        // 单字母 / 纯数字国籍 → 整份不合法并定位到「国籍」。
        for bad in [
            "| 2000472008| A | | X | 1 | 12 | 0 |\n",
            "| 2000472008| 123 | | X | 1 | 12 | 0 |\n",
        ] {
            let err = RtfScanner::new()
                .detect_format(&write_rtf(bad))
                .unwrap_err();
            assert!(err.contains("国籍"), "{err}");
        }
    }

    #[test]
    fn long_form_nationality_translated() {
        // 英文全名（long 格式）名单：语言判 English，解析后经 English 节点归一化为三字码。
        let sample = "\
| UID       | Nat                  | 2nd Nat              | Name           |    |    |   |
| 7458500   | Argentina            | U.S.A.               | Lionel Messi   | 1  | 11 | 1 |
| 29179241  | Norway               | Bosnia & Herzegovina | Erling Haaland | 1  | 7  | 0 |
| 2000197505| Hong Kong (China PR) | Nigeria              | Chan Tai Man   | 1  | 7  | 0 |
";
        let path = write_rtf(sample);
        let mut s = RtfScanner::new();
        s.detect_format(&path).unwrap();
        assert_eq!(s.language, "English");
        let logger = ChannelLogger::channel();
        let data = s
            .parse_roster(&path, true, &NAT_TRANSLATION, &logger.0)
            .unwrap();
        assert_eq!(data.len(), 3);
        assert_eq!(data[0].primary_nat, "ARG");
        assert_eq!(data[0].sec_nat, "USA"); // U.S.A. → USA
        assert_eq!(data[1].sec_nat, "BIH"); // Bosnia & Herzegovina → BIH
        assert_eq!(data[2].primary_nat, "HKG"); // FM26 带后缀显示名 → HKG
        assert!(warn_messages(&logger.1).is_empty()); // 全部命中，无警告
    }

    #[test]
    fn out_of_range_codes_rejected() {
        // 种族码 / 肤色码 / 发长 / 发色越界 → 整份不合法并定位到字段。
        for (row, field) in [
            ("| 2000472008| ESP | | X | 1 | 12 | 11 |", "种族码"),
            ("| 12315 | No | | X | MKD | | 54 | 188 cm | C | B | 0 | 20 | 1 | 12 | 27 | 122 | 27 |", "肤色码"),
            ("| 12315 | No | | X | MKD | | 54 | 188 cm | C | B | 0 | 5 | 6 | 12 | 27 | 122 | 27 |", "头发长度码"),
            ("| 12315 | No | | X | MKD | | 54 | 188 cm | C | B | 0 | 5 | 1 | 0 | 27 | 122 | 27 |", "头发颜色码"),
        ] {
            let err = RtfScanner::new().detect_format(&write_rtf(&format!("{row} |\n"))).unwrap_err();
            assert!(err.contains(field), "{field}: {err}");
        }
        // 发长 = 5 是合法上界（真实数据存在）。
        let ok =
            "| 12315 | No | | X | MKD | | 54 | 188 cm | C | B | 0 | 5 | 5 | 12 | 27 | 122 | 27 |\n";
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(ok)).unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
    }

    // ---------------- 真实导出样本（tests/testing_data，缺失时跳过） ----------------

    #[test]
    fn real_basic_files() {
        let mut s = RtfScanner::new();
        if let Some(f) = real_rtf("players_84617.rtf") {
            s.detect_format(&f).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::Basic, "English")
            );
        }
        if let Some(f) = real_rtf("players_84617_chn.rtf") {
            s.detect_format(&f).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::Basic, "简体中文")
            );
            let logger = ChannelLogger::channel();
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger.0)
                .unwrap();
            assert_eq!(data.len(), 84_617);
            // 英格兰 → ENG、北马其顿 → MKD 均已翻译（新版导出国籍为全名，全部命中翻译表）。
            assert!(data.iter().any(|r| r.primary_nat == "ENG"));
            assert!(data.iter().all(|r| r.primary_nat != "英格兰"));
            assert!(data.iter().any(|r| r.primary_nat == "MKD"));
            assert!(data.iter().all(|r| r.primary_nat != "北马其顿"));
            // 截断名/缺失码的警告日志路径由合成样本 translate_miss_logs_warning 覆盖。
        }
        if let Some(f) = real_rtf("staff_274.rtf") {
            s.detect_format(&f).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::Basic, "English")
            );
            let logger = ChannelLogger::channel().0;
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger)
                .unwrap();
            assert_eq!(data.len(), 274); // 基础职员名单（与球员同构），含 270 行 10 位 UID
            assert!(data.iter().all(|r| r.uid.starts_with(NEWGAN_UID_PREFIX)));
        }
        // 181855 双格式名单：同一批数据的 short（三字码）与 long（英文全名）导出。
        if let Some(f) = real_rtf("players_181855short.rtf") {
            s.detect_format(&f).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::Basic, "English")
            );
            let logger = ChannelLogger::channel();
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger.0)
                .unwrap();
            assert_eq!(data.len(), 181_855);
            // short 格式国籍列全是三字码，直接入库（第二国籍含新版新加坡码 SGP）。
            assert!(data.iter().any(|r| r.primary_nat == "ARG"));
            assert!(data.iter().any(|r| r.sec_nat == "SGP"));
            // 唯一非码值是第二国籍列的海外华侨 'SC'（未收录，保留原值 + 警告）。
            let warns = warn_messages(&logger.1);
            assert_eq!(warns.len(), 1, "{warns:?}");
            assert!(warns[0].contains("SC") && warns[0].contains("保留原值"));
        }
        if let Some(f) = real_rtf("players_181855long.rtf") {
            s.detect_format(&f).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::Basic, "English")
            );
            let logger = ChannelLogger::channel();
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger.0)
                .unwrap();
            assert_eq!(data.len(), 181_855);
            // long 格式的英文全名全部经 English 节点归一化为三字码（码形态值原样保留）。
            assert!(data.iter().any(|r| r.primary_nat == "ARG"));
            assert!(data.iter().all(|r| r.primary_nat != "Argentina"));
            assert!(
                data.iter().any(|r| r.primary_nat == "USA")
                    && data.iter().all(|r| r.primary_nat != "U.S.A.")
            );
            assert!(data.iter().any(|r| r.sec_nat == "SGP"));
            // 海外华侨在 long 版显示全名 SEA Chinese（已收录 → 'SC'），新版不再出现裸 'SC' 短形态。
            assert!(data.iter().any(|r| r.sec_nat == "SC"));
            // 新版导出的全部显示值（含 "Hong Kong (China PR)" 等带后缀全名）都已命中翻译表，零警告。
            let warns = warn_messages(&logger.1);
            assert!(warns.is_empty(), "{warns:?}");
        }
    }

    #[test]
    fn real_player_ext_files() {
        if let Some(f) = real_rtf("players_ext_84617.rtf") {
            let mut s = RtfScanner::new();
            s.detect_format(&f).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::ExtPlayer, "English")
            );
            // 42MB 冒烟：整份解析，验证列映射在大文件上成立。
            let logger = ChannelLogger::channel().0;
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger)
                .unwrap();
            assert!(
                data.len() >= 84_000,
                "应解析出全部数据行，实际 {}",
                data.len()
            );
            let newgan = data
                .iter()
                .filter(|r| r.uid.starts_with(NEWGAN_UID_PREFIX))
                .count();
            assert!(
                newgan > 10_000 && newgan < data.len(),
                "随机人/非随机人应同时存在：{newgan}/{}",
                data.len()
            );
            let first = &data[0];
            assert_eq!(first.uid, "12315");
            assert_eq!(first.primary_nat, "MKD");
            assert_eq!(first.name, "Toni Micevski");
            assert_eq!(first.hair_length, "1");
            assert_eq!(first.hair_color, "12");
            assert_eq!(first.ethnicity_code, "0");
            let d = first.detailed.as_ref().unwrap();
            assert_eq!(d.skin_code, "5");
            assert_eq!(d.club, "Poeševo");
            assert_eq!(d.age, "54");
            assert_eq!(d.height, "188 cm");
            assert_eq!(d.based, "North Macedonia (Fourth League)");
            assert_eq!(d.ca, "27");
            assert_eq!(d.pa, "122"); // PA 第 16 列（新序 CA|PA|R.C.A）
            assert_eq!(d.rca, "27"); // R.C.A 第 17 列（数字 → 判为球员名单）
            assert_eq!(d.is_newgan, "No");
        }
        if let Some(f) = real_rtf("players_ext_84617_chn.rtf") {
            let mut s = RtfScanner::new();
            s.detect_format(&f).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::ExtPlayer, "简体中文")
            );
            let logger = ChannelLogger::channel();
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger.0)
                .unwrap();
            assert!(data.iter().any(|r| r.primary_nat == "ENG"));
            assert!(data.iter().any(|r| r.primary_nat == "MKD")); // 北马其顿已翻译（新版导出为全名）
        }
    }

    #[test]
    fn real_wrong_files() {
        // UID 边界：4/7/10 位保留 3 行，11 位行跳过。
        if let Some(f) = real_rtf("test_wrong_UID.rtf") {
            let mut s = RtfScanner::new();
            s.detect_format(&f).unwrap();
            let logger = ChannelLogger::channel().0;
            let data = s.parse_roster(&f, false, &HashMap::new(), &logger).unwrap();
            assert_eq!(data.len(), 3);
            assert_eq!(data[0].uid, "r-191571");
            assert_eq!(data[1].uid, "r-1915576430");
            assert_eq!(data[2].uid, "r-1915576");
        }
        if let Some(f) = real_rtf("test_wrong_ethnicity.rtf") {
            let mut s = RtfScanner::new();
            s.detect_format(&f).unwrap();
            let logger = ChannelLogger::channel().0;
            let data = s.parse_roster(&f, false, &HashMap::new(), &logger).unwrap();
            assert_eq!(data.len(), 29);
        }
    }

    #[test]
    fn real_staff_ext_fixtures() {
        // 拓展职员新布局夹具（按规格样本持久化在 testing_data）。
        if let Some(f) = real_rtf("staff_ext_chn_sample.rtf") {
            let mut s = RtfScanner::new();
            s.detect_format(&f).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::ExtStaff, "简体中文")
            );
            let logger = ChannelLogger::channel().0;
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger)
                .unwrap();
            assert_eq!(data[0].uid, "r-2002243829"); // 是 → r- 前缀
            assert_eq!(data[0].detailed.as_ref().unwrap().gender, "男");
        }
        if let Some(f) = real_rtf("staff_ext_eng_sample.rtf") {
            let mut s = RtfScanner::new();
            s.detect_format(&f).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::ExtStaff, "English")
            );
            let logger = ChannelLogger::channel().0;
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger)
                .unwrap();
            assert_eq!(data[0].detailed.as_ref().unwrap().gender, "Female");
        }
    }

    #[test]
    fn real_invalid_fixtures_rejected() {
        // 旧布局 / 乱码夹具 → 整份判「RTF 格式不合法」。
        for name in [
            "invalid_legacy_detailed.rtf",
            "invalid_legacy_staff_ext.rtf",
            "invalid_garbage.rtf",
        ] {
            if let Some(f) = real_rtf(name) {
                let err = RtfScanner::new().detect_format(&f).unwrap_err();
                assert!(err.contains("RTF 格式不合法"), "{name}: {err}");
            }
        }
    }
}
