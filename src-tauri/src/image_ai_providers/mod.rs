//! AI 文生图适配器族：按 [`crate::image_ai::ProviderKind`] 分发到具体厂商协议。
//!
//! 【族内分工】
//! - [`openai`]：OpenAI Images 协议（P1 协议纯函数迁入），custom（OpenAI 兼容服务）同协议；
//! - [`volcano`]：火山方舟 Seedream，同步 Bearer，响应形态与 OpenAI 同构；
//! - [`dashscope`]：阿里云百炼（千问/万相/Z-Image 三系列），同步多模态消息式调用 +
//!   代码内置模型目录（模型能力不读 image_ai.json）。
//!
//! 【对象安全】trait 用「返回 `Pin<Box<dyn Future>>`」形式而非 `async fn`，
//! 故 [`for_kind`] 能返回 `Box<dyn ImageProvider>`。所有协议单测均为纯函数测试，不触网。

pub(crate) mod dashscope;
pub(crate) mod openai;
pub(crate) mod volcano;

use crate::image_ai::{GenRequest, ProviderKind};
use std::future::Future;
use std::pin::Pin;

/// 适配器产出的单张图片：原始字节 + 建议文件后缀（不含点，如 "png"）。
pub struct FetchedImage {
    pub bytes: Vec<u8>,
    pub extension: String,
}

/// 一次生成的适配器结果：图片字节序列。
pub struct ProviderOutcome {
    pub images: Vec<FetchedImage>,
}

/// 统一适配器：一次 async 调用内含 POST → 解析 →（b64 解码 / URL 立即下载）；
/// 异步厂商在此调用内含 submit → poll → fetch 全流程。
pub trait ImageProvider: Send + Sync {
    fn generate<'a>(
        &'a self,
        client: &'a reqwest::Client,
        key: &'a str,
        req: &'a GenRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, String>> + Send + 'a>>;
}

/// 唯一分发：按 ProviderKind 取适配器（openai/custom 共用 OpenAI 协议适配器）。
pub fn for_kind(kind: ProviderKind) -> Box<dyn ImageProvider> {
    match kind {
        ProviderKind::OpenAI | ProviderKind::Custom => Box::new(openai::OpenAiProvider),
        ProviderKind::Volcano => Box::new(volcano::VolcanoProvider),
        ProviderKind::Dashscope => Box::new(dashscope::DashscopeProvider),
    }
}
