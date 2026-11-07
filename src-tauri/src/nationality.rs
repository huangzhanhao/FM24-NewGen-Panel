//! 国籍对照表（三字码 ↔ 各语言国籍全称）：出厂默认 + 用户可编辑副本。
//!
//! 【双层来源】出厂默认由编译期内嵌的 `assets/nat_translation.json`（各语言显示文本 → 三字码）
//! 与 `assets/nat_codes.json`（码全集）在首次访问时转换而来；用户机器上的可编辑副本是
//! `<data>/nationalities.json`，由 [`ensure_user_file`] 首次启动导出（已存在不覆盖），
//! [`load`] 之后一律以用户副本为准。两份资产继续只读内嵌，不随用户修改变化，升级时仍有兜底。
//!
//! 【文件格式】扁平三元组，便于直接手改，也与界面表格一行一条对应：
//! ```json
//! {
//!   "languages": ["English", "简体中文"],
//!   "codes": ["CHN", "USA"],
//!   "mappings": [
//!     { "code": "CHN", "language": "English", "name": "China" },
//!     { "code": "CHN", "language": "简体中文", "name": "中国" }
//!   ]
//! }
//! ```
//! `codes` 是码全集，与全称解耦：可以只登记码、暂不给任何语言的全称（新增码不必同时补中/英文全称）。
//! 这种码本身仍是合法国籍值（名单里原样写着该码的行不会被丢），只是查不到它的名字。
//! 同码同语言可有多条全称/别名（如「中国」「China」「China PR」）；同语言下同一全称只能指向一个码，否则解析无定义、校验拒绝。
//!
//! 【生效方式】保存 = 校验 → 原子写盘 → [`install`] 换当前表；命令入口走 [`refresh`] 重读盘，
//! 故手改文件后下一次扫描/替换即生效。`RtfScanner` 构造时取快照，AI 提示词实时读当前表。
//!
//! 【边界】本表只回答「文本是哪个码」与「码的中文名是什么」；
//! 码到头像目录的映射在 `data/rules/standardN.json`（templates 模块），两者互不覆盖。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

use crate::json_store::{self, LoadError};

/// 用户副本文件名（`<data>/nationalities.json`）。
pub const FILE_NAME: &str = "nationalities.json";
/// 英文语言节点名：RTF 未识别到显示文本时按它兜底（与旧版行为一致）。
pub const EN_LANGUAGE: &str = "English";
/// 简体中文语言节点名：AI 提示词 `nationality` 变量取这个语言下的规范名。
pub const CN_LANGUAGE: &str = "简体中文";

/// 一条对照映射：三字码 + 语言 + 该语言下的国籍全称/别名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NationalityMapping {
    pub code: String,
    pub language: String,
    pub name: String,
}

/// 用户可编辑的对照表文档（`data/nationalities.json` 的落盘形状，也是前端编辑器的数据契约）。
/// `languages` 决定界面列顺序与解析时的语言优先级；`codes` 是码全集；`mappings` 一行一条，已按 code/语言/全称排序。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NationalityDoc {
    pub languages: Vec<String>,
    /// 码全集：可以只有码、还没有任何语言的全称（新增码不必同时补中/英文全称）。
    /// 缺省时按 `mappings` 推导，没有该字段的旧副本照常可读。
    #[serde(default)]
    pub codes: Vec<String>,
    pub mappings: Vec<NationalityMapping>,
}

/// 校验后的对照表：文档 + 索引 + 派生结论。
/// 【为什么带索引】RTF 逐行要按「文本 → 码」查表，线性扫 mappings 在上万行名单上太慢；
/// `by_language` 是构建期一次性建好的倒排表，解析时只做哈希/有序表查找。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NationalityTable {
    /// 语言节点顺序（文档里的 languages，去重后的结果）。
    languages: Vec<String>,
    /// 规范化并排序后的映射全集（保存/展示都以此为准）。
    mappings: Vec<NationalityMapping>,
    /// 码全集：值本身已是国籍码时直接放行（如 "CHN"、两位特例 "SC"）；含暂无全称的码。
    codes: BTreeSet<String>,
    /// 语言 → {显示文本: 三字码}：RTF 语言识别与全称归一化的主索引。
    by_language: BTreeMap<String, BTreeMap<String, String>>,
    /// 三字码 → 简体中文规范名：同一码有多个译名时取最长（同长取字典序小），与旧版口径一致。
    cn_by_code: BTreeMap<String, String>,
}

