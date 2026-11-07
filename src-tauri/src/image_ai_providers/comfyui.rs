//! 本地 ComfyUI 适配器 + 内置占位目录（原生节点工作流，无鉴权）。
//!
//! 【为什么单列一档】ComfyUI 是用户本机运行的 Python 服务（默认 `127.0.0.1:8188`），
//! 无 API Key、无厂商配额；协议不是「提交参数」而是「提交一张工作流图」——
//! `POST /prompt` 的 body 是 API 格式 workflow JSON，节点名即协议契约。
//!
//! 【工作流模板】只用官方原生节点（CheckpointLoaderSimple / CLIPTextEncode×2 /
//! EmptyLatentImage / KSampler / VAEDecode / SaveImage），不依赖任何自定义节点，
//! 因此不需要「模板文件 + 占位符替换」机制：请求体由本模块从 GenRequest 直接组装。
//! 采样器固定 euler/normal、denoise=1.0（文生图恒定），刻意不进能力编辑面。
//!
//! 【模型语义】`model` = ComfyUI checkpoints 目录下的 checkpoint 文件名（含扩展名），
//! 经 `CheckpointLoaderSimple.ckpt_name` 下发。内置目录只放一个占位行
//! （ComfyUI 整合包的经典示例 checkpoint），用户必须按本机实际文件修改。
//!
//! 【无鉴权】不发送 Authorization 头；`generate` 的 key 形参被刻意忽略
//! （调用方 commands.rs 对本协议免 Key 检查，见 `ProviderKind::needs_key`）。
//!
//! 【流程】提交 → 轮询 `GET /history/{prompt_id}`（history 只含**已完结**执行，
//! 查无此条 = 仍在排队/运行）→ 从 outputs 各节点的 `images[]` 取文件名 →
//! `GET /view` 逐张下载。本适配器无跨 await 持有的状态，`ComfyUIProvider` 为无状态
//! 单元结构体——「资源释放」即无事可做，连接由共用 client 托管。
//!
//! 【seed 语义】KSampler 的 seed 是 u64；`GenRequest.seed` 是 i64，负值由共用解释器
//! （`validate_model_caps`）统一拦下，未填时本地随机取值（官方口径：不指定则随机）。

use super::{validate_model_caps, FetchedImage, GenStage, ImageProvider, ProviderOutcome};
use crate::image_ai::types::{
    from_http_status, Capabilities, GenRequest, ImageGenError, ModelInfo, ProviderConfig,
    ProviderKind,
};
use rand::Rng; // 随机 seed：rand::rng().random::<u64>()（显式导入 trait）
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// 内置本地 ComfyUI 服务商的固定 ID（配置合并按此 ID 认领；与 kind 字符串一致）。
pub(crate) const PROVIDER_ID: &str = "localcomfyui";

/// 本机默认基址；用户自建/远程 ComfyUI 在服务商设置里改 base_url 即可。
const DEFAULT_BASE: &str = "http://127.0.0.1:8188";

/// 占位 checkpoint（ComfyUI 整合包经典示例文件名）：仅让内置目录可选中，
/// 实际值必须按用户本机 checkpoints 目录修改。
const DEFAULT_MODEL: &str = "v1-5-pruned-emaonly.safetensors";

/// 轮询节奏（本地）：初始 1s、每次 ×1.5、单次上限 5s。
const POLL_INITIAL_MS: u64 = 1000;
const POLL_FACTOR: f64 = 1.5;
const POLL_MAX_MS: u64 = 5000;
/// 总预算 600s：本地出图在低显存/大模型下可达数分钟，与共用 HTTP client 的
/// 600s 单请求超时对齐（见 image_ai::generate 的 client 构造）。
const POLL_TOTAL_BUDGET: Duration = Duration::from_secs(600);

/// SDXL 常用分辨率档（1024 基准）；ComfyUI 对尺寸无硬性约束，自定义值不臆造边界（交服务端）。
const SDXL_SIZES: &[&str] = &[
    "1024x1024",
    "1152x896",
    "896x1152",
    "1216x832",
    "832x1216",
    "1344x768",
    "768x1344",
];

// ===================== 内置模型目录 ==================================================================================

