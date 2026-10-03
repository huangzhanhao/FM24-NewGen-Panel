//! FM（Football Manager）名单 RTF 扫描器：识别导出视图的列结构，再把每一行解析成记录。
//!
//! 【模块职责】
//! 只负责两件事：识别名单属于哪种列结构，以及把每行解析成 `PlayerRecord`。写数据库、加载翻译表、
//! 界面展示都不在这里，由调用方（commands 层）完成。
//!
//! 【使用步骤与生命周期】
//! 识别和解析是分开的两步，必须按顺序使用同一个 `RtfScanner` 实例：
//!   1. 用 `new()` 创建一个空状态；
//!   2. 调用 `detect_format()`，它只读文件开头部分，识别出列结构和语言，成功后状态才变为有效；
//!   3. 调用 `parse_roster()` 做完整解析；如果还没识别过，它会直接报错。
//! 扫描器内部只保存识别结果（一个布尔值、一个字符串和一个枚举），不缓存文件内容，也没有全局可变
//! 状态，因此可以放心地在不同地方传递。但“先识别、后解析”的顺序需要调用方自己保证。
//!
//! 【格式说明】
//! 格式以真实导出文件（tests/testing_data/）为准。共有 3 种列结构 × 2 种数据语言 = 6 种情况：
//! - Basic 基础名单：7 列（编号、国籍、第二国籍、姓名、头发长度、发色、种族码），从「is newgen」
//!   过滤视图导出，所有记录都是随机人球员。
//! - ExtPlayer / ExtStaff 拓展名单：至少 18 列，前 7 列与 Basic 相同，第 8-18 列依次为肤色、头像、
//!   是新生球员、性别、年龄、身高、俱乐部、所在地（球员）或偏好工作（职员）、能力、潜力、R.C.A；
//!   第 19 列起是用户自己添加的列。
//! 国籍的权威来源是 nat_translation.json（显示文本 → 码）与 nat_codes.json（码全集）：解析时统一
//! 换成三字码。两个表都查不到的值判为非法，直接丢弃该行并记日志（用户可据日志自行增删翻译表）。
//!
//! 【行号与列号的计算方式】
//! 每个单元格向量的第 0 个位置固定存放“行号”，所以表格的第 k 列正好落在下标 k（下标 0 留给了
//! 行号），报错信息里的「第 N 列」可以直接用这个下标，不需要换算。
//! 行号只统计真正的表格行：第 0 行是表头，第 1 行是第 1 条数据……空行和分隔线不算表格行、也不占用
//! 行号，所以报错信息里的「第 N 行」就是第 N 条数据。
//!
//! 【如何判断格式】
//! 判断以表头为依据——表头是导出视图列结构的正式说明，一旦判断出错也能精确指出是哪一列出问题，
//! 同时还能顺便确认这个文件确实是本工具约定的视图：第 0 个表格行（表头）的第 1 列必须包含
//! “编号/UID”，然后依次判断 ① 列数（7 列或 ≥18 列）→ ② 第 1-4 列是否为编号/国籍/第二国籍/姓名
//! → ③ 第 10 列是否为“是新生球员”（用来区分基础还是拓展）→ ④ 拓展名单再看第 15 列是“偏好工作”
//! 还是“所在地”（用来区分职员还是球员），具体见 `header_format`。
//! 判断完格式后，还会抽查前 20 条数据行，要求至少有 1 条符合该格式，避免表头和数据行错位的文件
//! 被悄悄解析成空名单。
//! 数据语言不看表头（英文拓展名单的表头是中英混排的），而是拿数据行的国籍列去查翻译表：命中哪个
//! 语言节点就判为哪种语言；一个都没命中时默认 English（例如国籍已经是三字码，无需再判断）。
//!
//! 【错误处理的分工】
//! detect 严格：整份文件级别的问题（没有表头、表头不合法、没有一条合法数据行）都会报错，错误信息
//! 直接展示给用户，同时写入 Error 日志。
//! parse 也严格：行级问题（国籍查不到、字段不合法）会丢弃该行并逐行记 Warn 日志，便于用户核对并
//! 增删翻译表；整份文件是否合法已经在 detect 阶段检查过了。
use crate::logger::LogSink;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// 随机人 UID 的固定前缀。Replace 流程和前端都靠它区分随机人记录和原生记录，修改这个前缀会造成
/// 不兼容的破坏性变更。
pub const NEWGAN_UID_PREFIX: &str = "r-";

/// FM 中文导出在按列宽折行时会混入的不可见字符（U+200B），必须在切分前去掉。
const ZERO_WIDTH_SPACE: &str = "\u{200b}";

/// 识别格式时最多读取的字节数（64 KiB）。表头和数据行一定在文件开头，所以上百 MB 的大名单不需要
/// 全部读入。
const HEAD_BYTES: u64 = 64 * 1024;

/// 判断格式之后抽查的数据行数。至少要有一条符合判断出的格式，才认为这份文件有效。
const DETECT_LINE_LIMIT: usize = 20;

/// 解析阶段逐行记「跳过该行」日志的上限，超出部分合并成一条汇总，避免大名单把日志刷爆。
const DROP_LOG_MAX: usize = 50;

/// 随机人标志的正面取值。实测取值有“是/否”（中文导出）和“Yes/No”（英文导出）；这里用包含匹配而
/// 不是完全相等，是为了容忍值前后可能带的修饰文字。
const NEWGAN_FLAGS: [&str; 2] = ["是", "Yes"];

/// 基础名单的列数，同时也是拓展名单中与它完全相同的前 7 列的长度。
const BASIC_FIELD_COUNT: usize = 7;

/// 拓展名单的标准列数；第 19 列起是用户自定义列。
const EXT_FIELD_COUNT: usize = 18;

// 各数值码的合法范围，取值范围来自真实导出文件的统计。发色没有 0 值，所以从 1 开始。
const ETHNICITY_RANGE: (u32, u32) = (0, 10);
const SKIN_RANGE: (u32, u32) = (0, 19);
const HAIR_LENGTH_RANGE: (u32, u32) = (0, 5);
const HAIR_COLOR_RANGE: (u32, u32) = (1, 19);

// 下面的常量是列下标。因为单元格向量的第 0 个位置存放行号，所以表格的第 k 列正好落在下标 k
// （下标 0 留给了行号），报错信息里的「第 N 列」直接用下标即可。Basic 的 7 列和拓展名单的前 7 列
// 完全相同。
const UID_COL: usize = 1;
const NAT_COL: usize = 2; // 国籍。两种结构都在这一列，判断语言也看它
const SEC_NAT_COL: usize = 3;
const NAME_COL: usize = 4;
const HAIR_LENGTH_COL: usize = 5;
const HAIR_COLOR_COL: usize = 6;
const ETHNICITY_COL: usize = 7;