impl NationalityTable {
    /// 从文档构建并校验。所有字段先去首尾空白；三字码统一转大写（用户小写输入也接受）。
    /// 失败信息直接展示给用户，故一律指明第几条映射与具体原因。
    pub fn from_doc(doc: NationalityDoc) -> Result<Self, String> {
        // ---- 语言列表：去空白、去重（保留用户给定顺序） -------------------------------------------------------------
        let mut languages: Vec<String> = Vec::new();
        for (i, raw) in doc.languages.into_iter().enumerate() {
            let lang = raw.trim();
            if lang.is_empty() {
                return Err(format!("语言列表第 {} 项为空", i + 1));
            }
            if !languages.iter().any(|l| l == lang) {
                languages.push(lang.to_string());
            }
        }
        if languages.is_empty() {
            return Err("语言列表不能为空（至少保留一个语言节点）".into());
        }

        // ---- 码清单：只要码、不要全称也合法（新增码不必同时补中/英文全称） ------------------------------------------
        let mut codes: BTreeSet<String> = BTreeSet::new();
        for (i, raw) in doc.codes.into_iter().enumerate() {
            let code = normalize_code(&raw).map_err(|e| format!("码清单第 {} 项的{e}", i + 1))?;
            codes.insert(code);
        }

        // ---- 映射行：逐条校验、同一语言下的显示文本去重并查冲突 -----------------------------------------------------
        let mut mappings: Vec<NationalityMapping> = Vec::new();
        // (语言, 显示文本) → 已占用的码：同一语言下同一个全称只能指向一个码。
        let mut claimed: BTreeMap<(String, String), String> = BTreeMap::new();
        for (i, raw) in doc.mappings.into_iter().enumerate() {
            let n = i + 1;
            let code = normalize_code(&raw.code).map_err(|e| format!("第 {n} 条映射的{e}"))?;
            let language = raw.language.trim().to_string();
            let name = raw.name.trim().to_string();
            if language.is_empty() {
                return Err(format!("第 {n} 条映射的语言为空"));
            }
            if !languages.iter().any(|l| l == &language) {
                return Err(format!(
                    "第 {n} 条映射的语言「{language}」不在语言列表中；请先添加该语言"
                ));
            }
            if name.is_empty() {
                return Err(format!("第 {n} 条映射（{code} / {language}）的国籍全称为空"));
            }
            let key = (language.clone(), name.clone());
            match claimed.get(&key) {
                Some(existing) if existing == &code => continue, // 完全重复的一行：静默去重
                Some(existing) => {
                    return Err(format!(
                        "「{name}」（{language}）同时映射到 {existing} 与 {code}；同一语言下的全称必须唯一"
                    ))
                }
                None => {
                    claimed.insert(key, code.clone());
                    mappings.push(NationalityMapping { code, language, name });
                }
            }
        }

        // ---- 排序：码升序 → 语言按 languages 给定顺序 → 全称升序（保证保存结果可复现） ------------------------------
        let lang_pos = |lang: &str| languages.iter().position(|l| l == lang).unwrap_or(usize::MAX);
        mappings.sort_by(|a, b| {
            a.code
                .cmp(&b.code)
                .then_with(|| lang_pos(&a.language).cmp(&lang_pos(&b.language)))
                .then_with(|| a.name.cmp(&b.name))
        });

        // ---- 建索引 -------------------------------------------------------------------------------------------------
        let mut by_language: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut cn_by_code: BTreeMap<String, String> = BTreeMap::new();
        for m in &mappings {
            codes.insert(m.code.clone());
            by_language
                .entry(m.language.clone())
                .or_default()
                .insert(m.name.clone(), m.code.clone());
            if m.language == CN_LANGUAGE {
                let better = match cn_by_code.get(&m.code) {
                    None => true,
                    Some(cur) => (m.name.chars().count(), &m.name) > (cur.chars().count(), cur),
                };
                if better {
                    cn_by_code.insert(m.code.clone(), m.name.clone());
                }
            }
        }
        if codes.is_empty() {
            return Err(
                "至少保留一个国籍码或一条国籍映射；空表会让所有名单行都因国籍无法识别而被丢弃".into(),
            );
        }

        Ok(Self {
            languages,
            mappings,
            codes,
            by_language,
            cn_by_code,
        })
    }

