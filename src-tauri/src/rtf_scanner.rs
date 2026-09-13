//! FM（Football Manager）名单 RTF 扫描器：识别视图导出的列结构，逐行全字段解析入库。
//!
//! 【格式规格】以真实导出样本为准（tests/testing_data/）。共六种形态 = 三种列结构 × 两种
//! 数据语言：Basic 基础名单 7 列（编号/国籍/第二国籍/姓名/头发长度/发色/种族码），导出自
//! 「is newgen」过滤视图，全员随机生；ExtPlayer/ExtStaff 拓展名单 ≥18 列，前 7 列与 Basic
//! 同构，第 8-18 列 = 肤色/头像/是新生球员/性别/年龄/身高/俱乐部/所在地（球员）或偏好工作
//! （职员）/能力/潜力/R.C.A，第 19 列起为用户自定义列。国籍显示有四种形态（三位大写码/
//! 英文全名/中文国名/列宽截断别名），入库前统一查 nat_translation.json 归一化为三字码。
//!
//! 【行列计数约定】单元格向量第 0 列是行号计数列，第 k 列即表格第 k 列（1 起算）——
//! 报错文案的「第 N 列」直接用下标，不再换算。行号按表行计：第 0 行 = 表头，第 1 行 =
//! 第 1 条数据……空行与分隔线不是表行、不占行号，报错文案的「第 N 行」即第 N 条数据行。
//!
//! 【判别策略】判别由表头驱动——表头是视图列结构的权威声明，判错时能精确到列报错，
//! 也顺带完成「是否本工具约定视图」的身份校验：第 0 个表行须命中第 1 列 编号/UID，随后
//! ① 列数门（7 或 ≥18）→ ② 第 1-4 列身份词 → ③ 第 10 列标志词分基础/拓展轨道 →
//! ④ 拓展轨道第 15 列判别词分球员/职员（详见 `header_format`）。判型后仍抽查前 20 条
//! 数据行、要求至少 1 条按该格式合法——防止表头与数据行错位的文件被静默解析成空名单。
//! 数据语言不判自表头（英文拓展导出的表头中英混排），按数据行国籍列判定：含简体汉字
//! → 简体中文，否则 English。
//!
//! 【错误处理分工】detect 严格：整份层面的问题（无表头、表头不合法、无合法数据行）报错，
//! 文案面向最终用户并落 Error 日志；parse 宽容：行级瑕疵静默跳过——个别脏行/未加载对象
//! 不应中断整份，整份合法性问题已在 detect 兜底。
use crate::logger::LogSink;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// 随机生 UID 的固定前缀。Replace 流程与前端都靠它区分随机生/原生记录，改前缀是破坏性变更。
pub const NEWGAN_UID_PREFIX: &str = "r-";

const ZERO_WIDTH_SPACE: &str = "\u{200b}"; // FM 中文导出按列宽折行时混入的不可见字符
const HEAD_BYTES: u64 = 64 * 1024; // 格式识别的读入上限：表头与数据行必在文件开头，大名单（上百 MB）不必全读
const DETECT_LINE_LIMIT: usize = 20; // 判型后抽查的数据行数：至少 1 条按判出的格式合法才放行
const MISS_LOG_MAX: usize = 50; // 翻译缺失逐条记日志的上限，超出合并汇总防刷屏

/// 随机生标志的正向取值。实测取值域为 是/否（中文导出）、Yes/No（英文导出）；
/// 用 contains 匹配而非全等，容忍值前后可能带的修饰。
const NEWGAN_FLAGS: [&str; 2] = ["是", "Yes"];

const BASIC_FIELD_COUNT: usize = 7; // 基础名单列数，即拓展名单与其同构的前缀长度
const EXT_FIELD_COUNT: usize = 18; // 拓展名单标准列数（第 19 列起为用户自定义列）

// 数值码合法区间，实测口径（真实导出样本统计）。发色无 0 值，从 1 起。
const ETHNICITY_RANGE: (u32, u32) = (0, 10);
const SKIN_RANGE: (u32, u32) = (0, 19);
const HAIR_LENGTH_RANGE: (u32, u32) = (0, 5);
const HAIR_COLOR_RANGE: (u32, u32) = (1, 19);

// 列下标 = 表格列号（1 起算）：单元格向量第 0 列为行号计数列，第 k 列存于下标 k，
// 报错文案的「第 N 列」直接用下标。Basic 的 7 列与拓展名单前 7 列完全同构。
const UID_COL: usize = 1;
const NAT_COL: usize = 2; // 国籍；两种结构同位，语言判定也取此列
const SEC_NAT_COL: usize = 3;
const NAME_COL: usize = 4;
const HAIR_LENGTH_COL: usize = 5;
const HAIR_COLOR_COL: usize = 6;
const ETHNICITY_COL: usize = 7;

// 拓展名单独有列。基础/拓展由表头判型决定，这些列不承担结构判别职责。
const EXT_SKIN_COL: usize = 8;
const EXT_AVATAR_COL: usize = 9; // FM 图片列，导出为空文本
const EXT_NEWGAN_FLAG_COL: usize = 10; // 是新生球员：是/否、Yes/No；表头同列名，判型取此列
const EXT_GENDER_COL: usize = 11;
const EXT_AGE_COL: usize = 12;
const EXT_HEIGHT_COL: usize = 13;
const EXT_CLUB_COL: usize = 14;
const EXT_BASED_COL: usize = 15; // 球员=所在地（俱乐部赛事级别），职员=偏好工作；表头分型取此列
const EXT_CA_COL: usize = 16;
const EXT_PA_COL: usize = 17;
const EXT_RCA_COL: usize = 18; // 职员表头作 NP R.C.A，同为数值列
const EXT_EXTRA_COL: usize = 19; // 第 19 列起 = 用户自定义列（extra_fields 起点）

/// 表头第 1-4 列身份词与报错名。小写包含匹配：真实表头是「国籍/地区籍」「2nd Nat」
/// 这类带修饰的显示名，全等会漏。
const HEADER_IDENTITY: [(&str, [&str; 2]); 4] = [
    ("编号/UID", ["编号", "uid"]),
    ("国籍/Nat", ["国籍", "nat"]),
    ("第二国籍/2nd Nat", ["第二国籍", "2nd nat"]),
    ("姓名/Name", ["姓名", "name"]),
];
/// 表头第 10 列标志词：拓展名单才有的「是新生球员」列（真实表头 中文/英文两种写法）。
const HEADER_FLAG: (&str, [&str; 2]) = ("是新生球员/Is newgen", ["是新生球员", "is newgen"]);
/// 表头第 15 列的球员判别词（真实表头：中文「所在地」/英文「Based」；英文大小写不定）。
const PLAYER_BASED_HEADERS: [&str; 2] = ["based", "所在地"];
/// 表头第 15 列的职员判别词（真实表头：「偏好工作」/「Preferred Job」）。
const STAFF_JOB_HEADERS: [&str; 2] = ["preferred job", "偏好工作"];

/// 名单列结构。数据语言不在此（按国籍列内容判定），见 `RtfScanner::language`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RtfFormat {
    /// 基础名单：7 列，导出自「is newgen」过滤视图，全员随机生。
    #[default]
    Basic,
    /// 拓展球员：18 标准列，第 15 列为所在地（俱乐部赛事级别）。
    ExtPlayer,
    /// 拓展职员：与拓展球员同构，第 15 列为偏好工作，靠表头分型。
    ExtStaff,
}

impl RtfFormat {
    /// 中文标签（日志/摘要用）。
    pub fn label(self) -> &'static str {
        match self {
            RtfFormat::Basic => "基础名单",
            RtfFormat::ExtPlayer => "拓展球员名单",
            RtfFormat::ExtStaff => "拓展职员名单",
        }
    }
}

