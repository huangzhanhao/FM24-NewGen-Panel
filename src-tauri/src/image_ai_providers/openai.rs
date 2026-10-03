//! OpenAI Images 协议适配器（P1 协议纯函数自 `image_ai.rs` 整组迁入，语义不变）。
//!
//! custom（用户自填 OpenAI 兼容服务）走同一协议；区别仅在模型能力集
//! `supports_response_format`（gpt-image-1 不支持该参数，第三方兼容服务多需要它）。

use super::{FetchedImage, ImageProvider, ProviderOutcome};
use crate::image_ai::GenRequest;
use base64::Engine;
use std::future::Future;
use std::pin::Pin;

/// 拼接生图 endpoint：`{base_url}/images/generations`（base 尾部斜杠归一）。
pub(crate) fn endpoint(base_url: &str) -> String {
    format!("{}/images/generations", base_url.trim_end_matches('/'))
}

/// 组装 OpenAI Images 请求体。
/// `supports_response_format` 来自模型能力集（`ModelInfo.capabilities`）：
/// OpenAI 官方 gpt-image-1 系列**不支持** `response_format`（始终返回 base64，传了即 400），
/// 出厂该能力为 false；OpenAI 兼容的第三方服务多默认返回 URL，需显式声明 b64_json 免去一次下载。
pub(crate) fn build_body(req: &GenRequest, supports_response_format: bool) -> serde_json::Value {
    use serde_json::json;
    let mut v = json!({
        "model": req.model,
        "prompt": req.prompt,
        "n": req.n,
        "size": req.size,
    });
    let obj = v.as_object_mut().unwrap();
    if supports_response_format {
        obj.insert("response_format".into(), json!("b64_json"));
    }
    if let Some(q) = &req.quality {
        if q != "auto" {
            obj.insert("quality".into(), json!(q));
        }
    }
    // OpenAI 不支持 seed，故此处不插入；支持 seed 的适配器各自追加（见 volcano）。
    v
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ImagePayload {
    B64(String),
    Url(String),
}

/// 解析响应中的图片列表；厂商错误（`error.message`）或缺字段均返回中文错误。
pub(crate) fn parse_images(resp: &serde_json::Value) -> Result<Vec<ImagePayload>, String> {
    if let Some(err) = resp.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(format!("服务商返回错误：{msg}"));
    }
    let items = resp
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or("响应缺少 data 数组")?;
    let mut out = Vec::new();
    for item in items {
        if let Some(b64) = item.get("b64_json").and_then(|x| x.as_str()) {
            out.push(ImagePayload::B64(b64.to_string()));
        } else if let Some(url) = item.get("url").and_then(|x| x.as_str()) {
            out.push(ImagePayload::Url(url.to_string()));
        } else {
            return Err("响应中的图片既无 b64_json 也无 url".into());
        }
    }
    if out.is_empty() {
        return Err("data 数组为空".into());
    }
    Ok(out)
}

/// 把单个 payload 变为图片字节：b64 解码；URL 立即下载（链接有时效，不懒加载）。
async fn fetch_payload(client: &reqwest::Client, payload: ImagePayload) -> Result<Vec<u8>, String> {
    match payload {
        ImagePayload::B64(b64) => base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("图片 base64 解码失败: {e}")),
        ImagePayload::Url(u) => Ok(client
            .get(&u)
            .send()
            .await
            .map_err(|e| format!("图片下载失败: {e}"))?
            .error_for_status()
            .map_err(|e| format!("图片下载失败: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("图片读取失败: {e}"))?
            .to_vec()),
    }
}

pub struct OpenAiProvider;

impl ImageProvider for OpenAiProvider {
    fn generate<'a>(
        &'a self,
        client: &'a reqwest::Client,
        key: &'a str,
        req: &'a GenRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, String>> + Send + 'a>> {
        Box::pin(async move {
            // 能力标志由 image_ai::generate 从模型能力集写入 req 内部字段。
            let supports_response_format = req.supports_response_format;
            let url = endpoint(&req.base_url);
            let resp = client
                .post(&url)
                .bearer_auth(key)
                .json(&build_body(req, supports_response_format))
                .send()
                .await
                .map_err(|e| format!("请求发送失败: {e}"))?;
            let status = resp.status();
            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| format!("响应解析失败: {e}"))?;
            let payloads = parse_images(&json).map_err(|e| format!("HTTP {status}：{e}"))?;

            let mut images = Vec::with_capacity(payloads.len());
            for payload in payloads {
                let bytes = fetch_payload(client, payload).await?;
                images.push(FetchedImage {
                    bytes,
                    extension: "png".into(),
                });
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
            provider_id: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-image-1".into(),
            prompt: "一张脸".into(),
            negative_prompt: String::new(),
            size: "1024x1024".into(),
            n: 2,
            quality: Some("high".into()),
            seed: None,
            steps: None,
            uid: None,
            prompt_extend: None,
            watermark: None,
            prompt_extend_mode: None,
            enable_thinking: None,
            supports_response_format: false,
            // OpenAI 官方端点不认百炼扩展参数，样本一律置 false。
            supports_negative_prompt: false,
            supports_prompt_extend: false,
            supports_watermark: false,
            supports_seed: false,
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
    fn endpoint_trims_trailing_slash() {
        assert_eq!(
            endpoint("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/images/generations"
        );
    }

    #[test]
    fn build_body_quality_and_response_format_rules() {
        // 能力集允许时才带 response_format。
        let b = build_body(&gen_req(), true);
        assert_eq!(b["quality"], "high");
        assert_eq!(b["response_format"], "b64_json");
        // gpt-image-1 能力为 false：整条字段不得出现（否则官方 API 返回 400）。
        let b = build_body(&gen_req(), false);
        assert!(b.get("response_format").is_none());
        let mut r = gen_req();
        r.quality = Some("auto".into());
        assert!(build_body(&r, true).get("quality").is_none());
    }

    #[test]
    fn parse_images_accepts_b64_url_and_error() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"data":[{"b64_json":"QUJD"},{"url":"https://x/a.png"}]}"#)
                .unwrap();
        assert_eq!(parse_images(&v).unwrap().len(), 2);
        let v: serde_json::Value =
            serde_json::from_str(r#"{"error":{"message":"bad key"}}"#).unwrap();
        assert_eq!(parse_images(&v).unwrap_err(), "服务商返回错误：bad key");
    }

    #[test]
    fn parse_images_rejects_missing_data_and_empty_data() {
        let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(parse_images(&v).unwrap_err(), "响应缺少 data 数组");
        let v: serde_json::Value = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert_eq!(parse_images(&v).unwrap_err(), "data 数组为空");
    }
}
