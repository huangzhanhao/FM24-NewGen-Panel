//! 魔搭（ModelScope）适配器 + 内置模型目录（**强制异步**：提交 → 轮询 → 下载）。
//!
//! 【为什么单列一档】它响应不是 OpenAI 的 `data[]`（结果在任务端点的 `output_images[]`），
//! 且生图**必须**走异步任务流——契约差异超出「改 base_url」。
//!
//! 【流程】（官方 API-Inference）
//! 1. `POST {base}/images/generations`，头 `X-ModelScope-Async-Mode: true`
//!    → `{ "task_id": "<id>" }`；
//! 2. `GET {base}/tasks/{task_id}`，头 `X-ModelScope-Task-Type: image_generation`
//!    → `{ "task_status": "RUNNING|SUCCEED|FAILED", "output_images": ["<url>", ...] }`
//!    轮询至 `SUCCEED|FAILED`；成功取 `output_images[]` 立即下载。
//!
//! 【关键坑】提交回执里的 `task_status` 是假象，真正的图必须拿 `task_id` 去任务端点轮询；
//! 缺 `X-ModelScope-Task-Type` 头会报 task not found。
//!
//! 请求体：恒发 `model` / `prompt` / `size`（`宽x高`）；门控追加 `negative_prompt` / `seed` /
//! `steps`(1–100) / `guidance`。**无 `n`**（单张）。
//!
//! 【内置目录】`Qwen/Qwen-Image-2.1`（默认）、`Qwen/Qwen-Image`、`MAILAND/majicflus_v1`。
//! 用户配置里已存在该服务商时，模型行归用户所有、代码不接管
//! （见 [`crate::image_ai::reconcile_builtin_providers`]）。

use super::{validate_model_caps, FetchedImage, GenStage, ImageProvider, ProviderOutcome};
use crate::image_ai::{
    classify_provider_error, Capabilities, GenRequest, ImageGenError, ModelInfo, ProviderConfig,
    ProviderKind,
};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// 内置魔搭服务商的固定 ID（配置合并按此 ID 认领）。
pub(crate) const PROVIDER_ID: &str = "modelscope";

/// 魔搭 API-Inference 基址；生图端点 `{base}/images/generations`、任务端点 `{base}/tasks/{id}`。
const DEFAULT_BASE: &str = "https://api-inference.modelscope.cn/v1";

/// 默认模型。
const DEFAULT_MODEL: &str = "Qwen/Qwen-Image-2.1";

/// 提交任务必须带：开启异步模式。
const ASYNC_HEADER: &str = "X-ModelScope-Async-Mode";
const ASYNC_HEADER_VALUE: &str = "true";
/// 轮询任务必须带：任务类型；缺了会报 task not found。
const TASK_TYPE_HEADER: &str = "X-ModelScope-Task-Type";
const TASK_TYPE_VALUE: &str = "image_generation";

/// 轮询总预算（超时归 `Provider(true)` 供用户重试）。
const POLL_TOTAL_BUDGET: Duration = Duration::from_secs(120);

/// Qwen-Image 系列推荐分辨率（公开 API 单边上限 2048）。
const QWEN_IMAGE_SIZES: &[&str] = &[
    "1024x1024",
    "1152x2048",
    "2048x1152",
    "1536x1024",
    "1024x1536",
];

/// MAILAND/majicflus_v1 常用分辨率。
const MAJICFLUS_SIZES: &[&str] = &["1024x1024", "768x1024", "1024x768"];

// ===================== 内置模型目录 =====================

/// 魔搭内置服务商：模型目录代码托管（不读 assets/image_ai.json）。
pub(crate) fn builtin_provider() -> ProviderConfig {
    ProviderConfig {
        id: PROVIDER_ID.into(),
        kind: ProviderKind::Modelscope,
        name: "魔搭 ModelScope".into(),
        base_url: DEFAULT_BASE.into(),
        default_model: DEFAULT_MODEL.into(),
        models: vec![
            ModelInfo {
                model: DEFAULT_MODEL.into(),
                capabilities: modelscope_caps(QWEN_IMAGE_SIZES),
            },
            ModelInfo {
                model: "Qwen/Qwen-Image".into(),
                capabilities: modelscope_caps(QWEN_IMAGE_SIZES),
            },
            ModelInfo {
                model: "MAILAND/majicflus_v1".into(),
                capabilities: modelscope_caps(MAJICFLUS_SIZES),
            },
        ],
    }
}