/// 拓展名单第 8-18 列及用户自定义列的原文；serde camelCase 与前端 types.ts 对齐。
/// based 与 preferred_job 互斥——第 15 列按分型只有一种语义，另一种恒为空。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetailedInfo {
    pub skin_code: String,         // 肤色码（第 8 列）
    pub avatar: String,            // 头像列原文（第 9 列；FM 图片列，导出为空文本，照单入库）
    pub is_newgan: String,         // 随机生标志原文（第 10 列），不转布尔：译文形态随导出语言
    pub club: String,              // 俱乐部（第 14 列）
    pub age: String,               // 年龄（第 12 列）
    pub height: String,            // 身高（第 13 列）
    pub based: String,             // 所在地/俱乐部赛事级别（第 15 列，仅球员）
    pub preferred_job: String,     // 偏好工作（第 15 列，仅职员）
    pub ca: String,                // 当前能力 CA（第 16 列）
    pub pa: String,                // 潜力 PA（第 17 列）
    pub rca: String,               // 推荐能力 R.C.A（第 18 列，球员/职员都有）
    pub gender: String,            // 性别原文（第 11 列）：男/女、Male/Female
    pub extra_fields: Vec<String>, // 用户自定义列原文（第 19 列起），不解释不丢失
}

/// 一条名单记录（球员/职员通用）。uid 的 r- 前缀是随机生标记，Replace 流程与前端依赖它；
/// primary_nat/sec_nat 在 parse_roster 尾部归一化为三字码，翻译缺失时保留原值（不丢行）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRecord {
    pub uid: String,
    pub primary_nat: String,
    pub sec_nat: String,
    pub name: String,
    pub hair_length: String,
    pub hair_color: String,
    pub ethnicity_code: String, // "0".."10"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detailed: Option<DetailedInfo>, // 仅拓展名单有；skip 序列化让前端看不到空 detailed
}

/// 单行判定结论。Rejected 带字段级定位，供 detect 在无合法数据行时生成用户可读的报错。
enum RowVerdict {
    Data,
    NotDataRow,
    Rejected { field: &'static str, detail: String },
}

/// 扫描器状态：detect_format 识别结果的载体，解析阶段复用同一结论。
pub struct RtfScanner {
    pub is_valid: bool,    // parse_roster 的入口校验：未识别先解析直接报错
    pub language: String, // "English" / "简体中文" / "Unknown"（未识别时），翻译查表与摘要展示都用它
    pub format: RtfFormat, // 列结构判型结果，parse 按它取列组装，commands 层直接展示
}

impl RtfScanner {
    pub fn new() -> Self {
        Self {
            is_valid: false,
            language: "Unknown".into(),
            format: RtfFormat::default(),
        }
    }

    /// 识别格式与语言（只读文件头部，不做全量解析）。
    ///
    /// 头部 HEAD_BYTES 内完成表头判型 + 前 DETECT_LINE_LIMIT 条数据行抽查。
    ///
    /// 成功后 parse_roster 才可调用。Err 文案直接面向最终用户，前端原样弹窗，同时落
    /// Error 日志留档：表头问题精确到「表头第 N 列」，数据行问题定位到「第 N 行「字段」」。
    pub fn detect_format(&mut self, path: &Path, logger: &impl LogSink) -> Result<(), String> {
        self.is_valid = false;
        self.language = "Unknown".into();
        self.format = RtfFormat::default();
        let text = read_head(path, HEAD_BYTES)?;
        self.detect(&text).map_err(|e| {
            logger.write(log::Level::Error, "rtf", e.clone());
            e
        })
    }

    /// 判型主流程：表头四步判型 → 数据行抽查（≥1 条合法 + 语言判定）。
    fn detect(&mut self, text: &str) -> Result<(), String> {
        let mut rows = table_rows(text);
        // 表头 = 第 0 个表行：真实导出的表头必在文件开头（前导空行不是表行）。
        // 第 1 列命中 编号/UID 是表头的唯一标志——数据行同列是纯数字、分隔线是横线，都不会撞。
        let format = match rows.next() {
            Some((_, header)) => header_format(&header)?,
            None => return Err("RTF 格式不合法：未识别到表头行".into()),
        };
        // 数据行抽查：行结构与字段按表头判出的格式校验，至少 1 条合法才放行。
        let mut valid = 0usize;
        let mut first_rejection: Option<String> = None;
        let mut language: Option<&'static str> = None;
        for (row_no, cells) in rows.take(DETECT_LINE_LIMIT) {
            match classify_row(&cells, format) {
                RowVerdict::Data => {
                    valid += 1;
                    if language.is_none() {
                        let nat = cells.get(NAT_COL).map(String::as_str).unwrap_or("");
                        language = Some(if contains_cjk(nat) {
                            "简体中文"
                        } else {
                            "English"
                        });
                    }
                }
                RowVerdict::NotDataRow => {}
                RowVerdict::Rejected { field, detail } => {
                    // 只记首条：报错定位到最靠前的坏行即可，逐行罗列没有增量信息。
                    if first_rejection.is_none() {
                        first_rejection = Some(format!("第 {row_no} 行「{field}」{detail}"));
                    }
                }
            }
        }
        if valid == 0 {
            return Err(match first_rejection {
                Some(rej) => format!("RTF 格式不合法：{rej}"),
                None => "RTF 格式不合法：未识别到合法的数据行".into(),
            });
        }
        self.format = format;
        self.language = language.unwrap_or("English").into();
        self.is_valid = true;
        Ok(())
    }

    /// 全量解析名单（须先 detect_format 成功）。
    ///
    /// * keep_only_newgen=true 只保留 r- 前缀（随机生）记录——Replace 流程按此取目标；
    ///   扫描展示传 false。translations 为「语言 → 显示文本 → 三字码」多语言表（assets 加载）。
    /// * 与 detect 不同，解析阶段对非法行静默跳过：真实导出混有脏行/未加载对象，个别坏行
    ///   不应中断整份；整份合法性问题已在 detect 兜底。
    /// * 返回前统一做国籍归一化（见 `translate_nationalities`）。
    pub fn parse_roster(
        &self,
        path: &Path,
        keep_only_newgen: bool,
        translations: &HashMap<String, HashMap<String, String>>,
        logger: &impl LogSink,
    ) -> Result<Vec<PlayerRecord>, String> {
        if !self.is_valid {
            return Err("RTF 格式无效，请先执行格式识别".into());
        }
        let text = fs::read_to_string(path).map_err(|e| format!("读取 RTF 失败: {e}"))?;
        let mut data = Vec::new();
        for (_, cells) in table_rows(&text) {
            if let Some(record) = self.parse_line(&cells, keep_only_newgen) {
                data.push(record);
            }
        }
        let misses = translate_nationalities(&mut data, translations, &self.language);
        log_translation_misses(&misses, logger);
        Ok(data)
    }

    /// 行 → 记录；返回 None 表示该行不是当前格式的合法数据行（调用方跳过）。
    fn parse_line(&self, cells: &[String], keep_only_newgen: bool) -> Option<PlayerRecord> {
        if matches!(classify_row(cells, self.format), RowVerdict::Data) {
            let record = match self.format {
                RtfFormat::Basic => parse_basic_row(cells),
                _ => parse_ext_row(cells, self.format),
            };
            // 过滤只看 UID 字段：带 r- 前缀的是随机生。
            (record.uid.starts_with(NEWGAN_UID_PREFIX) || !keep_only_newgen).then_some(record)
        } else {
            None
        }
    }
}

// ---------------- 行管线：清洗、切分、判别 ----------------

/// 只读前 `max` 字节并按 UTF-8 lossy 解码：截断边界上的残缺多字节字符替换而非报错，
/// 头部判别不需要完整文件。
fn read_head(path: &Path, max: u64) -> Result<String, String> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|e| format!("读取 RTF 失败: {e}"))?;
    let mut buf = Vec::new();
    file.take(max)
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取 RTF 失败: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// FM 中文导出按列宽折行，表头与字段值里混入 U+200B（实测表头「编​号」、字段「180厘​米」
/// 均含）；不清除会破坏表头命中与翻译表查值。
fn strip_zero_width_chars(line: &str) -> String {
    line.replace(ZERO_WIDTH_SPACE, "")
}