    /// 出厂默认表：内嵌资产转成新格式（首次访问构建一次，之后 clone）。
    /// 注意：`builtin()` 每次 clone 整表；热路径请用 [`builtin_shared`](Self::builtin_shared)。
    pub fn builtin() -> Self {
        (*Self::builtin_shared()).clone()
    }

    /// 出厂默认的共享句柄：测试与未显式传表的调用只涨引用计数。
    pub fn builtin_shared() -> Arc<Self> {
        static BUILTIN: LazyLock<Arc<NationalityTable>> = LazyLock::new(|| Arc::new(build_builtin()));
        Arc::clone(&BUILTIN)
    }

    /// 导出为文档（排序后的 languages、码全集与 mappings）。
    pub fn doc(&self) -> NationalityDoc {
        NationalityDoc {
            languages: self.languages.clone(),
            codes: self.codes.iter().cloned().collect(),
            mappings: self.mappings.clone(),
        }
    }

    /// 解析国籍值 →（命中语言, 三字码）。
    /// - 命中某语言显示文本：返回语言名与该文本对应的码；
    /// - 值本身是国籍码：语言 None，码取原值；
    /// - 空值或未收录：None（调用方据此判非法行）。
    /// 语言按 `languages` 顺序查，同一文本出现在多个语言节点时（用户自建时可能发生）以靠前者为准，结果确定。
    pub fn resolve(&self, value: &str) -> Option<(Option<&str>, String)> {
        if value.is_empty() {
            return None;
        }
        for lang in &self.languages {
            if let Some(code) = self.by_language.get(lang).and_then(|t| t.get(value)) {
                return Some((Some(lang.as_str()), code.clone()));
            }
        }
        self.is_code(value).then(|| (None, value.to_string()))
    }

    /// 值命中的语言节点名（用于 RTF 语言识别）；已是国籍码或未收录返回 None。
    pub fn language_of(&self, value: &str) -> Option<&str> {
        self.resolve(value).and_then(|(lang, _)| lang)
    }

    /// 值本身是否是已收录的国籍码。
    pub fn is_code(&self, value: &str) -> bool {
        self.codes.contains(value)
    }

    /// 三字码 → 简体中文规范名（取该码下最长的中文全称）。
    pub fn cn_name(&self, code: &str) -> Option<&str> {
        self.cn_by_code.get(code).map(String::as_str)
    }

    /// 已收录的国籍码个数。
    pub fn code_count(&self) -> usize {
        self.codes.len()
    }

    /// 映射条数（同码同语言的全称与别名各算一条）。
    pub fn mapping_count(&self) -> usize {
        self.mappings.len()
    }
}

/// 码的规范化与合法性校验：FM 国籍码是 2–3 位大写字母（"SC" 是两位特例）；
/// 用户小写输入统一转大写，数字/超长一律拒绝。
fn normalize_code(raw: &str) -> Result<String, String> {
    let code = raw.trim().to_ascii_uppercase();
    if code.is_empty() {
        return Err("三字码为空".into());
    }
    if !(2..=3).contains(&code.len()) || !code.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(format!("三字码「{code}」不合法：只能是 2–3 位大写英文字母"));
    }
    Ok(code)
}

/// 内嵌资产的转换结果。校验失败属于开发期错误（资产与代码不同步），直接 panic 暴露。
fn build_builtin() -> NationalityTable {
    let translations = &*crate::assets::NAT_TRANSLATION;
    // 语言顺序：English → 简体中文 → 其余按名称升序，决定语言识别优先级与界面顺序。
    let mut languages: Vec<String> = translations.keys().cloned().collect();
    languages.sort_by(|a, b| (language_rank(a), a).cmp(&(language_rank(b), b)));
    // 码 → 该码在各语言下的全部显示文本（别名各占一条）。
    let mut rows: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    for (lang, table) in translations.iter() {
        for (name, code) in table {
            rows.entry(code.clone())
                .or_default()
                .insert((lang.clone(), name.clone()));
        }
    }
    let mut mappings = Vec::new();
    for (code, names) in rows {
        for (language, name) in names {
            mappings.push(NationalityMapping {
                code: code.clone(),
                language,
                name,
            });
        }
    }
    // 码全集来自 nat_codes.json：其中可能含暂无译名的码（无映射行），
    // 但「值本身已是国籍码」仍须放行，且随文档一起落到用户副本。
    let codes: Vec<String> = crate::assets::NAT_CODES.iter().cloned().collect();
    NationalityTable::from_doc(NationalityDoc { languages, mappings, codes })
        .unwrap_or_else(|e| panic!("内置国籍对照表不合法：{e}"))
}