/// 本地 ComfyUI 内置服务商：模型目录代码托管（不读 assets/image_ai.json）。
pub(crate) fn builtin_provider() -> ProviderConfig {
    ProviderConfig {
        id: PROVIDER_ID.into(),
        kind: ProviderKind::LocalComfyUI,
        name: "本地 ComfyUI".into(),
        base_url: DEFAULT_BASE.into(),
        default_model: DEFAULT_MODEL.into(),
        models: vec![ModelInfo {
            model: DEFAULT_MODEL.into(),
            capabilities: comfyui_caps(),
        }],
        api_key: None,
    }
}

/// ComfyUI 能力集：张数走 `EmptyLatentImage.batch_size`（1–4）；种子/负向词/步数/引导
/// 分别对应 KSampler 的 seed/steps/cfg。输出格式由 checkpoint/工作流决定，无 output_format 参数；
/// 提示词上限官方未给硬值（CLIP 编码器自身截断），不拦截。
fn comfyui_caps() -> Capabilities {
    Capabilities {
        sizes: SDXL_SIZES.iter().map(|s| (*s).to_string()).collect(),
        max_n: 4,
        // Capabilities::default() 给 bool 的是 false；代码构造必须显式声明。
        supports_n: true,
        supports_seed: true,
        supports_negative_prompt: true,
        supports_steps: true,
        steps_default: 20,
        steps_max: 100,
        supports_guidance: true,
        guidance_default: 7.0,
        guidance_max: 20.0,
        ..Default::default()
    }
}

// ===================== URL 与工作流（纯函数） ========================================================================

/// 工作流提交端点。
pub(crate) fn prompt_url(base: &str) -> String {
    format!("{}/prompt", base.trim_end_matches('/'))
}

/// 任务查询端点：history 以 prompt_id 为键、只含已完结执行。
pub(crate) fn history_url(base: &str, prompt_id: &str) -> String {
    format!("{}/history/{prompt_id}", base.trim_end_matches('/'))
}

/// 解析 宽×高（小写 `x`）/「宽*高」两种写法；EmptyLatentImage 需要独立的 width/height，提交前必须拆开。
pub(crate) fn parse_size(size: &str) -> Option<(u32, u32)> {
    let v: Vec<u32> = size
        .split(['x', '*'])
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .collect();
    match v.as_slice() {
        [w, h] if *w > 0 && *h > 0 => Some((*w, *h)),
        _ => None,
    }
}

/// 组装 API 格式文生图工作流（原生节点，节点编号任意但连线必须自洽）：
/// - `n` → `EmptyLatentImage.batch_size`（能力位门控，不支持时固定 1）；
/// - seed：用户填了用用户的（非负 i64），否则本地随机；步数/引导未填回落能力集默认值；
/// - 负向词允许空串（CLIPTextEncode 接受空文本）。
pub(crate) fn build_workflow(req: &GenRequest) -> Value {
    use serde_json::json;
    let caps = req.caps();
    let (w, h) = parse_size(&req.size).unwrap_or((1024, 1024)); // validate 已拦，兜底不触达
    let steps = req
        .steps
        .unwrap_or(if caps.steps_default > 0 { caps.steps_default } else { 20 });
    let cfg_v = req
        .guidance
        .unwrap_or(if caps.guidance_default > 0.0 { caps.guidance_default } else { 7.0 });
    let seed = match req.seed {
        Some(s) if s >= 0 => s as u64,
        _ => rand::rng().random::<u64>(),
    };
    let n = if caps.supports_n { req.n.max(1) } else { 1 };
    json!({
        "prompt": {
            "1": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": req.model}},
            "2": {"class_type": "CLIPTextEncode", "inputs": {"text": req.prompt, "clip": ["1", 1]}},
            "3": {"class_type": "CLIPTextEncode", "inputs": {"text": req.negative_prompt, "clip": ["1", 1]}},
            "4": {"class_type": "EmptyLatentImage", "inputs": {"width": w, "height": h, "batch_size": n}},
            "5": {"class_type": "KSampler", "inputs": {
                "seed": seed, "steps": steps, "cfg": cfg_v,
                "sampler_name": "euler", "scheduler": "normal", "denoise": 1.0,
                "model": ["1", 0], "positive": ["2", 0], "negative": ["3", 0], "latent_image": ["4", 0]}},
            "6": {"class_type": "VAEDecode", "inputs": {"samples": ["5", 0], "vae": ["1", 2]}},
            "7": {"class_type": "SaveImage", "inputs": {"filename_prefix": "newgan", "images": ["6", 0]}}
        }
    })
}

