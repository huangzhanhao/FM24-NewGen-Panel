//! 模板渲染与词库：`{{key}}` / `{{key|默认值}}` 展开 + 球员数据源 → 模板变量。
//!
//! 【内容在模板里，代码只留「出厂默认」的读取口】词库与码值映射是内容（中文措辞），
//! 随 `PromptTemplate::pools` 落盘、即改即生效；各池为空时逐组回落内置默认模板的同名结构
//! （[`BUILTIN_POOLS`]），手建模板不写也能跑。

use rand::Rng;
use std::collections::HashMap;
use std::sync::LazyLock;

use super::config::Config;
use super::types::{DetailPools, PromptTemplate};

// ===================== 模板渲染：{{key}} / {{key|默认值}} ============================================================

/// 渲染结果（跨 IPC 返回给前端，故需 Serialize + camelCase）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOutcome {
    /// 渲染后的文本（有默认值的占位符展开为默认值，命中 vars 的展开为变量值）。
    pub text: String,
    /// 既无 vars 值、又无默认值的占位符键名（无数据源时出现即需用户手工处理）。
    pub missing: Vec<String>,
}

/// 渲染模板正文：`{{key}}` 展开为 vars 中的值，`{{key|默认值}}` 在 vars 缺该键时展开为
/// 默认值；普通文本（含孤立 `{`）原样输出。
///
/// - `body`：模板正文，占位符的键名与默认值先 trim；
/// - `vars`：模板变量表（通常由 build_vars 构建）。
///
/// 既无变量值又无默认值的占位符展开为空串，键名去重后按首次出现顺序计入 missing，
/// 供前端提示手工补全。
pub fn render(body: &str, vars: &HashMap<String, String>) -> RenderOutcome {
    let mut out = String::new();
    let mut missing: Vec<String> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(rel) = body[i + 2..].find("}}") {
                let expr = body[i + 2..i + 2 + rel].trim();
                let (key, default) = match expr.split_once('|') {
                    Some((k, d)) => (k.trim(), d.trim().to_string()),
                    None => (expr, String::new()),
                };
                match vars.get(key) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push_str(&default);
                        if !missing.iter().any(|m| m == key) {
                            missing.push(key.to_string());
                        }
                    }
                }
                i = i + 2 + rel + 2;
                continue;
            }
        }
        let ch = body[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    RenderOutcome { text: out, missing }
}

// ===================== 词库与码值映射（内容在模板里，代码只留「出厂默认」的读取口） ==================================

/// 出厂默认词库 = 内置默认模板（id = "default"）自带的一份，模板没写词库时逐组套用，
/// 避免随机/码值维度静默退化。内置 JSON 是编译期常量，缓存一次即可（用户改的是 data/ 副本）。
static BUILTIN_POOLS: LazyLock<DetailPools> = LazyLock::new(|| {
    Config::builtin()
        .templates
        .into_iter()
        .find(|t| t.id == "default")
        .map(|t| t.pools)
        .unwrap_or_default()
});

// ---- 基础名单肤色随机区间（按族裔的统计分位，build_vars 随机补足时用） ----------------------------------------------

/// 肤色随机区间（族裔 0–10 各一项，取自真实导出名单的 5%~95% 分位）。肤色码
/// 0 = 最浅、19 = 最深；区间是统计值、个别赋值可能偏离，取用时上下各放 1 档余量
///（见 build_vars）。只在**基础名单**缺 `skinCode` 时使用，防「北欧人配最深肤色」类组合。
const SKIN_RANGE_BY_ETHNICITY: [(u32, u32); 11] = [
    (0, 6),   // 0 北欧/白人
    (2, 10),  // 1 地中海/西语裔
    (3, 13),  // 2 中东/北非
    (11, 18), // 3 非洲/加勒比
    (8, 16),  // 4 南亚
    (7, 14),  // 5 东南亚
    (7, 15),  // 6 太平洋岛民
    (2, 11),  // 7 美洲原住民
    (8, 17),  // 8 澳洲原住民
    (6, 16),  // 9 黑白混血
    (5, 8),   // 10 东亚
];

// ---- 随机选取与「模板优先、出厂兜底」的逐组回落工具 -----------------------------------------------------------------

/// 从候选池随机取一项；调用方保证池非空（空池在 build_vars 直接跳过该变量，
/// 由模板 `{{key|默认值}}` 接管）。
fn pick<T: AsRef<str>>(pool: &[T]) -> &str {
    pool[rand::rng().random_range(0..pool.len())].as_ref()
}

/// 性别原文是否表示女性：FM 性别列是导出语言原文（男/女、Male/Female），判据走中英双语
///（与第 15 列表头识别同口径）；双语之外不识别，胡须照常随机。
pub(crate) fn is_female(gender: &str) -> bool {
    let g = gender.trim();
    g == "女" || g.eq_ignore_ascii_case("female")
}

/// 某一组候选池：模板里写了就用模板的，没写回落出厂默认。
fn pool_or_builtin<'a>(pool: &'a [String], fallback: &'a [String]) -> &'a [String] {
    if pool.is_empty() {
        fallback
    } else {
        pool
    }
}