/// 逐行产出表行 (行号, 单元格)。行号即报错文案的「第 N 行」：第 0 行=表头，第 1 行=
/// 第 1 条数据……空行与分隔线不是表行、不占行号。单元格向量第 0 列恒为行号本身
/// （计数列），第 k 列即表格第 k 列，与报错文案的列号一致。
/// 【为何跳过全空/纯横线行】分隔线夹在表头与数据行之间，若占行号，「第 1 行是第 1 条
/// 数据」的约定就不成立。
fn table_rows(text: &str) -> impl Iterator<Item = (usize, Vec<String>)> + '_ {
    let mut row_no = 0usize;
    text.lines().filter_map(move |line| {
        let mut cells: Vec<String> = strip_zero_width_chars(line)
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        if cells.len() < 2 {
            return None; // 无竖线的杂行不是表行
        }
        cells.pop(); // 行以 | 结尾，尾段是切分产物
        if cells[1..]
            .iter()
            .all(|c| c.is_empty() || c.bytes().all(|b| b == b'-'))
        {
            return None; // 全空行与分隔线
        }
        cells[0] = row_no.to_string(); // 首段本为行首 | 的切分产物，改写为行号计数列
        let no = row_no;
        row_no += 1;
        Some((no, cells))
    })
}

/// 表头四步判型，报错文案面向最终用户、精确到列：
/// ① 列数门：≥18 拓展轨道、7（容忍尾部空列）基础轨道，8-17 列整份不合法；
/// ② 第 1-4 列命中 编号(UID)/国籍(Nat)/第二国籍(2nd Nat)/姓名(Name)；
/// ③ 第 10 列命中 是新生球员(Is newgen) → 拓展轨道；7 列轨道本无第 10 列 → Basic。
///    【为何 ≥18 列轨道未命中不判 Basic】基础名单恒 7 列，18 列视图判 Basic 必与数据行
///    矛盾，直接指明第 10 列比推迟到数据行报「应为 7 列，实际 18 列」更可定位；
/// ④ 拓展轨道第 15 列：偏好工作(Preferred Job) → ExtStaff、所在地(Based) → ExtPlayer，
///    未命中整份不合法（分型错会让第 15 列语义错位入库）。
fn header_format(header: &[String]) -> Result<RtfFormat, String> {
    let cols = header.len() - 1; // 第 0 列是行号计数列
    let ext_track = cols >= EXT_FIELD_COUNT;
    if !ext_track
        && !(cols >= BASIC_FIELD_COUNT
            && header[(BASIC_FIELD_COUNT + 1)..]
                .iter()
                .all(|c| c.is_empty()))
    {
        return Err(format!(
            "RTF 格式不合法：表头列数应为 7（基础名单）或至少 18（拓展名单），实际 {cols} 列"
        ));
    }
    for (offset, (name, words)) in HEADER_IDENTITY.iter().enumerate() {
        let col = offset + 1; // 第 1-4 列
        if !header_hit(&header[col], words) {
            return Err(format!(
                "RTF 格式不合法：表头第 {col} 列应为「{name}」，实际「{}」",
                header[col]
            ));
        }
    }
    if !ext_track {
        return Ok(RtfFormat::Basic);
    }
    if !header_hit(&header[EXT_NEWGAN_FLAG_COL], &HEADER_FLAG.1) {
        return Err(format!(
            "RTF 格式不合法：表头第 10 列应为「{}」，实际「{}」",
            HEADER_FLAG.0, header[EXT_NEWGAN_FLAG_COL]
        ));
    }
    if header_hit(&header[EXT_BASED_COL], &PLAYER_BASED_HEADERS) {
        return Ok(RtfFormat::ExtPlayer);
    }
    if header_hit(&header[EXT_BASED_COL], &STAFF_JOB_HEADERS) {
        return Ok(RtfFormat::ExtStaff);
    }
    Err(format!(
        "RTF 格式不合法：表头第 15 列应为「所在地/Based」（球员）或「偏好工作/Preferred Job」（职员），实际「{}」",
        header[EXT_BASED_COL]
    ))
}

/// 表头列命中判定：小写后包含匹配（真实表头如「国籍/地区籍」「2nd Nat」带修饰，
/// 英文列名大小写不定，全等匹配会漏）。
fn header_hit(cell: &str, words: &[&str]) -> bool {
    let lower = cell.to_lowercase();
    words.iter().any(|w| lower.contains(w))
}

/// 单行判别。检测抽查与解析过滤共用同一实现——两阶段对「合法数据行」的口径必须一致，
/// 否则检测认得的行解析会丢、或解析混入检测没见过的行。
/// 格式由表头判型决定（不做行内分流）；第 1 列纯数字仍是数据行的唯一标志
/// （表头同列是「编号/UID」文字）。
fn classify_row(cells: &[String], format: RtfFormat) -> RowVerdict {
    let Some(uid) = cells.get(UID_COL) else {
        return RowVerdict::NotDataRow;
    };
    if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
        return RowVerdict::NotDataRow;
    }
    if !is_valid_uid(uid) {
        return RowVerdict::Rejected {
            field: "编号(UID)",
            detail: format!("应为 4-10 位数字，实际「{uid}」"),
        };
    }
    match format {
        RtfFormat::Basic => classify_basic_row(cells),
        _ => classify_ext_row(cells),
    }
}

/// Basic 行校验：列数恰为 7（尾部空列容忍——FM 按视图宽度补齐空单元格；第 8 列起出现
/// 内容 = 数据行与表头判出的基础结构不符）+ 国籍形态 + 三个数值码。
/// 校验顺序与列序一致，报错定位到最靠前的坏字段。
fn classify_basic_row(cells: &[String]) -> RowVerdict {
    if cells.len() < BASIC_FIELD_COUNT + 1
        || cells[(BASIC_FIELD_COUNT + 1)..]
            .iter()
            .any(|c| !c.is_empty())
    {
        return RowVerdict::Rejected {
            field: "列数",
            detail: format!(
                "基础名单应为 {BASIC_FIELD_COUNT} 列，实际 {} 列",
                cells.len() - 1
            ),
        };
    }
    if let Some(v) = invalid_nationality(cells, NAT_COL, "国籍", false) {
        return v;
    }
    if let Some(v) = invalid_nationality(cells, SEC_NAT_COL, "第二国籍", true) {
        return v;
    }
    if let Some(v) = invalid_code(cells, HAIR_LENGTH_COL, "头发长度码", HAIR_LENGTH_RANGE) {
        return v;
    }
    if let Some(v) = invalid_code(cells, HAIR_COLOR_COL, "头发颜色码", HAIR_COLOR_RANGE) {
        return v;
    }
    if let Some(v) = invalid_code(cells, ETHNICITY_COL, "种族码", ETHNICITY_RANGE) {
        return v;
    }
    RowVerdict::Data
}