// 拓展名单特有的列。是基础名单还是拓展名单由表头判断决定，这些列本身不参与结构判断，
// 只在字段校验和组装记录时取值。
const EXT_SKIN_COL: usize = 8;
const EXT_AVATAR_COL: usize = 9; // 头像列。FM 的图片列，导出时为空文本
const EXT_NEWGAN_FLAG_COL: usize = 10; // “是新生球员”，值为 是/否 或 Yes/No；判断格式时也看这一列
const EXT_GENDER_COL: usize = 11;
const EXT_AGE_COL: usize = 12;
const EXT_HEIGHT_COL: usize = 13;
const EXT_CLUB_COL: usize = 14;
const EXT_BASED_COL: usize = 15; // 球员是“所在地”（俱乐部所在赛事级别），职员是“偏好工作”；判断球员/职员时看这一列
const EXT_CA_COL: usize = 16;
const EXT_PA_COL: usize = 17;
const EXT_RCA_COL: usize = 18; // 职员表头写作 NP R.C.A，同样是数值列
const EXT_EXTRA_COL: usize = 19; // 第 19 列起是用户自定义列（extra_fields 的起点）

/// 表头第 1-4 列应该包含的关键字，以及出错时使用的列名。
/// 匹配时会先转成小写再判断“是否包含”：真实表头是「国籍/地区籍」「2nd Nat」这种带修饰的显示名，
/// 用完全相等会漏掉。
const HEADER_IDENTITY: [(&str, [&str; 2]); 4] = [
    ("编号/UID", ["编号", "uid"]),
    ("国籍/Nat", ["国籍", "nat"]),
    ("第二国籍/2nd Nat", ["第二国籍", "2nd nat"]),
    ("姓名/Name", ["姓名", "name"]),
];
/// 表头第 10 列的关键字：只有拓展名单才有“是新生球员”这一列（真实表头有中文、英文两种写法）。
const HEADER_FLAG: (&str, [&str; 2]) = ("是新生球员/Is newgen", ["是新生球员", "is newgen"]);
/// 表头第 15 列用于判断球员的关键字（真实表头是中文「所在地」或英文「Based」，英文大小写不固定）。
const PLAYER_BASED_HEADERS: [&str; 2] = ["based", "所在地"];
/// 表头第 15 列用于判断职员的关键字（真实表头是「偏好工作」或「Preferred Job」）。
const STAFF_JOB_HEADERS: [&str; 2] = ["preferred job", "偏好工作"];

/// 名单的列结构。数据语言不在这里（它根据国籍列查翻译表判断），见 `RtfScanner::language`。
///
/// 序列化身份：该枚举直接 derive Serialize（camelCase 变体名）作为前端业务判定的协议码
/// （basic/extPlayer/extStaff），前端不得再拿 label 的中文文案做判定——
/// 重命名变体等于显式改协议，编译器会逼你处理所有 match 分支；而改 label 文案不影响逻辑。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RtfFormat {
    /// 基础名单：7 列，从「is newgen」过滤视图导出，所有记录都是随机人。
    #[default]
    Basic,
    /// 拓展球员：18 个标准列，第 15 列是所在地（俱乐部所在赛事级别）。
    ExtPlayer,
    /// 拓展职员：列结构与拓展球员相同，但第 15 列是偏好工作，靠表头区分。
    ExtStaff,
}

impl RtfFormat {
    /// 返回中文名称，用于日志和摘要展示（仅展示，不作协议判据）。
    pub fn label(self) -> &'static str {
        match self {
            RtfFormat::Basic => "基础名单",
            RtfFormat::ExtPlayer => "拓展球员名单",
            RtfFormat::ExtStaff => "拓展职员名单",
        }
    }
}

/// 一次名单解析的摘要（格式 / 语言 / 条数 / 随机人数）。
/// 【为何单独成类型】「扫描RTF」命令与「替换」流水线都会解析名单，两处要给出同一份摘要：
/// 替换成功时随 replace:finished 事件回传前端，名单预览因此不必再点一次「扫描RTF」就能
/// 知道名单格式（拓展名单才有的「随机人」筛选与职员列表头都靠它）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RosterSummary {
    pub total: usize,         // 名单总条数
    pub newgan: usize,        // 随机人（r- 前缀）条数
    pub detailed: bool,       // 是否含拓展信息（拓展名单为 true）
    pub format: &'static str, // 格式中文名（RtfFormat::label，仅展示/日志）
    /// 格式协议码：RtfFormat 枚举自身的序列化值（camelCase 变体名），前端业务判定只认它。
    pub format_code: RtfFormat,
    pub language: String, // "English" / "简体中文"
}

impl RosterSummary {
    /// 由解析完的扫描器状态 + 记录列表汇总（「扫描RTF」与替换流水线共用同一套口径）。
    pub fn from_records(scanner: &RtfScanner, data: &[PlayerRecord]) -> Self {
        Self {
            total: data.len(),
            newgan: data
                .iter()
                .filter(|p| p.uid.starts_with(NEWGAN_UID_PREFIX))
                .count(),
            detailed: data.iter().any(|p| p.detailed.is_some()),
            format: scanner.format.label(),
            format_code: scanner.format,
            language: scanner.language.clone(),
        }
    }
}

/// 一次名单解析的行数账目（只供日志用，不进前端摘要）。
/// 【为什么单独成类型】本工具的职责就是「N 个人 → N 张脸」，所以「多少行没变成记录」必须和
/// 「多少条记录」一样可见。此前这些计数只活在 debug 行里，而生产阈值是 Info（lib.rs 的
/// `LevelFilter::Info`），于是 1600 行名单丢了 500 行，用户看到的仍是「解析完成：1100 条」。
#[derive(Debug, Default, Clone, Copy)]
pub struct RosterCounts {
    /// 表行总数（进度分母；含表头行，不含杂行/分隔线，口径同 `table_rows`）。
    pub rows: usize,
    /// 解析成功且通过过滤、最终存活的记录条数。
    pub kept: usize,
    /// 按「仅随机人」口径过滤掉的行数（不是错误，是筛选）。
    pub filtered: usize,
    /// 校验失败被跳过的行数（多为国籍不在翻译表里，逐行有 Warn）。
    pub dropped: usize,
    /// 表头之后「编号列根本不是纯数字」而被整行忽略的行数（历史上既无计数也无日志）。
    pub stray: usize,
}

impl RosterCounts {
    /// 日志尾注：`（名单 1600 行：保留 1100，过滤 0，跳过 480，非数据行 20）`。
    /// 四个计数与 rows 的关系由调用方日志承载，这里只负责口径统一（「扫描RTF」与替换流水线同一条）。
    pub fn log_tail(&self) -> String {
        format!(
            "（名单 {} 行：保留 {}、按仅随机人过滤 {}、跳过不合法 {}、编号列非数字 {}）",
            self.rows, self.kept, self.filtered, self.dropped, self.stray
        )
    }
}

/// 拓展名单第 8-18 列以及用户自定义列的原始文本；序列化时用 camelCase，与前端 types.ts 保持一致。
/// `based` 和 `preferred_job` 不会同时有值——第 15 列按判断结果只有一种含义，另一种始终为空。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetailedInfo {
    pub skin_code: String,         // 肤色码（第 8 列）
    pub avatar: String,            // 头像列原文（第 9 列）；FM 的图片列导出时为空文本，照样原样保存
    pub is_newgan: String, // 随机人标志原文（第 10 列）；不转成布尔值，因为译文写法随导出语言变化
    pub gender: String,    // 性别原文（第 11 列），如 男/女、Male/Female
    pub age: String,       // 年龄（第 12 列）
    pub height: String,    // 身高（第 13 列）
    pub club: String,      // 俱乐部（第 14 列）
    pub based: String,     // 所在地/俱乐部所在赛事级别（第 15 列，仅球员有）
    pub preferred_job: String, // 偏好工作（第 15 列，仅职员有）
    pub ca: String,        // 当前能力 CA（第 16 列）
    pub pa: String,        // 潜力 PA（第 17 列）
    pub rca: String,       // 推荐能力 R.C.A（第 18 列，球员和职员都有）
    pub extra_fields: Vec<String>, // 用户自定义列原文（第 19 列起），原样保存，不做解释也不丢弃
}

