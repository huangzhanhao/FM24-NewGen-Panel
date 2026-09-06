//! 【新手导读】只读映射表与模板规则默认数据：编译期嵌入，内容随版本分发。
//! 【关键手法】include_str!("...") 在「编译时」就把文件内容读进来变成字符串常量，
//! 打包后不依赖外部文件。配合 LazyLock，这些数据在首次使用时解析一次，之后全局共享。
use std::collections::HashMap; // HashMap=无序字典（查找快）
use std::sync::LazyLock;                   // 「懒加载 + 线程安全」的全局静态量：首次访问时才初始化

// ---------------- 模板规则内置默认（rule_config::builtin 解析并导出为用户文件） ----------------
// 注意：这两个文件是「出厂默认」，用户机器上的可编辑副本在 <data>/rules/templateN.json，
// 由 rule_config::ensure_user_files 首次启动时导出（已存在不覆盖）。
/// 模板二（14 种族目录）规则默认：FolderList + Ethnics（国籍码→目录）+ CodeRules（码修正）。
pub const TEMPLATE2_RAW: &str = include_str!("../assets/template2.json");
/// 模板三（细分目录）规则默认：FolderList + DefaultCodes + Mappings + Fallback。
pub const TEMPLATE3_RAW: &str = include_str!("../assets/template3.json");

// ---------------- 国籍对照表（基础数据，非模板规则） ----------------
/// 语言 → {中文国籍全称: 英文三字码}。
/// 【作用】解析中文 RTF 名单时，把「美国」这样的中文国籍翻译成 "USA"，方便后续统一处理。
/// 嵌套 HashMap：外层 key 是语言（如 "简体中文"），内层是「中文名 → 三字码」的对照表。
pub static NAT_TRANSLATION: LazyLock<HashMap<String, HashMap<String, String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/nat_translation.json"))
        .expect("nat_translation.json 格式错误")
});

/// config.xml 固定头尾模板，[players] 为占位符。
/// 【作用】生成 config.xml 时，把球员映射行填到 [players] 位置，其余头尾结构保持不变（FM 要求的格式）。
/// const 是编译期常量；include_str! 同样在编译时把模板文件内容嵌进来。
pub const CONFIG_TEMPLATE: &str = include_str!("../assets/config_template");
