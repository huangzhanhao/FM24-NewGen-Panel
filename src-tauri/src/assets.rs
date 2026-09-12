//! 【新手导读】只读映射表与标准规则默认数据：编译期嵌入，内容随版本分发。
//! 【关键手法】include_str!("...") 在「编译时」就把文件内容读进来变成字符串常量，
//! 打包后不依赖外部文件。配合 LazyLock，这些数据在首次使用时解析一次，之后全局共享。
use std::collections::HashMap; // HashMap=无序字典（查找快）
use std::sync::LazyLock;                   // 「懒加载 + 线程安全」的全局静态量：首次访问时才初始化

// ---------------- 标准规则内置默认（templates::builtin 解析并导出为用户文件） ----------------
// 注意：这两个文件是「出厂默认」，用户机器上的可编辑副本在 <data>/rules/standardN.json，
// 由 templates::ensure_user_files 首次启动时导出（已存在不覆盖）。
/// 标准二（14 种族目录）规则默认：FolderList + Ethnics（国籍码→目录）+ CodeRules（码修正）。
pub const STANDARD2_RAW: &str = include_str!("../assets/standard2.json");
/// 标准三（细分目录）规则默认：FolderList + Ethnics + DefaultCodes + 族群映射规则块 + Fallback。
pub const STANDARD3_RAW: &str = include_str!("../assets/standard3.json");

// ---------------- 国籍对照表（基础数据，非标准规则） ----------------
/// 语言 → {国籍显示文本: 英文三字码}。
/// 【作用】解析 RTF 名单时，把各语言显示的国籍文本归一化成三字码（如「美国」/"U.S.A." → "USA"），
/// 方便后续模块统一按码处理。内层同时收录全称与简称/截断别名（如「美国」与列宽截断的「阿根」）。
/// 【扩展】新增语言（德语、法语…）只需在 JSON 顶层加一个同名节点，扫描器按名单语言自动查表。
/// 嵌套 HashMap：外层 key 是语言（如 "简体中文"），内层是「显示文本 → 三字码」的对照表。
pub static NAT_TRANSLATION: LazyLock<HashMap<String, HashMap<String, String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/nat_translation.json"))
        .expect("nat_translation.json 格式错误")
});

/// FM 全部三字母国籍码清单（nat_codes.json，与翻译表的码一一对应，另含暂无译名的码）。
/// 【作用】国籍码的权威全集参照：校验翻译表覆盖度、供后续按码筛选/下拉列表等功能使用。
/// 【allow(dead_code)】当前仅在测试中消费，属随版本分发的数据资产。
#[allow(dead_code)]
pub static NAT_CODES: LazyLock<Vec<String>> = LazyLock::new(|| {
    #[derive(serde::Deserialize)]
    struct Codes {
        codes: Vec<String>,
    }
    let parsed: Codes = serde_json::from_str(include_str!("../assets/nat_codes.json"))
        .expect("nat_codes.json 格式错误");
    parsed.codes
});

/// config.xml 固定头尾模板，[players] 为占位符。
/// 【作用】生成 config.xml 时，把球员映射行填到 [players] 位置，其余头尾结构保持不变（FM 要求的格式）。
/// const 是编译期常量；include_str! 同样在编译时把模板文件内容嵌进来。
pub const CONFIG_TEMPLATE: &str = include_str!("../assets/config_template");

#[cfg(test)]
mod tests {
    use super::*;

    /// nat_codes.json 码全集必须覆盖翻译表各语言节点的全部三字码（防两份资产脱节）。
    /// 非 3 位字母的值（如中文表把「海外华侨」映射到显示值 'SC'）不属于码体系，跳过。
    #[test]
    fn nat_codes_cover_translation_table() {
        let codes: std::collections::HashSet<_> = NAT_CODES.iter().collect();
        assert!(!codes.is_empty());
        for (lang, table) in NAT_TRANSLATION.iter() {
            assert!(!table.is_empty(), "{lang} 节点为空");
            for code in table.values().filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_uppercase())) {
                assert!(
                    codes.contains(code),
                    "{lang} 节点的码 {code} 不在 nat_codes.json"
                );
            }
        }
    }
}