/// 一条名单记录（球员和职员通用）。`uid` 上的 r- 前缀是随机人标记，Replace 流程和前端都依赖它；
/// `primary_nat`/`sec_nat` 解析时已统一换成三字码（查不到国籍的行已被丢弃，不会产生记录）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRecord {
    pub uid: String,
    pub primary_nat: String,
    pub sec_nat: String,
    pub name: String,
    pub hair_length: String,
    pub hair_color: String,
    pub ethnicity_code: String, // 取值 "0".."10"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detailed: Option<DetailedInfo>, // 只有拓展名单才有；为空时不序列化，避免前端看到空的 detailed
}

/// 单行判断的结果。`Rejected` 会带上具体出错的字段，方便 detect 在一条合法数据行都没有时，
/// 生成用户看得懂的报错信息。
enum RowVerdict {
    Data,
    NotDataRow,
    Rejected { field: &'static str, detail: String },
}

/// 扫描器状态：保存 `detect_format` 得到的识别结果，供后面的解析阶段使用。
///
/// 使用顺序是“先识别、后解析”：只有 `detect_format` 成功后（`is_valid` 被置为 true），
/// `parse_roster` 才能调用，否则会直接报错。它只保存识别结论、不缓存文件内容，是普通的值类型，
/// 可以放心传递。
pub struct RtfScanner {
    pub is_valid: bool,    // parse_roster 的入口校验：没识别过就解析会直接报错
    pub language: String,  // "English" / "简体中文" / "Unknown"（未识别时）；供摘要展示
    pub format: RtfFormat, // 列结构的判断结果；parse 按它取列组装，commands 层直接展示
}

impl RtfScanner {
    /// 创建一个空状态：尚未识别（`is_valid = false`），语言为 `"Unknown"`，结构取默认值 `Basic`。
    ///
    /// 必须先用 `detect_format` 识别成功，`parse_roster` 才会放行。
    pub fn new() -> Self {
        Self {
            is_valid: false,
            language: "Unknown".into(),
            format: RtfFormat::default(),
        }
    }

    /// 识别格式和语言（只读文件开头，不做完整解析）。
    ///
    /// 会在文件开头 HEAD_BYTES 字节内完成表头判断，并抽查前 DETECT_LINE_LIMIT 条数据行。
    ///
    /// 识别成功后 `parse_roster` 才能调用。出错信息直接展示给用户（前端原样弹窗），同时写入
    /// Error 日志备份：表头问题会精确到「表头第 N 列」，数据行问题会定位到「第 N 行「字段」」。
    pub fn detect_format(&mut self, path: &Path, logger: &impl LogSink) -> Result<(), String> {
        // 扫描启动：Trace 级执行细节（函数入口）。Info 级摘要由调用方与「解析完成」行给出，
        // 扫描器自身不再逐阶段记 Info，避免同一状态重复刷屏。
        logger.trace("rtf", format!("开始识别 RTF 格式：{}", path.display()));
        self.is_valid = false;
        self.language = "Unknown".into();
        self.format = RtfFormat::default();
        // 文件读取失败也要留痕：此前这里的 Err 直接返回、没有日志，用户只看到弹窗。
        let text = read_head(path, HEAD_BYTES).map_err(|e| {
            logger.write(log::Level::Error,"rtf", format!("RTF 格式识别失败（读取文件 {}）：{e}", path.display()),
            );
            e
        })?;
        self.detect(&text).map_err(|e| {
            logger.write(log::Level::Error, "rtf", format!("RTF 格式识别失败（{}）：{e}", path.display()),
            );
            e
        })?;
        // 匹配结果：识别成功后的列结构与语言（Debug 级）。这份状态已并入「解析完成」汇总行，
        // 单独记 Info 只会让一次扫描出现两条内容相近的日志。
        logger.debug("rtf", format!(
                "RTF 格式识别完成：{}（语言 {}）",
                self.format.label(),
                self.language
            ),
        );
        Ok(())
    }

