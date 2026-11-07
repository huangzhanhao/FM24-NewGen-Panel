//! 阿里云百炼（DashScope）适配器 + 三系列内置模型目录。
//!
//! 【三系列，两条端点】千问 Qwen-Image（3.0）、万相 Wan（2.6/2.7）、Z-Image（turbo）
//! 共享同一把 API Key、同一个基址与同一套 `model/input/parameters` 请求壳，同步/异步的
//! 请求体逐字相同（官方口径「共用相同的请求参数结构」），故共用 `build_body`：
//! - 同步：`POST {base}/services/aigc/multimodal-generation/generation`
//!   → `output.choices[].message.content[].image`（结果 URL，24h 有效，立即下载）；
//! - 异步：`POST {base}/services/aigc/image-generation/generation`（带 `X-DashScope-Async: enable`）
//!   → `output.task_id`，再 `GET {base}/tasks/{task_id}` 轮询到 `SUCCEEDED`，结果仍在
//!   同一 `output.choices[].message.content[].image` 形态（**不是**旧协议的 `output.results[].url`）。
//! 是否异步由 `image_ai::generate` 依模型 `supports_async` 门控后写入 `req.call_mode`，适配器据此分流。
//!
//! 【内置目录】只接入各系列当前在售且走上述端点的模型（2026-10 官方口径）：
//! 千问 `qwen-image-3.0-pro/3.0`；万相 `wan2.7-image-pro/2.7-image`、`wan2.6-t2i`；Z-Image `z-image-turbo`。
//! `qwen-image-2.0` 系列已下线；`qwen-image-2.1-pro` 虽同在千问 3.0 端点的 model 可选值内（参数集一致），
//! 但官方「模型选型」不推荐它、主打图生图（最多 10 张参考图），本项目只做文生图，故不接入。
//! 更早的 wan2.5/2.2/wanx2.1、qwen-image-max/plus 走旧版 `text2image/image-synthesis` 端点，同样不接入。
//!
//! 【系列参数差异】（由内置模型目录的能力集门控，能力不声明就不发字段）：
//! | 参数 | qwen-image 3.0 | wan2.7 | wan2.6 | z-image |
//! |---|---|---|---|---|
//! | size | `宽*高` 像素 | `1K/2K/4K` 档位或 `宽*高` 像素 | `宽*高` 像素 | `宽*高` 像素 |
//! | n | 1–6 | 1–4 | 1–4 | **无此参数**（固定 1） |
//! | negative_prompt | ✓ | ✗（在提示词里写排除项） | ✓ | ✗ |
//! | prompt_extend | ✓ 默认开 | ✗（思考开关官方名为 thinking_mode，本项目不暴露） | ✓ 默认开 | ✓ **默认关**（开启即思考模式，单价翻倍） |
//! | prompt_extend_mode | ✓ direct/agent | ✗ | ✗ | ✗ |
//! | enable_thinking | ✓ 默认 true（仅改写开启时生效） | ✗ | ✗ | ✗ |
//! | watermark | ✓ | ✓ | ✓ | ✗ |
//! | seed | ✓ | ✓ | ✓ | ✓ |
//! | async | ✓ | ✓ | ✓ | ✗（官方仅同步） |
//!
//! wan2.7 另有三个本项目未暴露的官方参数：`thinking_mode`（默认 true，质量开关）、
//! `enable_sequential`（组图生成，开启后 n 上限由 4 变 12，且与 thinking_mode/color_palette 互斥）、
//! `color_palette`（自定义配色，3–10 色且占比合计 100%）。不发字段即走官方默认，行为无需补偿；
//! 组图产出的是「一组叙事连贯的图」，与本项目「同一提示词出多张备选脸」的用法不是一回事，故不接入。
//!
//! 模型目录代码内置（[`builtin_provider`]），不读 assets/image_ai.json：新装用户直接得到完整目录。
//! 但用户配置里**已存在** dashscope 服务商时，模型行与默认模型归用户所有、代码不接管
//! （见 [`crate::image_ai::reconcile_builtin_providers`]），所以改这里的目录只对新装生效；
//! 老配置要在「服务商设置」里手动同步模型行，或删掉该服务商重新注入。
//!
//! 协议全部拆为纯函数（URL / 体组装 / 解析 / 退避 / 分流判定），单测不触网。

use super::{validate_model_caps, FetchedImage, GenStage, ImageProvider, ProviderOutcome};
use crate::image_ai::types::{
    classify_provider_error, Capabilities, GenRequest, ImageGenError, ModelInfo, ProviderConfig,
    ProviderKind, SizeRules,
};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// 内置百炼服务商的固定 ID（配置合并按此 ID 认领）。
pub(crate) const PROVIDER_ID: &str = "dashscope";

/// 阿里云直连默认基址（华北2·北京公共域）。官方文档已把**业务空间专属域**
/// `https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/api/v1` 列为建议迁移的新域名
/// （称性能与稳定性更好），但同时明写「现有域名仍可正常使用」；新域名要用户自己的
/// WorkspaceId 才拼得出来，不能当出厂默认，需要的人在服务商设置里改 base_url 即可。
/// 【地域一致性】官方要求**模型、endpoint URL、API Key 三者同地域**，跨地域调用直接失败
/// （新加坡的公共域是 `dashscope-intl.aliyuncs.com`）。
const DEFAULT_BASE: &str = "https://dashscope.aliyuncs.com/api/v1";

/// 异步开关头：该端点只接异步，缺头会被服务端拒为「不支持同步调用」。
pub(crate) const ASYNC_HEADER: &str = "X-DashScope-Async";
pub(crate) const ASYNC_HEADER_VALUE: &str = "enable";

/// 轮询节奏：初始 3s、每次 ×1.5、单次上限 15s；总预算 120s（官方示例口径）。
const POLL_INITIAL_MS: u64 = 3000;
const POLL_FACTOR: f64 = 1.5;
const POLL_MAX_MS: u64 = 15000;
const POLL_TOTAL_BUDGET: Duration = Duration::from_secs(120);

/// 默认模型（Z-Image Turbo：极速低成本，适合批量出图）。
const DEFAULT_MODEL: &str = "z-image-turbo";

// ===================== 内置模型目录 ==================================================================================

