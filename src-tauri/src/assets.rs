//! 编译期嵌入的只读数据资产：标准规则出厂默认、国籍对照表、config.xml 模板。
//!
//! 【嵌入方式】include_str! 在编译时把文件内容读成字符串常量，打包后不依赖外部文件；
//! 需要按结构使用的（规则 json、国籍表）再用 LazyLock 在首次访问时解析一次、全局共享。
//!
//! 【双层来源】本文件只提供「出厂默认」；用户机器上的可编辑副本在 <data>/rules/standardN.json
//! 与 <data>/nationalities.json，由 templates::ensure_user_files / nationality::ensure_user_file
//! 首次启动时导出（已存在不覆盖），用哪份分别由 RuleConfig::load / nationality::load 决定。
use std::collections::HashMap;
use std::sync::LazyLock;

// ---------------- 标准规则内置默认（templates::builtin 解析并导出为用户文件） ----------------
/// 标准二（14 种族目录）规则的出厂默认：FolderList + Ethnics（国籍码→目录）+ CodeRules（种族码修正）。
pub const STANDARD2_RAW: &str = include_str!("../assets/standard2.json");
/// 标准三（细分目录）规则的出厂默认：FolderList + Ethnics + DefaultCodes + 族群映射规则块 + Fallback。
pub const STANDARD3_RAW: &str = include_str!("../assets/standard3.json");
/// AI 文生图出厂服务商与设置（image_ai::Config::builtin 解析；首次启动导出 data/image_ai.json）。
pub const IMAGE_AI_RAW: &str = include_str!("../assets/image_ai.json");
/// AI 文生图出厂提示词模板（含码值映射 codes 与随机词库 pools，导出 data/prompts/templates.json）。
pub const IMAGE_AI_TEMPLATES_RAW: &str = include_str!("../assets/image_ai_templates.json");

// ---------------- 国籍对照表（基础数据，非标准规则） ----------------
/// 语言 → {国籍显示文本: 三字码}，本文件只作**出厂默认**：启动时转成 `data/nationalities.json`
/// 的用户副本（见 nationality 模块），之后以用户副本为准。
/// 扫描器解析名单时按数据语言查当前表，把各语言的国籍显示文本归一化成三字码
/// （如「美国」/"U.S.A." → "USA"）；detect 判断数据语言也靠它——国籍值命中哪个语言节点就是哪种语言。
/// 内层同时收录全称与简称/截断别名（如「美国」与列宽截断的「阿根」）。
pub static NAT_TRANSLATION: LazyLock<HashMap<String, HashMap<String, String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/nat_translation.json"))
        .expect("nat_translation.json 格式错误")
});

/// FM 全部国籍码的权威全集（nat_codes.json）：三位码、两位特例 "SC"、暂无译名的码都收录。
/// 国籍模块用它组装出厂表的码全集（码与全称解耦，暂无译名的码同样放行）；
/// 本文件的自测另用它校验翻译表覆盖度（防两份资产脱节）。
pub static NAT_CODES: LazyLock<Vec<String>> = LazyLock::new(|| {
    #[derive(serde::Deserialize)]
    struct Codes {
        codes: Vec<String>,
    }
    let parsed: Codes = serde_json::from_str(include_str!("../assets/nat_codes.json"))
        .expect("nat_codes.json 格式错误");
    parsed.codes
});

/// config.xml 固定头尾模板，[players] 为占位符：xml_parser 生成文件时把映射行填进该位置，
/// 头尾结构保持 FM 要求的格式。
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