    /// 判断格式的主流程：先做表头判断，再抽查数据行（至少 1 条合法，并顺便判断语言）。
    fn detect(&mut self, text: &str) -> Result<(), String> {
        let mut rows = table_rows(text);
        // 表头就是第 0 个表格行：真实导出的表头一定在文件开头（前面的空行不算表格行）。
        // 第 1 列包含“编号/UID”是表头的唯一标志——数据行同一列是纯数字，分隔线是横线，都不会误判。
        let format = match rows.next() {
            Some((_, header)) => header_format(&header)?,
            None => return Err("RTF 格式不合法：未识别到表头行".into()),
        };
        // 抽查数据行：按表头判断出的格式校验行结构和字段，至少要有 1 条合法才放行。
        // 顺便用翻译表判断语言：国籍值在哪个语言节点里作为显示文本命中，就是哪种语言。
        let mut valid = 0usize;
        let mut first_rejection: Option<String> = None;
        let mut language: Option<&'static str> = None;
        for (row_no, cells) in rows.take(DETECT_LINE_LIMIT) {
            match classify_row(&cells, format) {
                RowVerdict::Data => {
                    valid += 1;
                    if language.is_none() {
                        // 主国籍、第二国籍都查一遍；已是三字码的值解析出的语言为 None，会自然跳过。
                        language = [NAT_COL, SEC_NAT_COL]
                            .iter()
                            .filter_map(|&col| cells.get(col))
                            .find_map(|nat| resolve_nationality(nat).and_then(|(lang, _)| lang));
                    }
                }
                RowVerdict::NotDataRow => {}
                RowVerdict::Rejected { field, detail } => {
                    // 只记录第一条坏行：报错定位到最靠前的坏行就够了，逐行列出没有额外价值。
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

    /// 完整解析名单（必须先 `detect_format` 成功）。
    ///
    /// * `keep_only_newgen = true` 时只保留带 r- 前缀（随机人）的记录，Replace 流程用它取目标；
    ///   扫描展示时传 false。
    /// * 国籍必须能在 nat_translation.json（显示文本）或 nat_codes.json（国籍码）里查到，否则该行
    ///   判为非法：丢弃并逐行记 Warn 日志，便于用户据日志增删翻译表（国籍错会连累后续抽图，需严格）。
    /// * 其它行级问题（字段不合法、列数不符）同样丢行并记日志；整份文件是否合法已在 detect 阶段检查。
    /// 【仅测试】生产的两条路径（「扫描RTF」命令、替换流水线）都要报进度，一律走 parse_roster_progress；
    /// 留这个无进度版只是让测试少写一个空回调，故标 cfg(test) 免得在发布构建里报 dead_code。
    #[cfg(test)]
    pub fn parse_roster(
        &self,
        path: &Path,
        keep_only_newgen: bool,
        logger: &impl LogSink,
    ) -> Result<Vec<PlayerRecord>, String> {
        // 测试不关心行数账目，也不关心进度：取消标志给一个恒为 false 的临时值。
        self.parse_roster_progress(
            path,
            keep_only_newgen,
            logger,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .map(|(data, _)| data)
    }

    /// 同 `parse_roster`，额外把「已解析第 n 条 / 共 total 条」上报给进度回调（「扫描RTF」状态栏进度用）。
    /// 【为何先数一遍数据行】table_rows 是懒迭代器，不先取总数就没有分母，「第 n 条 / 共 N 条」无从谈起；
    /// 计数只做行切分、不做字段解析，相对完整解析开销很小。
    pub fn parse_roster_progress(
        &self,
        path: &Path,
        keep_only_newgen: bool,
        logger: &impl LogSink,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<(Vec<PlayerRecord>, RosterCounts), String> {
        if !self.is_valid {
            let err = "RTF 格式无效，请先执行格式识别".to_string();
            // 入口校验失败：属于调用方顺序错误，记 Error 便于定位（返回文案不变）。
            logger.write(log::Level::Error,"rtf",format!("解析 RTF 被拒绝（{}）：{err}", path.display()),
            );
            return Err(err);
        }
        // 文件解析入口：记录目标文件、判定出的格式与过滤口径（Debug 级，默认不输出）。
        logger.debug(
            "rtf",
            format!(
                "开始解析 RTF 名单：{}（格式 {}，仅随机人={keep_only_newgen}）",
                path.display(),
                self.format.label()
            ),
        );
        let text = fs::read_to_string(path).map_err(|e| {
            logger.write(log::Level::Error, "rtf", format!("读取 RTF 失败（{}）：{e}", path.display()),
            );
            format!("读取 RTF 失败: {e}")
        })?;
        let total = table_rows(&text).count(); // 数据行总数（进度分母）
        logger.debug("rtf", format!("名单数据行总数：{total}"));
        let mut data = Vec::new();
        let mut counts = RosterCounts {
            rows: total,
            ..Default::default()
        };
        let mut done = 0usize;
        for (row_no, cells) in table_rows(&text) {
            done += 1;
            progress(done, total); // 逐行上报（订阅方按百分比去重后才 emit，不会刷爆 IPC）
                                   // 【逐行取消检查点】万行名单解析要跑几秒，而这一段在替换流水线的「取消」生效区间之内：
                                   // 原来这里没有检查点，取消后仍要把整份解析跑完才停（用户判定程序假死 → 强杀 → 正好写坏）。
            if cancel.load(Ordering::Relaxed) {
                logger.warn("rtf", format!("名单解析已取消（停在第 {row_no} / {total} 行）"),
                );
                return Err("名单解析被取消".into());
            }
            match classify_row(&cells, self.format) {
                RowVerdict::Data => {
                    let record = match self.format {
                        RtfFormat::Basic => parse_basic_row(&cells),
                        _ => parse_ext_row(&cells, self.format),
                    };
                    // 过滤只看 uid：带 r- 前缀的就是随机人。
                    if record.uid.starts_with(NEWGAN_UID_PREFIX) || !keep_only_newgen {
                        data.push(record);
                    } else {
                        // 追踪执行细节：逐行说明为何被过滤（Trace 级别默认被全局阈值挡住，不影响正常日志）。
                        counts.filtered += 1;
                        logger.trace( "rtf", format!("第 {row_no} 行非随机人，已按「仅随机人」过滤"),
                        );
                    }
                }
                // 表头行（第 0 行）必然落在这里——它的编号列是「UID」这样的文字，本来就不是数据。
                // 它之后的行还走到这里，就是编号格里混进了 `*`、零宽变体或整行说明文字：
                // 这类行原来既不进任何计数也不记日志（整行 100% 静默消失），现在逐行留痕并入摘要。
                RowVerdict::NotDataRow => {
                    if row_no > 0 {
                        counts.stray += 1;
                        if counts.stray <= DROP_LOG_MAX {
                            logger.warn( "rtf", format!("第 {row_no} 行编号列不是数字，不是名单数据行，已忽略该行"),
                            );
                        }
                    }
                }
                RowVerdict::Rejected { field, detail } => {
                    // 逐行记日志（上限 DROP_LOG_MAX 条，超出合并汇总）；国籍查不到时可据此补表。
                    counts.dropped += 1;
                    if counts.dropped <= DROP_LOG_MAX {
                        logger.write(log::Level::Warn, "rtf", format!("第 {row_no} 行「{field}」{detail}，已跳过该行"),
                        );
                    }
                }
            }
        }
        if counts.dropped > DROP_LOG_MAX {
            logger.warn("rtf", format!("另有 {} 行不合法，已跳过", counts.dropped - DROP_LOG_MAX));
        }
        if counts.stray > DROP_LOG_MAX {
            logger.warn("rtf", format!("另有 {} 行编号列非数字，已忽略", counts.stray - DROP_LOG_MAX));
        }
        counts.kept = data.len();
        // 扫描完成：并入格式/语言，作为本模块唯一的汇总行，用 Debug 输出。
        // 【为何不记 Info】「扫描RTF」命令与替换流水线都会各记一条 Info 级摘要，行数账目现在
        // 由那两条一并承载（见 RosterCounts::log_tail）；这里保留同一口径供排查时开 Debug 查看。
        logger.debug(
            "rtf",
            format!(
                "RTF 名单解析完成：{}（语言 {}），保留 {} 条{}",
                self.format.label(),
                self.language,
                data.len(),
                counts.log_tail(),
            ),
        );
        Ok((data, counts))
    }
}

// ---------------- 行管线：清洗、切分、判别 ----------------

/// 只读取文件开头的 `max` 字节，并用“损失式”（lossy）方式解码成 UTF-8：如果正好在截断位置切断了
/// 多字节字符，就用替换字符代替，而不是报错。识别格式不需要完整文件。
fn read_head(path: &Path, max: u64) -> Result<String, String> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|e| format!("读取 RTF 失败: {e}"))?;
    let mut buf = Vec::new();
    file.take(max)
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取 RTF 失败: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// FM 中文导出会按列宽折行，导致表头和字段值里混入 U+200B（实测表头「编​号」、字段「180厘​米」
/// 都含有）；如果不清除，会导致表头匹配失败、翻译表查不到值。
fn strip_zero_width_chars(line: &str) -> String {
    line.replace(ZERO_WIDTH_SPACE, "")
}

/// 逐行产出表格行 `(行号, 单元格)`。行号就是报错信息里的「第 N 行」：第 0 行是表头，第 1 行是
/// 第 1 条数据……空行和分隔线不算表格行，也不占用行号。单元格向量的第 0 个位置固定放行号
/// （计数列），第 k 个位置就是表格的第 k 列，与报错信息里的列号一致。
///
/// 为什么要跳过全空行和纯横线行：分隔线夹在表头和第一条数据之间，如果它占用了行号，
/// “第 1 行就是第 1 条数据”这个约定就不成立了。
fn table_rows(text: &str) -> impl Iterator<Item = (usize, Vec<String>)> + '_ {
    let mut row_no = 0usize;
    text.lines().filter_map(move |line| {
        let line = strip_zero_width_chars(line);
        let ends_with_bar = line.trim_end().ends_with('|');
        let mut cells: Vec<String> = line.split('|').map(|c| c.trim().to_string()).collect();
        if cells.len() < 2 {
            return None; // 无竖线的杂行不是表行
        }
        // 只在行确实以 `|` 收尾时丢掉那个切分产物。机器导出的名单每行都有结尾管道符，
        // 但手改/截断过的没有——原来这里是无条件 pop，于是那种行会连带丢掉**最后一个真实数据列**：
        // 拓展名单仍够列数 → 静默少一列自定义值；基础名单则 7 变 6 → 整行被拒，还给出一条误导的诊断。
        if ends_with_bar {
            cells.pop();
        }
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

/// 表头的四步判断，报错信息直接展示给用户、并精确到列：
/// ① 列数检查：≥18 列走拓展分支，7 列（允许末尾有多余空列）走基础分支，8-17 列整份文件不合法；
/// ② 第 1-4 列必须分别是 编号(UID)/国籍(Nat)/第二国籍(2nd Nat)/姓名(Name)；
/// ③ 第 10 列必须是“是新生球员(Is newgen)”才算拓展；7 列的情况本来就没有第 10 列，直接判为 Basic。
///    为什么 ≥18 列却没匹配上第 10 列时不判为 Basic：基础名单一定是 7 列，18 列的视图判成 Basic
///    必然和数据行矛盾，直接指出第 10 列有问题，比等到数据行才报“应为 7 列，实际 18 列”更容易定位；
/// ④ 拓展名单再看第 15 列：是“偏好工作(Preferred Job)”就是职员，是“所在地(Based)”就是球员；
///    两者都不匹配则整份文件不合法（判错会让第 15 列的含义错位后入库）。
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

/// 判断表头的某一列是否包含关键字：先转小写再做“包含”匹配（真实表头如「国籍/地区籍」「2nd Nat」
/// 都带修饰，英文列名大小写也不固定，用完全相等会漏掉）。
fn header_hit(cell: &str, words: &[&str]) -> bool {
    let lower = cell.to_lowercase();
    words.iter().any(|w| lower.contains(w))
}

/// 单行判断。检测阶段的抽查和解析阶段的过滤共用这一份实现——两个阶段对“什么是合法数据行”
/// 必须用同一套标准，否则检测时认可的行在解析时可能被丢掉，或者解析时混入检测没见过的行。
/// 格式由表头判断决定（这里不再按行内内容分流）；第 1 列是纯数字仍然是数据行的唯一标志
/// （表头同一列是「编号/UID」文字）。
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

/// Basic 行的校验：列数正好 7 列（允许末尾有空列——FM 会按视图宽度补齐空单元格；如果第 8 列起
/// 有内容，说明数据行与表头判断出的基础结构不符），再校验国籍和三个数值码。
/// 校验顺序与列顺序一致，报错定位到最靠前的坏字段。
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

/// 拓展行的校验（球员和职员共用，球员/职员已经由表头区分，行内不用再分）：列数 ≥18，再校验国籍、
/// 四个数值码以及第 18 列的 R.C.A。校验顺序与列顺序一致。
///
/// 为什么允许肤色和 R.C.A 为空：FM 对未加载的对象会把整段属性导出成空白（实测 ext_staff_short_chn
/// 的 37 行肤色与 R.C.A 全为空），空值原样入库；只有非空时才校验，防止列错位的文本混进来。
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
    // 第 18 列 R.C.A 只拒绝“非空且不是数字”这种列错位情况；能力类数值没有实测上界，不做范围校验。
    let rca = cells[EXT_RCA_COL].as_str();
    if !rca.is_empty() && !rca.bytes().all(|b| b.is_ascii_digit()) {
        return RowVerdict::Rejected {
            field: "第 18 列",
            detail: format!("R.C.A 应为数字，实际「{rca}」"),
        };
    }
    RowVerdict::Data
}

/// 校验国籍列：不合法返回 `Rejected`（列号就是表格列号），合法返回 `None`。
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
            "第 {col} 列「{value}」不是 nat_translation.json 收录的国籍显示文本，也不是 nat_codes.json 收录的国籍码"
        ),
    })
}

/// 校验数值码列：不合法返回 `Rejected`（列号就是表格列号），合法返回 `None`。
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

// ---------------- 谓词（统一用 is_* 命名） ----------------

/// UID 必须是 4-10 位数字。4-10 位是真实 FM 数据里实测的位数范围；11 位以上属于溢出的脏数据，
/// 会拒绝该行（见 test_wrong_UID 测试文件）。
fn is_valid_uid(uid: &str) -> bool {
    (4..=10).contains(&uid.chars().count()) && uid.bytes().all(|b| b.is_ascii_digit())
}

/// 空值表示没有数据，原样入库（FM 对未加载对象会导出空白单元格，直接拒绝会丢掉整批数据）；
/// 非空时必须是纯数字且落在 [min, max] 内。先按字节判断是不是纯数字再解析：超长字符串解析会溢出
/// 返回 Err，也一并视为不合法。
fn is_valid_code(value: &str, min: u32, max: u32) -> bool {
    value.is_empty()
        || (value.bytes().all(|b| b.is_ascii_digit())
            && value
                .parse::<u32>()
                .map(|n| (min..=max).contains(&n))
                .unwrap_or(false))
}

/// 判断一个值是否已经是国籍码：直接查 nat_codes.json 的码全集。
/// 三位码与两位特例 "SC" 都收录在其中，命中即视为「已是目标形态」，无需再翻译。
fn is_nationality_code(nat: &str) -> bool {
    crate::assets::NAT_CODES
        .iter()
        .any(|code| code.as_str() == nat)
}

/// 国籍是否合法：必须能在权威表里查到——要么是 nat_translation.json 收录的显示文本，要么是
/// nat_codes.json 收录的国籍码。两者都查不到一律判非法（国籍错会连累后续抽图，必须在前置环节拦下）。
/// 空值按 `allow_empty` 处理（第二国籍允许为空）。
fn is_valid_nationality(nat: &str, allow_empty: bool) -> bool {
    if nat.is_empty() {
        return allow_empty;
    }
    resolve_nationality(nat).is_some()
}

/// 解析一个国籍值，返回（语言节点, 三字码）：
/// - 命中翻译表某语言节点的「显示文本」（键）→ 返回该语言节点名与映射出的三字码；
/// - 值本身已是国籍码（在 nat_codes.json 的码全集里，如三位码或两位特例 "SC"）→ 语言返回 None，
///   码取原值；
/// - 空值或彻底无法识别 → None。
fn resolve_nationality(nat: &str) -> Option<(Option<&'static str>, String)> {
    if nat.is_empty() {
        return None;
    }
    // 先按显示文本查：在哪个语言节点里命中，就同时拿到语言和对应的码。
    for (lang, table) in crate::assets::NAT_TRANSLATION.iter() {
        if let Some(code) = table.get(nat) {
            return Some((Some(lang.as_str()), code.clone()));
        }
    }
    // 再看是不是已经是国籍码（查 nat_codes.json 全集，含 "SC" 特例）。
    if is_nationality_code(nat) {
        return Some((None, nat.to_string()));
    }
    None
}

/// 判断是否为随机人：标志里包含“是”或“Yes”就是随机人（正向判断），uid 会加上 r- 前缀。
fn is_newgan(flag: &str) -> bool {
    NEWGAN_FLAGS.iter().any(|f| flag.contains(f))
}

// ---------------- 行解析（classify 通过后取列组装，不再重复校验） ----------------

/// 组装 Basic 行。视图本身就按 is newgen 过滤过，所以 uid 无条件加上 r- 前缀；
/// 国籍两列就地查表转成三字码。
fn parse_basic_row(cells: &[String]) -> PlayerRecord {
    PlayerRecord {
        uid: format!("{NEWGAN_UID_PREFIX}{}", cells[UID_COL]),
        primary_nat: resolve_nat(&cells[NAT_COL]),
        sec_nat: resolve_nat(&cells[SEC_NAT_COL]),
        name: cells[NAME_COL].clone(),
        hair_length: cells[HAIR_LENGTH_COL].clone(),
        hair_color: cells[HAIR_COLOR_COL].clone(),
        ethnicity_code: cells[ETHNICITY_COL].clone(),
        detailed: None,
    }
}

/// 组装拓展行。是否随机人按标志正向判断（包含“是”或“Yes”）；第 15 列按判断结果写入 `based` 或
/// `preferred_job`（另一个留空，前端据此区分球员/职员记录）；第 19 列起的用户自定义列原文
/// 放进 `extra_fields`；国籍两列就地查表转成三字码。
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
        primary_nat: resolve_nat(&cells[NAT_COL]),
        sec_nat: resolve_nat(&cells[SEC_NAT_COL]),
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

/// 把国籍列的值解析成三字码：命中翻译表就换成码，是 nat_codes.json 里的码则原样保留。
/// 未收录的值在 classify 阶段已被判为非法行，不会走到这里；空值返回空串。
fn resolve_nat(value: &str) -> String {
    resolve_nationality(value)
        .map(|(_, code)| code)
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logger::{ChannelLogger, LogLine};
    use std::io::Write;

    /// 把样本内容写入一个临时文件并返回路径（用 `keep` 防止文件被自动删除），方便 detect/parse 直接读取。
    fn write_rtf(content: &str) -> std::path::PathBuf {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.keep().unwrap().1
    }

    /// 返回 tests/testing_data 下的夹具路径（不存在则 `None`，调用方跳过对应断言）。
    /// 注意：文件名以 `test` / `invalid` 开头的是人为构造的“坏数据”，其余才是真实导出样本。
    fn real_rtf(name: &str) -> Option<std::path::PathBuf> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/testing_data")
            .join(name);
        p.is_file().then_some(p)
    }

    /// 非阻塞地取出日志通道中已经到达的 WARNING 文案，用来断言翻译缺失的警告。
    fn warn_messages(rx: &std::sync::mpsc::Receiver<LogLine>) -> Vec<String> {
        rx.try_iter()
            .filter(|l| l.level == "WARNING")
            .map(|l| l.message)
            .collect()
    }

    // ---------------- 合成样本（列的排列和表头用词与真实导出一致，可在 CI 上运行） ----------------

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

    // 职员表头的最后一列是 NP R.C.A（真实导出就是这样），和球员的 R.C.A 一样都是第 18 列的数值。
    const EXT_STAFF_CN_SAMPLE: &str = "\
| 编号 | 国籍/地区籍 | 第二国籍/地区籍 | 姓名 | 头发长度 | 发色 | 种族码 | 肤色 | 头像 | 是新生球员 | 性别 | 年龄 | 身高 | 俱乐部 | 偏好工作 | 能力 | 潜力 | NP R.C.A |
| 2002243829 | 中国 | | 王晨 | 1 | 16 | 10 | 8 | | 是 | 男 | 62 | 182厘米 | | 足球总监 | 144 | 199 | 140 |
| 306951 | 巴西 | | 库卡 | 1 | 11 | 0 | 8 | | 否 | 男 | 68 | 182厘米 | 梅州客家 | 足球总监 | 142 | 160 | 138 |
";

    // 真实英文导出的表头是中英混排的（头发长度/发色等列名没有翻译）；判断格式时只看第 1-4、10、15 列。
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
        // 乱码文件：第一个表格行的第 1 列不是「编号/UID」，所以报「未识别到表头行」。
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
        // 基础名单里所有记录都是随机人：即使 keep_only_newgen 为 true，也不会过滤掉任何行。
        let data = s.parse_roster(&path, true, &logger).unwrap();
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
        // 表头判断为基础名单后，8-17 列的数据行（旧版 14 列详细、旧版 17 列职员拓展）与表头
        // 结构不符，因此在抽查阶段整份拒绝，报「列数」。
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
        // UID 不是纯数字的行会被当成非数据行跳过；种族码越界的行会按非法行跳过，合法的行保留。
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
        let data = s.parse_roster(&path, true, &logger).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-2001234567");
        // 同一份名单的行数账目必须对得上：4 个表行里 1 行编号列非数字（stray）、
        // 1 行种族码越界（dropped）。这两类过去只会让记录静默变少（stray 连一行日志都没有）。
        let (_, counts) = s
            .parse_roster_progress(&path, true, &logger, &AtomicBool::new(false), &mut |_, _| {}, )
            .unwrap();
        assert_eq!(
            (counts.rows, counts.kept, counts.dropped, counts.stray),
            (4, 1, 1, 1),
            "行数账目不对：{counts:?}"
        );
        assert!(counts.log_tail().contains("编号列非数字 1"), "{}", counts.log_tail());
    }

