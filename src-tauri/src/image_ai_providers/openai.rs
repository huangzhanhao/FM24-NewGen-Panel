//! OpenAI Images 协议适配器：官方 OpenAI 与用户自填的 OpenAI 兼容服务共用本协议，
//! 差别只在模型能力集（如 `supports_response_format`：gpt-image-1 不支持该参数，
//! 第三方兼容服务多需要它）。
//!
//! 【百炼兼容模式】阿里云百炼的 `{base}/compatible-mode/v1/images/generations` 只有同步、
//! 无异步，参数全部平铺在请求体顶层：除 OpenAI 官方的 `model`/`prompt`/`n`/`size`（分隔符是
//! 字母 `x`，不是 DashScope 的 `*`）外，还接受 `negative_prompt`/`seed`/`prompt_extend`/
//! `prompt_extend_mode`/`enable_thinking`/`watermark` 六个官方明列的「阿里云百炼扩展字段」。
//! 故本适配器与 DashScope 共用同一份能力位门控（[`super::capability_params`]）：勾了才发、
//! 没勾一个不发——给真 OpenAI 模型保持能力位默认 false 即可，不会多发字段。
//! `response_format=b64_json` 在兼容模式下传了不报错但被忽略，响应仍返回图像 URL
//! （`parse_images` 对 b64 与 url 两种载荷都吃）。

use super::{validate_model_caps, FetchedImage, GenStage, ImageProvider, ProviderOutcome};
use crate::image_ai::types::{
    from_http_status, Capabilities, GenRequest, ImageGenError};
use base64::Engine;
use std::future::Future;
use std::pin::Pin;

/// 拼接生图 endpoint：`{base_url}/images/generations`（base 尾部斜杠归一）。
pub(crate) fn endpoint(base_url: &str) -> String {
    format!("{}/images/generations", base_url.trim_end_matches('/'))
}

/// 组装 OpenAI Images 请求体：参数一律平铺在顶层（没有 DashScope 的 input/parameters 嵌套）。
/// `supports_response_format` 来自模型能力集（`ModelInfo.capabilities`）：
/// OpenAI 官方 gpt-image-1 系列**不支持** `response_format`（始终返回 base64，传了即 400），
/// 出厂该能力为 false；OpenAI 兼容的第三方服务多默认返回 URL，需显式声明 b64_json 免去一次下载。
pub(crate) fn build_body(req: &GenRequest, supports_response_format: bool) -> serde_json::Value {
    use serde_json::json;
    let mut v = json!({
        "model": req.model,
        "prompt": req.prompt,
        "size": req.size,
    });
    let obj = v.as_object_mut().unwrap();
    // n 与六个百炼扩展字段与 DashScope 同源产出（同一份能力位门控），此处只是摆在顶层。
    for (k, val) in super::capability_params(req) {
        obj.insert(k, val);
    }
    if supports_response_format {
        obj.insert("response_format".into(), json!("b64_json"));
    }
    // 质量按能力位门控，与其余字段同口径（能力不声明就不发）。store 只在切**服务商**时把
    // quality 归位 auto，切模型不归位：从 gpt-image-1 带过来的 "high" 会被原样发给
    // 不认该参数的模型（如兼容模式下的 qwen-image），换来一个看不懂的 400。
    if req.caps().supports_quality {
        if let Some(q) = &req.quality {
            if q != "auto" {
                obj.insert("quality".into(), json!(q));
            }
        }
    }
    v
}

/// 由请求自身（能力快照）组装请求体：`response_format` 门控读 `req.caps()`。
/// generate 与单测都经此入口，保证「读快照」这条路径被真正覆盖。
pub(crate) fn build_body_for(req: &GenRequest) -> serde_json::Value {
    build_body(req, req.caps().supports_response_format)
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ImagePayload {
    B64(String),
    Url(String),
}

/// 解析响应中的图片列表；厂商错误（`error.message`）或缺字段均返回结构化错误。
pub(crate) fn parse_images(resp: &serde_json::Value) -> Result<Vec<ImagePayload>, ImageGenError> {
    // 已知边界：仅按 HTTP status 分类；兼容中转若以 200 返回 error 对象，一律落 Provider
    // （error 里没有可映射的业务码字段，无从细分）。
    if let Some(err) = resp.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(ImageGenError::provider(
            format!("服务商返回错误：{msg}"),
            false,
        ));
    }
    let items = resp
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| ImageGenError::provider("响应缺少 data 数组", false))?;
    let mut out = Vec::new();
    for item in items {
        if let Some(b64) = item.get("b64_json").and_then(|x| x.as_str()) {
            out.push(ImagePayload::B64(b64.to_string()));
        } else if let Some(url) = item.get("url").and_then(|x| x.as_str()) {
            out.push(ImagePayload::Url(url.to_string()));
        } else {
            return Err(ImageGenError::provider(
                "响应中的图片既无 b64_json 也无 url",
                false,
            ));
        }
    }
    if out.is_empty() {
        return Err(ImageGenError::provider("data 数组为空", false));
    }
    Ok(out)
}

