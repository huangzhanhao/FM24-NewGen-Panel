//! 【新手导读】只读映射表：编译期嵌入，内容与 v1 .config/ 完全一致。
//! 【关键手法】include_str!("...") 在「编译时」就把文件内容读进来变成字符串常量，
//! 打包后不依赖外部文件。配合 LazyLock，这些数据在首次使用时解析一次，之后全局共享。
use std::collections::{BTreeMap, HashMap}; // BTreeMap=按 key 排序的字典（行为可复现）；HashMap=无序字典（查找快）
use std::sync::LazyLock;                   // 「懒加载 + 线程安全」的全局静态量：首次访问时才初始化

// #[derive(serde::Deserialize)] 自动生成「从 JSON 反序列化为结构体」的能力（serde 是 Rust 最流行的序列化库）。
// 这个结构体只用于解析 eth_cfg.json 的外层结构（只取其中的 "Ethnics" 字段）。
#[derive(serde::Deserialize)]
struct EthCfgFile {
    // #[serde(rename = "Ethnics")]：JSON 里的键名是大写开头的 "Ethnics"，而 Rust 字段习惯用蛇形命名，
    // 用 rename 把两者对应起来。BTreeMap<String,String> 对应 JSON 的对象 {"ARG":"SAMed", ...}。
    #[serde(rename = "Ethnics")]
    ethnics: BTreeMap<String, String>,
}

/// 国籍三字码 → 种族目录名（如 ARG → SAMed）。
/// 【作用】把球员的国籍映射到 FM 头像包里对应的种族子目录，是换脸映射的基础数据。
/// LazyLock::new(|| ...) 里的闭包只会在首次访问 ETH_CFG 时执行一次，解析嵌入的 JSON。
/// expect(...) 表示若 JSON 格式错误就 panic（因为是编译期内嵌资源，出错属于构建问题）。
pub static ETH_CFG: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str::<EthCfgFile>(include_str!("../assets/eth_cfg.json"))
        .expect("eth_cfg.json 格式错误")
        .ethnics
});

/// 语言 → {中文国籍: 英文三字码}。
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
