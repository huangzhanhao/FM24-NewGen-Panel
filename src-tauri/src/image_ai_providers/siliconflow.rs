//! 硅基流动（SiliconFlow）适配器 + 内置模型目录（同步 Bearer）。
//!
//! 【为什么单列一档】它虽自称 OpenAI 兼容，但官方 API 参考的**响应不是 `data[]`**，
//! 而是 `{ "images": [ { "url" } ], "timings", "seed" }`；尺寸字段是 `image_size`（非 `size`）——
//! 契约差异超出「改 base_url」，`openai::parse_images` 直接复用会报「响应缺少 data 数组」。
//!
//! Endpoint：`POST https://api.siliconflow.cn/v1/images/generations`（Bearer 鉴权）。
//! 请求体（官方「创建图片生成请求」逐字）：
//! - 恒发：`model` / `prompt` / `image_size`（`宽x高`，小写 `x`，原样透传不转换）；
//! - `negative_prompt`；`seed`(0–9999999999)；`num_inference_steps`(1–100，默认 20)；
//! - `batch_size`(1–4，**仅 Kwai-Kolors/Kolors 适用**)；`guidance_scale`(0–20，**仅 Kolors 适用**)；
//! - `output_format`(png/jpeg，适配器固定 png)。
//!
//! 【内置目录】只接入两个官方明列推荐分辨率的模型：
//! - `Qwen/Qwen-Image`（出厂默认）
//! - `Kwai-Kolors/Kolors`
//!
//! 用户配置里已存在该服务商时，模型行归用户所有、代码不接管
//! （见 [`crate::image_ai::reconcile_builtin_providers`]）。

use super::{validate_model_caps, GenStage, ImageProvider, ProviderOutcome};
use crate::image_ai::{
    from_http_status, Capabilities, GenRequest, ImageGenError, ModelInfo, ProviderConfig,
    ProviderKind,
};
use std::future::Future;
use std::pin::Pin;

/// 内置硅基流动服务商的固定 ID（配置合并按此 ID 认领）。
pub(crate) const PROVIDER_ID: &str = "siliconflow";

/// 硅基流动公共基址；生图端点为 `{base}/images/generations`。
const DEFAULT_BASE: &str = "https://api.siliconflow.cn/v1";

/// 默认模型：Qwen-Image，中文文字渲染与综合质量均衡。
const DEFAULT_MODEL: &str = "Qwen/Qwen-Image";

/// Qwen-Image 官方推荐分辨率。
const QWEN_IMAGE_SIZES: &[&str] = &[
    "1328x1328",
    "1664x928",
    "928x1664",
    "1472x1140",
    "1140x1472",
    "1584x1056",
    "1056x1584",
];

/// Kwai-Kolors/Kolors 官方推荐分辨率。
const KOLORS_SIZES: &[&str] = &["1024x1024", "960x1280", "768x1024", "720x1440", "720x1280"];

// ===================== 内置模型目录 =====================

/// 硅基流动内置服务商：模型目录代码托管（不读 assets/image_ai.json）。
pub(crate) fn builtin_provider() -> ProviderConfig {
    ProviderConfig {
        id: PROVIDER_ID.into(),
        kind: ProviderKind::Siliconflow,
        name: "硅基流动 SiliconFlow".into(),
        base_url: DEFAULT_BASE.into(),
        default_model: DEFAULT_MODEL.into(),
        models: vec![
            ModelInfo {
                model: DEFAULT_MODEL.into(),
                // Qwen-Image：不支持 batch_size/guidance_scale（官方明示仅 Kolors 适用）。
                capabilities: siliconflow_caps(QWEN_IMAGE_SIZES, 1, false, false),
            },
            ModelInfo {
                model: "Kwai-Kolors/Kolors".into(),
                // Kolors：batch_size 1–4、guidance_scale 0–20（默认 7.5）。
                capabilities: siliconflow_caps(KOLORS_SIZES, 4, true, true),
            },
        ],
    }
}

/// 逐模型能力集：
/// - `sizes`：官方推荐分辨率（自定义尺寸走 `size_rules`；此处不设规则，交服务端校验）；
/// - `max_n`/`supports_n`：仅 Kolors 有 `batch_size`（1–4）；
/// - `supports_steps`：全模型支持 `num_inference_steps`（1–100，默认 20）；
/// - `supports_guidance`：仅 Kolors 支持 `guidance_scale`（0–20，默认 7.5）；
/// - `supports_output_format`：全模型可设 png/jpeg（适配器固定 png）。
fn siliconflow_caps(
    sizes: &[&str],
    max_n: u32,
    supports_n: bool,
    supports_guidance: bool,
) -> Capabilities {
    Capabilities {
        sizes: sizes.iter().map(|s| (*s).to_string()).collect(),
        max_n,
        // Capabilities::default() 给 bool 的是 false；代码构造必须显式声明
        // （serde(default) 只对 JSON 缺字段反序列化生效）。
        supports_n,
        supports_seed: true,
        supports_negative_prompt: true,
        supports_steps: true,
        steps_default: 20,
        steps_max: 100,
        supports_guidance,
        guidance_default: if supports_guidance { 7.5 } else { 0.0 },
        guidance_max: if supports_guidance { 20.0 } else { 0.0 },
        supports_output_format: true,
        supports_response_format: false,
        // 官方未给硬性提示词上限：不拦截。
        max_prompt_chars: 0,
        size_rules: None,
        ..Default::default()
    }
}