/// 拓展行校验（球员/职员共用，分型已由表头完成、行内无需再区分）：列数 ≥18 + 国籍、
/// 四个数值码、第 18 列 R.C.A。校验顺序与列序一致。
/// 【为何接受空肤色/R.C.A】FM 对未加载对象整段属性导出空白（实测 ext_staff_short_chn 37 行
/// 肤色与 R.C.A 全空），空值原样入库；非空才校验，防止列错位的文本混入。
fn classify_ext_row(cells: &[String]) -> RowVerdict {
    if cells.len() < EXT_FIELD_COUNT + 1 {
        return RowVerdict::Rejected {
            field: "列数",
            detail: format!(
                "拓展名单应至少 {EXT_FIELD_COUNT} 列，实际 {} 列",
                cells.len() - 1
            ),
        };
    }
    if let Some(v) = invalid_nationality(cells, NAT_COL, "国籍", false) {
        return v;
    }
    if let Some(v) = invalid_nationality(cells, SEC_NAT_COL, "第二国籍", true) {
        return v;
    }
    if let Some(v) = invalid_code(cells, HAIR_LENGTH_COL, "头发长度码", HAIR_LENGTH_RANGE) {
        return v;
    }
    if let Some(v) = invalid_code(cells, HAIR_COLOR_COL, "头发颜色码", HAIR_COLOR_RANGE) {
        return v;
    }
    if let Some(v) = invalid_code(cells, ETHNICITY_COL, "种族码", ETHNICITY_RANGE) {
        return v;
    }
    if let Some(v) = invalid_code(cells, EXT_SKIN_COL, "肤色码", SKIN_RANGE) {
        return v;
    }
    // 第 18 列 R.C.A 只拒「非空且非数字」的列错位；能力类值无实测上界，不做区间校验。
    let rca = cells[EXT_RCA_COL].as_str();
    if !rca.is_empty() && !rca.bytes().all(|b| b.is_ascii_digit()) {
        return RowVerdict::Rejected {
            field: "第 18 列",
            detail: format!("R.C.A 应为数字，实际「{rca}」"),
        };
    }
    RowVerdict::Data
}

/// 国籍列校验：不合法返回 Rejected（列号即表格列号），合法返回 None。
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
        detail: format!("第 {col} 列应为三位大写国籍码、国籍英文名或中文国名，实际「{value}」"),
    })
}

/// 数值码列校验：不合法返回 Rejected（列号即表格列号），合法返回 None。
fn invalid_code(
    cells: &[String],
    col: usize,
    field: &'static str,
    (min, max): (u32, u32),
) -> Option<RowVerdict> {
    let value = cells.get(col).map(String::as_str).unwrap_or("");
    if is_valid_code(value, min, max) {
        return None;
    }
    Some(RowVerdict::Rejected {
        field,
        detail: format!("第 {col} 列应为 {min}-{max} 的数字，实际「{value}」"),
    })
}

// ---------------- 谓词（统一 is_* 风格） ----------------

/// 文本是否含汉字。判 CJK 统一表意区即可：FM 中文导出只有简体，无需区分简繁。
fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// 4-10 位是实测 FM UID 位数口径；11 位以上为溢出脏数据，拒行（见 test_wrong_UID 夹具）。
fn is_valid_uid(uid: &str) -> bool {
    (4..=10).contains(&uid.chars().count()) && uid.bytes().all(|b| b.is_ascii_digit())
}

/// 空值 = 无数据，原样入库（FM 对未加载对象导出空白单元格，拒行会丢整批数据）；
/// 非空必须是纯数字且落在 [min, max]。先按字节判纯数字再 parse：超长串 parse 溢出
/// 返回 Err，一并视为不合法。
fn is_valid_code(value: &str, min: u32, max: u32) -> bool {
    value.is_empty()
        || (value.bytes().all(|b| b.is_ascii_digit())
            && value
                .parse::<u32>()
                .map(|n| (min..=max).contains(&n))
                .unwrap_or(false))
}

/// 三位大写英文国籍码（short 形态，如 CHN）。
fn is_three_letter_code(nat: &str) -> bool {
    nat.len() == 3 && nat.bytes().all(|b| b.is_ascii_uppercase())
}

/// 国籍英文全名（long 形态）：字母开头、≥2 字符、不含数字，可含连接符与变音字母——
/// 实测出现 "U.S.A."、"Türkiye"、"São Tomé & Príncipe"、"Hong Kong (China PR)"。
/// 只做形态判定不查表：查不到翻译的值由「保留原值 + 警告」兜底，判别阶段不拒行。
fn is_long_form_name(nat: &str) -> bool {
    nat.chars().next().is_some_and(|c| c.is_alphabetic())
        && nat.chars().count() >= 2
        && nat.chars().all(|c| {
            c.is_alphabetic() || matches!(c, ' ' | '&' | '.' | '-' | '\'' | '’' | '(' | ')')
        })
}

/// 国籍四种合法形态：三位大写码（short）、英文全名（long）、中文国名与列宽截断别名
/// （后两者含汉字）。纯形态判定：截断别名未收录翻译表时保留原值，不能在此拒行。
fn is_valid_nationality(nat: &str, allow_empty: bool) -> bool {
    if nat.is_empty() {
        return allow_empty;
    }
    is_three_letter_code(nat) || contains_cjk(nat) || is_long_form_name(nat)
}

/// 随机生判定：标志含 是/Yes 即为随机生（正向判定），UID 加 r- 前缀。
fn is_newgan(flag: &str) -> bool {
    NEWGAN_FLAGS.iter().any(|f| flag.contains(f))
}

// ---------------- 行解析（classify 通过后取列组装，不再重复校验） ----------------

/// Basic 行组装。视图本身按 is newgen 过滤，UID 无条件加 r- 前缀。
fn parse_basic_row(cells: &[String]) -> PlayerRecord {
    PlayerRecord {
        uid: format!("{NEWGAN_UID_PREFIX}{}", cells[UID_COL]),
        primary_nat: cells[NAT_COL].clone(),
        sec_nat: cells[SEC_NAT_COL].clone(),
        name: cells[NAME_COL].clone(),
        hair_length: cells[HAIR_LENGTH_COL].clone(),
        hair_color: cells[HAIR_COLOR_COL].clone(),
        ethnicity_code: cells[ETHNICITY_COL].clone(),
        detailed: None,
    }
}

/// 拓展行组装。随机生按标志正向判定（含 是/Yes）；第 15 列按分型写入 based 或
/// preferred_job（另一个置空，前端据此区分球员/职员记录）；第 19 列起用户自定义列
/// 原文进 extra_fields。
fn parse_ext_row(cells: &[String], format: RtfFormat) -> PlayerRecord {
    let is_player = format == RtfFormat::ExtPlayer;
    let flag = cells[EXT_NEWGAN_FLAG_COL].as_str();
    let uid = if is_newgan(flag) {
        format!("{NEWGAN_UID_PREFIX}{}", cells[UID_COL])
    } else {
        cells[UID_COL].clone()
    };
    PlayerRecord {
        uid,
        primary_nat: cells[NAT_COL].clone(),
        sec_nat: cells[SEC_NAT_COL].clone(),
        name: cells[NAME_COL].clone(),
        hair_length: cells[HAIR_LENGTH_COL].clone(),
        hair_color: cells[HAIR_COLOR_COL].clone(),
        ethnicity_code: cells[ETHNICITY_COL].clone(),
        detailed: Some(DetailedInfo {
            skin_code: cells[EXT_SKIN_COL].clone(),
            avatar: cells[EXT_AVATAR_COL].clone(),
            is_newgan: flag.to_string(),
            club: cells[EXT_CLUB_COL].clone(),
            age: cells[EXT_AGE_COL].clone(),
            height: cells[EXT_HEIGHT_COL].clone(),
            based: if is_player {
                cells[EXT_BASED_COL].clone()
            } else {
                String::new()
            },
            preferred_job: if is_player {
                String::new()
            } else {
                cells[EXT_BASED_COL].clone()
            },
            ca: cells[EXT_CA_COL].clone(),
            pa: cells[EXT_PA_COL].clone(),
            rca: cells[EXT_RCA_COL].clone(),
            gender: cells[EXT_GENDER_COL].clone(),
            extra_fields: cells[EXT_EXTRA_COL..].to_vec(),
        }),
    }
}