/// 由 URL 路径推断落盘后缀：兼容中转可能回 jpeg/webp；无扩展名或形态不可信回落 png。
/// 查询串/锚点先剥掉（OSS 签名链接形如 `.../x.png?Expires=...`）。
pub(crate) fn extension_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let name = path.rsplit('/').next().unwrap_or("");
    match name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty()
                && (1..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            ext.to_ascii_lowercase()
        }
        _ => "png".into(),
    }
}

/// 把单个 payload 变为图片：b64 解码；URL 立即下载（链接有时效，不懒加载）。
/// 后缀随载荷判定：URL 取路径扩展名；b64 无从判型（gpt-image-1 恒 png），回落 png。
/// `pub(crate)`：硅基流动/魔搭等「返回 URL 列表」的协议复用它完成下载。
pub(crate) async fn fetch_payload(
    client: &reqwest::Client,
    payload: ImagePayload,
) -> Result<FetchedImage, ImageGenError> {
    match payload {
        ImagePayload::B64(b64) => Ok(FetchedImage {
            bytes: base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| ImageGenError::provider(format!("图片 base64 解码失败: {e}"), false))?,
            extension: "png".into(),
        }),
        ImagePayload::Url(u) => Ok(FetchedImage {
            bytes: client
                .get(&u)
                .send()
                .await
                .map_err(|e| ImageGenError::network(format!("图片下载失败: {e}")))?
                .error_for_status()
                .map_err(|e| ImageGenError::network(format!("图片下载失败: {e}")))?
                .bytes()
                .await
                .map_err(|e| ImageGenError::network(format!("图片读取失败: {e}")))?
                .to_vec(),
            extension: extension_from_url(&u),
        }),
    }
}

pub struct OpenAiProvider;