/// 解析提交响应：成功取 `prompt_id`。
pub(crate) fn parse_submit(resp: &Value) -> Result<String, ImageGenError> {
    resp.get("prompt_id")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ImageGenError::provider("响应缺少 prompt_id", false))
}

/// 提取 `error.message` 形态的 ComfyUI 错误描述（400 校验失败时返回）。
fn error_message(json: &Value) -> Option<String> {
    let err = json.get("error")?;
    let msg = err
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("未知错误");
    Some(match err.get("type").and_then(|t| t.as_str()) {
        Some(t) => format!("{t}: {msg}"),
        None => msg.to_string(),
    })
}

/// 一张待下载的结果图（`/view` 的查询参数三元组）。
pub(crate) struct OutputImage {
    /// 文件名（`/view` 的 filename 参数）
    pub(crate) filename: String,
    /// 子目录（空串 = 根目录）
    pub(crate) subfolder: String,
    /// 输出类型（output / temp）
    pub(crate) kind: String,
}

/// 任务状态三态。
pub(crate) enum TaskState {
    /// 仍在排队/运行（history 查无此条，或已见条目但还没产出图片）。
    Pending,
    /// 已完结并取到结果图。
    Done(Vec<OutputImage>),
    /// 失败（执行报错，或已完结但无图）。
    Failed(ImageGenError),
}