/// 某一组码值映射：模板里写了就用模板的，没写回落出厂默认。
fn map_or_builtin<'a>(
    map: &'a HashMap<String, String>,
    fallback: &'a HashMap<String, String>,
) -> &'a HashMap<String, String> {
    if map.is_empty() {
        fallback
    } else {
        map
    }
}

/// 从发色映射里随机取一个**真实颜色**（排除「未知」这一档本身）。排序后再取，
/// 使结果不依赖 HashMap 迭代顺序。
fn random_hair_color(hair_colors: &HashMap<String, String>) -> String {
    let mut pool: Vec<&String> = hair_colors
        .iter()
        .filter(|(code, name)| code.as_str() != "1" && name.as_str() != "未知")
        .map(|(_, name)| name)
        .collect();
    pool.sort_unstable();
    if pool.is_empty() {
        return "黑色".into();
    }
    pick(&pool).to_string()
}

// ===================== 球员数据源 → 模板变量（与 render 同层的纯函数） ===============================================

/// 从球员记录构建模板变量，并补足 RTF 未声明的维度。
///
/// # 参数
/// - `rec`：RTF 扫出的球员记录（基础名单 detailed=None，拓展名单 detailed=Some）；
/// - `tpl`：提示词模板，pools 里为空的分组逐组回落出厂默认。
///
/// 【缺字段不入 map vs 随机补足】码值类字段（`ethnicity`/`hairColor`/`skinCode`）为空
/// 或查不到该码时一律不入 map，留给模板 `{{key|默认值}}` 兜底——裸数字进提示词对 AI
/// 无意义，空串占位还会绕过默认值机制；`gender`/`age`/`height`/`skinCode` 在**基础名单**
/// （`detailed` 为 None）属结构性缺席，按约定从合理区间随机补足。
///
/// 【随机是常态】发型样式/胡须/表情/景别+动作（联动一掷）/打光/背景/球衣细节 RTF 本就不
/// 提供，每次构建重掷（同一球员每次生成换一套）；发色码 `1`（未知）同样随机取真实色名。
/// 词库与码值映射随**模板**传入（[`PromptTemplate::pools`]），未写逐组回落出厂默认。
///
/// 【身份二分】记录自带的数据源类别决定职员路径：职位非空时 identity 渲染原文，服装/球衣细节/
/// 景别动作换职员池（`pools.staff`）；空职位不写 identity，交给模板默认值兜底。
///
/// 返回可直接交给 [`render`] 的变量表。
pub fn build_vars(
    rec: &crate::rtf_scanner::PlayerRecord,
    tpl: &PromptTemplate,
) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    vars.insert("uid".into(), rec.uid.clone());
    vars.insert("name".into(), rec.name.clone());
    vars.insert("primaryNat".into(), rec.primary_nat.clone());
    vars.insert("secNat".into(), rec.sec_nat.clone());
    // 国籍中文名（三字码反查当前国籍对照表，用户在「国籍对照表」里改过的中文全称即时生效）；
    // 查不到码则不入 map，由模板默认值兜底。
    let nat = crate::nationality::current();
    if let Some(cn) = nat.cn_name(&rec.primary_nat) {
        vars.insert("nationality".into(), cn.to_string());
    }
    let pools = &tpl.pools;
    // 人员类别由数据源确定；空职位的职员不写 identity，避免错误回落成球员称谓。
    let is_staff = rec.kind.is_staff();
    if is_staff {
        if let Some(job) = rec
            .detailed
            .as_ref()
            .map(|d| d.preferred_job.trim())
            .filter(|job| !job.is_empty())
        {
            // 英文职位与中文性别/国籍直接拼接会出「中国男Performance Analyst」，故中英
            // 边界补空格；纯中文职位不受影响。
            let mut spaced = String::new();
            if job.starts_with(|c: char| c.is_ascii_alphabetic()) {
                spaced.push(' ');
            }
            spaced.push_str(job);
            if job.ends_with(|c: char| c.is_ascii_alphabetic()) {
                spaced.push(' ');
            }
            vars.insert("identity".into(), spaced);
        }
    } else {
        let identity_map = if pools.identity.is_empty() {
            &BUILTIN_POOLS.identity
        } else {
            &pools.identity
        };
        if let Some(v) = identity_map.get("player") {
            vars.insert("identity".into(), v.clone());
        }
    }
    if let Some(v) =
        map_or_builtin(&pools.ethnicity, &BUILTIN_POOLS.ethnicity).get(&rec.ethnicity_code)
    {
        vars.insert("ethnicity".into(), v.clone());
    }
    // 发色解析：映射值「未知」→ 随机真实色名；码查不到或光头（发长码 0）→ None。
    // 两处消费：独立变量 hairColor + 并入 hairStyle 出口（见发型段），光头自然无色。
    let hair_color_name: Option<String> = if !rec.hair_color.is_empty() && rec.hair_length != "0" {
        let hair_colors = map_or_builtin(&pools.hair_color, &BUILTIN_POOLS.hair_color);
        hair_colors.get(&rec.hair_color).map(|name| {
            if name == "未知" {
                random_hair_color(hair_colors)
            } else {
                name.clone()
            }
        })
    } else {
        None
    };
    if let Some(c) = &hair_color_name {
        vars.insert("hairColor".into(), c.clone());
    }

    let mut club = String::new();
    if let Some(d) = &rec.detailed {
        if !d.skin_code.is_empty() {
            if let Some(v) =
                map_or_builtin(&pools.skin_code, &BUILTIN_POOLS.skin_code).get(&d.skin_code)
            {
                vars.insert("skinCode".into(), v.clone());
            }
        }
        for (key, val) in [
            ("gender", &d.gender),
            ("age", &d.age),
            ("height", &d.height),
            ("ca", &d.ca),
            ("pa", &d.pa),
        ] {
            if !val.is_empty() {
                vars.insert(key.into(), val.clone());
            }
        }
        if !d.club.is_empty() {
            club = d.club.clone();
        }
        if !d.based.is_empty() {
            vars.insert("based".into(), d.based.clone());
        }
    }
    // 生涯句按身份组装整句（身份差异由变量承载，render 不加分支）：职员「任职于足球
    // 俱乐部」（总监/队医不「效力于联赛」）；球员拼 based，缺 based 时不入 map。
    let career = if is_staff {
        Some("任职于足球俱乐部".to_string())
    } else {
        rec.detailed
            .as_ref()
            .filter(|d| !d.based.is_empty())
            .map(|d| format!("效力于{}", d.based))
    };
    if let Some(c) = career {
        vars.insert("career".into(), c);
    }
    // 基础名单缺拓展字段：肤色按族裔合理区间随机，性别按人员类别补足（非女即男），
    // 年龄/身高取合理区间。
    if !vars.contains_key("skinCode") {
        let (lo, hi) = rec
            .ethnicity_code
            .parse::<usize>()
            .ok()
            .and_then(|e| SKIN_RANGE_BY_ETHNICITY.get(e).copied())
            .unwrap_or((0, 19));
        let code = rand::rng()
            .random_range(lo.saturating_sub(1)..=(hi + 1).min(19))
            .to_string();
        if let Some(v) = map_or_builtin(&pools.skin_code, &BUILTIN_POOLS.skin_code).get(&code) {
            vars.insert("skinCode".into(), v.clone());
        }
    }
    // 没有性别信息就按人员类别默认补男：is_female（PlayerFemale/StaffFemale）确认为女
    // 才补女，杜绝「类别为女但提示词写男」的反向矛盾。
    if !vars.contains_key("gender") {
        vars.insert(
            "gender".into(),
            if rec.kind.is_female() { "女" } else { "男" }.into(),
        );
    }
    if !vars.contains_key("age") {
        vars.insert("age".into(), rand::rng().random_range(16..=35).to_string());
    }
    if !vars.contains_key("height") {
        vars.insert(
            "height".into(),
            format!("{} cm", rand::rng().random_range(168..=194)),
        );
    }
    if !club.is_empty() {
        vars.insert("club".into(), club.clone());
    }

    // 球衣归属：职员从职员池随机取便装/训练外套，池空不入 map；球员俱乐部优先，
    // 缺俱乐部退回国籍，两者都缺用通用描述。
    let jersey = if is_staff {
        let cand = pool_or_builtin(&pools.staff.jersey, &BUILTIN_POOLS.staff.jersey);
        (!cand.is_empty()).then(|| pick(cand).to_string())
    } else if !club.is_empty() {
        Some(format!("{club}的比赛球衣"))
    } else if let Some(cn) = vars.get("nationality").cloned() {
        Some(format!("{cn}国家队球衣"))
    } else {
        Some("深色比赛球衣，带队长袖标与徽章".into())
    };
    if let Some(j) = jersey {
        vars.insert("jersey".into(), j);
    }

    // 随机细节：词库来自模板（未写逐组回落出厂默认），每次构建重掷；池为空则不入
    // 变量，由模板 `{{key|默认值}}` 接管。
    let tiers = if pools.hair_style_by_length.is_empty() {
        &BUILTIN_POOLS.hair_style_by_length
    } else {
        &pools.hair_style_by_length
    };
    // 发长码查不到（脏数据）时依次回落到码 2 的档、再落到第一个档，不 panic。
    let style_pool = tiers
        .get(&rec.hair_length)
        .or_else(|| tiers.get("2"))
        .or_else(|| tiers.values().next());
    if let Some(p) = style_pool.filter(|p| !p.is_empty()) {
        let style = pick(p).to_string();
        vars.insert(
            "hairStyle".into(),
            // 发色并入发型出口：单一变量承载发色+剪法，光头（无色）不渲染发色。
            match &hair_color_name {
                Some(c) => format!("一头{c}，{style}"),
                None => style,
            },
        );
    }
    // 景别与动作联动随机：先选档、动作只从该档池内取（两变量不独立掷）；职员换职员池。
    let shots = if is_staff {
        if pools.staff.shots.is_empty() {
            &BUILTIN_POOLS.staff.shots
        } else {
            &pools.staff.shots
        }
    } else if pools.shots.is_empty() {
        &BUILTIN_POOLS.shots
    } else {
        &pools.shots
    };
    if !shots.is_empty() {
        let shot = &shots[rand::rng().random_range(0..shots.len())];
        vars.insert("shot".into(), shot.label.clone());
        if !shot.poses.is_empty() {
            vars.insert("pose".into(), pick(&shot.poses).to_string());
        }
    }
    // 球衣细节：职员换职员池；球员池在循环外选好，随其余四池一并随机。
    let (kit_pool, kit_fallback) = if is_staff {
        (&pools.staff.kit_detail, &BUILTIN_POOLS.staff.kit_detail)
    } else {
        (&pools.kit_detail, &BUILTIN_POOLS.kit_detail)
    };
    for (key, pool, fallback) in [
        ("beard", &pools.beard, &BUILTIN_POOLS.beard),
        ("expression", &pools.expression, &BUILTIN_POOLS.expression),
        ("lighting", &pools.lighting, &BUILTIN_POOLS.lighting),
        ("bgTone", &pools.bg_tone, &BUILTIN_POOLS.bg_tone),
        ("kitDetail", kit_pool, kit_fallback),
    ] {
        let cand = pool_or_builtin(pool, fallback);
        if !cand.is_empty() {
            vars.insert(key.into(), pick(cand).to_string());
        }
    }
    // 女性不随机胡须：移除该变量 → 模板默认值「干净无胡须」兜底；判据只看数据源类别，
    // 不再解释可能为空或本地化的性别文本。
    if rec.kind.is_female() {
        vars.remove("beard");
    }

    vars
}