impl ImageProvider for OpenAiProvider {
    fn generate<'a>(
        &'a self,
        client: &'a reqwest::Client,
        key: &'a str,
        req: &'a GenRequest,
        progress: &'a (dyn Fn(GenStage) + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, ImageGenError>> + Send + 'a>> {
        Box::pin(async move {
            // 同步链：提交 → 下载。
            progress(GenStage::Submitting);
            let url = endpoint(&req.base_url);
            let resp = client
                .post(&url)
                .bearer_auth(key)
                .json(&build_body_for(req))
                .send()
                .await
                .map_err(|e| ImageGenError::network(format!("请求发送失败: {e}")))?;
            let status = resp.status();
            // 429 的 `Retry-After` 必须在读 body 前取（json() 会消费响应）；其它状态一律 None。
            let retry_after = if status.as_u16() == 429 {
                super::retry_after_secs(resp.headers())
            } else {
                None
            };
            let json: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ImageGenError::provider(format!("HTTP {status}：响应解析失败: {e}"), false))?;
            // status 是分类的唯一稳定依据，故在拼串前按状态码分流（解析失败也一并归类）。
            let payloads = parse_images(&json).map_err(|e| {
                from_http_status(status.as_u16(), format!("HTTP {status}：{}", e.message))
                    .with_retry_after(retry_after)
            })?;

            progress(GenStage::Downloading);
            let mut images = Vec::with_capacity(payloads.len());
            for payload in payloads {
                images.push(fetch_payload(client, payload).await?);
            }
            Ok(ProviderOutcome { images })
        })
    }

    /// 端点由 base_url 拼出，空基址是本协议特有的门控；其余数值/尺寸约束走共用解释。
    fn validate(&self, req: &GenRequest, caps: &Capabilities) -> Result<(), ImageGenError> {
        if req.base_url.trim().is_empty() {
            return Err(ImageGenError::not_configured(
                "请先在「服务商设置」中填写接口地址 Base URL",
            ));
        }
        validate_model_caps(req, caps)
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
            size: "1024x1024".into(),
            n: 2,
            quality: Some("high".into()),
            // 能力快照：OpenAI 官方端点不认百炼扩展参数，故仅声明本协议自己的两项。
            caps: Some(Capabilities {
                supports_n: true,
                supports_quality: true,
                ..Default::default()
            }),
            ..crate::image_ai::types::test_req_base()
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
        // 质量也受能力位门控：切模型不归位 quality，不看能力位就会把上一家模型的 "high"
        // 发给不认该参数的模型（兼容模式下的 qwen-image 会 400）。
        let mut r = gen_req();
        r.caps = Some(Capabilities {
            supports_n: true,
            ..Default::default()
        });
        assert!(build_body(&r, true).get("quality").is_none());
    }

    #[test]
    fn bailian_extension_fields_are_flat_and_capability_gated() {
        // 兼容模式下的百炼模型：能力位勾上 ⇒ 扩展字段平铺在顶层（没有 parameters 嵌套）。
        let mut r = gen_req();
        r.caps = Some(Capabilities {
            supports_n: true,
            supports_seed: true,
            supports_negative_prompt: true,
            supports_prompt_extend: true,
            prompt_extend_default: true,
            supports_prompt_extend_mode: true,
            prompt_extend_mode_default: "agent".into(),
            supports_enable_thinking: true,
            enable_thinking_default: true,
            supports_watermark: true,
            watermark_default: false,
            ..Default::default()
        });
        r.negative_prompt = "模糊".into();
        r.seed = Some(7);
        r.watermark = Some(true);
        let b = build_body_for(&r);
        assert_eq!(b["seed"], 7);
        assert_eq!(b["negative_prompt"], "模糊");
        assert_eq!(b["prompt_extend"], true, "用户未置 → 回落能力集声明的默认");
        assert_eq!(b["prompt_extend_mode"], "agent");
        assert_eq!(b["enable_thinking"], true);
        assert_eq!(b["watermark"], true, "用户显式选择压过默认");
        assert_eq!(b["n"], 2);
        assert!(
            b.get("parameters").is_none(),
            "兼容模式没有 parameters 嵌套"
        );
        assert_eq!(b["size"], "1024x1024", "分隔符保持 x，不转 *");

        // 官方 OpenAI 模型（能力位默认 false）：一个扩展字段都不发，避免陌生参数吃 400。
        let mut r2 = gen_req();
        r2.seed = Some(7);
        r2.negative_prompt = "模糊".into();
        let b2 = build_body_for(&r2);
        for k in [
            "seed",
            "negative_prompt",
            "prompt_extend",
            "prompt_extend_mode",
            "enable_thinking",
            "watermark",
        ] {
            assert!(b2.get(k).is_none(), "{k} 不该出现");
        }
        assert_eq!(b2["n"], 2, "n 是 OpenAI 官方参数，default_caps 声明支持");
    }

    #[test]
    fn response_format_gate_follows_req_caps_snapshot() {
        // 覆盖 generate 的真实读取路径：build_body_for → req.caps().supports_response_format。
        // （build_body 的显式布尔形参用例绕开了这条路径，接错字段它也照过。）
        let mut r = gen_req();
        r.caps = Some(Capabilities {
            supports_response_format: true,
            ..Default::default()
        });
        assert_eq!(build_body_for(&r)["response_format"], "b64_json");
        r.caps = Some(Capabilities {
            supports_response_format: false,
            ..Default::default()
        });
        assert!(build_body_for(&r).get("response_format").is_none());
        // caps == None：走访问器兜底，等价旧 serde 缺省 false ⇒ 同样不发。
        r.caps = None;
        assert!(build_body_for(&r).get("response_format").is_none());
    }

    #[test]
    fn parse_images_accepts_b64_url_and_error() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"data":[{"b64_json":"QUJD"},{"url":"https://x/a.png"}]}"#)
                .unwrap();
        assert_eq!(parse_images(&v).unwrap().len(), 2);
        let v: serde_json::Value =
            serde_json::from_str(r#"{"error":{"message":"bad key"}}"#).unwrap();
        let err = parse_images(&v).unwrap_err();
        assert_eq!(err.message, "服务商返回错误：bad key");
        assert_eq!(err.kind, crate::image_ai::types::GenErrorKind::Provider);
    }

    #[test]
    fn parse_images_rejects_missing_data_and_empty_data() {
        let v: serde_json::Value = serde_json::from_str(r#"{}"#).unwrap();
        let err = parse_images(&v).unwrap_err();
        assert_eq!(err.message, "响应缺少 data 数组");
        assert_eq!(err.kind, crate::image_ai::types::GenErrorKind::Provider);
        let v: serde_json::Value = serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert_eq!(parse_images(&v).unwrap_err().message, "data 数组为空");
    }

    #[test]
    fn extension_from_url_reads_path_and_ignores_query() {
        // OSS 签名链接：扩展名在路径尾、查询串里没有第二个 '.' 干扰。
        assert_eq!(
            extension_from_url("https://r/a/ComfyUI_00001.png?Expires=1&x=a.b"),
            "png"
        );
        assert_eq!(extension_from_url("https://r/out.jpeg"), "jpeg");
        assert_eq!(extension_from_url("https://r/out.WEBP"), "webp");
        // 无扩展名 / 不可信形态 → 回落 png。
        assert_eq!(extension_from_url("https://r/download"), "png");
        assert_eq!(extension_from_url("https://r/a.b.clongextension"), "png");
        assert_eq!(extension_from_url("https://r/.png"), "png");
    }
}