fn language_rank(lang: &str) -> u8 {
    match lang {
        EN_LANGUAGE => 0,
        CN_LANGUAGE => 1,
        _ => 2,
    }
}

// ===================== 落盘 / 进程内当前表 ===========================================================================

/// 用户副本路径：`<data>/nationalities.json`。
pub fn file_path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// 首次运行导出出厂默认（已存在不覆盖，避免冲掉用户修改）。
/// 【注意：升级不合并】内嵌资产新增的语言/别名不会自动补进旧副本；要回到出厂内容
/// 只能删掉该文件再重启（[`refresh`] 会重新导出）。改成「总是覆盖」会冲掉用户手改内容。
pub fn ensure_user_file(data_dir: &Path) -> Result<(), String> {
    let path = file_path(data_dir);
    if path.is_file() {
        return Ok(());
    }
    json_store::save(&path, &NationalityTable::builtin().doc())
}

/// 读取生效表：用户副本存在则解析校验（错误含文件路径），缺失回退出厂默认。
pub fn load(data_dir: &Path) -> Result<NationalityTable, String> {
    let path = file_path(data_dir);
    match json_store::load::<NationalityDoc>(&path) {
        Ok(doc) => NationalityTable::from_doc(doc).map_err(|e| format!("国籍对照表无效：{}（{e}）", path.display())),
        Err(LoadError::Missing) => Ok(NationalityTable::builtin()),
        Err(LoadError::Corrupt(e)) => Err(format!("国籍对照表无法解析：{}（{e}）", path.display())),
    }
}

/// 校验 + 原子写盘（不改变进程内当前表；调用方随后 [`install`]）。
pub fn save_doc(data_dir: &Path, doc: NationalityDoc) -> Result<NationalityTable, String> {
    let table = NationalityTable::from_doc(doc).map_err(|e| format!("国籍对照表无效：{e}"))?;
    json_store::save(&file_path(data_dir), &table.doc())?;
    Ok(table)
}

/// 重读盘、换成当前表，并返回本轮的共享句柄。
/// 命令入口在扫描/替换前调用：手改文件即时生效，且本轮流程固定持有这份表，不受后续安装影响。
/// 文件缺失时先补一份出厂副本，保证 data/ 里始终有可编辑文件。
pub fn refresh(data_dir: &Path) -> Result<Arc<NationalityTable>, String> {
    ensure_user_file(data_dir)?;
    let table = Arc::new(load(data_dir)?);
    install(Arc::clone(&table));
    Ok(table)
}

/// 进程内当前表：默认内置，启动 / 读取 / 保存后更新。
/// 【使用口径】扫描与替换走 [`refresh`] 的 Arc 显式传入本轮流程（`RtfScanner` / `ReplaceParams`）；
/// 全局表只服务没有表参数的路径（AI 提示词的国籍中文名、测试直接构造扫描器）。表是只读值类型，
/// 写入仅「启动 / 读取 / 保存」三处，读侧取 Arc 快照即隔离。
static INSTALLED: LazyLock<RwLock<Option<Arc<NationalityTable>>>> = LazyLock::new(|| RwLock::new(None));

/// 换掉进程内当前表（保存后立即生效）。
pub fn install(table: Arc<NationalityTable>) {
    *INSTALLED.write().unwrap() = Some(table);
}