// ---------------- 国籍翻译 ----------------

/// 国籍显示文本 → 三字码（就地改写）。空值与三字码（已是目标形态）跳过；查不到的保留
/// 原值并按值计数返回。排序为行数降序、同数按值升序——日志顺序确定、可复现。
fn translate_nationalities(
    data: &mut [PlayerRecord],
    translations: &HashMap<String, HashMap<String, String>>,
    language: &str,
) -> Vec<(String, usize)> {
    let empty = HashMap::new();
    let map = translations.get(language).unwrap_or(&empty);
    let mut misses: HashMap<String, usize> = HashMap::new();
    for record in data.iter_mut() {
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

/// 缺失值逐条 Warn、至多 MISS_LOG_MAX 条，其余合并为一条汇总：大名单可含几十种未收录
/// 截断别名，全量逐条会淹没日志。
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

    /// 真实导出样本（tests/testing_data）；缺失时跳过对应断言。
    fn real_rtf(name: &str) -> Option<std::path::PathBuf> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/testing_data")
            .join(name);
        p.is_file().then_some(p)
    }

    fn warn_messages(rx: &std::sync::mpsc::Receiver<LogLine>) -> Vec<String> {
        rx.try_iter()
            .filter(|l| l.level == "WARNING")
            .map(|l| l.message)
            .collect()
    }

    // ---------------- 合成样本（列位与表头词与真实导出一致，CI 可跑） ----------------

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

    const BASIC_EN_HEADER: &str = "| UID | Nat | 2nd Nat | Name |    |    |   |";
    const EXT_EN_HEADER: &str =
        "| UID | Nat | 2nd Nat | Name | | | | | | Is newgen | Gender | Age | Height | Club | Based | CA | PA | R.C.A |";

    const EXT_PLAYER_EN_SAMPLE: &str = "\
| UID | Nat | 2nd Nat | Name          |    |    |    |    |    | Is newgen | Gender | Age | Height | Club       | Based                           |     |     | R.C.A |
| 12315 | MKD | | Toni Micevski | 1 | 12 | 0 | 5 | | No | Male | 54 | 188 cm | Poeševo | North Macedonia (Fourth League) | 27 | 122 | 27 |
| 103607 | ENG | SCO | James Henry | 1 | 7 | 0 | 4 | | Yes | Male | 35 | 185 cm | Shrewsbury | England (Sky Bet League One) | 79 | 122 | 78 |
";

    const EXT_PLAYER_CN_SAMPLE: &str = "\
| 编号 | 国籍/地区籍 | 第二国籍/地区籍 | 姓名 | 头发长度 | 发色 | 种族码 | 肤色 | 头像 | 是新生球员 | 性别 | 年龄 | 身高 | 俱乐部 | 所在地 | 能力 | 潜力 | R.C.A |
| 55012106 | 葡萄牙 | 安哥拉 | 威廉·卡瓦略 | 1 | 15 | 3 | 17 | | 否 | 男 | 31 | 187厘米 | 皇家贝蒂斯 | 西班牙 (西甲联赛) | 148 | 156 | 148 |
| 67202350 | 西班牙 | | 丹尼尔·塞瓦略斯 | 1 | 11 | 1 | 11 | | 是 | 男 | 26 | 179厘米 | 皇家马德里 | 西班牙 (西甲联赛) | 150 | 163 | 150 |
";

    // 职员表头末列为 NP R.C.A（真实表头如此），与球员的 R.C.A 同为第 18 列数值。
    const EXT_STAFF_CN_SAMPLE: &str = "\
| 编号 | 国籍/地区籍 | 第二国籍/地区籍 | 姓名 | 头发长度 | 发色 | 种族码 | 肤色 | 头像 | 是新生球员 | 性别 | 年龄 | 身高 | 俱乐部 | 偏好工作 | 能力 | 潜力 | NP R.C.A |
| 2002243829 | 中国 | | 王晨 | 1 | 16 | 10 | 8 | | 是 | 男 | 62 | 182厘米 | | 足球总监 | 144 | 199 | 140 |
| 306951 | 巴西 | | 库卡 | 1 | 11 | 0 | 8 | | 否 | 男 | 68 | 182厘米 | 梅州客家 | 足球总监 | 142 | 160 | 138 |
";

    // 真实英文导出的表头中英混排（头发长度/发色等列名未翻译），判型只看第 1-4/10/15 列。
    const EXT_STAFF_EN_SAMPLE: &str = "\
| UID | Nat | 2nd Nat | Name | 头发长度 | 发色 | 种族码 | 肤色 | 头像 | Is newgen | Gender | Age | Height | Club | Preferred Job | CA | PA | NP R.C.A |
| 67214325 | KOR | ENG | 亚由美 | 1 | 7 | 0 | 8 | | No | Female | 26 | 167 cm | | Performance Analyst | 200 | 200 | 195 |
";

    #[test]
    fn detect_basic_formats() {
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(EN_SAMPLE), &logger).unwrap();
        assert_eq!(s.format, RtfFormat::Basic);
        assert_eq!(s.language, "English");
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(CN_SAMPLE), &logger).unwrap();
        assert_eq!(s.format, RtfFormat::Basic);
        assert_eq!(s.language, "简体中文");
        // 乱码：首个表行第 1 列不是「编号/UID」→ 报「未识别到表头行」。
        let err = RtfScanner::new()
            .detect_format(&write_rtf("乱七八糟没有数据行"), &logger)
            .unwrap_err();
        assert!(
            err.contains("RTF 格式不合法") && err.contains("表头"),
            "{err}"
        );
    }

    #[test]
    fn detect_ext_formats() {
        let logger = ChannelLogger::channel().0;
        let cases = [
            (EXT_PLAYER_EN_SAMPLE, RtfFormat::ExtPlayer, "English"),
            (EXT_PLAYER_CN_SAMPLE, RtfFormat::ExtPlayer, "简体中文"),
            (EXT_STAFF_CN_SAMPLE, RtfFormat::ExtStaff, "简体中文"),
            (EXT_STAFF_EN_SAMPLE, RtfFormat::ExtStaff, "English"),
        ];
        for (sample, format, language) in cases {
            let mut s = RtfScanner::new();
            s.detect_format(&write_rtf(sample), &logger).unwrap();
            assert_eq!(s.format, format);
            assert_eq!(s.language, language);
        }
    }

    #[test]
    fn parse_basic_english() {
        let path = write_rtf(EN_SAMPLE);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        // 基础名单全员随机生：keep_only_newgen 过滤不掉任何行。
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
    fn reject_unsupported_column_counts() {
        // 表头判为基础后，数据行 8-17 列（旧版 14 列详细、旧版 17 列职员拓展）与表头结构
        // 不符 → 抽查阶段整份拒绝，报「列数」。
        let logger = ChannelLogger::channel().0;
        let legacy_detailed = "| 2001234567| ESP | USA | Newgen Kid | 1 | 12 | 1 | 2 | 1 | FC Test | 19 | 180 | 75 | Yes |\n";
        let legacy_staff_ext = "| 67214325 | 否 | 女 | | 亚由美 | KOR | ENG | 26 | 167厘米 | | 表现分析师 | 10 | 8 | 1 | 16 | 200 | 200 |\n";
        for row in [legacy_detailed, legacy_staff_ext] {
            let err = RtfScanner::new()
                .detect_format(&write_rtf(&format!("{BASIC_EN_HEADER}\n{row}")), &logger)
                .unwrap_err();
            assert!(
                err.contains("RTF 格式不合法") && err.contains("列数"),
                "{err}"
            );
        }
    }

    #[test]
    fn invalid_rows_skipped() {
        // UID 非纯数字的行被当非数据行跳过；种族码越界的行按非法行跳过，合法行保留。
        let sample = "\
| UID       | Nat  | 2nd Nat | Name          |    |    |   |
| 2001234567| ESP  |         | Ok Player | 1  | 12 | 1 |
| abc123    | ESP  |         | Bad Uid   | 1  | 12 | 1 |
| 2001234569| ESP  |         | Bad Eth   | 1  | 12 | 99 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        let data = s
            .parse_roster(&path, true, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-2001234567");
    }

    #[test]
    fn translate_chinese_nationality() {
        let path = write_rtf(CN_SAMPLE);
        let logger = ChannelLogger::channel();
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger.0).unwrap();
        let mut tr = HashMap::new();
        tr.insert("简体中文".to_string(), {
            let mut m = HashMap::new();
            m.insert("美国".to_string(), "USA".to_string());
            m
        });
        let data = s.parse_roster(&path, true, &tr, &logger.0).unwrap();
        assert_eq!(data[0].primary_nat, "USA");
        assert!(warn_messages(&logger.1).is_empty());
    }

    #[test]
    fn translate_miss_keeps_value_and_logs_warning() {
        // 翻译表查不到的中文值（截断别名、未收录国名）→ 保留原值不丢行，记 WARNING；
        // 用表里必然没有的虚构国名验证缺失路径。
        let sample = "\
| 编号      | 国籍/地区籍 | 第二国籍 | 姓名        |    |    |   |
| 2002075172| 西兰公国   | 美国     | 扎克·布劳顿 | 3  | 9  | 0 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel();
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger.0).unwrap();
        let data = s
            .parse_roster(&path, false, &NAT_TRANSLATION, &logger.0)
            .unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].primary_nat, "西兰公国");
        assert_eq!(data[0].sec_nat, "USA");
        let warns = warn_messages(&logger.1);
        assert!(
            warns
                .iter()
                .any(|m| m.contains("西兰公国") && m.contains("保留原值")),
            "应有翻译缺失警告：{:?}",
            warns.first()
        );
        assert!(!warns.iter().any(|m| m.contains("美国")));
    }

    #[test]
    fn ext_player_detect_parse_and_full_fields() {
        let path = write_rtf(EXT_PLAYER_EN_SAMPLE);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
        assert_eq!(s.language, "English");
        // keep_only_newgen=true：只剩 Yes 行（r- 前缀）。
        let data = s
            .parse_roster(&path, true, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-103607");
        // 不过滤：No 行保留且为裸 UID，18 列全字段入库。
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 2);
        let first = &data[0];
        assert_eq!(first.uid, "12315"); // No（非随机生）：无前缀
        assert_eq!(first.primary_nat, "MKD"); // 国籍第 2 列（与基础同构）
        assert_eq!(first.sec_nat, "");
        assert_eq!(first.name, "Toni Micevski");
        assert_eq!(first.hair_length, "1"); // 头发长度第 5 列
        assert_eq!(first.hair_color, "12"); // 发色第 6 列
        assert_eq!(first.ethnicity_code, "0"); // 种族码第 7 列
        let d = first.detailed.as_ref().unwrap();
        assert_eq!(d.skin_code, "5"); // 肤色第 8 列
        assert_eq!(d.avatar, ""); // 头像第 9 列（FM 图片列，恒空）
        assert_eq!(d.is_newgan, "No"); // 是新生球员第 10 列
        assert_eq!(d.gender, "Male"); // 性别第 11 列
        assert_eq!(d.age, "54"); // 年龄第 12 列
        assert_eq!(d.height, "188 cm"); // 身高第 13 列
        assert_eq!(d.club, "Poeševo"); // 俱乐部第 14 列
        assert_eq!(d.based, "North Macedonia (Fourth League)"); // 所在地第 15 列
        assert_eq!(d.preferred_job, ""); // 球员无偏好工作
        assert_eq!(d.ca, "27"); // 第 16 列
        assert_eq!(d.pa, "122"); // 第 17 列
        assert_eq!(d.rca, "27"); // R.C.A 第 18 列
        assert!(d.extra_fields.is_empty());
    }

    #[test]
    fn ext_player_cn_detect_parse_and_gender() {
        // 标志 是 → r- 前缀、否 → 裸 UID；国籍经翻译节点归一化三字码；性别/所在地入库。
        let path = write_rtf(EXT_PLAYER_CN_SAMPLE);
        let logger = ChannelLogger::channel();
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger.0).unwrap();
        assert_eq!(
            (s.format, s.language.as_str()),
            (RtfFormat::ExtPlayer, "简体中文")
        );
        let data = s
            .parse_roster(&path, false, &NAT_TRANSLATION, &logger.0)
            .unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].uid, "55012106");
        assert_eq!(data[1].uid, "r-67202350");
        assert_eq!(data[0].primary_nat, "POR"); // 葡萄牙 → POR
        assert_eq!(data[1].primary_nat, "ESP");
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(d.gender, "男");
        assert_eq!(d.based, "西班牙 (西甲联赛)");
        assert_eq!(d.preferred_job, "");
        assert_eq!(d.rca, "148");
        assert!(d.extra_fields.is_empty());
        assert!(warn_messages(&logger.1).is_empty()); // 全部命中翻译表
    }

    #[test]
    fn ext_player_extra_columns_stored() {
        // 第 18 列之后追加用户自定义列 → 仍合法，原文进 extra_fields。
        let sample = "\
| UID | Nat | 2nd Nat | Name | | | | | | Is newgen | Gender | Age | Height | Club | Based | CA | PA | R.C.A |
| 12315 | MKD | | Toni Micevski | 1 | 12 | 0 | 5 | | No | Male | 54 | 188 cm | Poeševo | North Macedonia (Fourth League) | 27 | 122 | 27 | 惯用脚 | 左 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(
            data[0].detailed.as_ref().unwrap().extra_fields,
            vec!["惯用脚".to_string(), "左".to_string()]
        );
    }

    #[test]
    fn ext_row_with_empty_cells_kept() {
        // 未加载对象的整段属性为空白（肤色/标志/性别/…/R.C.A 全空，实测
        // ext_staff_short_chn 37 行）→ 空值原样入库、行不丢；空标志按非随机生（裸 UID）。
        let sample = "\
| 编号 | 国籍/地区籍 | 第二国籍/地区籍 | 姓名 | 头发长度 | 发色 | 种族码 | 肤色 | 头像 | 是新生球员 | 性别 | 年龄 | 身高 | 俱乐部 | 偏好工作 | 能力 | 潜力 | NP R.C.A |
| 67268144 | 西班牙 | | | 1 | 3 | 1 | | | | | | | | | | | |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 1);
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(data[0].uid, "67268144");
        assert_eq!(d.skin_code, "");
        assert_eq!(d.gender, "");
        assert_eq!(d.rca, "");
        assert_eq!(d.preferred_job, "");
        assert_eq!(d.based, "");
    }

    #[test]
    fn ext_header_discriminates() {
        // 表头驱动判型：第 10 列标志词分基础/拓展轨道，第 15 列判别词分球员/职员；
        // 身份列/标志列/判别词任一不合法 → 整份拒绝并精确到列。
        let logger = ChannelLogger::channel().0;
        let row18 = "| 12315 | MKD | | X | 1 | 12 | 0 | 5 | | No | 男 | 54 | 188 cm | C | B | 27 | 122 | 27 |";
        let header_based = EXT_EN_HEADER.to_string();
        let header_job = header_based.replace("Based", "Preferred Job");
        for header in [
            header_based.clone(),
            header_based.replace("Based", "所在地"),
        ] {
            let mut s = RtfScanner::new();
            s.detect_format(&write_rtf(&format!("{header}\n{row18}\n")), &logger)
                .unwrap();
            assert_eq!(s.format, RtfFormat::ExtPlayer);
        }
        for header in [
            header_job.clone(),
            header_job.replace("Preferred Job", "偏好工作"),
        ] {
            let mut s = RtfScanner::new();
            s.detect_format(&write_rtf(&format!("{header}\n{row18}\n")), &logger)
                .unwrap();
            assert_eq!(s.format, RtfFormat::ExtStaff);
        }
        // 无表头（数据行第 1 列是数字，不会命中「编号/UID」）→ 报「表头」定位。
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{row18}\n")), &logger)
            .unwrap_err();
        assert!(
            err.contains("RTF 格式不合法") && err.contains("表头"),
            "{err}"
        );
        // 第 15 列不是判别词 → 报表头第 15 列。
        let bad15 = header_based.replace("Based", "俱乐部");
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{bad15}\n{row18}\n")), &logger)
            .unwrap_err();
        assert!(err.contains("表头第 15 列"), "{err}");
        // 18 列表头但第 10 列不是标志列 → 不能判 Basic（基础名单恒 7 列），报表头第 10 列。
        let bad10 = header_based.replace("Is newgen", "随机人");
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{bad10}\n{row18}\n")), &logger)
            .unwrap_err();
        assert!(err.contains("表头第 10 列"), "{err}");
        // 身份列不匹配（第 2 列不是国籍）→ 报表头第 2 列。
        let bad2 = header_based.replace("| Nat |", "| 俱乐部 |");
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{bad2}\n{row18}\n")), &logger)
            .unwrap_err();
        assert!(err.contains("表头第 2 列"), "{err}");
        // 数据行抽查：R.C.A 非空非数字 → 定位「第 18 列」（字段校验先于放行）。
        let err = RtfScanner::new()
            .detect_format(
                &write_rtf(&format!(
                    "{header_based}\n| 12315 | MKD | | X | 1 | 12 | 0 | 5 | | No | 男 | 54 | 188 cm | C | B | 27 | 122 | 12a |\n"
                )),
                &logger,
            )
            .unwrap_err();
        assert!(
            err.contains("RTF 格式不合法") && err.contains("第 18 列"),
            "{err}"
        );
        // 17 列数据行与拓展表头不符 → 列数不合法。
        let row17 =
            "| 12315 | No | | X | MKD | | 54 | 188 cm | C | B | 0 | 5 | 1 | 12 | 27 | 122 | 27 |";
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{header_based}\n{row17}\n")), &logger)
            .unwrap_err();
        assert!(err.contains("列数"), "{err}");
    }

    #[test]
    fn ext_staff_detect_parse_and_full_fields() {
        let path = write_rtf(EXT_STAFF_CN_SAMPLE);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        assert_eq!(s.language, "简体中文");
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 2);
        // 是 → r- 前缀；否 → 裸 UID。
        assert_eq!(data[0].uid, "r-2002243829");
        assert_eq!(data[1].uid, "306951");
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(d.preferred_job, "足球总监"); // 第 15 列仅职员
        assert_eq!(d.based, "");
        assert_eq!(d.gender, "男"); // 第 11 列
        assert_eq!(d.club, ""); // 王晨无俱乐部
        assert_eq!(d.age, "62"); // 第 12 列
        assert_eq!(d.height, "182厘米"); // 第 13 列
        assert_eq!(d.ca, "144");
        assert_eq!(d.pa, "199");
        assert_eq!(d.rca, "140"); // NP R.C.A 同为第 18 列
        assert_eq!(d.avatar, "");
        // 英文导出：表头中英混排不影响判型；性别 Male/Female 原样入库。
        let path = write_rtf(EXT_STAFF_EN_SAMPLE);
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        assert_eq!(s.language, "English");
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
        // 头像列有内容时原文入库；第 19 列起用户自定义列进 extra_fields。
        let sample = "\
| 编号 | 国籍/地区籍 | 第二国籍/地区籍 | 姓名 | 头发长度 | 发色 | 种族码 | 肤色 | 头像 | 是新生球员 | 性别 | 年龄 | 身高 | 俱乐部 | 偏好工作 | 能力 | 潜力 | NP R.C.A |
| 306951 | 巴西 | | 库卡 | 1 | 11 | 0 | 8 | face_kuka | 否 | 男 | 68 | 182厘米 | 梅州客家 | 足球总监 | 142 | 160 | 138 | 合同年 | 2027 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
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
    fn uid_length_boundaries() {
        // 3 位 / 11 位 → 非法行；4 位 / 10 位 → 合法。
        let sample = "\
| UID       | Nat  | 2nd Nat | Name          |    |    |   |
| 123| ESP  |     | Short Uid  | 1  | 12 | 1 |
| 12345678901| ESP |    | Long Uid   | 1  | 12 | 1 |
| 1234| ESP  |     | Min Uid    | 1  | 12 | 1 |
| 1234567890| ESP  |     | Max Uid    | 1  | 12 | 1 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        let data = s
            .parse_roster(&path, false, &HashMap::new(), &logger)
            .unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].uid, "r-1234");
        assert_eq!(data[1].uid, "r-1234567890");
        // 整份只有越界 UID 行 → 报「编号(UID)」定位。
        let err = RtfScanner::new()
            .detect_format(
                &write_rtf(&format!(
                    "{BASIC_EN_HEADER}\n| 123| ESP | | X | 1 | 12 | 1 |\n"
                )),
                &logger,
            )
            .unwrap_err();
        assert!(err.contains("编号(UID)"), "{err}");
    }

    #[test]
    fn nationality_form_checks() {
        // 三位大写码 / 含汉字 / 英文全名 → 合法；单字母 / 纯数字 → 不合法。
        // 未收录翻译表的英文全名（海外华侨 'SC'）→ 保留原值 + 警告。
        let sample = "\
| UID       | Nat  | 2nd Nat | Name          |    |    |   |
| 2000472008| SGP |         | Ok Code Nat    | 1 | 12 | 0 |
| 2000472009| 北马其 |        | Ok Cjk Nat     | 1 | 12 | 0 |
| 2000472010| SC  |        | Ok Long Nat    | 1 | 12 | 0 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel();
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger.0).unwrap();
        let data = s
            .parse_roster(&path, false, &NAT_TRANSLATION, &logger.0)
            .unwrap();
        assert_eq!(data.len(), 3);
        assert_eq!(data[2].primary_nat, "SC");
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
                .detect_format(&write_rtf(&format!("{BASIC_EN_HEADER}\n{bad}")), &logger.0)
                .unwrap_err();
            assert!(err.contains("国籍"), "{err}");
        }
    }

    #[test]
    fn long_form_nationality_translated() {
        // long 形态名单：语言判 English，解析后经 English 节点归一化为三字码。
        let sample = "\
| UID       | Nat                  | 2nd Nat              | Name           |    |    |   |
| 7458500   | Argentina            | U.S.A.               | Lionel Messi   | 1  | 11 | 1 |
| 29179241  | Norway               | Bosnia & Herzegovina | Erling Haaland | 1  | 7  | 0 |
| 2000197505| Hong Kong (China PR) | Nigeria              | Chan Tai Man   | 1  | 7  | 0 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel();
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger.0).unwrap();
        assert_eq!(s.language, "English");
        let data = s
            .parse_roster(&path, true, &NAT_TRANSLATION, &logger.0)
            .unwrap();
        assert_eq!(data.len(), 3);
        assert_eq!(data[0].primary_nat, "ARG");
        assert_eq!(data[0].sec_nat, "USA"); // U.S.A. → USA
        assert_eq!(data[1].sec_nat, "BIH"); // Bosnia & Herzegovina → BIH
        assert_eq!(data[2].primary_nat, "HKG"); // 带括号后缀显示名 → HKG
        assert!(warn_messages(&logger.1).is_empty());
    }

    #[test]
    fn out_of_range_codes_rejected() {
        // 非空但越界的数值码 → 整份不合法并定位到字段；校验顺序与列序一致。
        let logger = ChannelLogger::channel().0;
        for (header, row, field) in [
            (
                BASIC_EN_HEADER,
                "| 2000472008| ESP | | X | 1 | 12 | 11 |",
                "种族码",
            ),
            (
                EXT_EN_HEADER,
                "| 12315 | MKD | | X | 6 | 12 | 0 | 5 | | No | 男 | 54 | 188 cm | C | B | 27 | 122 | 27 |",
                "头发长度码",
            ),
            (
                EXT_EN_HEADER,
                "| 12315 | MKD | | X | 1 | 0 | 0 | 5 | | No | 男 | 54 | 188 cm | C | B | 27 | 122 | 27 |",
                "头发颜色码",
            ),
            (
                EXT_EN_HEADER,
                "| 12315 | MKD | | X | 1 | 12 | 11 | 5 | | No | 男 | 54 | 188 cm | C | B | 27 | 122 | 27 |",
                "种族码",
            ),
            (
                EXT_EN_HEADER,
                "| 12315 | MKD | | X | 1 | 12 | 0 | 20 | | No | 男 | 54 | 188 cm | C | B | 27 | 122 | 27 |",
                "肤色码",
            ),
        ] {
            let err = RtfScanner::new()
                .detect_format(&write_rtf(&format!("{header}\n{row} |\n")), &logger)
                .unwrap_err();
            assert!(err.contains(field), "{field}: {err}");
        }
        // 发长 = 5 是合法上界（真实数据存在）。
        let ok = "\
| UID | Nat | 2nd Nat | Name | | | | | | Is newgen | Gender | Age | Height | Club | Based | CA | PA | R.C.A |
| 12315 | MKD | | X | 5 | 12 | 0 | 5 | | No | 男 | 54 | 188 cm | C | B | 27 | 122 | 27 |
";
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(ok), &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
    }

    // ---------------- 真实导出样本（行数与关键值由夹具实测得出） ----------------

    #[test]
    fn real_basic_files() {
        // 1599 行全员随机生；国籍显示名（含中文全名/英文全名）全部归一化为三字码。
        for (name, language) in [
            ("basic_long.rtf", "English"),
            ("basic_long_chn.rtf", "简体中文"),
        ] {
            let Some(f) = real_rtf(name) else { continue };
            let logger = ChannelLogger::channel().0;
            let mut s = RtfScanner::new();
            s.detect_format(&f, &logger).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::Basic, language),
                "{name}"
            );
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger)
                .unwrap();
            assert_eq!(data.len(), 1599, "{name}");
            assert!(
                data.iter().all(|r| r.uid.starts_with(NEWGAN_UID_PREFIX)),
                "{name}"
            );
            assert!(
                data.iter().all(|r| !contains_cjk(&r.primary_nat)),
                "{name}: 国籍应全部归一化"
            );
        }
    }

    #[test]
    fn real_player_ext_files() {
        // 4 份球员拓展名单：首行同为路易斯·迪亚斯（否/No → 裸 UID），short 版国籍为列宽
        // 截断别名（哥伦比），翻译表已收录，归一化结果与 long 版一致。
        for (name, language, club, based, gender) in [
            (
                "ext_player_long.rtf",
                "English",
                "Liverpool",
                "England (Premier League)",
                "Male",
            ),
            (
                "ext_player_long_chn.rtf",
                "简体中文",
                "利物浦",
                "英格兰 (超级联赛)",
                "男",
            ),
            (
                "ext_player_short.rtf",
                "English",
                "Liverpool",
                "England (Premier League)",
                "Male",
            ),
            (
                "ext_player_short_chn.rtf",
                "简体中文",
                "利物浦",
                "英格兰 (超级联赛)",
                "男",
            ),
        ] {
            let Some(f) = real_rtf(name) else { continue };
            let logger = ChannelLogger::channel().0;
            let mut s = RtfScanner::new();
            s.detect_format(&f, &logger).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::ExtPlayer, language),
                "{name}"
            );
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger)
                .unwrap();
            assert_eq!(data.len(), 1599, "{name}");
            let first = &data[0];
            assert_eq!(first.uid, "76049803", "{name}");
            assert_eq!(first.primary_nat, "COL", "{name}"); // 哥伦比亚/Colombia → COL
            let d = first.detailed.as_ref().unwrap();
            assert_eq!(
                (d.club.as_str(), d.based.as_str(), d.gender.as_str()),
                (club, based, gender),
                "{name}"
            );
            assert_eq!(d.rca, "121", "{name}");
            // 随机生（是/Yes）与非随机生（否/No）同时存在。
            assert!(
                data.iter().any(|r| r.uid.starts_with(NEWGAN_UID_PREFIX))
                    && data.iter().any(|r| !r.uid.starts_with(NEWGAN_UID_PREFIX)),
                "{name}"
            );
        }
    }

    #[test]
    fn real_staff_ext_files() {
        // 4 份职员拓展名单：398 数据行中 1 行 UID 越界被跳过；short_chn 版另有整段属性
        // 空白的未加载对象行（肤色/R.C.A 空），空值容忍后全部入库。
        for (name, language) in [
            ("ext_staff_long.rtf", "English"),
            ("ext_staff_long_chn.rtf", "简体中文"),
            ("ext_staff_short.rtf", "English"),
            ("ext_staff_short_chn.rtf", "简体中文"),
        ] {
            let Some(f) = real_rtf(name) else { continue };
            let logger = ChannelLogger::channel().0;
            let mut s = RtfScanner::new();
            s.detect_format(&f, &logger).unwrap();
            assert_eq!(
                (s.format, s.language.as_str()),
                (RtfFormat::ExtStaff, language),
                "{name}"
            );
            let data = s
                .parse_roster(&f, false, &NAT_TRANSLATION, &logger)
                .unwrap();
            assert_eq!(data.len(), 397, "{name}");
            assert!(
                data.iter()
                    .all(|r| r.detailed.as_ref().unwrap().based.is_empty()),
                "{name}: 职员不该有所在地"
            );
        }
    }

    #[test]
    fn real_wrong_files() {
        // UID 边界：4/7/10 位保留 3 行，11 位行跳过。
        if let Some(f) = real_rtf("test_wrong_UID.rtf") {
            let logger = ChannelLogger::channel().0;
            let mut s = RtfScanner::new();
            s.detect_format(&f, &logger).unwrap();
            let data = s.parse_roster(&f, false, &HashMap::new(), &logger).unwrap();
            assert_eq!(data.len(), 3);
            assert_eq!(data[0].uid, "r-191571");
            assert_eq!(data[1].uid, "r-1915576430");
            assert_eq!(data[2].uid, "r-1915576");
        }
        if let Some(f) = real_rtf("test_wrong_ethnicity.rtf") {
            let logger = ChannelLogger::channel().0;
            let mut s = RtfScanner::new();
            s.detect_format(&f, &logger).unwrap();
            let data = s.parse_roster(&f, false, &HashMap::new(), &logger).unwrap();
            assert_eq!(data.len(), 29);
        }
    }

    #[test]
    fn real_invalid_fixtures_rejected() {
        for name in ["invalid_garbage.rtf"] {
            if let Some(f) = real_rtf(name) {
                let logger = ChannelLogger::channel().0;
                let err = RtfScanner::new().detect_format(&f, &logger).unwrap_err();
                assert!(err.contains("RTF 格式不合法"), "{name}: {err}");
            }
        }
    }
}