// ===================== 请求体（纯函数，字段逐字照抄官方文档）=====================

/// 组装硅基流动文生图请求体：恒发 `model`/`prompt`/`image_size`；其余按能力位门控。
/// **不发** `size`/`n`（官方字段名是 `image_size`/`batch_size`）。
pub(crate) fn build_siliconflow_body(req: &GenRequest) -> serde_json::Value {
    use serde_json::json;
    let caps = req.caps();
    let mut v = json!({
        "model": req.model,
        "prompt": req.prompt,
        "image_size": req.size,
    });
    let obj = v.as_object_mut().unwrap();
    if caps.supports_negative_prompt {
        obj.insert("negative_prompt".into(), json!(req.negative_prompt));
    }
    // 张数：官方字段是 batch_size（仅 Kolors 适用）。
    if caps.supports_n {
        obj.insert("batch_size".into(), json!(req.n));
    }
    // seed 只在用户填了才发（未传入时服务端随机选种子）。
    if caps.supports_seed {
        if let Some(seed) = req.seed {
            obj.insert("seed".into(), json!(seed));
        }
    }
    if caps.supports_steps {
        if let Some(steps) = req.steps {
            obj.insert("num_inference_steps".into(), json!(steps));
        }
    }
    if caps.supports_guidance {
        if let Some(g) = req.guidance {
            obj.insert("guidance_scale".into(), json!(g));
        }
    }
    if caps.supports_output_format {
        obj.insert("output_format".into(), json!("png"));
    }
    v
}

// ===================== 响应解析 =====================

/// 解析响应：成功取 `images[].url`；无 `images` 时按 `message` 归为服务商错误。
pub(crate) fn parse_images(resp: &serde_json::Value) -> Result<Vec<String>, ImageGenError> {
    if let Some(items) = resp.get("images").and_then(|d| d.as_array()) {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let u = item
                .get("url")
                .and_then(|x| x.as_str())
                .ok_or_else(|| ImageGenError::provider("响应中的图片缺少 url", false))?;
            out.push(u.to_string());
        }
        if out.is_empty() {
            return Err(ImageGenError::provider("images 数组为空", false));
        }
        return Ok(out);
    }
    // 错误形如 `{ "code": 20012, "message": "...", "data": "..." }`。
    let msg = resp
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("未知错误");
    Err(ImageGenError::provider(
        format!("服务商返回错误：{msg}"),
        false,
    ))
}

pub struct SiliconflowProvider;

impl ImageProvider for SiliconflowProvider {
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
            let url = super::openai::endpoint(&req.base_url);
            let resp = client
                .post(&url)
                .bearer_auth(key)
                .json(&build_siliconflow_body(req))
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
                .map_err(|e| ImageGenError::provider(format!("响应解析失败: {e}"), false))?;
            let urls = parse_images(&json).map_err(|e| {
                from_http_status(status.as_u16(), format!("HTTP {status}：{}", e.message))
                    .with_retry_after(retry_after)
            })?;