/// 百炼内置服务商：模型目录代码托管（不读 JSON）。
pub(crate) fn builtin_provider() -> ProviderConfig {
    ProviderConfig {
        id: PROVIDER_ID.into(),
        kind: ProviderKind::Dashscope,
        name: "阿里云百炼（千问/万相/Z-Image）".into(),
        base_url: DEFAULT_BASE.into(),
        default_model: DEFAULT_MODEL.into(),
        models: vec![
            // ---- 千问 Qwen-Image ------------------------------------------------------------------------------------
            ModelInfo {
                model: "qwen-image-3.0-pro".into(),
                capabilities: qwen_caps(),
            },
            ModelInfo {
                model: "qwen-image-3.0".into(),
                capabilities: qwen_caps(),
            },
            // ---- 万相 Wan -------------------------------------------------------------------------------------------
            ModelInfo {
                model: "wan2.7-image-pro".into(),
                capabilities: wan27_caps(&["1K", "2K", "4K"], &REC_4K, 4096 * 4096),
            },
            ModelInfo {
                model: "wan2.7-image".into(),
                capabilities: wan27_caps(&["1K", "2K"], &REC_2K, 2048 * 2048),
            },
            ModelInfo {
                model: "wan2.6-t2i".into(),
                capabilities: wan26_caps(),
            },
            // ---- Z-Image --------------------------------------------------------------------------------------------
            ModelInfo {
                model: "z-image-turbo".into(),
                capabilities: zimage_caps(),
            },
        ],
        api_key: None,
    }
}

/// 官方「推荐分辨率」表：同一宽高比在各分辨率档下的推荐像素值（1:1、16:9、9:16、4:3、3:4）。
/// 2K 列由 wan2.7-image 与 qwen-image-3.0 系列共用；1K 列属 Wan t2i，见 [`wan26_caps`]。
const REC_4K: [&str; 5] = [
    "4096*4096",
    "4096*2304",
    "2304*4096",
    "4096*3072",
    "3072*4096",
];
const REC_2K: [&str; 5] = [
    "2048*2048",
    "2688*1536",
    "1536*2688",
    "2368*1728",
    "1728*2368",
];