// ===================== 单元测试：渲染契约 + build_vars 全部维度 ======================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_ai::types::{ShotPool, StaffPools};
    use crate::rtf_scanner::{DetailedInfo, PersonKind, PlayerRecord};

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// 取内置默认模板：所有 build_vars 用例的共同夹具。
    fn builtin_tpl() -> PromptTemplate {
        Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .expect("内置默认模板")
    }

    /// 造一条可指定 detailed 的样本记录（各测试互不影响，每次现造）。
    fn sample_record(detailed: Option<DetailedInfo>) -> PlayerRecord {
        PlayerRecord {
            uid: "2000123456".into(),
            primary_nat: "CHN".into(),
            sec_nat: "SWE".into(),
            name: "测试球员".into(),
            hair_length: "2".into(),
            hair_color: "3".into(),
            ethnicity_code: "0".into(),
            detailed,
            ..Default::default()
        }
    }

    /// 空模板（不写词库）：构造「回落出厂默认」路径的测试样本。
    fn blank_tpl() -> PromptTemplate {
        PromptTemplate {
            id: "t".into(),
            name: String::new(),
            body: String::new(),
            negative: String::new(),
            pools: DetailPools::default(),
        }
    }

    #[test]
    fn render_expands_defaults_and_preserves_edge_inputs() {
        // 展开：命中变量用变量值；缺变量有默认值用默认值、无默认值报缺失（去重、按首次出现序）。
        let r = render(
            "一名{{ethnicity}}球员，{{age}}岁",
            &vars(&[("ethnicity", "东亚"), ("age", "18")]),
        );
        assert_eq!(r.text, "一名东亚球员，18岁");
        assert!(r.missing.is_empty());
        let r = render("{{hairLength|短发}}的{{name|球员}}", &vars(&[]));
        assert_eq!(r.text, "短发的球员");
        assert_eq!(r.missing, vec!["hairLength", "name"]);
        let r = render("{{legacy}}{{legacy|旧}}", &vars(&[]));
        assert_eq!(r.text, "旧");
        assert_eq!(r.missing, vec!["legacy"], "同一缺失键只计一次");
        // 手写模板常带空格：占位符两侧空白不得导致查不中变量、或残留在展开结果里。
        let r = render("{{ age | 18 }}", &vars(&[]));
        assert_eq!(r.text, "18");
        assert_eq!(r.missing, vec!["age"]);
        let r = render("{{ age }}", &vars(&[("age", "24")]));
        assert_eq!(r.text, "24");
        assert!(r.missing.is_empty());
        // 半成品括号（编辑中被截断的 "{{"）与孤立 "}}"/"{" 一律按普通文本原样输出，不吞字符、不误报缺失。
        let r = render("只有 { 一个括号", &vars(&[]));
        assert_eq!(r.text, "只有 { 一个括号");
        let r = render("评分{{age 分", &vars(&[("age", "24")]));
        assert_eq!(r.text, "评分{{age 分");
        assert!(r.missing.is_empty());
        let r = render("a}}b", &vars(&[]));
        assert_eq!(r.text, "a}}b");
    }

    #[test]
    fn build_vars_expanded_roster_translates_codes() {
        // 拓展名单全字段：码值经模板映射转译，其余原文。
        let tpl = builtin_tpl();
        let vars = build_vars(
            &sample_record(Some(DetailedInfo {
                skin_code: "12".into(),
                gender: "男".into(),
                age: "24".into(),
                height: "182".into(),
                club: "Test FC".into(),
                ca: "120".into(),
                pa: "150".into(),
                ..Default::default()
            })),
            &tpl,
        );
        assert_eq!(vars["uid"], "2000123456");
        assert_eq!(vars["name"], "测试球员");
        assert_eq!(vars["primaryNat"], "CHN");
        assert_eq!(vars["secNat"], "SWE");
        assert_eq!(vars["nationality"], "中国"); // 三字码反查中文名
        assert_eq!(vars["ethnicity"], "北欧/白人"); // "0" 经模板映射转译
        assert!(
            !vars.contains_key("hairLength"),
            "发长译名不产出——词库分档已承载长度语义"
        );
        assert_eq!(vars["hairColor"], "金色"); // "3" 经模板映射转译
        assert!(
            vars["hairStyle"].starts_with("一头金色，"),
            "发色并入发型出口：有发为「一头{{色}}，{{剪法}}」，实际「{}」",
            vars["hairStyle"]
        );
        assert_eq!(vars["skinCode"], "红棕褐色（#b47454）"); // "12" → 色卡号文本
        assert_eq!(vars["gender"], "男");
        assert_eq!(vars["age"], "24");
        assert_eq!(vars["height"], "182");
        assert_eq!(vars["club"], "Test FC");
        assert_eq!(vars["jersey"], "Test FC的比赛球衣"); // 球衣：俱乐部优先
        assert_eq!(vars["ca"], "120");
        assert_eq!(vars["pa"], "150");
    }

    #[test]
    fn build_vars_basic_roster_randomizes_extended_fields() {
        // 基础名单 detailed=None：拓展字段本就不在 RTF 里，按约定随机补足而非空着；
        // RTF 同样不提供的随机细节（发型/景别+动作/胡须/表情/打光/背景/球衣细节）
        // 也必须存在（缺了会在模板里报未填充）；胡须由记录类别控制。
        let tpl = builtin_tpl();
        let vars = build_vars(&sample_record(None), &tpl);
        for key in ["skinCode", "gender", "age", "height"] {
            assert!(vars.contains_key(key), "基础名单应随机补足 {key}");
            assert!(!vars[key].is_empty(), "{key} 不得为空串");
        }
        // 基础名单 kind 恒男球员：性别按人员类别补足，必须落「男」而非随机。
        assert_eq!(vars["gender"], "男");
        for key in [
            "hairStyle",
            "shot",
            "expression",
            "pose",
            "lighting",
            "bgTone",
            "kitDetail",
        ] {
            assert!(vars.contains_key(key), "{key} 必须存在");
        }
        assert!(vars.contains_key("beard"), "男球员类别必须随机胡须");
        // 俱乐部不编造真实队名：缺省会退成国家队/通用球衣描述。
        assert!(!vars.contains_key("club"));
        assert!(!vars.contains_key("ca"));
        assert!(!vars.contains_key("pa"));
        assert_eq!(vars["jersey"], "中国国家队球衣");
        assert_eq!(vars["ethnicity"], "北欧/白人");
        assert_eq!(vars["identity"], "足球运动员"); // 基础名单无拓展字段，分辨不了职员

        // 球衣归属三级回落：俱乐部 → 国籍 → 通用描述；国籍码查不到中文名时
        // nationality 不入 map，球衣也不得编造队名。
        let mut rec = sample_record(None);
        rec.primary_nat = "ZZZ".into(); // 词表外的代码
        let vars = build_vars(&rec, &tpl);
        assert!(!vars.contains_key("nationality"));
        assert_eq!(vars["jersey"], "深色比赛球衣，带队长袖标与徽章");
    }

    #[test]
    fn build_vars_skips_empty_code_fields() {
        // 码值字段为空或映射查不到该码时一律不入 map（空串占位会让 render「命中变量」
        // 而不再展开默认值，裸数字对 AI 无意义）；skinCode/gender/age/height 属结构性
        // 缺席，会被随机补足——两条口径互不冲突。
        let tpl = builtin_tpl();
        let mut rec = PlayerRecord {
            uid: "u1".into(),
            primary_nat: "SWE".into(),
            sec_nat: String::new(),
            name: "n".into(),
            hair_length: String::new(),
            hair_color: String::new(),
            ethnicity_code: "99".into(), // 映射外的脏码：不入 map，由模板默认值兜底
            detailed: Some(DetailedInfo {
                skin_code: "99".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let vars = build_vars(&rec, &tpl);
        for key in ["ethnicity", "hairColor"] {
            assert!(!vars.contains_key(key), "空字段/映射外码 {key} 不应入 map");
        }
        assert!(
            !vars.contains_key("skinCode") || vars["skinCode"] != "99",
            "映射外的肤色码不得以裸数字入 map"
        );
        for key in ["gender", "age", "height"] {
            assert!(vars.contains_key(key), "{key} 应随机补足");
        }
        assert_eq!(vars["uid"], "u1");
        assert_eq!(vars["primaryNat"], "SWE");
        assert_eq!(vars["nationality"], "瑞典");
        // secNat 与 uid/name 同为「原文」字段：无条件入 map（空串也是其原文）。
        assert!(vars.contains_key("secNat"));
        // 空字段路径（码为空串）单独验证：同样不入 map。
        rec.ethnicity_code = String::new();
        rec.detailed = None;
        let vars = build_vars(&rec, &tpl);
        assert!(!vars.contains_key("ethnicity"));
        assert!(!vars.contains_key("hairColor"));
    }

    #[test]
    fn build_vars_unknown_hair_color_never_reaches_prompt() {
        // 发色码 1 = 未知：「未知」不得进提示词，改取映射内的真实色名；
        // 真实色名被用户删光（映射只剩「未知」档）时随机池为空 → 兜底「黑色」。
        let mut tpl = builtin_tpl();
        let mut rec = sample_record(None);
        rec.hair_color = "1".into();
        for _ in 0..50 {
            let vars = build_vars(&rec, &tpl);
            let got = &vars["hairColor"];
            assert_ne!(got, "未知");
            assert!(
                tpl.pools.hair_color.values().any(|v| v == got),
                "应是映射内的真实色名，实际 {got}"
            );
        }
        tpl.pools.hair_color = [("1".to_string(), "未知".to_string())]
            .into_iter()
            .collect();
        assert_eq!(build_vars(&rec, &tpl)["hairColor"], "黑色");
    }

    #[test]
    fn build_vars_hair_style_length_code_edges() {
        // 光头（发长码 0）不发色：避免「黑色光头」这类矛盾描述；长度语义由发型词库第 0 档承载。
        let tpl = builtin_tpl();
        let mut rec = sample_record(None);
        rec.hair_length = "0".into();
        let vars = build_vars(&rec, &tpl);
        assert!(!vars.contains_key("hairColor"));
        assert!(
            !vars["hairStyle"].starts_with("一头"),
            "光头发型不得带发色前缀"
        );
        // 光头档取词库原文（0 档可多候选）
        assert!(
            tpl.pools.hair_style_by_length["0"].contains(&vars["hairStyle"]),
            "光头发型「{}」应取自发长码 0 档词库",
            vars["hairStyle"]
        );

        // 发长码查不到（脏数据）→ 先落码 2 档；模板连码 2 档都没有 → 落 BTreeMap 第一档；
        // 命中的档位存在但为空 → 不入 map，交由模板默认值接管。
        let mut dirty = sample_record(None);
        dirty.hair_length = "9".into();
        dirty.hair_color = String::new(); // 隔离发色前缀：只验证剪法来源
        let vars = build_vars(&dirty, &tpl);
        assert!(
            tpl.pools.hair_style_by_length["2"].contains(&vars["hairStyle"]),
            "脏码应回落码 2 档，实际「{}」",
            vars["hairStyle"]
        );

        let mut other = tpl.clone();
        other.pools.hair_style_by_length = [("0".to_string(), vec!["第一档词".into()])]
            .into_iter()
            .collect();
        assert_eq!(
            build_vars(&dirty, &other)["hairStyle"],
            "第一档词",
            "码 2 档缺失时应落第一档"
        );

        let mut empty_tier = tpl.clone();
        empty_tier.pools.hair_style_by_length =
            [("2".to_string(), Vec::new())].into_iter().collect();
        let mut rec2 = sample_record(None);
        rec2.hair_length = "2".into();
        assert!(
            !build_vars(&rec2, &empty_tier).contains_key("hairStyle"),
            "档位为空时不由空串占位"
        );
    }

    #[test]
    fn build_vars_uses_person_kind_for_identity() {
        // 数据源类别决定身份路径；职员职位是自由文本，非空时原样渲染，空值交给模板默认值。
        let tpl = builtin_tpl();
        let mut staff = sample_record(Some(DetailedInfo {
            preferred_job: "足球总监".into(),
            ..Default::default()
        }));
        staff.kind = PersonKind::StaffMale;
        assert_eq!(build_vars(&staff, &tpl)["identity"], "足球总监");
        let player = sample_record(Some(DetailedInfo {
            based: "英格兰".into(),
            ..Default::default()
        }));
        let player_vars = build_vars(&player, &tpl);
        assert_eq!(player_vars["identity"], "足球运动员");
        assert_eq!(player_vars["career"], "效力于英格兰");
        // 英文职位原文与中文性别/国籍拼接时补中英边界空格（纯中文职位不受影响）
        let mut en_staff_record = sample_record(Some(DetailedInfo {
            preferred_job: "Performance Analyst".into(),
            ..Default::default()
        }));
        en_staff_record.kind = PersonKind::StaffMale;
        let en_staff = build_vars(&en_staff_record, &tpl);
        assert_eq!(en_staff["identity"], " Performance Analyst ");
        let mut blank_staff = sample_record(Some(DetailedInfo::default()));
        blank_staff.kind = PersonKind::StaffMale;
        let blank_staff_vars = build_vars(&blank_staff, &tpl);
        assert!(
            !blank_staff_vars.contains_key("identity"),
            "空职位的职员不得回落成球员称谓"
        );
        assert_eq!(blank_staff_vars["career"], "任职于足球俱乐部");
        // 基础名单无 based：career 不入 map，由模板默认值「效力于职业联赛」兜底
        assert!(!build_vars(&sample_record(None), &tpl).contains_key("career"));
    }

    #[test]
    fn build_vars_female_skips_beard() {
        // 女性类别不随机胡须（变量缺席 → 模板默认值「干净无胡须」兜底），男性类别照常随机；
        // 性别文本只负责提示词原文，不参与资源分流。
        let tpl = builtin_tpl();
        for gender in ["女", "Female"] {
            let mut record = sample_record(Some(DetailedInfo {
                gender: gender.into(),
                ..Default::default()
            }));
            record.kind = PersonKind::PlayerFemale;
            let vars = build_vars(&record, &tpl);
            assert!(
                !vars.contains_key("beard"),
                "性别原文「{gender}」不得随机胡须"
            );
        }
        // 性别原文为空串 + 女球员类别：补足必须落「女」，且无胡须。
        let mut record = sample_record(Some(DetailedInfo {
            gender: String::new(),
            ..Default::default()
        }));
        record.kind = PersonKind::PlayerFemale;
        let vars = build_vars(&record, &tpl);
        assert_eq!(vars["gender"], "女");
        assert!(!vars.contains_key("beard"));
        for gender in ["男", "Male"] {
            let mut record = sample_record(Some(DetailedInfo {
                gender: gender.into(),
                ..Default::default()
            }));
            record.kind = PersonKind::PlayerMale;
            let vars = build_vars(&record, &tpl);
            assert!(
                vars.contains_key("beard"),
                "性别原文「{gender}」必须随机胡须"
            );
        }
    }

    #[test]
    fn build_vars_staff_uses_staff_pools() {
        // 职员的服装/球衣细节/景别动作全部取自职员池——即使记录带 club 也不拼
        // 「{club}的比赛球衣」；球员对照路径仍走球员拼装与球员池。
        let tpl = builtin_tpl();
        let mut staff = sample_record(Some(DetailedInfo {
            preferred_job: "足球总监".into(),
            club: "曼联".into(),
            ..Default::default()
        }));
        staff.kind = PersonKind::StaffMale;
        for _ in 0..50 {
            let vars = build_vars(&staff, &tpl);
            assert_eq!(vars["career"], "任职于足球俱乐部");
            assert!(
                tpl.pools.staff.jersey.contains(&vars["jersey"]),
                "职员球衣「{}」应取自职员池",
                vars["jersey"]
            );
            assert!(
                tpl.pools.staff.kit_detail.contains(&vars["kitDetail"]),
                "职员球衣细节「{}」应取自职员池",
                vars["kitDetail"]
            );
            let shot = tpl
                .pools
                .staff
                .shots
                .iter()
                .find(|s| s.label == vars["shot"])
                .expect("职员景别应取自职员档位");
            assert!(
                shot.poses.contains(&vars["pose"]),
                "职员动作「{}」应落在景别「{}」的动作池内",
                vars["pose"],
                shot.label
            );
        }
        // 球员对照：同样带 club，仍是球员拼装口径
        let player = sample_record(Some(DetailedInfo {
            club: "曼联".into(),
            ..Default::default()
        }));
        assert_eq!(build_vars(&player, &tpl)["jersey"], "曼联的比赛球衣");

        // 手建模板没写职员池（blank_tpl）：职员路径逐子组回落出厂职员池——
        // 否则教练/队医会被随机到球员的踢球动作与比赛球衣。
        let mut medic = sample_record(Some(DetailedInfo {
            preferred_job: "队医".into(),
            ..Default::default()
        }));
        medic.kind = PersonKind::StaffMale;
        let vars = build_vars(&medic, &blank_tpl());
        assert!(
            BUILTIN_POOLS.staff.jersey.contains(&vars["jersey"]),
            "职员球衣「{}」应取自出厂职员池",
            vars["jersey"]
        );
        assert!(BUILTIN_POOLS.staff.kit_detail.contains(&vars["kitDetail"]));
        let shot = BUILTIN_POOLS
            .staff
            .shots
            .iter()
            .find(|s| s.label == vars["shot"])
            .expect("职员景别应落在出厂职员档位内");
        assert!(shot.poses.contains(&vars["pose"]));
        assert_eq!(vars["career"], "任职于足球俱乐部");
        assert_eq!(vars["identity"], "队医");
    }

    #[test]
    fn build_vars_random_skin_stays_within_ethnicity_band() {
        // 基础名单随机补足的肤色必须落在该族裔的合理色带内（含 1 档容差），
        // 否则会出现「北欧人配最深肤色」这类组合。东亚数据分位 5–8 → 容差后 4–9。
        let tpl = builtin_tpl();
        let mut rec = sample_record(None);
        rec.ethnicity_code = "10".into();
        let allowed: Vec<String> = (4..=9)
            .filter_map(|c| tpl.pools.skin_code.get(&c.to_string()).cloned())
            .collect();
        assert!(!allowed.is_empty());
        for _ in 0..50 {
            let vars = build_vars(&rec, &tpl);
            assert!(
                allowed.contains(&vars["skinCode"]),
                "肤色「{}」越出东亚色带",
                vars["skinCode"]
            );
        }
    }

    #[test]
    fn build_vars_prefers_template_pools_and_falls_back_to_builtin() {
        // 内容随模板走：模板写了自己的词库/码值映射就必须用它；没写（blank_tpl，全空）则
        // 逐组回落出厂默认——两个维度（词库/码值）在两条路径上都不允许静默消失。
        // 模板优先路径：样本沿用 sample_record 的发长码 "2"/发色码 "3"/族裔码 "0"。
        let mut tpl = builtin_tpl();
        tpl.pools = DetailPools {
            hair_style_by_length: [("2".to_string(), vec!["自定发型".into()])]
                .into_iter()
                .collect(),
            ethnicity: [("0".to_string(), "自定义族裔描述".to_string())]
                .into_iter()
                .collect(),
            hair_color: [("3".to_string(), "自定义金色".to_string())]
                .into_iter()
                .collect(),
            skin_code: [("12".to_string(), "自定义肤色".to_string())]
                .into_iter()
                .collect(),
            identity: [("player".to_string(), "自定义身份".to_string())]
                .into_iter()
                .collect(),
            shots: vec![ShotPool {
                label: "自定义景别".into(),
                poses: vec!["自定动作".into()],
            }],
            beard: vec!["自定胡须".into()],
            expression: vec!["自定表情".into()],
            lighting: vec!["自定打光".into()],
            bg_tone: vec!["自定背景".into()],
            kit_detail: vec!["自定球衣".into()],
            staff: StaffPools::default(),
        };

        let vars = build_vars(
            &sample_record(Some(DetailedInfo {
                skin_code: "12".into(),
                gender: "男".into(), // 提示词原文与样本的男球员类别保持一致
                ..Default::default()
            })),
            &tpl,
        );
        // 词库全部取自模板池子（景别与动作联动：动作必须落在选中档的池内；
        // 发色并入发型出口——「一头{色}，{剪法}」）
        assert_eq!(vars["hairStyle"], "一头自定义金色，自定发型");
        assert_eq!(vars["shot"], "自定义景别");
        assert_eq!(vars["pose"], "自定动作");
        assert_eq!(vars["beard"], "自定胡须");
        assert_eq!(vars["expression"], "自定表情");
        assert_eq!(vars["lighting"], "自定打光");
        assert_eq!(vars["bgTone"], "自定背景");
        assert_eq!(vars["kitDetail"], "自定球衣");
        // 码值映射全部取自模板映射
        assert_eq!(vars["ethnicity"], "自定义族裔描述");
        assert_eq!(vars["hairColor"], "自定义金色");
        assert_eq!(vars["skinCode"], "自定义肤色");

        // 回落路径：内置默认模板必须自带词库与码值映射（回落源非空是逐组回落的前提）。
        let builtin_tpl = builtin_tpl();
        assert!(
            !BUILTIN_POOLS.ethnicity.is_empty(),
            "内置默认模板必须自带码值映射"
        );
        assert!(!builtin_tpl.pools.is_empty(), "内置默认模板必须自带词库");

        let vars = build_vars(&sample_record(None), &blank_tpl());
        // 码值维度：族裔直接对照出厂映射；随机补足的肤色应落在出厂映射内
        assert_eq!(vars["ethnicity"], BUILTIN_POOLS.ethnicity["0"]);
        assert!(
            BUILTIN_POOLS
                .skin_code
                .values()
                .any(|v| *v == vars["skinCode"]),
            "随机补足的肤色「{}」应在出厂映射内",
            vars["skinCode"]
        );
        // 随机维度：各项细节都应取自出厂词库；动作应落在出厂景别档位内。
        // 样本的类别是男球员，胡须也必须落在出厂词库。
        for (key, pool) in [
            ("expression", &builtin_tpl.pools.expression),
            ("lighting", &builtin_tpl.pools.lighting),
            ("bgTone", &builtin_tpl.pools.bg_tone),
            ("kitDetail", &builtin_tpl.pools.kit_detail),
        ] {
            assert!(pool.contains(&vars[key]), "「{}」不在出厂词库里", vars[key]);
        }
        assert!(
            builtin_tpl.pools.beard.contains(&vars["beard"]),
            "「{}」不在出厂词库里",
            vars["beard"]
        );
        let shot = builtin_tpl
            .pools
            .shots
            .iter()
            .find(|s| s.label == vars["shot"])
            .expect("景别应在出厂档位内");
        assert!(
            shot.poses.iter().any(|p| *p == vars["pose"]),
            "动作「{}」应属于景别「{}」的档位池",
            vars["pose"],
            vars["shot"]
        );
        assert_eq!(vars["identity"], "足球运动员");
    }
}
