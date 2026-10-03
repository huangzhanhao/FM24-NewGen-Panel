//! 火山方舟 Seedream 适配器（同步 Bearer）。
//!
//! Endpoint 与响应形态与 OpenAI Images 同构（`{base}/images/generations`、
//! `data[].b64_json|url`、`error.code/message`），故 endpoint 与响应解析复用
//! [`super::openai`]；差异仅在请求体：固定 `response_format:"b64_json"`、
//! `sequential_image_generation:"disabled"`，并支持 seed。

use super::{FetchedImage, ImageProvider, ProviderOutcome};
use crate::image_ai::GenRequest;
use base64::Engine; // STANDARD.decode() 来自 Engine trait
use std::future::Future;
use std::pin::Pin;

/// 组装火山方舟 Seedream 请求体（字段逐字照抄厂商文档）：
/// `model/prompt/size/response_format:"b64_json"/sequential_image_generation:"disabled"`，
/// seed 仅在 Some 时携带。
pub(crate) fn build_volcano_body(req: &GenRequest) -> serde_json::Value {
    use serde_json::json;
    let mut v = json!({
        "model": req.model,
        "prompt": req.prompt,
        "size": req.size,
        "response_format": "b64_json",
        // 关闭「连续生成」（多轮续写），本场景只做单轮文生图。
        "sequential_image_generation": "disabled",
    });
    if let Some(seed) = req.seed {
        v.as_object_mut().unwrap().insert("seed".into(), json!(seed));
    }
    v
}

/// 提取火山错误描述：`error.code` 与 `error.message` 同时存在时拼成 `code: message`。
pub(crate) fn error_message(resp: &serde_json::Value) -> Option<String> {
    let err = resp.get("error")?;
    let message = err.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误");
    Some(match err.get("code").and_then(|c| c.as_str()) {
        Some(code) => format!("{code}: {message}"),
        None => message.to_string(),
    })
}

pub struct VolcanoProvider;

impl ImageProvider for VolcanoProvider {
    fn generate<'a>(
        &'a self,
        client: &'a reqwest::Client,
        key: &'a str,
        req: &'a GenRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, String>> + Send + 'a>> {
        Box::pin(async move {
            let url = super::openai::endpoint(&req.base_url);
            let resp = client
                .post(&url)
                .bearer_auth(key)
                .json(&build_volcano_body(req))
                .send()
                .await
                .map_err(|e| format!("请求发送失败: {e}"))?;
            let status = resp.status();
            let json: serde_json::Value =
                resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
            // 先取火山 error.code/message，再复用 OpenAI 的 data 解析（成功路径）。
            if !status.is_success() {
                if let Some(msg) = error_message(&json) {
                    return Err(format!("HTTP {status}：{msg}"));
                }
            }
            let payloads = super::openai::parse_images(&json)
                .map_err(|e| format!("HTTP {status}：{e}"))?;

            let mut images = Vec::with_capacity(payloads.len());
            for payload in payloads {
                let bytes = match payload {
                    super::openai::ImagePayload::B64(b64) => {
                        base64::engine::general_purpose::STANDARD
                            .decode(b64.trim())
                            .map_err(|e| format!("图片 base64 解码失败: {e}"))?
                    }
                    super::openai::ImagePayload::Url(u) => client
                        .get(&u)
                        .send()
                        .await
                        .map_err(|e| format!("图片下载失败: {e}"))?
                        .error_for_status()
                        .map_err(|e| format!("图片下载失败: {e}"))?
                        .bytes()
                        .await
                        .map_err(|e| format!("图片读取失败: {e}"))?
                        .to_vec(),
                };
                images.push(FetchedImage { bytes, extension: "png".into() });
            }
            Ok(ProviderOutcome { images })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen_req() -> GenRequest {
        GenRequest {
            provider_id: "volcano".into(),
            base_url: "https://ark.cn-beijing.volces.com/api/v3".into(),
            model: "seedream-5.0-flash".into(),
            prompt: "一张脸".into(),
            negative_prompt: String::new(),
            size: "2048x2048".into(),
            n: 1,
            quality: None,
            seed: None,
            steps: None,
            uid: None,
            prompt_extend: None,
            watermark: None,
            prompt_extend_mode: None,
            enable_thinking: None,
            supports_response_format: true,
            // 火山 Seedream 只支持 seed；负向/改写/水印字段不发送。
            supports_negative_prompt: false,
            supports_prompt_extend: false,
            supports_watermark: false,
            supports_seed: true,
            supports_n: true,
            // 百炼专属：改写模式/思考模式与该模型的默认值，其他协议不声明。
            supports_prompt_extend_mode: false,
            prompt_extend_mode_default: "direct".into(),
            supports_enable_thinking: false,
            enable_thinking_default: true,
            watermark_default: false,
        }
    }

    #[test]
    fn body_has_volcano_fields_and_passthrough_size() {
        let b = build_volcano_body(&gen_req());
        assert_eq!(b["model"], "seedream-5.0-flash");
        assert_eq!(b["prompt"], "一张脸");
        // size 原样透传（不做 x→* 转换，那是 dashscope 的规则）。
        assert_eq!(b["size"], "2048x2048");
        assert_eq!(b["response_format"], "b64_json");
        assert_eq!(b["sequential_image_generation"], "disabled");
        // seed 缺省不带：火山模型 maxN=1，文档字段须逐字。
        assert!(b.get("seed").is_none());
    }

    #[test]
    fn body_includes_seed_when_some() {
        let mut r = gen_req();
        r.seed = Some(42);
        let b = build_volcano_body(&r);
        assert_eq!(b["seed"], 42);
    }

    #[test]
    fn error_response_exposes_code_and_message() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"error":{"code":"InvalidParameter","message":"size 不合法"}}"#,
        )
        .unwrap();
        assert_eq!(error_message(&v).unwrap(), "InvalidParameter: size 不合法");
        // 经 OpenAI 同构解析同样能识别错误（成功路径不会误入）。
        assert!(super::super::openai::parse_images(&v).is_err());
    }
}