/// 魔搭逐模型共同能力集：单张、无 n；支持 negative_prompt/seed/steps(1–100)/guidance；
/// 输出固定 png（无 output_format 参数）。`sizes` 只列推荐档，自定义尺寸不臆造边界（交服务端）。
fn modelscope_caps(sizes: &[&str]) -> Capabilities {
    Capabilities {
        sizes: sizes.iter().map(|s| (*s).to_string()).collect(),
        max_n: 1,
        // Capabilities::default() 给 bool 的是 false；代码构造必须显式声明。
        supports_n: false,
        supports_seed: true,
        supports_negative_prompt: true,
        supports_steps: true,
        steps_default: 30,
        steps_max: 100,
        supports_guidance: true,
        guidance_default: 3.5,
        guidance_max: 20.0,
        supports_output_format: false,
        supports_response_format: false,
        // 官方未给硬性提示词上限：不拦截。
        max_prompt_chars: 0,
        size_rules: None,
        ..Default::default()
    }
}

// ===================== URL =====================

/// 任务查询端点：`{base}/tasks/{task_id}`。
pub(crate) fn task_url(base: &str, task_id: &str) -> String {
    format!("{}/tasks/{task_id}", base.trim_end_matches('/'))
}

// ===================== 请求体（纯函数）=====================

/// 组装魔搭文生图请求体：恒发 `model`/`prompt`/`size`；其余按能力位门控。
/// **不发 `n`**（官方为单张）；尺寸字段就是 `size`（非硅基流动的 `image_size`）。
pub(crate) fn build_modelscope_body(req: &GenRequest) -> serde_json::Value {
    use serde_json::json;
    let caps = req.caps();
    let mut v = json!({
        "model": req.model,
        "prompt": req.prompt,
        "size": req.size,
    });
    let obj = v.as_object_mut().unwrap();
    if caps.supports_negative_prompt {
        obj.insert("negative_prompt".into(), json!(req.negative_prompt));
    }
    if caps.supports_seed {
        if let Some(seed) = req.seed {
            obj.insert("seed".into(), json!(seed));
        }
    }
    if caps.supports_steps {
        if let Some(steps) = req.steps {
            obj.insert("steps".into(), json!(steps));
        }
    }
    if caps.supports_guidance {
        if let Some(g) = req.guidance {
            obj.insert("guidance".into(), json!(g));
        }
    }
    v
}

// ===================== 响应解析（纯函数）=====================

/// 解析提交响应：成功取 `task_id`。
pub(crate) fn parse_submit(resp: &Value) -> Result<String, ImageGenError> {
    resp.get("task_id")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ImageGenError::provider("响应缺少 task_id", false))
}

/// 任务状态三态。
pub(crate) enum TaskState {
    /// 仍在排队/运行，继续轮询。
    Pending,
    /// 成功：结果图片 URL 列表。
    Done(Vec<String>),
    /// 失败（含成功但无图）。
    Failed(ImageGenError),
}

/// 解析任务端点响应：`SUCCEED` 取 `output_images`；`FAILED` 归服务商错误；
/// 官方进行态（PENDING/RUNNING）继续轮询；**未知状态一律失败**（与 dashscope 的显式
/// 枚举同口径：畸形响应/新增终态白等 120s 毫无收益，预算只兜「任务真在跑」的情形）。
pub(crate) fn parse_task(resp: &Value) -> TaskState {
    match resp
        .get("task_status")
        .and_then(|s| s.as_str())
        .unwrap_or("")
    {
        "SUCCEED" => {
            let urls: Vec<String> = resp
                .get("output_images")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if urls.is_empty() {
                TaskState::Failed(ImageGenError::provider("任务成功但未返回图片", false))
            } else {
                TaskState::Done(urls)
            }
        }
        "FAILED" => {
            let code = resp.get("code");
            let code = code.and_then(|c| c.as_str());
            let msg = resp
                .get("message")
                .and_then(|m| m.as_str())
                .filter(|m| !m.is_empty())
                .unwrap_or("魔搭生图任务失败");
            TaskState::Failed(classify_provider_error(200, code, msg.to_string()))
        }
        "PENDING" | "RUNNING" => TaskState::Pending,
        _ => TaskState::Failed(ImageGenError::provider(
            "任务状态未知（响应畸形或任务已过期）",
            false,
        )),
    }
}

/// 提取 `code: message` 形态的厂商错误细节；字段缺失用 fallback 兜底。
fn error_detail(json: &Value) -> String {
    let code = json
        .get("code")
        .map(|c| c.to_string())
        .unwrap_or_else(|| "Error".into());
    let msg = json
        .get("message")
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .unwrap_or("生图失败");
    format!("{code}: {msg}")
}