            progress(GenStage::Downloading);
            let mut images = Vec::with_capacity(urls.len());
            for u in urls {
                images.push(
                    super::openai::fetch_payload(client, super::openai::ImagePayload::Url(u))
                        .await?,
                );
            }
            Ok(ProviderOutcome { images })
        })
    }

    /// 端点由 base_url 拼出，空基址是本协议特有的门控；其余约束走共用解释。
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

    /// 以内置目录的真实能力快照构造请求（size 取该模型首个预设）。
    fn req_for(model: &str) -> GenRequest {
        let p = builtin_provider();
        let m = p.models.iter().find(|m| m.model == model).unwrap();
        GenRequest {
            provider_id: PROVIDER_ID.into(),
            base_url: DEFAULT_BASE.into(),
            model: model.into(),
            prompt: "一张东亚球员的脸".into(),
            size: m.capabilities.sizes[0].clone(),
            caps: Some(m.capabilities.clone()),
            ..crate::image_ai::test_req_base()
        }
    }

    fn caps_of(model: &str) -> Capabilities {
        builtin_provider()
            .models
            .iter()
            .find(|m| m.model == model)
            .unwrap()
            .capabilities
            .clone()
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    // ===================== 内置目录 =====================

    #[test]
    fn catalog_lists_two_models() {
        let p = builtin_provider();
        assert_eq!(p.id, PROVIDER_ID);
        assert_eq!(p.kind, ProviderKind::Siliconflow);
        assert_eq!(p.base_url, DEFAULT_BASE);
        assert_eq!(p.default_model, "Qwen/Qwen-Image");
        let ids: Vec<&str> = p.models.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(ids, vec!["Qwen/Qwen-Image", "Kwai-Kolors/Kolors"]);
        assert!(ids.contains(&p.default_model.as_str()));
    }

    #[test]
    fn catalog_caps_match_official_spec() {
        let qwen = caps_of("Qwen/Qwen-Image");
        assert_eq!(qwen.sizes, strs(QWEN_IMAGE_SIZES));
        assert_eq!(qwen.max_n, 1);
        assert!(!qwen.supports_n, "Qwen-Image 无 batch_size");
        assert!(!qwen.supports_guidance, "Qwen-Image 无 guidance_scale");
        assert!(qwen.supports_steps);
        assert_eq!((qwen.steps_default, qwen.steps_max), (20, 100));
        assert!(qwen.supports_output_format);
        assert!(qwen.supports_seed && qwen.supports_negative_prompt);
        assert_eq!(qwen.max_prompt_chars, 0);
        assert!(qwen.size_rules.is_none(), "不臆造尺寸边界");

        let kolors = caps_of("Kwai-Kolors/Kolors");
        assert_eq!(kolors.sizes, strs(KOLORS_SIZES));
        assert_eq!(kolors.max_n, 4);
        assert!(kolors.supports_n, "Kolors 支持 batch_size 1–4");
        assert!(kolors.supports_guidance);
        assert_eq!((kolors.guidance_default, kolors.guidance_max), (7.5, 20.0));
        assert!(kolors.supports_steps);
    }

    // ===================== 请求体组装 =====================

    #[test]
    fn body_qwen_image_sends_image_size_not_size_or_n() {
        let b = build_siliconflow_body(&req_for("Qwen/Qwen-Image"));
        assert_eq!(b["model"], "Qwen/Qwen-Image");
        assert_eq!(b["image_size"], "1328x1328");
        assert_eq!(b["output_format"], "png");
        // 官方字段名是 image_size / batch_size，绝不能发 size / n。
        for forbidden in ["size", "n", "batch_size", "guidance_scale"] {
            assert!(b.get(forbidden).is_none(), "不应出现 {forbidden}");
        }
        // 未填的 seed/steps 不发。
        assert!(b.get("seed").is_none());
        assert!(b.get("num_inference_steps").is_none());
    }

    #[test]
    fn body_kolors_carries_batch_size_and_guidance() {
        let mut r = req_for("Kwai-Kolors/Kolors");
        r.n = 3;
        r.seed = Some(42);
        r.steps = Some(30);
        r.guidance = Some(6.5);
        let b = build_siliconflow_body(&r);
        assert_eq!(b["image_size"], "1024x1024");
        assert_eq!(b["batch_size"], 3);
        assert_eq!(b["seed"], 42);
        assert_eq!(b["num_inference_steps"], 30);
        assert_eq!(b["guidance_scale"], 6.5);
        for forbidden in ["size", "n"] {
            assert!(b.get(forbidden).is_none(), "不应出现 {forbidden}");
        }
    }

    // ===================== 响应解析 =====================

    #[test]
    fn parse_images_reads_images_list() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"images":[{"url":"https://x/a.png"}],"seed":1}"#).unwrap();
        assert_eq!(parse_images(&v).unwrap(), vec!["https://x/a.png"]);
    }

    #[test]
    fn parse_images_reports_error_and_missing() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"code":20012,"message":"invalid token","data":""}"#).unwrap();
        let err = parse_images(&v).unwrap_err();
        assert_eq!(err.message, "服务商返回错误：invalid token");
        assert_eq!(err.kind, crate::image_ai::GenErrorKind::Provider);

        let v: serde_json::Value = serde_json::from_str(r#"{"images":[]}"#).unwrap();
        assert_eq!(parse_images(&v).unwrap_err().message, "images 数组为空");
    }

    // ===================== 校验 =====================

    #[test]
    fn validate_rejects_steps_and_guidance_over_max() {
        let kolors = caps_of("Kwai-Kolors/Kolors");
        let mut r = req_for("Kwai-Kolors/Kolors");
        r.steps = Some(101);
        assert!(validate_model_caps(&r, &kolors).is_err(), "步数 101 超上限");
        r.steps = Some(100);
        assert!(validate_model_caps(&r, &kolors).is_ok());
        r.guidance = Some(20.5);
        assert!(validate_model_caps(&r, &kolors).is_err(), "引导系数超上限");
        r.guidance = Some(20.0);
        assert!(validate_model_caps(&r, &kolors).is_ok());
    }
}