/// 解析 history 响应：entries 以 prompt_id 为键且只含已完结执行。
/// - 查无此条 → Pending（任务尚未跑完）；
/// - `status.status_str == "error"` → 失败（细节在 ComfyUI 控制台，HTTP 层拿不到）；
/// - outputs 里任意节点的 `images[]` 非空 → Done；
/// - 已完结但没有图片 → 失败。
pub(crate) fn parse_history(resp: &Value, prompt_id: &str) -> TaskState {
    let Some(entry) = resp.get(prompt_id) else {
        return TaskState::Pending;
    };
    let status_str = entry
        .get("status")
        .and_then(|s| s.get("status_str"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if status_str == "error" {
        return TaskState::Failed(ImageGenError::provider(
            "ComfyUI 执行出错（请查看 ComfyUI 控制台日志）",
            false,
        ));
    }
    let mut imgs = Vec::new();
    if let Some(outputs) = entry.get("outputs").and_then(|o| o.as_object()) {
        for out in outputs.values() {
            if let Some(list) = out.get("images").and_then(|v| v.as_array()) {
                for img in list {
                    if let Some(f) = img.get("filename").and_then(|v| v.as_str()) {
                        imgs.push(OutputImage {
                            filename: f.to_string(),
                            subfolder: img
                                .get("subfolder")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            kind: img
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("output")
                                .to_string(),
                        });
                    }
                }
            }
        }
    }
    let completed = entry
        .get("status")
        .and_then(|s| s.get("completed"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if imgs.is_empty() {
        if completed || status_str == "success" {
            TaskState::Failed(ImageGenError::provider("任务完成但没有返回图片", false))
        } else {
            TaskState::Pending
        }
    } else {
        TaskState::Done(imgs)
    }
}

/// 轮询退避：初始 1s，每次 ×1.5，单次上限 5s。
pub(crate) fn poll_delay(attempt: u32) -> Duration {
    let ms = (POLL_INITIAL_MS as f64) * POLL_FACTOR.powi(attempt as i32);
    Duration::from_millis(ms.min(POLL_MAX_MS as f64) as u64)
}

/// 结果文件名 → 落盘后缀（SaveImage 产出扩展名随输出格式变化，通常 png）。
fn image_extension(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "png".into())
}

/// ComfyUI 适配器（无鉴权、无状态；提交原生节点工作流并轮询 history）。
pub struct ComfyUIProvider;

impl ImageProvider for ComfyUIProvider {
    fn generate<'a>(
        &'a self,
        client: &'a reqwest::Client,
        _key: &'a str, // 本地 ComfyUI 无鉴权：key 恒为空串，刻意不用
        req: &'a GenRequest,
        progress: &'a (dyn Fn(GenStage) + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, ImageGenError>> + Send + 'a>> {
        Box::pin(async move {
            progress(GenStage::Submitting);
            let base = req.base_url.trim().trim_end_matches('/').to_string();
            let resp = client
                .post(prompt_url(&base))
                .json(&build_workflow(req))
                .send()
                .await
                .map_err(|e| ImageGenError::network(format!("请求发送失败: {e}")))?;
            let status = resp.status();
            let json: Value = resp
                .json()
                .await
                .map_err(|e| ImageGenError::provider(format!("HTTP {status}：响应解析失败: {e}"), false))?;
            let prompt_id = if status.is_success() {
                parse_submit(&json)?
            } else {
                // 400 = 工作流校验失败（缺 checkpoint/节点参数错），InvalidParam 提示改配置。
                let msg = error_message(&json).unwrap_or_else(|| "工作流提交失败".into());
                return Err(from_http_status(
                    status.as_u16(),
                    format!("HTTP {status}：{msg}"),
                ));
            };

            progress(GenStage::Polling);
            let started = Instant::now();
            let mut attempt = 0u32;
            loop {
                // 退避取「本轮退避值」与「剩余预算」的较小者，与 dashscope/modelscope 同款。
                let remaining = POLL_TOTAL_BUDGET.saturating_sub(started.elapsed());
                let delay = poll_delay(attempt).min(remaining);
                if !delay.is_zero() {
                    tauri::async_runtime::spawn_blocking(move || std::thread::sleep(delay))
                        .await
                        .ok();
                }
                attempt += 1;
                if started.elapsed() >= POLL_TOTAL_BUDGET {
                    return Err(ImageGenError::provider(
                        format!("本地生图超时（{}s）", POLL_TOTAL_BUDGET.as_secs()),
                        true,
                    ));
                }

                let resp = client
                    .get(history_url(&base, &prompt_id))
                    .send()
                    .await
                    .map_err(|e| ImageGenError::network(format!("任务查询失败: {e}")))?;
                if !resp.status().is_success() {
                    return Err(from_http_status(
                        resp.status().as_u16(),
                        format!("HTTP {}：任务查询失败", resp.status()),
                    ));
                }
                let json: Value = resp
                    .json()
                    .await
                    .map_err(|e| ImageGenError::provider(format!("HTTP {status}：响应解析失败: {e}"), false))?;
                match parse_history(&json, &prompt_id) {
                    TaskState::Pending => continue,
                    TaskState::Failed(e) => return Err(e),
                    TaskState::Done(images) => {
                        progress(GenStage::Downloading);
                        let mut out = Vec::with_capacity(images.len());
                        for img in images {
                            // 文件名/子目录可能含空格，走 query 编码而不是手拼 URL。
                            let bytes = client
                                .get(format!("{}/view", base))
                                .query(&[
                                    ("filename", img.filename.as_str()),
                                    ("subfolder", img.subfolder.as_str()),
                                    ("type", img.kind.as_str()),
                                ])
                                .send()
                                .await
                                .map_err(|e| ImageGenError::network(format!("图片下载失败: {e}")))?
                                .error_for_status()
                                .map_err(|e| ImageGenError::network(format!("图片下载失败: {e}")))?
                                .bytes()
                                .await
                                .map_err(|e| ImageGenError::network(format!("图片读取失败: {e}")))?
                                .to_vec();
                            out.push(FetchedImage {
                                bytes,
                                extension: image_extension(&img.filename),
                            });
                        }
                        return Ok(ProviderOutcome { images: out });
                    }
                }
            }
        })
    }

    /// 本协议特有门控：服务地址必填（本地默认址会随内置服务商注入，空值只出现在用户清空后）；
    /// 尺寸必须能拆成 width/height（EmptyLatentImage 契约）。seed 非负由共用解释器统一把关
    /// （KSampler 是 u64，见 validate_model_caps）。
    fn validate(&self, req: &GenRequest, caps: &Capabilities) -> Result<(), ImageGenError> {
        if req.base_url.trim().is_empty() {
            return Err(ImageGenError::not_configured(
                "请先在「服务商设置」中填写 ComfyUI 服务地址（如 http://127.0.0.1:8188）",
            ));
        }
        if parse_size(&req.size).is_none() {
            return Err(ImageGenError::invalid_param(format!(
                "尺寸「{}」格式不正确，应为 宽×高 或 宽*高（如 1024x1024）",
                req.size
            )));
        }
        validate_model_caps(req, caps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen_req() -> GenRequest {
        GenRequest {
            provider_id: PROVIDER_ID.into(),
            base_url: DEFAULT_BASE.into(),
            model: DEFAULT_MODEL.into(),
            prompt: "一张东亚球员的脸".into(),
            size: "1024x1024".into(),
            n: 2,
            caps: Some(comfyui_caps()),
            ..crate::image_ai::types::test_req_base()
        }
    }

    // ===================== 内置目录 ==================================================================================

    #[test]
    fn catalog_is_single_placeholder_model() {
        let p = builtin_provider();
        assert_eq!(p.id, PROVIDER_ID);
        assert_eq!(p.kind, ProviderKind::LocalComfyUI);
        assert!(!p.kind.needs_key(), "本地 ComfyUI 无鉴权");
        assert_eq!(p.base_url, DEFAULT_BASE);
        let ids: Vec<&str> = p.models.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(ids, vec![DEFAULT_MODEL]);
    }

    #[test]
    fn caps_map_to_ksampler_inputs() {
        let c = comfyui_caps();
        assert_eq!(c.max_n, 4);
        assert!(c.supports_n, "张数走 batch_size");
        assert!(c.supports_seed && c.supports_negative_prompt);
        assert_eq!((c.steps_default, c.steps_max), (20, 100));
        assert_eq!((c.guidance_default, c.guidance_max), (7.0, 20.0));
        assert!(!c.supports_output_format, "无 output_format 参数");
        assert!(!c.supports_async, "异步轮询是适配器内部细节，不走 call_mode");
        assert_eq!(c.max_prompt_chars, 0);
        assert!(c.size_rules.is_none(), "不臆造尺寸边界");
    }

    // ===================== URL / 尺寸 / 退避 =========================================================================

    #[test]
    fn urls_trim_trailing_slash() {
        assert_eq!(prompt_url("http://127.0.0.1:8188/"), "http://127.0.0.1:8188/prompt");
        assert_eq!(
            history_url("http://127.0.0.1:8188/", "p-1"),
            "http://127.0.0.1:8188/history/p-1"
        );
    }

    #[test]
    fn parse_size_accepts_x_and_star() {
        assert_eq!(parse_size("1024x1024"), Some((1024, 1024)));
        assert_eq!(parse_size("896*1152"), Some((896, 1152)));
        assert_eq!(parse_size("0x100"), None);
        assert_eq!(parse_size("1024"), None);
        assert_eq!(parse_size("abc"), None);
    }

    #[test]
    fn poll_delay_grows_and_caps_at_5s() {
        assert_eq!(poll_delay(0), Duration::from_millis(1000));
        assert_eq!(poll_delay(1), Duration::from_millis(1500));
        assert_eq!(poll_delay(3), Duration::from_millis(3375));
        assert_eq!(poll_delay(4), Duration::from_millis(5000)); // 5062.5 → 触顶
        assert_eq!(poll_delay(10), Duration::from_millis(5000));
    }

    // ===================== 工作流组装 ================================================================================

    #[test]
    fn workflow_carries_prompt_negative_size_and_n() {
        let mut r = gen_req();
        r.negative_prompt = "模糊".into();
        r.size = "896x1152".into();
        let b = build_workflow(&r);
        let p = &b["prompt"];
        assert_eq!(p["1"]["class_type"], "CheckpointLoaderSimple");
        assert_eq!(p["1"]["inputs"]["ckpt_name"], DEFAULT_MODEL);
        assert_eq!(p["2"]["inputs"]["text"], "一张东亚球员的脸");
        assert_eq!(p["3"]["inputs"]["text"], "模糊");
        assert_eq!(p["4"]["inputs"]["width"], 896);
        assert_eq!(p["4"]["inputs"]["height"], 1152);
        assert_eq!(p["4"]["inputs"]["batch_size"], 2);
        // KSampler 连线指向前置节点。
        assert_eq!(p["5"]["inputs"]["model"], serde_json::json!(["1", 0]));
        assert_eq!(p["5"]["inputs"]["positive"], serde_json::json!(["2", 0]));
        assert_eq!(p["7"]["inputs"]["images"], serde_json::json!(["6", 0]));
    }

    #[test]
    fn workflow_defaults_and_overrides_for_seed_steps_cfg() {
        // 未填：随机 seed（合法 u64）、步数/引导回落能力集默认 20 / 7.0。
        let b = build_workflow(&gen_req());
        assert!(b["prompt"]["5"]["inputs"]["seed"].is_u64(), "随机 seed 必须落在 u64 域");
        assert_eq!(b["prompt"]["5"]["inputs"]["steps"], 20);
        assert_eq!(b["prompt"]["5"]["inputs"]["cfg"], 7.0);
        // 用户显式值必须如实下发。
        let mut r = gen_req();
        r.seed = Some(42);
        r.steps = Some(30);
        r.guidance = Some(6.5);
        let b = build_workflow(&r);
        assert_eq!(b["prompt"]["5"]["inputs"]["seed"], 42);
        assert_eq!(b["prompt"]["5"]["inputs"]["steps"], 30);
        assert_eq!(b["prompt"]["5"]["inputs"]["cfg"], 6.5);
    }

    // ===================== 提交与轮询解析 ============================================================================

    #[test]
    fn parse_submit_reads_prompt_id() {
        let v: Value = serde_json::from_str(r#"{"prompt_id":"p-1","number":1}"#).unwrap();
        assert_eq!(parse_submit(&v).unwrap(), "p-1");
        let v: Value = serde_json::from_str(r#"{"error":{"type":"invalid_prompt"}}"#).unwrap();
        assert_eq!(parse_submit(&v).unwrap_err().message, "响应缺少 prompt_id");
    }

    #[test]
    fn parse_history_missing_entry_is_pending() {
        // history 只含已完结执行：查无此条 = 仍在跑。
        let v: Value = serde_json::from_str(r#"{}"#).unwrap();
        assert!(matches!(parse_history(&v, "p-1"), TaskState::Pending));
    }

    #[test]
    fn parse_history_reads_outputs_across_nodes() {
        let v: Value = serde_json::from_str(
            r#"{"p-1":{"status":{"status_str":"success","completed":true},
                "outputs":{"7":{"images":[{"filename":"ComfyUI_00001_.png","subfolder":"","type":"output"}]},
                          "9":{"images":[{"filename":"x.png","subfolder":"s","type":"temp"}]}}}}"#,
        )
        .unwrap();
        match parse_history(&v, "p-1") {
            TaskState::Done(imgs) => {
                assert_eq!(imgs.len(), 2);
                assert_eq!(imgs[0].filename, "ComfyUI_00001_.png");
                assert_eq!(imgs[1].subfolder, "s");
                assert_eq!(imgs[1].kind, "temp");
            }
            _ => panic!("应为 Done"),
        }
    }

    #[test]
    fn parse_history_error_and_completed_without_images() {
        let v: Value = serde_json::from_str(
            r#"{"p-1":{"status":{"status_str":"error","completed":false},"outputs":{}}}"#,
        )
        .unwrap();
        match parse_history(&v, "p-1") {
            TaskState::Failed(e) => assert!(e.message.contains("控制台")),
            _ => panic!("应为 Failed"),
        }
        // 已完结但无图：不能静默当 Pending（会白等到超时）。
        let v: Value = serde_json::from_str(
            r#"{"p-1":{"status":{"status_str":"success","completed":true},"outputs":{}}}"#,
        )
        .unwrap();
        assert!(matches!(parse_history(&v, "p-1"), TaskState::Failed(_)));
    }

    #[test]
    fn image_extension_follows_filename() {
        assert_eq!(image_extension("ComfyUI_00001_.png"), "png");
        assert_eq!(image_extension("out.JPEG"), "jpeg");
        assert_eq!(image_extension("noext"), "png");
    }

    // ===================== 校验 ======================================================================================

    #[test]
    fn validate_requires_base_url_parses_size_and_nonnegative_seed() {
        let caps = comfyui_caps();
        let mut r = gen_req();
        r.base_url = String::new();
        let err = ComfyUIProvider.validate(&r, &caps).unwrap_err();
        assert_eq!(err.kind, crate::image_ai::types::GenErrorKind::NotConfigured);

        let mut r = gen_req();
        r.size = "1024".into();
        let err = ComfyUIProvider.validate(&r, &caps).unwrap_err();
        assert_eq!(err.kind, crate::image_ai::types::GenErrorKind::InvalidParam);

        let mut r = gen_req();
        r.seed = Some(-1);
        let err = ComfyUIProvider.validate(&r, &caps).unwrap_err();
        assert_eq!(err.kind, crate::image_ai::types::GenErrorKind::InvalidParam);

        assert!(ComfyUIProvider.validate(&gen_req(), &caps).is_ok());
    }

    #[test]
    fn validate_rejects_n_and_steps_over_max() {
        let caps = comfyui_caps();
        let mut r = gen_req();
        r.n = 5;
        assert!(validate_model_caps(&r, &caps).is_err(), "batch_size 上限 4");
        r.n = 4;
        r.steps = Some(101);
        assert!(validate_model_caps(&r, &caps).is_err(), "步数上限 100");
    }
}