    #[test]
    fn row_without_trailing_bar_keeps_its_last_column() {
        // 手改/截断过的名单不一定以 `|` 收尾。原来 table_rows 无条件 pop 掉那个切分产物，
        // 于是这种行连带丢掉最后一个真实数据列：基础名单 7 列变 6 列 → 整行被拒。
        let sample = "\
| UID       | Nat  | 2nd Nat | Name          |    |    |   |
| 2001234567| ESP  |         | Ok Player     | 1  | 12 | 1
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        let (data, counts) = s
            .parse_roster_progress(&path, true, &logger, &AtomicBool::new(false), &mut |_, _| {}, )
            .unwrap();
        assert_eq!(data.len(), 1, "结尾没有管道符的行被整行拒了");
        assert_eq!((counts.dropped, counts.stray), (0, 0), "行数账目不对：{counts:?}");
    }

    #[test]
    fn translate_chinese_nationality() {
        let path = write_rtf(CN_SAMPLE);
        let logger = ChannelLogger::channel();
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger.0).unwrap();
        // 中文国籍在组装记录时直接查翻译表转成三字码，无需额外的翻译步骤。
        let data = s.parse_roster(&path, true, &logger.0).unwrap();
        assert_eq!(data[0].primary_nat, "USA");
        assert!(warn_messages(&logger.1).is_empty());
    }