/// 下载任务结果 URL 列表（链接有时效，立即拉取；后缀由 fetch_payload 按 URL 路径推断）。
async fn download_all(
    client: &reqwest::Client,
    urls: Vec<String>,
) -> Result<Vec<FetchedImage>, ImageGenError> {
    let mut images = Vec::with_capacity(urls.len());
    for u in urls {
        images
            .push(super::openai::fetch_payload(client, super::openai::ImagePayload::Url(u)).await?);
    }
    Ok(images)
}

pub struct ModelscopeProvider;

impl ImageProvider for ModelscopeProvider {
    fn generate<'a>(
        &'a self,
        client: &'a reqwest::Client,
        key: &'a str,
        req: &'a GenRequest,
        progress: &'a (dyn Fn(GenStage) + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, ImageGenError>> + Send + 'a>> {
        Box::pin(async move {
            // 第 1 步：提交任务（带异步头）。
            progress(GenStage::Submitting);
            let submit_url = super::openai::endpoint(&req.base_url);
            let created = client
                .post(&submit_url)
                .bearer_auth(key)
                .header(ASYNC_HEADER, ASYNC_HEADER_VALUE)
                .json(&build_modelscope_body(req))
                .send()
                .await
                .map_err(|e| ImageGenError::network(format!("请求发送失败: {e}")))?;
            let status = created.status();
            // 429 的 `Retry-After` 必须在读 body 前取（json() 会消费响应）；其它状态一律 None。
            let retry_after = if status.as_u16() == 429 {
                super::retry_after_secs(created.headers())
            } else {
                None
            };
            let json: Value = created
                .json()
                .await
                .map_err(|e| ImageGenError::provider(format!("响应解析失败: {e}"), false))?;
            let task_id = if status.is_success() {
                parse_submit(&json)?
            } else {
                let code = json.get("code").and_then(|c| c.as_str());
                return Err(classify_provider_error(
                    status.as_u16(),
                    code,
                    format!("HTTP {status}：{}", error_detail(&json)),
                )
                .with_retry_after(retry_after));
            };

            // 第 2 步：轮询任务端点（带任务类型头），退避复用 dashscope 的节奏。
            let started = Instant::now();
            progress(GenStage::Polling);
            let mut attempt = 0u32;
            loop {
                // 退避取「本轮退避值」与「剩余预算」的较小者，避免最后一次退避把墙钟顶过预算。
                let remaining = POLL_TOTAL_BUDGET.saturating_sub(started.elapsed());
                let delay = super::dashscope::poll_delay(attempt).min(remaining);
                if !delay.is_zero() {
                    tauri::async_runtime::spawn_blocking(move || std::thread::sleep(delay))
                        .await
                        .ok();
                }
                attempt += 1;
                if started.elapsed() >= POLL_TOTAL_BUDGET {
                    return Err(ImageGenError::provider("魔搭生图超时（120s）", true));
                }

                let resp = client
                    .get(task_url(&req.base_url, &task_id))
                    .bearer_auth(key)
                    .header(TASK_TYPE_HEADER, TASK_TYPE_VALUE)
                    .send()
                    .await
                    .map_err(|e| ImageGenError::network(format!("任务查询失败: {e}")))?;
                let status = resp.status();
                let retry_after = if status.as_u16() == 429 {
                    super::retry_after_secs(resp.headers())
                } else {
                    None
                };
                let json: Value = resp
                    .json()
                    .await
                    .map_err(|e| ImageGenError::provider(format!("响应解析失败: {e}"), false))?;
                if !status.is_success() {
                    let code = json.get("code").and_then(|c| c.as_str());
                    return Err(classify_provider_error(
                        status.as_u16(),
                        code,
                        format!("HTTP {status}：{}", error_detail(&json)),
                    )
                    .with_retry_after(retry_after));
                }
                match parse_task(&json) {
                    TaskState::Pending => continue,
                    TaskState::Failed(e) => return Err(e),
                    TaskState::Done(urls) => {
                        progress(GenStage::Downloading);
                        let images = download_all(client, urls).await?;
                        return Ok(ProviderOutcome { images });
                    }
                }
            }
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

    // ===================== 内置目录 =====================

    #[test]
    fn catalog_lists_three_models() {
        let p = builtin_provider();
        assert_eq!(p.id, PROVIDER_ID);
        assert_eq!(p.kind, ProviderKind::Modelscope);
        assert_eq!(p.base_url, DEFAULT_BASE);
        assert_eq!(p.default_model, "Qwen/Qwen-Image-2.1");
        let ids: Vec<&str> = p.models.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "Qwen/Qwen-Image-2.1",
                "Qwen/Qwen-Image",
                "MAILAND/majicflus_v1"
            ]
        );
        assert!(ids.contains(&p.default_model.as_str()));
    }

    #[test]
    fn catalog_caps_are_single_image_with_steps_and_guidance() {
        for m in &builtin_provider().models {
            let c = &m.capabilities;
            assert_eq!(c.max_n, 1, "{} 单张", m.model);
            assert!(!c.supports_n, "{} 无 n", m.model);
            assert!(c.supports_seed);
            assert!(c.supports_negative_prompt);
            assert!(c.supports_steps);
            assert_eq!((c.steps_default, c.steps_max), (30, 100));
            assert!(c.supports_guidance);
            assert_eq!((c.guidance_default, c.guidance_max), (3.5, 20.0));
            assert!(!c.supports_output_format);
            assert_eq!(c.max_prompt_chars, 0);
            assert!(c.size_rules.is_none());
        }
    }

    // ===================== 请求体组装 =====================

    #[test]
    fn body_sends_size_not_image_size_and_no_n() {
        let mut r = req_for("Qwen/Qwen-Image-2.1");
        r.n = 5;
        r.seed = Some(7);
        r.steps = Some(40);
        r.guidance = Some(4.0);
        let b = build_modelscope_body(&r);
        assert_eq!(b["model"], "Qwen/Qwen-Image-2.1");
        assert_eq!(b["size"], "1024x1024");
        assert_eq!(b["seed"], 7);
        assert_eq!(b["steps"], 40);
        assert_eq!(b["guidance"], 4.0);
        for forbidden in ["image_size", "n", "batch_size"] {
            assert!(b.get(forbidden).is_none(), "不应出现 {forbidden}");
        }
    }

    // ===================== 响应解析 =====================

    #[test]
    fn parse_submit_reads_task_id() {
        let v: Value = serde_json::from_str(r#"{"task_id":"t-1"}"#).unwrap();
        assert_eq!(parse_submit(&v).unwrap(), "t-1");
        let v: Value = serde_json::from_str(r#"{"task_status":"SUCCEED"}"#).unwrap();
        assert_eq!(
            parse_submit(&v).unwrap_err().message,
            "响应缺少 task_id",
            "提交回执里的 task_status 是假象，不能当成功"
        );
    }

    #[test]
    fn parse_task_success_failure_and_pending() {
        let v: Value = serde_json::from_str(
            r#"{"task_status":"SUCCEED","output_images":["https://x/a.png"]}"#,
        )
        .unwrap();
        match parse_task(&v) {
            TaskState::Done(urls) => assert_eq!(urls, vec!["https://x/a.png"]),
            _ => panic!("应判成功"),
        }
        let v: Value =
            serde_json::from_str(r#"{"task_status":"FAILED","message":"boom"}"#).unwrap();
        assert!(matches!(parse_task(&v), TaskState::Failed(_)));
        let v: Value = serde_json::from_str(r#"{"task_status":"RUNNING"}"#).unwrap();
        assert!(matches!(parse_task(&v), TaskState::Pending));
    }

    #[test]
    fn parse_task_success_without_images_is_failure() {
        let v: Value =
            serde_json::from_str(r#"{"task_status":"SUCCEED","output_images":[]}"#).unwrap();
        assert!(matches!(parse_task(&v), TaskState::Failed(_)));
    }

    #[test]
    fn parse_task_unknown_and_missing_status_are_failure() {
        // 只有官方进行态（PENDING/RUNNING）才继续轮询；畸形响应/未知终态径直失败。
        for s in [
            r#"{"task_status":"UNKNOWN"}"#,
            r#"{"task_status":""}"#,
            r#"{}"#,
        ] {
            let v: Value = serde_json::from_str(s).unwrap();
            assert!(matches!(parse_task(&v), TaskState::Failed(_)), "{s}");
        }
    }

    // ===================== 校验 =====================

    #[test]
    fn task_url_appends_path() {
        assert_eq!(
            task_url("https://api-inference.modelscope.cn/v1/", "t-9"),
            "https://api-inference.modelscope.cn/v1/tasks/t-9"
        );
    }

    #[test]
    fn validate_checks_bounds() {
        // 空基址的 NotConfigured 闸在 mod.rs 的 validate_requires_base_url_for_url_gated_kinds 覆盖。
        let caps = caps_of("Qwen/Qwen-Image-2.1");
        let mut r = req_for("Qwen/Qwen-Image-2.1");
        r.steps = Some(101);
        assert!(validate_model_caps(&r, &caps).is_err(), "步数 101 超上限");
        r.steps = Some(100);
        assert!(validate_model_caps(&r, &caps).is_ok());
    }
}
