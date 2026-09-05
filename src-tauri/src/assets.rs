//! 只读映射表：编译期嵌入，内容与 v1 .config/ 完全一致
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

#[derive(serde::Deserialize)]
struct EthCfgFile {
    #[serde(rename = "Ethnics")]
    ethnics: BTreeMap<String, String>,
}

/// 国籍三字码 → 种族目录名（如 ARG → SAMed）
pub static ETH_CFG: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str::<EthCfgFile>(include_str!("../assets/eth_cfg.json"))
        .expect("eth_cfg.json 格式错误")
        .ethnics
});

/// 语言 → {中文国籍: 英文三字码}
pub static NAT_TRANSLATION: LazyLock<HashMap<String, HashMap<String, String>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/nat_translation.json"))
        .expect("nat_translation.json 格式错误")
});

/// config.xml 固定头尾模板，[players] 为占位符
pub const CONFIG_TEMPLATE: &str = include_str!("../assets/config_template");