/// 千问 3.0 能力基线：
/// - 官方推荐分辨率五档（总像素均落在 2048² 内）；
/// - 总像素 [512², 2048²]、宽高比 [1:8, 8:1]；
/// - n≤6；负向词/改写（默认开）/水印/seed/异步全开；
/// - 改写模式（`prompt_extend_mode`）与思考模式（`enable_thinking`）是 3.0 独有的协议参数。
/// `max_prompt_chars`：官方「推荐不超过 4500 Token」，前端按字符计数（中文约 1 字 ≈ 1–2 Token），
/// 取值宽松只拦明显超长。
fn qwen_caps() -> Capabilities {
    Capabilities {
        sizes: REC_2K.iter().map(|s| (*s).to_string()).collect(),
        max_n: 6,
        // 注：Capabilities::default() 给 bool 的是 false；代码构造必须显式声明，
        // serde(default="yes") 只对 JSON 缺字段反序列化生效。
        supports_n: true,
        supports_seed: true,
        supports_negative_prompt: true,
        supports_prompt_extend: true,
        prompt_extend_default: true,
        supports_prompt_extend_mode: true,
        prompt_extend_mode_default: "direct".into(),
        supports_enable_thinking: true,
        enable_thinking_default: true,
        supports_watermark: true,
        supports_async: true,
        max_prompt_chars: 4500,
        size_rules: Some(SizeRules {
            total_min: 512 * 512,
            total_max: 2048 * 2048,
            ratio_min: 0.125,
            ratio_max: 8.0,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 万相 2.7 能力：`size` 有两种写法且不可混用——`1K/2K/4K` 档位串，或自定义 `宽*高`
/// （总像素 ≥768²、宽高比 [1:8, 8:1]；总像素上限随型号，由 `total_max` 传入）。
/// **档位串在没有图片输入时一律输出正方形**，所以非正方形只能走自定义像素，sizeRules 必须声明，
/// 否则越界值会一路送到厂商吃 400。
/// 官方不支持 negative_prompt 与 prompt_extend（排除项写进提示词；思考模式是另一个
/// 参数 thinking_mode，本项目不暴露，走官方默认开）；水印/seed/异步支持，n≤4，提示词 ≤5000 字符。
fn wan27_caps(tiers: &[&str], pixels: &[&str], total_max: u64) -> Capabilities {
    Capabilities {
        sizes: tiers
            .iter()
            .chain(pixels.iter())
            .map(|s| (*s).to_string())
            .collect(),
        max_n: 4,
        supports_n: true,
        supports_seed: true,
        supports_watermark: true,
        supports_async: true,
        max_prompt_chars: 5000,
        size_rules: Some(SizeRules {
            total_min: 768 * 768,
            total_max,
            ratio_min: 0.125,
            ratio_max: 8.0,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 万相 2.6 能力（`wan2.6-t2i`）：总像素 [1280², 1440²]、宽高比 [1:4, 4:1]，预设即官方推荐五档；
/// 负向词/改写（默认开）/水印/seed/异步全开，n≤4，提示词 ≤2100 字符。
fn wan26_caps() -> Capabilities {
    Capabilities {
        sizes: vec![
            "1280*1280".into(),
            "1104*1472".into(),
            "1472*1104".into(),
            "960*1696".into(),
            "1696*960".into(),
        ],
        max_n: 4,
        supports_n: true,
        supports_seed: true,
        supports_negative_prompt: true,
        supports_prompt_extend: true,
        prompt_extend_default: true,
        supports_watermark: true,
        supports_async: true,
        max_prompt_chars: 2100,
        size_rules: Some(SizeRules {
            total_min: 1280 * 1280,
            total_max: 1440 * 1440,
            ratio_min: 0.25,
            ratio_max: 4.0,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Z-Image 能力：快速低成本档，参数面最窄——
/// - 没有 n 参数（出图固定 1 张，[`Capabilities::supports_n`]=false）；
/// - 不支持负向词、水印；
/// - `prompt_extend` 官方**默认关**（开启即「智能思考」，返回改写词与推理过程，单价翻倍）；
/// - 提示词硬上限 800 字符。
/// 尺寸约束：官方只写「总像素在 [512², 2048²] 之间」；同区间推出的**单边**区间一并填上
/// （宽/高各自 512–2048，见 [`SizeRules::min_side`]），**宽高比留空**——官方对该模型无比例限制。
/// 预设取官方「总像素 1024² 推荐分辨率」整组 11 个比例；官方另有 1280²/1536² 两组（推荐区间
/// [1024², 1536²]，`size` 默认值 1024*1536 即落在其间），不铺进下拉，走自定义即可命中。
fn zimage_caps() -> Capabilities {
    Capabilities {
        sizes: vec![
            "1024*1024".into(),
            "1248*832".into(),
            "832*1248".into(),
            "1152*864".into(),
            "864*1152".into(),
            "1152*896".into(),
            "896*1152".into(),
            "1280*720".into(),
            "720*1280".into(),
            "1344*576".into(),
            "576*1344".into(),
        ],
        max_n: 1,
        supports_n: false,
        supports_seed: true,
        supports_prompt_extend: true,
        prompt_extend_default: false,
        // 官方仅同步调用，不接入异步链。
        supports_async: false,
        max_prompt_chars: 800,
        size_rules: Some(SizeRules {
            min_side: 512,
            max_side: 2048,
            total_min: 512 * 512,
            total_max: 2048 * 2048,
            ..Default::default()
        }),
        ..Default::default()
    }
}

// ===================== URL 与请求体（纯函数） ========================================================================

/// 解析基址：空白配置回落默认值，统一去掉结尾 '/'（官方 SDK base_http_api_url 口径）。
pub(crate) fn base(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    if b.is_empty() {
        DEFAULT_BASE.into()
    } else {
        b.to_string()
    }
}

/// 同步多模态消息式端点。
pub(crate) fn sync_url(base: &str) -> String {
    format!("{base}/services/aigc/multimodal-generation/generation")
}

/// 异步创建端点（该端点只接异步：缺 `X-DashScope-Async` 头会被拒为「不支持同步调用」）。
pub(crate) fn async_url(base: &str) -> String {
    format!("{base}/services/aigc/image-generation/generation")
}

/// 异步任务查询端点。
pub(crate) fn task_url(base: &str, task_id: &str) -> String {
    format!("{base}/tasks/{task_id}")
}

/// 尺寸显示用的 'x' 统一转为厂商文档口径的 '*'（1024x1024 → 1024*1024）。
/// 万相 2.7 的分辨率档位（"1K/2K/4K"）不含分隔符，原样透传。
fn vendor_size(size: &str) -> String {
    size.trim().replace('x', "*")
}

/// 组装 `parameters` 对象：可选生成参数一律由族内共用的 [`super::capability_params`]
/// 按能力位产出（能力不声明就不带字段，避免给不支持的模型塞陌生参数触发 400），
/// 本层只补协议专有的 `size`——`x` 转官方口径的 `*`，万相 2.7 的档位串原样透传。
fn build_params(req: &GenRequest) -> Value {
    let mut params = Value::Object(super::capability_params(req));
    params
        .as_object_mut()
        .unwrap()
        .insert("size".into(), serde_json::json!(vendor_size(&req.size)));
    params
}

/// 组装同步请求体——官方多模态模板：单条 user 消息、content 内一个 text 对象（文生图）。
pub(crate) fn build_body(req: &GenRequest) -> Value {
    use serde_json::json;
    json!({
        "model": req.model,
        "input": {
            "messages": [
                {"role": "user", "content": [{"text": req.prompt}]}
            ]
        },
        "parameters": build_params(req),
    })
}

/// 从 `choices[].message.content[].image` 提取所有非空图片 URL——
/// 同步响应与异步任务结果同构（新协议都在这条路径上），故两处共用此提取。
fn extract_image_urls(choices: &[Value]) -> Vec<String> {
    let mut urls = Vec::new();
    for choice in choices {
        if let Some(items) = choice
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
        {
            for item in items {
                if let Some(url) = item.get("image").and_then(|i| i.as_str()) {
                    if !url.is_empty() {
                        urls.push(url.to_string());
                    }
                }
            }
        }
    }
    urls
}

/// 解析同步响应：`output.choices[].message.content[].image`（结果 URL）。
pub(crate) fn parse_sync(resp: &Value) -> Result<Vec<String>, ImageGenError> {
    if let Some(code) = resp.get("code").and_then(|c| c.as_str()) {
        if !code.is_empty() {
            // 200 响应里带业务 code 也算失败：业务码优先，未命中回落 Provider。
            return Err(classify_provider_error(
                200,
                Some(code),
                error_detail(resp, "同步生图失败"),
            ));
        }
    }
    let choices = resp
        .get("output")
        .and_then(|o| o.get("choices"))
        .and_then(|c| c.as_array())
        .ok_or_else(|| ImageGenError::provider("响应缺少 output.choices", false))?;
    let urls = extract_image_urls(choices);
    if urls.is_empty() {
        return Err(ImageGenError::provider("响应中没有图片结果", false));
    }
    Ok(urls)
}

/// 解析异步创建响应：成功取 `output.task_id`；顶层业务码按既有分类，未命中回落 Provider。
pub(crate) fn parse_submit(resp: &Value) -> Result<String, ImageGenError> {
    if let Some(code) = resp.get("code").and_then(|c| c.as_str()) {
        if !code.is_empty() {
            return Err(classify_provider_error(
                200,
                Some(code),
                error_detail(resp, "异步生图任务创建失败"),
            ));
        }
    }
    resp.get("output")
        .and_then(|o| o.get("task_id"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .ok_or_else(|| ImageGenError::provider("响应缺少 output.task_id", false))
}

/// 异步任务查询响应的归类。
pub(crate) enum TaskState {
    /// 任务仍在排队。
    Pending,
    /// 仍在运行（与 Pending 分开仅用于阶段区分）。
    Running,
    /// 成功：图片 URL 列表。
    Succeeded(Vec<String>),
    /// 失败：已分类的厂商错误。
    Failed(ImageGenError),
}

/// 解析任务查询响应：`output.task_status` 驱动四态。
/// 失败详情**两处都要看**：任务级失败惯常写在 `output.code`/`output.message`，
/// 而官方「响应参数」表把 `code`/`message` 列在顶层。只读一处会漏掉业务码，漏了就把
/// `DataInspectionFailed` / `IPInfringementSuspect` 这类用户能自救的错误（改提示词、关智能改写）
/// 降级成没有因由、也没有处置建议的 Provider。
pub(crate) fn parse_task(resp: &Value) -> TaskState {
    let output = resp.get("output");
    let status = output
        .and_then(|o| o.get("task_status"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    match status {
        "PENDING" => TaskState::Pending,
        "RUNNING" => TaskState::Running,
        "SUCCEEDED" => {
            let choices = output
                .and_then(|o| o.get("choices"))
                .and_then(|c| c.as_array());
            TaskState::Succeeded(choices.map(|c| extract_image_urls(c)).unwrap_or_default())
        }
        "FAILED" => {
            let field = |key: &str| {
                output
                    .and_then(|o| o.get(key))
                    .and_then(|v| v.as_str())
                    .filter(|v| !v.is_empty())
                    .or_else(|| {
                        resp.get(key)
                            .and_then(|v| v.as_str())
                            .filter(|v| !v.is_empty())
                    })
            };
            let code = field("code");
            let msg = field("message").unwrap_or("异步生图任务失败");
            TaskState::Failed(classify_provider_error(200, code, msg.to_string()))
        }
        // 服务端取消不是「本地取消」（Cancelled 语义不同），归 Provider 让用户可原样重试。
        "CANCELED" => TaskState::Failed(ImageGenError::provider("任务已被服务端取消", false)),
        // 任务 24h 后失效；UNKNOWN/缺状态都不得判成功，归 Provider 提示重新提交。
        _ => TaskState::Failed(ImageGenError::provider(
            "任务状态未知（task_id 可能已过期）",
            false,
        )),
    }
}

/// 异步轮询退避：初始 3s，每次 ×1.5，单次上限 15s（官方示例节奏）。
pub(crate) fn poll_delay(attempt: u32) -> Duration {
    let ms = (POLL_INITIAL_MS as f64) * POLL_FACTOR.powi(attempt as i32);
    Duration::from_millis(ms.min(POLL_MAX_MS as f64) as u64)
}

/// 是否走异步链：唯一判据是 `generate` 门控后写入的 `call_mode == "async"`。
pub(crate) fn use_async(req: &GenRequest) -> bool {
    req.call_mode.as_deref() == Some("async")
}

/// 提取 `code: message` 形态的厂商错误细节；字段缺失用 fallback 兜底。
fn error_detail(json: &Value, fallback: &str) -> String {
    let code = json.get("code").and_then(|c| c.as_str()).unwrap_or("Error");
    let msg = json
        .get("message")
        .and_then(|m| m.as_str())
        .filter(|m| !m.is_empty())
        .unwrap_or(fallback);
    format!("{code}: {msg}")
}

/// 下载结果 URL 为字节：24h 有效，调用方必须此刻拉取；后缀由 URL 路径推断（OSS 链接带 .png）。
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

/// 百炼适配器（同步/异步双链；无状态）。
pub struct DashscopeProvider;

impl ImageProvider for DashscopeProvider {
    fn generate<'a>(
        &'a self,
        client: &'a reqwest::Client,
        key: &'a str,
        req: &'a GenRequest,
        progress: &'a (dyn Fn(GenStage) + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, ImageGenError>> + Send + 'a>> {
        Box::pin(async move {
            let base = base(&req.base_url);
            // 提交阶段为同步/异步两链共有：先在提交前上报一次。
            progress(GenStage::Submitting);
            if use_async(req) {
                return run_async(client, key, req, &base, progress).await;
            }
            // 同步链：一次请求直接出图（大图/多图可能耗时数分钟，
            // HTTP 客户端总超时已在 image_ai::generate 放宽到 600s）。
            let resp = client
                .post(sync_url(&base))
                .bearer_auth(key)
                .json(&build_body(req))
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
            let json: Value = resp
                .json()
                .await
                .map_err(|e| ImageGenError::provider(format!("HTTP {status}：响应解析失败: {e}"), false))?;
            if !status.is_success() {
                // 顶层 code 优先分类，否则按 HTTP 状态分流；message 沿用既有中文文案。
                let code = json.get("code").and_then(|c| c.as_str());
                return Err(classify_provider_error(
                    status.as_u16(),
                    code,
                    format!("HTTP {status}：{}", error_detail(&json, "同步生图失败")),
                )
                .with_retry_after(retry_after));
            }
            let urls = parse_sync(&json)?;
            progress(GenStage::Downloading);
            let images = download_all(client, urls).await?;
            Ok(ProviderOutcome { images })
        })
    }

    /// 本协议无特有门控：端点/基址由适配器内部固定，约束全部来自模型声明的数据。
    fn validate(&self, req: &GenRequest, caps: &Capabilities) -> Result<(), ImageGenError> {
        validate_model_caps(req, caps)
    }
}

/// 异步链：创建任务（带异步头）→ 按 [`poll_delay`] 轮询 → 成功后下载结果。
/// 请求体与同步链逐字相同（官方「共用相同的请求参数结构」），故复用 [`build_body`]。
/// 总预算 [`POLL_TOTAL_BUDGET`]；超预算归 `Provider(true)` 供用户重试。
async fn run_async(
    client: &reqwest::Client,
    key: &str,
    req: &GenRequest,
    base: &str,
    progress: &(dyn Fn(GenStage) + Sync),
) -> Result<ProviderOutcome, ImageGenError> {
    let created = client
        .post(async_url(base))
        .bearer_auth(key)
        .header(ASYNC_HEADER, ASYNC_HEADER_VALUE)
        .json(&build_body(req))
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
        .map_err(|e| ImageGenError::provider(format!("HTTP {status}：响应解析失败: {e}"), false))?;
    let task_id = if status.is_success() {
        parse_submit(&json)?
    } else {
        let code = json.get("code").and_then(|c| c.as_str());
        return Err(classify_provider_error(
            status.as_u16(),
            code,
            format!(
                "HTTP {status}：{}",
                error_detail(&json, "异步生图任务创建失败")
            ),
        )
        .with_retry_after(retry_after));
    };

    let started = Instant::now();
    // 进入轮询阶段只上报一次（后续每轮不再重复，避免刷事件；调用方另有按阶段去重兜底）。
    progress(GenStage::Polling);
    let mut attempt = 0u32;
    loop {
        // 退避取「本轮退避值」与「剩余预算」的较小者，避免最后一次退避把墙钟顶过 120s；
        // 睡满后若预算已耗尽就径直超时、不再发 GET ⇒ 轮询阶段墙钟不超过 120s。
        // 超时判据 `started` 建在建任务之后：单次尝试墙钟 = submit + 120s 轮询预算 + 至多一次在途 GET。
        let remaining = POLL_TOTAL_BUDGET.saturating_sub(started.elapsed());
        let delay = poll_delay(attempt).min(remaining);
        if !delay.is_zero() {
            // async_runtime 未再导出 sleep（无新依赖前提下）：spawn_blocking 里睡，与 run_queue 同款。
            tauri::async_runtime::spawn_blocking(move || std::thread::sleep(delay))
                .await
                .ok();
        }
        attempt += 1;
        if started.elapsed() >= POLL_TOTAL_BUDGET {
            return Err(ImageGenError::provider("异步生图超时（120s）", true));
        }

        let resp = client
            .get(task_url(base, &task_id))
            .bearer_auth(key)
            .send()
            .await
            .map_err(|e| ImageGenError::network(format!("任务查询失败: {e}")))?;
        let status = resp.status();
        // 429 的 `Retry-After` 必须在读 body 前取（json() 会消费响应）；其它状态一律 None。
        let retry_after = if status.as_u16() == 429 {
            super::retry_after_secs(resp.headers())
        } else {
            None
        };
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ImageGenError::provider(format!("HTTP {status}：响应解析失败: {e}"), false))?;
        if !status.is_success() {
            let code = json.get("code").and_then(|c| c.as_str());
            return Err(classify_provider_error(
                status.as_u16(),
                code,
                format!("HTTP {status}：{}", error_detail(&json, "任务查询失败")),
            )
            .with_retry_after(retry_after));
        }
        match parse_task(&json) {
            TaskState::Pending | TaskState::Running => continue,
            TaskState::Failed(e) => return Err(e),
            TaskState::Succeeded(urls) => {
                if urls.is_empty() {
                    return Err(ImageGenError::provider("任务成功但没有返回图片", false));
                }
                progress(GenStage::Downloading);
                let images = download_all(client, urls).await?;
                return Ok(ProviderOutcome { images });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 请求体门控相关的能力快照夹具（generate 会整体回填，测试直接构造）。
    fn fixture_caps() -> Capabilities {
        Capabilities {
            supports_n: true,
            supports_negative_prompt: true,
            supports_prompt_extend: true,
            prompt_extend_default: true,
            supports_watermark: true,
            supports_seed: true,
            supports_prompt_extend_mode: false,
            prompt_extend_mode_default: "direct".into(),
            supports_enable_thinking: false,
            enable_thinking_default: true,
            watermark_default: false,
            ..Default::default()
        }
    }

    fn gen_req(model: &str) -> GenRequest {
        GenRequest {
            provider_id: PROVIDER_ID.into(),
            model: model.into(),
            prompt: "一张脸".into(),
            negative_prompt: "模糊".into(),
            size: "1024x1024".into(),
            n: 2,
            seed: Some(7),
            caps: Some(fixture_caps()),
            ..crate::image_ai::types::test_req_base()
        }
    }

    // ===================== 内置目录 ==================================================================================

    #[test]
    fn catalog_lists_six_models() {
        let p = builtin_provider();
        assert_eq!(p.id, PROVIDER_ID);
        assert_eq!(p.kind, ProviderKind::Dashscope);
        assert_eq!(p.default_model, "z-image-turbo");
        let ids: Vec<&str> = p.models.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "qwen-image-3.0-pro",
                "qwen-image-3.0",
                "wan2.7-image-pro",
                "wan2.7-image",
                "wan2.6-t2i",
                "z-image-turbo",
            ]
        );
        // 默认模型必须在目录中。
        assert!(ids.contains(&p.default_model.as_str()));
        // 已下线的 2.0 系列不得再出现。
        assert!(!ids.iter().any(|m| m.contains("2.0")));
    }

    #[test]
    fn catalog_async_flags_match_official_list() {
        // 官方异步清单：千问 3.0 / 万相 2.7 / 万相 2.6 可异步；Z-Image 仅同步。
        let p = builtin_provider();
        let flag = |id: &str| {
            p.models
                .iter()
                .find(|m| m.model == id)
                .unwrap()
                .capabilities
                .supports_async
        };
        for id in [
            "qwen-image-3.0-pro",
            "qwen-image-3.0",
            "wan2.7-image-pro",
            "wan2.7-image",
            "wan2.6-t2i",
        ] {
            assert!(flag(id), "{id} 应支持异步");
        }
        assert!(!flag("z-image-turbo"), "z-image 官方仅同步");
        let true_count = p
            .models
            .iter()
            .filter(|m| m.capabilities.supports_async)
            .count();
        assert_eq!(true_count, 5, "5 个支持异步 / 1 个不支持");
    }

    #[test]
    fn catalog_qwen_caps_match_official_spec() {
        let p = builtin_provider();
        let m = |id: &str| p.models.iter().find(|m| m.model == id).unwrap();
        for id in ["qwen-image-3.0-pro", "qwen-image-3.0"] {
            let c = &m(id).capabilities;
            assert_eq!(c.max_n, 6, "{id} n≤6");
            assert!(c.supports_negative_prompt, "{id} 负向词");
            assert!(
                c.supports_prompt_extend && c.prompt_extend_default,
                "{id} 改写默认开"
            );
            assert!(c.supports_watermark, "{id} 水印");
            assert!(c.supports_seed, "{id} seed");
            assert!(c.supports_n, "{id} 有 n 参数");
            assert!(c.supports_async, "{id} 支持异步");
            let r = c.size_rules.as_ref().unwrap();
            assert_eq!(r.total_min, 512 * 512);
            assert_eq!(r.total_max, 2048 * 2048);
            // 3.0 有 1:8–8:1 宽高比门限。
            assert_eq!(r.ratio_min, 0.125);
            assert_eq!(r.ratio_max, 8.0);
            // 提示词上限 4500。
            assert_eq!(c.max_prompt_chars, 4500);
            // 改写模式 / 思考模式是 3.0 的协议参数。
            assert!(c.supports_prompt_extend_mode, "{id} 有改写模式");
            assert!(c.supports_enable_thinking, "{id} 有思考模式");
            assert_eq!(c.prompt_extend_mode_default, "direct");
            assert!(c.enable_thinking_default);
        }
    }

    #[test]
    fn catalog_wan27_mixes_size_tiers_with_pixel_rules() {
        let p = builtin_provider();
        let m = |id: &str| p.models.iter().find(|m| m.model == id).unwrap();
        let pro = m("wan2.7-image-pro");
        assert_eq!(
            pro.capabilities.sizes,
            vec![
                "1K".to_string(),
                "2K".to_string(),
                "4K".to_string(),
                "4096*4096".to_string(),
                "4096*2304".to_string(),
                "2304*4096".to_string(),
                "4096*3072".to_string(),
                "3072*4096".to_string(),
            ]
        );
        assert_eq!(pro.capabilities.max_n, 4);
        assert!(!pro.capabilities.supports_negative_prompt);
        assert!(!pro.capabilities.supports_prompt_extend);
        assert!(pro.capabilities.supports_watermark);
        assert!(pro.capabilities.supports_seed);
        // 档位串在无图片输入时只出正方形，非正方形必须走自定义像素 → 官方区间必须声明。
        let r = pro.capabilities.size_rules.as_ref().unwrap();
        assert_eq!(r.total_min, 768 * 768);
        assert_eq!(r.total_max, 4096 * 4096, "pro 文生图总像素上限 4096²");
        assert_eq!(r.ratio_min, 0.125);
        assert_eq!(r.ratio_max, 8.0);

        let std = m("wan2.7-image");
        assert_eq!(
            std.capabilities.sizes,
            vec![
                "1K".to_string(),
                "2K".to_string(),
                "2048*2048".to_string(),
                "2688*1536".to_string(),
                "1536*2688".to_string(),
                "2368*1728".to_string(),
                "1728*2368".to_string(),
            ]
        );
        assert_eq!(
            std.capabilities.size_rules.as_ref().unwrap().total_max,
            2048 * 2048,
            "非 pro 总像素上限 2048²"
        );
        // prompt_extend_default 对不支持改写的模型无意义，保持缺省 true 不影响门控。
    }

    #[test]
    fn wan27_size_rules_pass_tiers_and_police_custom_pixels() {
        let p = builtin_provider();
        let caps = &p
            .models
            .iter()
            .find(|m| m.model == "wan2.7-image")
            .unwrap()
            .capabilities;
        let check = |size: &str| {
            let mut r = gen_req("wan2.7-image");
            r.size = size.into();
            r.n = 1;
            validate_model_caps(&r, caps)
        };
        // 档位串不在像素校验射程内（validate_model_caps 只校验非预设值）。
        assert!(check("1K").is_ok());
        assert!(check("2K").is_ok());
        // 预设像素值天然合法。
        assert!(check("1536*2688").is_ok());
        // 自定义像素按官方区间放行/拦下。
        assert!(check("768*768").is_ok(), "总像素下限 768²");
        assert!(check("512*1024").is_err(), "低于 768² 下限");
        assert!(check("2048*2049").is_err(), "非 pro 超 2048² 上限");
        assert!(check("300*8000").is_err(), "宽高比超出 1:8–8:1");
    }

    #[test]
    fn catalog_wan26_matches_v2_constraints() {
        let p = builtin_provider();
        let c = &p
            .models
            .iter()
            .find(|m| m.model == "wan2.6-t2i")
            .unwrap()
            .capabilities;
        assert_eq!(c.max_n, 4);
        assert!(c.supports_negative_prompt && c.supports_prompt_extend && c.supports_watermark);
        let r = c.size_rules.as_ref().unwrap();
        assert_eq!(r.total_min, 1280 * 1280);
        assert_eq!(r.total_max, 1440 * 1440);
        assert_eq!(r.ratio_min, 0.25);
        assert_eq!(r.ratio_max, 4.0);
    }

    #[test]
    fn catalog_zimage_is_single_image_with_extend_off_by_default() {
        let p = builtin_provider();
        let c = &p
            .models
            .iter()
            .find(|m| m.model == "z-image-turbo")
            .unwrap()
            .capabilities;
        assert_eq!(c.max_n, 1);
        assert!(!c.supports_n, "z-image 无 n 参数");
        assert!(!c.supports_negative_prompt, "z-image 无负向词");
        assert!(!c.supports_watermark, "z-image 无水印");
        assert!(c.supports_prompt_extend);
        assert!(!c.prompt_extend_default, "思考改写官方默认关，开启翻倍计费");
        assert!(c.supports_seed);
        assert_eq!(c.max_prompt_chars, 800);
        assert!(!c.supports_prompt_extend_mode, "z-image 无改写模式");
        assert!(!c.supports_enable_thinking, "z-image 无思考模式");
        // 预设 = 官方「总像素 1024²」整组 11 个比例：缺 7:9/9:7/21:9/9:21 时比例选择器就选不到。
        assert_eq!(c.sizes.len(), 11);
        for s in ["1152*896", "896*1152", "1344*576", "576*1344"] {
            assert!(c.sizes.contains(&s.to_string()), "{s} 应在预设内");
        }
        // 尺寸约束：官方只给总像素区间，单边按同区间推出；**比例留空**（官方无比例限制）。
        let r = c.size_rules.as_ref().unwrap();
        assert_eq!((r.min_side, r.max_side), (512, 2048));
        assert_eq!((r.total_min, r.total_max), (512 * 512, 2048 * 2048));
        assert_eq!((r.ratio_min, r.ratio_max), (0.0, 0.0));
    }

    // ===================== 请求体组装 ================================================================================

    #[test]
    fn body_uses_messages_shape_with_star_size() {
        let b = build_body(&gen_req("qwen-image-3.0-pro"));
        assert_eq!(b["model"], "qwen-image-3.0-pro");
        assert_eq!(b["input"]["messages"][0]["role"], "user");
        assert_eq!(b["input"]["messages"][0]["content"][0]["text"], "一张脸");
        assert_eq!(b["parameters"]["size"], "1024*1024"); // x → *
        assert_eq!(b["parameters"]["n"], 2);
        assert_eq!(b["parameters"]["negative_prompt"], "模糊");
        assert_eq!(b["parameters"]["prompt_extend"], true); // None → 回落能力集声明的默认
        assert_eq!(b["parameters"]["watermark"], false);
        assert_eq!(b["parameters"]["seed"], 7);
    }

    #[test]
    fn zimage_omits_n_negative_watermark_and_defaults_extend_off() {
        let mut r = gen_req("z-image-turbo");
        r.caps.as_mut().unwrap().supports_n = false;
        r.caps.as_mut().unwrap().supports_negative_prompt = false;
        r.caps.as_mut().unwrap().supports_watermark = false;
        // 改写默认值同样来自能力集（不再按模型名推）：z-image 官方默认关，开启即思考模式、单价翻倍。
        r.caps.as_mut().unwrap().prompt_extend_default = false;
        r.n = 1;
        let b = build_body(&r);
        assert!(b["parameters"].get("n").is_none());
        assert!(b["parameters"].get("negative_prompt").is_none());
        assert!(b["parameters"].get("watermark").is_none());
        assert_eq!(b["parameters"]["prompt_extend"], false); // None → z-image 默认关
        assert_eq!(b["parameters"]["seed"], 7);
        assert_eq!(b["parameters"]["size"], "1024*1024");
        // 用户显式开启思考：必须如实发送 true。
        r.prompt_extend = Some(true);
        assert_eq!(build_body(&r)["parameters"]["prompt_extend"], true);
    }

    #[test]
    fn wan27_passes_size_tier_verbatim_and_skips_unsupported_fields() {
        let mut r = gen_req("wan2.7-image-pro");
        r.size = "4K".into();
        r.caps.as_mut().unwrap().supports_negative_prompt = false;
        r.caps.as_mut().unwrap().supports_prompt_extend = false;
        r.n = 4;
        let b = build_body(&r);
        assert_eq!(b["parameters"]["size"], "4K"); // 档位不做 x→* 之外的任何变换
        assert_eq!(b["parameters"]["n"], 4);
        assert!(b["parameters"].get("negative_prompt").is_none());
        assert!(b["parameters"].get("prompt_extend").is_none());
        assert_eq!(b["parameters"]["watermark"], false);
    }

    #[test]
    fn user_overrides_are_honored() {
        let mut r = gen_req("wan2.6-t2i");
        r.prompt_extend = Some(false);
        r.watermark = Some(true);
        r.seed = None;
        let b = build_body(&r);
        assert_eq!(b["parameters"]["prompt_extend"], false);
        assert_eq!(b["parameters"]["watermark"], true);
        assert!(b["parameters"].get("seed").is_none());
    }

    #[test]
    fn extend_mode_and_thinking_follow_capability_gate() {
        // 不支持的模型（默认 fixture，此处取 z-image：无改写模式/思考模式）：两个字段都不出现。
        let b = build_body(&gen_req("z-image-turbo"));
        assert!(b["parameters"].get("prompt_extend_mode").is_none());
        assert!(b["parameters"].get("enable_thinking").is_none());

        // 3.0：用户未选 → 回流模型声明的默认（direct / true）。
        let mut r = gen_req("qwen-image-3.0");
        r.caps.as_mut().unwrap().supports_prompt_extend_mode = true;
        r.caps.as_mut().unwrap().supports_enable_thinking = true;
        let b = build_body(&r);
        assert_eq!(b["parameters"]["prompt_extend_mode"], "direct");
        assert_eq!(b["parameters"]["enable_thinking"], true);

        // 用户显式选择必须如实发送。
        r.prompt_extend_mode = Some("agent".into());
        r.enable_thinking = Some(false);
        let b = build_body(&r);
        assert_eq!(b["parameters"]["prompt_extend_mode"], "agent");
        assert_eq!(b["parameters"]["enable_thinking"], false);

        // 默认档为空串（旧配置/程序构造未声明）时兜官方 direct，绝不发空值。
        r.prompt_extend_mode = None;
        r.caps.as_mut().unwrap().prompt_extend_mode_default = String::new();
        assert_eq!(build_body(&r)["parameters"]["prompt_extend_mode"], "direct");
    }

    #[test]
    fn watermark_uses_model_declared_default() {
        let mut r = gen_req("wan2.6-t2i");
        r.caps.as_mut().unwrap().watermark_default = true;
        assert_eq!(build_body(&r)["parameters"]["watermark"], true);
        r.watermark = Some(false);
        assert_eq!(build_body(&r)["parameters"]["watermark"], false);
    }

    // ===================== 响应解析与 URL ============================================================================

    #[test]
    fn sync_response_extracts_image_urls() {
        let v: Value = serde_json::from_str(
            r#"{"output":{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":[{"type":"image","image":"https://r/1.png"}]}}]}}"#,
        )
        .unwrap();
        assert_eq!(parse_sync(&v).unwrap(), vec!["https://r/1.png"]);
        // z-image 思考模式会混排 text/image：只取 image。
        let v: Value = serde_json::from_str(
            r#"{"output":{"choices":[{"message":{"content":[{"type":"text","text":"思考过程"},{"type":"image","image":"https://r/z.png"}]}}]}}"#,
        )
        .unwrap();
        assert_eq!(parse_sync(&v).unwrap(), vec!["https://r/z.png"]);
    }

    #[test]
    fn sync_error_uses_top_level_code() {
        let v: Value =
            serde_json::from_str(r#"{"code":"BadRequest","message":"size invalid"}"#).unwrap();
        let err = parse_sync(&v).unwrap_err();
        assert_eq!(err.message, "BadRequest: size invalid");
        // 业务码未命中分类、非 HTTP 错误路径 → Provider
        assert_eq!(err.kind, crate::image_ai::types::GenErrorKind::Provider);
        let v: Value = serde_json::from_str(r#"{"output":{"choices":[]}}"#).unwrap();
        assert!(parse_sync(&v).unwrap_err().message.contains("没有图片结果"));
    }

    #[test]
    fn urls_resolve_from_given_base() {
        assert_eq!(base("   "), DEFAULT_BASE);
        assert_eq!(
            base("https://maas.qianwenaiapi.com/api/v1/"),
            "https://maas.qianwenaiapi.com/api/v1"
        );
        assert_eq!(
            sync_url(DEFAULT_BASE),
            "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation"
        );
    }

    // ===================== 异步：URL / 常量 / 解析 / 退避 / 分流 =====================================================

    #[test]
    fn async_urls_and_header_constants() {
        assert_eq!(
            async_url(DEFAULT_BASE),
            "https://dashscope.aliyuncs.com/api/v1/services/aigc/image-generation/generation"
        );
        assert_eq!(
            task_url(DEFAULT_BASE, "abc123"),
            "https://dashscope.aliyuncs.com/api/v1/tasks/abc123"
        );
        // 头名/值抽成常量：断言字面值即钉住「确实用官方异步头」的契约（真发头无法单测）。
        assert_eq!(ASYNC_HEADER, "X-DashScope-Async");
        assert_eq!(ASYNC_HEADER_VALUE, "enable");
    }

    #[test]
    fn parse_submit_reads_task_id_and_classifies_top_level_code() {
        let v: Value =
            serde_json::from_str(r#"{"output":{"task_id":"t-1","task_status":"PENDING"}}"#)
                .unwrap();
        assert_eq!(parse_submit(&v).unwrap(), "t-1");
        // 顶层业务码失败：DataInspection → ContentRejected。
        let v: Value =
            serde_json::from_str(r#"{"code":"DataInspectionFailed","message":"内容不合规"}"#)
                .unwrap();
        assert_eq!(
            parse_submit(&v).unwrap_err().kind,
            crate::image_ai::types::GenErrorKind::ContentRejected
        );
        // 缺 task_id：结构错误归 Provider。
        let v: Value = serde_json::from_str(r#"{"output":{}}"#).unwrap();
        assert_eq!(
            parse_submit(&v).unwrap_err().kind,
            crate::image_ai::types::GenErrorKind::Provider
        );
    }

    #[test]
    fn parse_task_maps_states_and_reads_failure_from_both_levels() {
        let p = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert!(matches!(
            parse_task(&p(r#"{"output":{"task_status":"PENDING"}}"#)),
            TaskState::Pending
        ));
        assert!(matches!(
            parse_task(&p(r#"{"output":{"task_status":"RUNNING"}}"#)),
            TaskState::Running
        ));
        // 成功结果仍在 output.choices[].message.content[].image（新协议，不是 output.results）。
        match parse_task(&p(
            r#"{"output":{"task_status":"SUCCEEDED","choices":[{"message":{"content":[{"image":"https://r/a.png"},{"image":"https://r/b.png"}]}}]}}"#,
        )) {
            TaskState::Succeeded(urls) => {
                assert_eq!(urls, vec!["https://r/a.png", "https://r/b.png"])
            }
            _ => panic!("应为 Succeeded"),
        }
        // 任务级失败看 output.code：DataInspectionFailed → ContentRejected。
        match parse_task(&p(
            r#"{"output":{"task_status":"FAILED","code":"DataInspectionFailed","message":"内容不合规"}}"#,
        )) {
            TaskState::Failed(e) => {
                assert_eq!(e.kind, crate::image_ai::types::GenErrorKind::ContentRejected);
                assert_eq!(e.message, "内容不合规");
            }
            _ => panic!("应为 Failed"),
        }
        // 官方「响应参数」把 code/message 列在顶层：只有顶层时同样要认出业务码，
        // 否则内容审核类错误会退化成无因由、无处置建议的 Provider。
        match parse_task(&p(
            r#"{"code":"IPInfringementSuspect","message":"疑似侵权","output":{"task_status":"FAILED"}}"#,
        )) {
            TaskState::Failed(e) => {
                assert_eq!(e.kind, crate::image_ai::types::GenErrorKind::ContentRejected);
                assert_eq!(e.message, "疑似侵权");
            }
            _ => panic!("应为 Failed"),
        }
        // 两处都有时 output 级优先（它才是任务级失败的位置，顶层可能只是请求级残留）。
        match parse_task(&p(
            r#"{"code":"Throttling","message":"顶层","output":{"task_status":"FAILED","code":"DataInspectionFailed","message":"任务级"}}"#,
        )) {
            TaskState::Failed(e) => {
                assert_eq!(e.kind, crate::image_ai::types::GenErrorKind::ContentRejected);
                assert_eq!(e.message, "任务级");
            }
            _ => panic!("应为 Failed"),
        }
        // 服务端取消 ≠ 本地取消：归 Provider（可重试），绝不归 Cancelled。
        for s in [
            r#"{"output":{"task_status":"CANCELED"}}"#,
            r#"{"output":{"task_status":"UNKNOWN"}}"#,
            r#"{"output":{}}"#,
        ] {
            match parse_task(&p(s)) {
                TaskState::Failed(e) => {
                    assert_eq!(e.kind, crate::image_ai::types::GenErrorKind::Provider, "{s}")
                }
                _ => panic!("应为 Failed: {s}"),
            }
        }
    }

    #[test]
    fn poll_delay_grows_and_caps_at_15s() {
        assert_eq!(poll_delay(0), Duration::from_millis(3000));
        // 1.5× 契约：把倍率与封顶拐点钉死（因子改成 2.0 或封顶阈值改动都会红）。
        assert_eq!(poll_delay(1), Duration::from_millis(4500));
        assert_eq!(poll_delay(2), Duration::from_millis(6750));
        assert_eq!(poll_delay(3), Duration::from_millis(10125));
        assert_eq!(poll_delay(4), Duration::from_millis(15000)); // 15187.5 → 触顶
                                                                 // 单调不减。
        for n in 0..12u32 {
            assert!(poll_delay(n + 1) >= poll_delay(n), "attempt {n} 应单调递增");
        }
        // 单次上限 15s：拐点之后（无论多靠后）都保持封顶。
        assert_eq!(poll_delay(100), Duration::from_millis(15000));
    }

    #[test]
    fn use_async_reads_call_mode() {
        let mut r = gen_req("qwen-image-3.0");
        assert!(!use_async(&r), "缺省 None → 同步");
        r.call_mode = Some("sync".into());
        assert!(!use_async(&r), "非 async 一律同步");
        r.call_mode = Some("async".into());
        assert!(use_async(&r));
    }
}