/// 取当前表的共享句柄（尚未安装时用出厂默认）。
pub fn current() -> Arc<NationalityTable> {
    match INSTALLED.read().unwrap().as_ref() {
        Some(t) => Arc::clone(t),
        None => NationalityTable::builtin_shared(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with(languages: &[&str], mappings: &[(&str, &str, &str)]) -> NationalityDoc {
        NationalityDoc {
            languages: languages.iter().map(|s| s.to_string()).collect(),
            codes: Vec::new(),
            mappings: mappings
                .iter()
                .map(|(code, language, name)| NationalityMapping {
                    code: code.to_string(),
                    language: language.to_string(),
                    name: name.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn builtin_resolves_names_and_codes() {
        let t = NationalityTable::builtin();
        // 中文/英文全称都归一化成同一个码，并带回命中的语言节点。
        assert_eq!(t.resolve("中国"), Some((Some(CN_LANGUAGE), "CHN".into())));
        assert_eq!(t.resolve("China"), Some((Some(EN_LANGUAGE), "CHN".into())));
        // 已是国籍码：语言为 None，码原样保留（两位特例 SC 也算码）。
        assert_eq!(t.resolve("CHN"), Some((None, "CHN".into())));
        assert_eq!(t.resolve("SC"), Some((None, "SC".into())));
        assert_eq!(t.resolve("Atlantis"), None);
        assert_eq!(t.resolve(""), None);
        // 中文规范名取最长的别名（ALB 同时收录「阿尔巴」与「阿尔巴尼亚」）。
        assert_eq!(t.cn_name("ALB"), Some("阿尔巴尼亚"));
        assert_eq!(t.cn_name("CHN"), Some("中国"));
        assert_eq!(t.cn_name("XXX"), None);
        assert_eq!(t.language_of("中国"), Some(CN_LANGUAGE));
        assert_eq!(t.language_of("CHN"), None);
        assert!(t.mapping_count() > 0);
        assert!(t.code_count() > 0);
    }

    #[test]
    fn builtin_covers_every_embedded_code() {
        // 两份内嵌资产不能脱节：nat_codes.json 的每个码都必须能在转换后的表里识别，
        // 且导出成用户副本后仍在（码与全称解耦，码不能只在内存里活着）。
        let t = NationalityTable::builtin();
        let doc = t.doc();
        for code in crate::assets::NAT_CODES.iter() {
            assert!(t.is_code(code), "码 {code} 不在内置对照表中");
            assert!(doc.codes.iter().any(|c| c == code), "码 {code} 未导出到用户副本");
        }
    }

    #[test]
    fn code_without_name_is_recognized_and_survives_roundtrip() {
        // 新增码可以暂不给任何语言全称：值本身已是该码时按合法国籍处理，
        // 查不到名字只是没有名字（提示词走模板默认值兜底），不能因此判非法或丢行。
        let doc = NationalityDoc {
            languages: vec![EN_LANGUAGE.into(), CN_LANGUAGE.into()],
            codes: vec!["XYZ".into()],
            mappings: vec![NationalityMapping {
                code: "CHN".into(),
                language: EN_LANGUAGE.into(),
                name: "China".into(),
            }],
        };
        let t = NationalityTable::from_doc(doc).unwrap();
        assert!(t.is_code("XYZ"));
        assert_eq!(t.resolve("XYZ"), Some((None, "XYZ".into())));
        assert_eq!(t.cn_name("XYZ"), None);
        // 导出再读回，码不丢（用户副本往返后名单里的该码仍然合法）。
        let again = NationalityTable::from_doc(t.doc()).unwrap();
        assert!(again.is_code("XYZ"));
        assert_eq!(again.doc(), t.doc());
    }

    #[test]
    fn doc_without_codes_field_still_loads() {
        // 老副本只有 languages + mappings（没有 codes 字段）：码按映射行推导，照常可读。
        let raw = r#"{"languages":["English"],"mappings":[{"code":"CHN","language":"English","name":"China"}]}"#;
        let doc: NationalityDoc = serde_json::from_str(raw).unwrap();
        let t = NationalityTable::from_doc(doc).unwrap();
        assert!(t.is_code("CHN"));
        assert_eq!(t.code_count(), 1);
    }

    #[test]
    fn doc_roundtrip_is_stable() {
        let t = NationalityTable::builtin();
        let doc = t.doc();
        let again = NationalityTable::from_doc(doc.clone()).unwrap();
        // 排序与语言顺序原样保留，重复构建不会漂移（保存结果可复现）。
        assert_eq!(again.doc(), doc);
        assert_eq!(again, t);
    }

    #[test]
    fn normalization_trims_and_uppercases() {
        let t = NationalityTable::from_doc(doc_with(
            &[" English ", "简体中文", "English"],
            &[(" chn ", " English ", " China ")],
        ))
        .unwrap();
        assert_eq!(t.doc().languages, vec!["English", "简体中文"]);
        assert_eq!(
            t.doc().mappings,
            vec![NationalityMapping {
                code: "CHN".into(),
                language: "English".into(),
                name: "China".into(),
            }]
        );
        assert_eq!(t.resolve("China"), Some((Some("English"), "CHN".into())));
    }

    #[test]
    fn duplicate_identical_rows_are_deduped() {
        let t = NationalityTable::from_doc(doc_with(
            &["English"],
            &[("CHN", "English", "China"), ("chn", "English", "China")],
        ))
        .unwrap();
        assert_eq!(t.mapping_count(), 1);
    }

    #[test]
    fn rejects_invalid_documents() {
        // 空语言列表 / 空映射表
        assert!(NationalityTable::from_doc(doc_with(&[], &[])).is_err());
        assert!(NationalityTable::from_doc(doc_with(&["English"], &[])).is_err());
        // 三字码不合法（1 位、4 位、数字）
        for code in ["C", "CHIN", "12"] {
            let err = NationalityTable::from_doc(doc_with(&["English"], &[(code, "English", "X")])).unwrap_err();
            assert!(err.contains("三字码"), "{code}: {err}");
        }
        // 空全称
        assert!(NationalityTable::from_doc(doc_with(&["English"], &[("CHN", "English", " ")])).is_err());
        // 语言不在列表里
        let err = NationalityTable::from_doc(doc_with(&["English"], &[("CHN", "Deutsch", "China")])).unwrap_err();
        assert!(err.contains("不在语言列表"), "{err}");
        // 同一语言下同一全称指向两个码
        let err = NationalityTable::from_doc(doc_with(
            &["English"],
            &[("CHN", "English", "China"), ("TWN", "English", "China")],
        ))
        .unwrap_err();
        assert!(err.contains("必须唯一"), "{err}");
        // 码清单里的非法码
        let err = NationalityTable::from_doc(NationalityDoc {
            languages: vec!["English".into()],
            codes: vec!["CHIN".into()],
            mappings: Vec::new(),
        })
        .unwrap_err();
        assert!(err.contains("码清单"), "{err}");
    }

    #[test]
    fn languages_control_resolution_priority() {
        // 同一文本在两个语言节点里都出现：按 languages 的顺序先命中 English。
        let t = NationalityTable::from_doc(doc_with(
            &["English", "简体中文"],
            &[("CHN", "English", "China"), ("CHN", "简体中文", "China")],
        ))
        .unwrap();
        assert_eq!(t.resolve("China"), Some((Some("English"), "CHN".into())));
        assert_eq!(t.cn_name("CHN"), Some("China"));
    }

    #[test]
    fn cn_name_prefers_longest_alias() {
        let t = NationalityTable::from_doc(doc_with(
            &["English", "简体中文"],
            &[("ALB", "简体中文", "阿尔巴"), ("ALB", "简体中文", "阿尔巴尼亚")],
        ))
        .unwrap();
        assert_eq!(t.cn_name("ALB"), Some("阿尔巴尼亚"));
    }

    #[test]
    fn ensure_user_file_exports_and_load_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!file_path(dir.path()).exists());
        ensure_user_file(dir.path()).unwrap();
        assert!(file_path(dir.path()).is_file());
        // 再调一次不覆盖：模拟用户已改过文件后重启。
        let before = std::fs::read_to_string(file_path(dir.path())).unwrap();
        ensure_user_file(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(file_path(dir.path())).unwrap(), before);

        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded.doc(), NationalityTable::builtin().doc());
    }

    #[test]
    fn save_doc_validates_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        ensure_user_file(dir.path()).unwrap();
        let before = std::fs::read_to_string(file_path(dir.path())).unwrap();
        // 非法文档：写盘前就被拦下，原文件一个字节都不变。
        let bad = doc_with(&["English"], &[("CHN", "English", "")]);
        assert!(save_doc(dir.path(), bad).is_err());
        assert_eq!(std::fs::read_to_string(file_path(dir.path())).unwrap(), before);

        let ok = doc_with(
            &["English", "简体中文"],
            &[("CHN", "English", "China"), ("CHN", "简体中文", "中国")],
        );
        let table = save_doc(dir.path(), ok.clone()).unwrap();
        // 落盘文档补齐码全集（码由映射行派生），码与全称各自承载。
        let mut expected = ok;
        expected.codes = vec!["CHN".into()];
        assert_eq!(table.doc(), expected);
        assert_eq!(load(dir.path()).unwrap().doc(), expected);
    }
}