    #[test]
    fn unrecognized_nationality_drops_row_and_logs() {
        // 国籍必须能在 nat_translation.json 或 nat_codes.json 里查到，否则整行丢弃并记 WARNING；
        // 这里用一个表里肯定没有的虚构国名来验证这条严格路径。
        let sample = "\
| 编号      | 国籍/地区籍 | 第二国籍 | 姓名        |    |    |   |
| 2002075172| 西兰公国   | 美国     | 扎克·布劳顿 | 3  | 9  | 0 |
| 2002075173| 中国      |         | 张三        | 1  | 12 | 0 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel();
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger.0).unwrap();
        let data = s.parse_roster(&path, false, &logger.0).unwrap();
        // 第一行国籍「西兰公国」未收录 → 丢弃；第二行正常保留并换成码。
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].primary_nat, "CHN");
        let warns = warn_messages(&logger.1);
        assert!(
            warns
                .iter()
                .any(|m| m.contains("西兰公国") && m.contains("已跳过该行")),
            "应有丢行告警：{:?}",
            warns.first()
        );
    }

    #[test]
    fn ext_player_detect_parse_and_full_fields() {
        let path = write_rtf(EXT_PLAYER_EN_SAMPLE);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
        assert_eq!(s.language, "English");
        // keep_only_newgen = true：只留下标志为 Yes 的行（带 r- 前缀）。
        let data = s.parse_roster(&path, true, &logger).unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].uid, "r-103607");
        // 不过滤（keep_only_newgen = false）：标志为 No 的行也保留，uid 不带前缀，18 列字段全部入库。
        let data = s.parse_roster(&path, false, &logger).unwrap();
        assert_eq!(data.len(), 2);
        let first = &data[0];
        assert_eq!(first.uid, "12315"); // 标志为 No（不是随机人）：不加前缀
        assert_eq!(first.primary_nat, "MKD"); // 国籍在第 2 列（和基础名单一样）
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
        // 标志为“是”的加 r- 前缀，“否”的用不带前缀的 uid；国籍经过翻译节点转换成三字码；性别和所在地正常入库。
        let path = write_rtf(EXT_PLAYER_CN_SAMPLE);
        let logger = ChannelLogger::channel();
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger.0).unwrap();
        assert_eq!(
            (s.format, s.language.as_str()),
            (RtfFormat::ExtPlayer, "简体中文")
        );
        let data = s.parse_roster(&path, false, &logger.0).unwrap();
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
        assert!(warn_messages(&logger.1).is_empty()); // 全部都在翻译表里
    }

    #[test]
    fn ext_player_extra_columns_stored() {
        // 在第 18 列后面追加用户自定义列：仍然合法，原文会存进 extra_fields。
        let sample = "\
| UID | Nat | 2nd Nat | Name | | | | | | Is newgen | Gender | Age | Height | Club | Based | CA | PA | R.C.A |
| 12315 | MKD | | Toni Micevski | 1 | 12 | 0 | 5 | | No | Male | 54 | 188 cm | Poeševo | North Macedonia (Fourth League) | 27 | 122 | 27 | 惯用脚 | 左 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
        let data = s.parse_roster(&path, false, &logger).unwrap();
        assert_eq!(
            data[0].detailed.as_ref().unwrap().extra_fields,
            vec!["惯用脚".to_string(), "左".to_string()]
        );
    }

    #[test]
    fn ext_row_with_empty_cells_kept() {
        // 未加载对象的整段属性都是空白（肤色、标志、性别……R.C.A 全空，实测
        // ext_staff_short_chn 有 37 行是这样）——空值原样入库、行不丢；空标志按非随机人处理（uid 不带前缀）。
        let sample = "\
| 编号 | 国籍/地区籍 | 第二国籍/地区籍 | 姓名 | 头发长度 | 发色 | 种族码 | 肤色 | 头像 | 是新生球员 | 性别 | 年龄 | 身高 | 俱乐部 | 偏好工作 | 能力 | 潜力 | NP R.C.A |
| 67268144 | 西班牙 | | | 1 | 3 | 1 | | | | | | | | | | | |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        let data = s.parse_roster(&path, false, &logger).unwrap();
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
        // 由表头决定一切：第 10 列的关键字用来区分基础/拓展，第 15 列的关键字用来区分球员/职员；
        // 身份列、标志列、判别词里任何一个不合法，都会整份拒绝并精确指出是哪一列。
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
        // 没有表头（数据行第 1 列是数字，匹配不上「编号/UID」），所以报和「表头」有关的错误。
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{row18}\n")), &logger)
            .unwrap_err();
        assert!(
            err.contains("RTF 格式不合法") && err.contains("表头"),
            "{err}"
        );
        // 第 15 列不是判别词，所以报表头第 15 列有问题。
        let bad15 = header_based.replace("Based", "俱乐部");
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{bad15}\n{row18}\n")), &logger)
            .unwrap_err();
        assert!(err.contains("表头第 15 列"), "{err}");
        // 有 18 列表头但第 10 列不是标志列时，不能判成 Basic（基础名单一定是 7 列），所以报表头第 10 列有问题。
        let bad10 = header_based.replace("Is newgen", "随机人");
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{bad10}\n{row18}\n")), &logger)
            .unwrap_err();
        assert!(err.contains("表头第 10 列"), "{err}");
        // 身份列不匹配（第 2 列不是国籍），所以报表头第 2 列有问题。
        let bad2 = header_based.replace("| Nat |", "| 俱乐部 |");
        let err = RtfScanner::new()
            .detect_format(&write_rtf(&format!("{bad2}\n{row18}\n")), &logger)
            .unwrap_err();
        assert!(err.contains("表头第 2 列"), "{err}");
        // 抽查数据行时发现 R.C.A 非空但不是数字，定位到「第 18 列」（字段校验在放行之前）。
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
        // 17 列的数据行和拓展表头不符，所以报列数不合法。
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
        let data = s.parse_roster(&path, false, &logger).unwrap();
        assert_eq!(data.len(), 2);
        // 标志为“是”的加 r- 前缀，“否”的用不带前缀的 uid。
        assert_eq!(data[0].uid, "r-2002243829");
        assert_eq!(data[1].uid, "306951");
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(d.preferred_job, "足球总监"); // 只有职员才会在第 15 列出现
        assert_eq!(d.based, "");
        assert_eq!(d.gender, "男"); // 第 11 列
        assert_eq!(d.club, ""); // 王晨没有俱乐部
        assert_eq!(d.age, "62"); // 第 12 列
        assert_eq!(d.height, "182厘米"); // 第 13 列
        assert_eq!(d.ca, "144");
        assert_eq!(d.pa, "199");
        assert_eq!(d.rca, "140"); // NP R.C.A 同为第 18 列
        assert_eq!(d.avatar, "");
        // 英文导出：表头中英混排不影响格式判断；性别 Male/Female 原样入库。
        let path = write_rtf(EXT_STAFF_EN_SAMPLE);
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        assert_eq!(s.language, "English");
        let data = s.parse_roster(&path, false, &logger).unwrap();
        let d = data[0].detailed.as_ref().unwrap();
        assert_eq!(d.gender, "Female");
        assert_eq!(d.rca, "195");
        assert_eq!(d.preferred_job, "Performance Analyst");
        assert_eq!(d.club, "");
    }

    #[test]
    fn ext_staff_avatar_and_extras_stored() {
        // 头像列有内容时按原文入库；第 19 列起的用户自定义列存进 extra_fields。
        let sample = "\
| 编号 | 国籍/地区籍 | 第二国籍/地区籍 | 姓名 | 头发长度 | 发色 | 种族码 | 肤色 | 头像 | 是新生球员 | 性别 | 年龄 | 身高 | 俱乐部 | 偏好工作 | 能力 | 潜力 | NP R.C.A |
| 306951 | 巴西 | | 库卡 | 1 | 11 | 0 | 8 | face_kuka | 否 | 男 | 68 | 182厘米 | 梅州客家 | 足球总监 | 142 | 160 | 138 | 合同年 | 2027 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel().0;
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtStaff);
        let data = s.parse_roster(&path, false, &logger).unwrap();
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
        // 3 位 / 11 位是不合法的行；4 位 / 10 位是合法的。
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
        let data = s.parse_roster(&path, false, &logger).unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].uid, "r-1234");
        assert_eq!(data[1].uid, "r-1234567890");
        // 整份文件只有越界的 UID 行，所以报「编号(UID)」相关错误。
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
        // 只有 nat_translation.json 收录的显示文本、或 nat_codes.json 收录的国籍码才算合法。
        // "SC" 在码全集里，按码原样保留；没收录的 "Atlantis" 直接丢行并记警告。
        let sample = "\
| UID       | Nat      | 2nd Nat | Name           |    |    |   |
| 2000472008| SGP      |         | Ok Code Nat    | 1 | 12 | 0 |
| 2000472009| 北马其   |         | Ok Cjk Nat     | 1 | 12 | 0 |
| 2000472010| SC       |         | Ok Special Code| 1 | 12 | 0 |
| 2000472011| Atlantis |         | Ok Long Nat    | 1 | 12 | 0 |
";
        let path = write_rtf(sample);
        let logger = ChannelLogger::channel();
        let mut s = RtfScanner::new();
        s.detect_format(&path, &logger.0).unwrap();
        let data = s.parse_roster(&path, false, &logger.0).unwrap();
        // 前三条合法保留，第四条「Atlantis」未收录被丢弃。
        assert_eq!(data.len(), 3);
        assert_eq!(data[2].primary_nat, "SC");
        let warns = warn_messages(&logger.1);
        assert!(
            warns
                .iter()
                .any(|m| m.contains("Atlantis") && m.contains("已跳过该行")),
            "应有丢行告警：{:?}",
            warns.first()
        );
        assert!(!warns.iter().any(|m| m.contains("SC")));
        // 单字母 / 纯数字国籍同样不合法，整份文件只有这类行时报「国籍」定位。
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
        // 全称形式的名单：语言判为 English，解析后通过 English 节点转换成三字码。
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
        let data = s.parse_roster(&path, true, &logger.0).unwrap();
        assert_eq!(data.len(), 3);
        assert_eq!(data[0].primary_nat, "ARG");
        assert_eq!(data[0].sec_nat, "USA"); // U.S.A. → USA
        assert_eq!(data[1].sec_nat, "BIH"); // Bosnia & Herzegovina → BIH
        assert_eq!(data[2].primary_nat, "HKG"); // 带括号后缀显示名 → HKG
        assert!(warn_messages(&logger.1).is_empty());
    }

    #[test]
    fn out_of_range_codes_rejected() {
        // 非空但越界的数值码会让整份文件不合法并定位到字段；校验顺序与列顺序一致。
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
        // 头发长度 = 5 是合法的上界（真实数据里确实存在）。
        let ok = "\
| UID | Nat | 2nd Nat | Name | | | | | | Is newgen | Gender | Age | Height | Club | Based | CA | PA | R.C.A |
| 12315 | MKD | | X | 5 | 12 | 0 | 5 | | No | 男 | 54 | 188 cm | C | B | 27 | 122 | 27 |
";
        let mut s = RtfScanner::new();
        s.detect_format(&write_rtf(ok), &logger).unwrap();
        assert_eq!(s.format, RtfFormat::ExtPlayer);
    }

    // ---------------- 真实导出样本（行数和关键值来自实际文件的统计） ----------------

    #[test]
    fn real_basic_files() {
        // 共 1599 行，全部是随机人；国籍显示名（含中文全名、英文全名）全部转换成三字码。
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
            let data = s.parse_roster(&f, false, &logger).unwrap();
            assert_eq!(data.len(), 1599, "{name}");
            assert!(
                data.iter().all(|r| r.uid.starts_with(NEWGAN_UID_PREFIX)),
                "{name}"
            );
            assert!(
                data.iter().all(|r| is_nationality_code(&r.primary_nat)),
                "{name}: 国籍应全部是 nat_codes.json 里的码"
            );
        }
    }

    #[test]
    fn real_player_ext_files() {
        // 4 份球员拓展名单：第一行都是路易斯·迪亚斯（标志为否/No，uid 不带前缀）；short 版国籍是
        // 因列宽被截断的别名（哥伦比），翻译表已收录，转换结果与 long 版一致。
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
            let data = s.parse_roster(&f, false, &logger).unwrap();
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
            // 随机人（是/Yes）与非随机人（否/No）同时存在。
            assert!(
                data.iter().any(|r| r.uid.starts_with(NEWGAN_UID_PREFIX))
                    && data.iter().any(|r| !r.uid.starts_with(NEWGAN_UID_PREFIX)),
                "{name}"
            );
        }
    }

    #[test]
    fn real_staff_ext_files() {
        // 4 份职员拓展名单：398 条数据中有 1 行因 UID 越界被跳过；short_chn 版还有整段属性
        // 为空白的未加载对象行（肤色、R.C.A 为空），容忍空值后全部入库。
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
            let data = s.parse_roster(&f, false, &logger).unwrap();
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
        // 以 test/invalid 开头的夹具是人为构造的“坏数据”，不是真实导出。
        // test_wrong_UID.rtf：UID 位数边界，4/7/10 位的 3 行保留，11 位的行丢弃。
        if let Some(f) = real_rtf("test_wrong_UID.rtf") {
            let logger = ChannelLogger::channel().0;
            let mut s = RtfScanner::new();
            s.detect_format(&f, &logger).unwrap();
            let data = s.parse_roster(&f, false, &logger).unwrap();
            assert_eq!(data.len(), 3);
            assert_eq!(data[0].uid, "r-191571");
            assert_eq!(data[1].uid, "r-1915576430");
            assert_eq!(data[2].uid, "r-1915576");
        }
        // test_wrong_ethnicity.rtf：含一个编造的非法国籍码（不在 nat_translation.json / nat_codes.json），
        // 按严格口径该行被丢弃（夹具 29 行 → 保留 28 行）。
        if let Some(f) = real_rtf("test_wrong_ethnicity.rtf") {
            let logger = ChannelLogger::channel().0;
            let mut s = RtfScanner::new();
            s.detect_format(&f, &logger).unwrap();
            let data = s.parse_roster(&f, false, &logger).unwrap();
            assert_eq!(data.len(), 28);
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
