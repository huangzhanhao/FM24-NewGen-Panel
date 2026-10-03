//! 阿里云百炼（DashScope）适配器 + 三系列内置模型目录。
//!
//! 【三系列，一条端点】千问 Qwen-Image（2.0/3.0）、万相 Wan（2.6/2.7）、Z-Image（turbo）
//! 在百炼平台共享同一把 API Key、同一个基址与同一套 `model/input/parameters` 请求壳，
//! 文生图统一走**同步多模态消息式**接口（逐字对齐官方 HTTP 模板）：
//!
//! `POST {base}/services/aigc/multimodal-generation/generation`
//! 请求体：`{"model","input":{"messages":[{"role":"user","content":[{"text":…}]}]},
//!          "parameters":{…}}`
//! 响应：`output.choices[].message.content[].image`（结果 URL，24h 有效，立即下载）。
//!
//! 本项目只实现各系列**最新代/次新代**模型（2026-09 官方文档口径）：
//! - 千问：`qwen-image-3.0-pro/3.0`、`qwen-image-2.0-pro/2.0`；
//! - 万相：`wan2.7-image-pro/2.7-image`、`wan2.6-t2i`；
//! - Z-Image：`z-image-turbo`。
//! 更早的 wan2.5/2.2/wanx2.1、qwen-image-max/plus/初代均已下线不接入——它们走的旧版
//! `text2image/image-synthesis` 异步任务端点因此整体删除，适配器只保留同步一条路。
//!
//! 【系列参数差异】（由内置模型目录的能力集门控，能力不声明就不发字段）：
//! | 参数 | qwen-image 3.0 | qwen-image 2.0 | wan2.7 | wan2.6 | z-image |
//! |---|---|---|---|---|---|
//! | size | `宽*高` 像素 | `宽*高` 像素 | `1K/2K/4K` 档位 | `宽*高` 像素 | `宽*高` 像素 |
//! | n | 1–6 | 1–6 | 1–4 | 1–4 | **无此参数**（固定 1） |
//! | negative_prompt | ✓ | ✓ | ✗（在提示词里写排除项） | ✓ | ✗ |
//! | prompt_extend | ✓ 默认开 | ✓ 默认开 | ✗（思考开关官方名为 thinking_mode，本项目不暴露） | ✓ 默认开 | ✓ **默认关**（开启即思考模式，单价翻倍） |
//! | prompt_extend_mode | ✓ direct/agent | ✗ | ✗ | ✗ | ✗ |
//! | enable_thinking | ✓ 默认 true（仅改写开启时生效） | ✗ | ✗ | ✗ | ✗ |
//! | watermark | ✓ | ✓ | ✓ | ✓ | ✗ |
//! | seed | ✓ | ✓ | ✓ | ✓ | ✓ |
//!
//! 尺寸约束一律用「单边 / 总像素 / 宽高比」三条表达（0 = 不检查），官方未给的项**留空**：
//! z-image 官方只写总像素区间，其单边 512–2048 是同区间推出的口径，比例留空。
//!
//! 模型目录代码内置（[`builtin_provider`]），不再写在 assets/image_ai.json：
//! 新增/调整模型随版本发布即对全部用户生效，旧用户 image_ai.json 里的 dashscope
//! 服务商仅保留其可编辑项（显示名 / base_url），模型行与默认模型由代码接管迁移。
//!
//! 协议全部拆为纯函数（体组装 / 解析），单测不触网。

use super::{FetchedImage, ImageProvider, ProviderOutcome};
use crate::image_ai::{
    Capabilities, GenRequest, ModelInfo, ProviderConfig, ProviderKind, SizeRules,
};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

/// 内置百炼服务商的固定 ID（配置合并/迁移按此 ID 认领）。
pub(crate) const PROVIDER_ID: &str = "dashscope";

/// 阿里云直连默认基址（国内）。服务商配置了 base_url（千问 AI 平台等代理）时以配置为准。
const DEFAULT_BASE: &str = "https://dashscope.aliyuncs.com/api/v1";

/// 默认模型（Z-Image Turbo：极速低成本，适合批量出图）。
const DEFAULT_MODEL: &str = "z-image-turbo";

// ===================== 内置模型目录 =====================

/// 百炼内置服务商：模型目录代码托管（不读 JSON）。
pub(crate) fn builtin_provider() -> ProviderConfig {
    ProviderConfig {
        id: PROVIDER_ID.into(),
        kind: ProviderKind::Dashscope,
        name: "阿里云百炼（千问/万相/Z-Image）".into(),
        base_url: DEFAULT_BASE.into(),
        default_model: DEFAULT_MODEL.into(),
        models: vec![
            // ---------- 千问 Qwen-Image ----------
            ModelInfo {
                model: "qwen-image-3.0-pro".into(),
                capabilities: qwen_caps(6, 4500, true, true),
            },
            ModelInfo {
                model: "qwen-image-3.0".into(),
                capabilities: qwen_caps(6, 4500, true, true),
            },
            ModelInfo {
                model: "qwen-image-2.0-pro".into(),
                // 2.0 系列官方未公布宽高比区间，只约束总像素，故不设 ratio 门限。
                capabilities: qwen_caps(6, 1300, false, false),
            },
            ModelInfo {
                model: "qwen-image-2.0".into(),
                capabilities: qwen_caps(6, 1300, false, false),
            },
            // ---------- 万相 Wan ----------
            ModelInfo {
                model: "wan2.7-image-pro".into(),
                capabilities: wan27_caps(&["1K", "2K", "4K"]),
            },
            ModelInfo {
                model: "wan2.7-image".into(),
                capabilities: wan27_caps(&["1K", "2K"]),
            },
            ModelInfo {
                model: "wan2.6-t2i".into(),
                capabilities: wan26_caps(),
            },
            // ---------- Z-Image ----------
            ModelInfo {
                model: "z-image-turbo".into(),
                capabilities: zimage_caps(),
            },
        ],
    }
}

/// 千问 2.0/3.0 能力基线：
/// - 官方推荐分辨率五档（总像素均落在 2048² 内）；
/// - 总像素 [512², 2048²]；3.0 另有宽高比 [1:8, 8:1]（2.0 无此约束）；
/// - n≤6；负向词/改写（默认开）/水印/seed 全开。
/// - `gen3`：改写模式（`prompt_extend_mode`）与思考模式（`enable_thinking`）是 3.0 独有的
///   协议参数，2.0 文档里没有，故只在这一代置位。
/// `max_prompt_chars`：3.0 官方「推荐不超过 4500 Token」、2.0 上限 1300 Token，
/// 前端按字符计数（中文约 1 字 ≈ 1–2 Token），取值宽松只拦明显超长。
fn qwen_caps(max_n: u32, max_prompt_chars: u32, ratio_checked: bool, gen3: bool) -> Capabilities {
    Capabilities {
        sizes: vec![
            "2048*2048".into(),
            "2688*1536".into(),
            "1536*2688".into(),
            "2368*1728".into(),
            "1728*2368".into(),
        ],
        max_n,
        // 注：Capabilities::default() 给 bool 的是 false；代码构造必须显式声明，
        // serde(default="yes") 只对 JSON 缺字段反序列化生效。
        supports_n: true,
        supports_seed: true,
        supports_negative_prompt: true,
        supports_prompt_extend: true,
        prompt_extend_default: true,
        supports_prompt_extend_mode: gen3,
        prompt_extend_mode_default: "direct".into(),
        supports_enable_thinking: gen3,
        enable_thinking_default: true,
        supports_watermark: true,
        max_prompt_chars,
        size_rules: Some(SizeRules {
            total_min: 512 * 512,
            total_max: 2048 * 2048,
            ratio_min: if ratio_checked { 0.125 } else { 0.0 },
            ratio_max: if ratio_checked { 8.0 } else { 0.0 },
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 万相 2.7 能力：size 为 `1K/2K/4K` 分辨率档位（非像素，故无 sizeRules）、n≤4；
/// 官方不支持 negative_prompt 与 prompt_extend（排除项写进提示词；思考模式是另一个
/// 参数 thinking_mode，本项目不暴露，走官方默认开）；水印/seed 支持。
fn wan27_caps(sizes: &[&str]) -> Capabilities {
    Capabilities {
        sizes: sizes.iter().map(|s| (*s).to_string()).collect(),
        max_n: 4,
        supports_n: true,
        supports_seed: true,
        supports_watermark: true,
        max_prompt_chars: 5000,
        ..Default::default()
    }
}

/// 万相 2.6 能力（与上一版出厂配置一致）：总像素 [1280², 1440²]、宽高比 [1:4, 4:1]；
/// 负向词/改写（默认开）/水印/seed 全开，n≤4。
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
fn zimage_caps() -> Capabilities {
    Capabilities {
        sizes: vec![
            "1024*1024".into(),
            "1248*832".into(),
            "832*1248".into(),
            "1152*864".into(),
            "864*1152".into(),
            "1280*720".into(),
            "720*1280".into(),
        ],
        max_n: 1,
        supports_n: false,
        supports_seed: true,
        supports_prompt_extend: true,
        prompt_extend_default: false,
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

/// `prompt_extend` 缺省（用户未显式选择）时的官方默认：仅 z-image 为关，其余为开。
/// 未知模型（代理平台的新模型）按开处理，与历史行为一致。
pub(crate) fn prompt_extend_default(model: &str) -> bool {
    !model.starts_with("z-image")
}

// ===================== URL 与请求体（纯函数）=====================

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

/// 尺寸显示用的 'x' 统一转为厂商文档口径的 '*'（1024x1024 → 1024*1024）。
/// 万相 2.7 的分辨率档位（"1K/2K/4K"）不含分隔符，原样透传。
fn vendor_size(size: &str) -> String {
    size.trim().replace('x', "*")
}

/// 组装 `parameters` 对象——按模型能力集门控，能力未声明就不带字段，
/// 避免给不支持的模型（如 z-image 收 n、wan2.7 收 negative_prompt）塞陌生参数触发 400。
fn build_params(req: &GenRequest) -> Value {
    use serde_json::json;
    let mut params = json!({
        "size": vendor_size(&req.size),
    });
    let obj = params.as_object_mut().unwrap();
    // z-image 没有 n 参数（固定 1 张）；其余系列按用户值发送。
    if req.supports_n {
        obj.insert("n".into(), json!(req.n));
    }
    if req.supports_negative_prompt {
        obj.insert("negative_prompt".into(), json!(req.negative_prompt));
    }
    if req.supports_prompt_extend {
        // None＝用户未置：回落该模型的官方默认（z-image 关、其余开）。
        obj.insert(
            "prompt_extend".into(),
            json!(req
                .prompt_extend
                .unwrap_or_else(|| prompt_extend_default(&req.model))),
        );
    }
    if req.supports_prompt_extend_mode {
        // 用户未选则回流该模型声明的默认档；两者都缺（空串）时兜官方 direct，绝不发空值。
        let chosen = req
            .prompt_extend_mode
            .clone()
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| req.prompt_extend_mode_default.clone());
        obj.insert(
            "prompt_extend_mode".into(),
            json!(if chosen.is_empty() { "direct" } else { chosen.as_str() }),
        );
    }
    if req.supports_enable_thinking {
        // 思考模式官方默认 true。官方那句「仅 prompt_extend=true 时生效」交给服务端，
        // 客户端照发用户选择（发了不生效 ≠ 发错），本层不做二次门控。
        obj.insert(
            "enable_thinking".into(),
            json!(req.enable_thinking.unwrap_or(req.enable_thinking_default)),
        );
    }
    if req.supports_watermark {
        // 水印默认值随模型声明（官方口径为关）。
        obj.insert(
            "watermark".into(),
            json!(req.watermark.unwrap_or(req.watermark_default)),
        );
    }
    if req.supports_seed {
        if let Some(seed) = req.seed {
            obj.insert("seed".into(), json!(seed));
        }
    }
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

/// 解析同步响应：`output.choices[].message.content[].image`（结果 URL）。
pub(crate) fn parse_sync(resp: &Value) -> Result<Vec<String>, String> {
    if let Some(code) = resp.get("code").and_then(|c| c.as_str()) {
        if !code.is_empty() {
            return Err(error_detail(resp, "同步生图失败"));
        }
    }
    let choices = resp
        .get("output")
        .and_then(|o| o.get("choices"))
        .and_then(|c| c.as_array())
        .ok_or("响应缺少 output.choices")?;
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
    if urls.is_empty() {
        return Err("响应中没有图片结果".into());
    }
    Ok(urls)
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

/// 下载结果 URL 为字节：24h 有效，调用方必须此刻拉取。
async fn download_all(
    client: &reqwest::Client,
    urls: Vec<String>,
) -> Result<Vec<FetchedImage>, String> {
    let mut images = Vec::with_capacity(urls.len());
    for u in urls {
        let bytes = client
            .get(&u)
            .send()
            .await
            .map_err(|e| format!("图片下载失败: {e}"))?
            .error_for_status()
            .map_err(|e| format!("图片下载失败: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("图片读取失败: {e}"))?
            .to_vec();
        images.push(FetchedImage {
            bytes,
            extension: "png".into(),
        });
    }
    Ok(images)
}

pub struct DashscopeProvider;

impl ImageProvider for DashscopeProvider {
    fn generate<'a>(
        &'a self,
        client: &'a reqwest::Client,
        key: &'a str,
        req: &'a GenRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, String>> + Send + 'a>> {
        Box::pin(async move {
            let base = base(&req.base_url);
            // 三系列文生图统一同步调用：一次请求直接出图（大图/多图可能耗时数分钟，
            // HTTP 客户端总超时已在 image_ai::generate 放宽到 600s）。
            let resp = client
                .post(sync_url(&base))
                .bearer_auth(key)
                .json(&build_body(req))
                .send()
                .await
                .map_err(|e| format!("请求发送失败: {e}"))?;
            let status = resp.status();
            let json: Value = resp
                .json()
                .await
                .map_err(|e| format!("响应解析失败: {e}"))?;
            if !status.is_success() {
                return Err(format!(
                    "HTTP {status}：{}",
                    error_detail(&json, "同步生图失败")
                ));
            }
            let urls = parse_sync(&json)?;
            let images = download_all(client, urls).await?;
            Ok(ProviderOutcome { images })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen_req(model: &str) -> GenRequest {
        GenRequest {
            provider_id: PROVIDER_ID.into(),
            base_url: String::new(),
            model: model.into(),
            prompt: "一张脸".into(),
            negative_prompt: "模糊".into(),
            size: "1024x1024".into(),
            n: 2,
            quality: None,
            seed: Some(7),
            steps: None,
            uid: None,
            prompt_extend: None,
            watermark: None,
            prompt_extend_mode: None,
            enable_thinking: None,
            supports_response_format: false,
            supports_n: true,
            supports_negative_prompt: true,
            supports_prompt_extend: true,
            supports_watermark: true,
            supports_seed: true,
            supports_prompt_extend_mode: false,
            prompt_extend_mode_default: "direct".into(),
            supports_enable_thinking: false,
            enable_thinking_default: true,
            watermark_default: false,
        }
    }

    // ===================== 内置目录 =====================

    #[test]
    fn catalog_covers_three_families_latest_two_gens() {
        let p = builtin_provider();
        assert_eq!(p.id, PROVIDER_ID);
        assert_eq!(p.kind, ProviderKind::Dashscope);
        assert_eq!(p.default_model, "qwen-image-3.0-pro");
        let ids: Vec<&str> = p.models.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "qwen-image-3.0-pro",
                "qwen-image-3.0",
                "qwen-image-2.0-pro",
                "qwen-image-2.0",
                "wan2.7-image-pro",
                "wan2.7-image",
                "wan2.6-t2i",
                "z-image-turbo",
            ]
        );
        // 默认模型必须在目录中。
        assert!(ids.contains(&p.default_model.as_str()));
    }

    #[test]
    fn catalog_qwen_caps_match_official_spec() {
        let p = builtin_provider();
        let m = |id: &str| p.models.iter().find(|m| m.model == id).unwrap();
        for id in [
            "qwen-image-3.0-pro",
            "qwen-image-3.0",
            "qwen-image-2.0-pro",
            "qwen-image-2.0",
        ] {
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
            let r = c.size_rules.as_ref().unwrap();
            assert_eq!(r.total_min, 512 * 512);
            assert_eq!(r.total_max, 2048 * 2048);
        }
        // 仅 3.0 有 1:8–8:1 宽高比门限。
        assert_eq!(
            m("qwen-image-3.0-pro")
                .capabilities
                .size_rules
                .as_ref()
                .unwrap()
                .ratio_min,
            0.125
        );
        assert_eq!(
            m("qwen-image-2.0")
                .capabilities
                .size_rules
                .as_ref()
                .unwrap()
                .ratio_min,
            0.0
        );
        // 提示词上限：3.0=4500、2.0=1300。
        assert_eq!(m("qwen-image-3.0").capabilities.max_prompt_chars, 4500);
        assert_eq!(m("qwen-image-2.0").capabilities.max_prompt_chars, 1300);
        // 改写模式 / 思考模式是 3.0 独有的协议参数，2.0 文档里没有这两个字段。
        for id in ["qwen-image-3.0-pro", "qwen-image-3.0"] {
            let c = &m(id).capabilities;
            assert!(c.supports_prompt_extend_mode, "{id} 有改写模式");
            assert!(c.supports_enable_thinking, "{id} 有思考模式");
            assert_eq!(c.prompt_extend_mode_default, "direct");
            assert!(c.enable_thinking_default);
        }
        for id in ["qwen-image-2.0-pro", "qwen-image-2.0"] {
            let c = &m(id).capabilities;
            assert!(!c.supports_prompt_extend_mode, "{id} 无改写模式");
            assert!(!c.supports_enable_thinking, "{id} 无思考模式");
        }
        // 异步能力位出厂一律 false（适配器尚未实现异步，见 Capabilities::supports_async）。
        for m in &p.models {
            assert!(!m.capabilities.supports_async, "{} 异步默认关", m.model);
        }
    }

    #[test]
    fn catalog_wan27_uses_size_tiers_without_negative_or_extend() {
        let p = builtin_provider();
        let pro = p
            .models
            .iter()
            .find(|m| m.model == "wan2.7-image-pro")
            .unwrap();
        assert_eq!(
            pro.capabilities.sizes,
            vec!["1K".to_string(), "2K".to_string(), "4K".to_string()]
        );
        assert!(
            pro.capabilities.size_rules.is_none(),
            "档位尺寸不走像素校验"
        );
        assert_eq!(pro.capabilities.max_n, 4);
        assert!(!pro.capabilities.supports_negative_prompt);
        assert!(!pro.capabilities.supports_prompt_extend);
        assert!(pro.capabilities.supports_watermark);
        assert!(pro.capabilities.supports_seed);
        let std = p.models.iter().find(|m| m.model == "wan2.7-image").unwrap();
        assert_eq!(
            std.capabilities.sizes,
            vec!["1K".to_string(), "2K".to_string()]
        );
        // prompt_extend_default 对不支持改写的模型无意义，保持缺省 true 不影响门控。
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
        // 尺寸约束：官方只给总像素区间，单边按同区间推出；**比例留空**（官方无比例限制）。
        let r = c.size_rules.as_ref().unwrap();
        assert_eq!((r.min_side, r.max_side), (512, 2048));
        assert_eq!((r.total_min, r.total_max), (512 * 512, 2048 * 2048));
        assert_eq!((r.ratio_min, r.ratio_max), (0.0, 0.0));
        assert!(!prompt_extend_default("z-image-turbo"));
        assert!(prompt_extend_default("qwen-image-3.0"));
        assert!(prompt_extend_default("wan2.7-image-pro"));
    }

    // ===================== 请求体组装 =====================

    #[test]
    fn body_uses_messages_shape_with_star_size() {
        let b = build_body(&gen_req("qwen-image-3.0-pro"));
        assert_eq!(b["model"], "qwen-image-3.0-pro");
        assert_eq!(b["input"]["messages"][0]["role"], "user");
        assert_eq!(b["input"]["messages"][0]["content"][0]["text"], "一张脸");
        assert_eq!(b["parameters"]["size"], "1024*1024"); // x → *
        assert_eq!(b["parameters"]["n"], 2);
        assert_eq!(b["parameters"]["negative_prompt"], "模糊");
        assert_eq!(b["parameters"]["prompt_extend"], true); // None → 千问默认开
        assert_eq!(b["parameters"]["watermark"], false);
        assert_eq!(b["parameters"]["seed"], 7);
    }

    #[test]
    fn zimage_omits_n_negative_watermark_and_defaults_extend_off() {
        let mut r = gen_req("z-image-turbo");
        r.supports_n = false;
        r.supports_negative_prompt = false;
        r.supports_watermark = false;
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
        r.supports_negative_prompt = false;
        r.supports_prompt_extend = false;
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
        // 不支持的模型（默认 fixture）：两个字段都不出现。
        let b = build_body(&gen_req("qwen-image-2.0"));
        assert!(b["parameters"].get("prompt_extend_mode").is_none());
        assert!(b["parameters"].get("enable_thinking").is_none());

        // 3.0：用户未选 → 回流模型声明的默认（direct / true）。
        let mut r = gen_req("qwen-image-3.0");
        r.supports_prompt_extend_mode = true;
        r.supports_enable_thinking = true;
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
        r.prompt_extend_mode_default = String::new();
        assert_eq!(build_body(&r)["parameters"]["prompt_extend_mode"], "direct");
    }

    #[test]
    fn watermark_uses_model_declared_default() {
        let mut r = gen_req("wan2.6-t2i");
        r.watermark_default = true;
        assert_eq!(build_body(&r)["parameters"]["watermark"], true);
        r.watermark = Some(false);
        assert_eq!(build_body(&r)["parameters"]["watermark"], false);
    }

    // ===================== 响应解析与 URL =====================

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
        assert_eq!(parse_sync(&v).unwrap_err(), "BadRequest: size invalid");
        let v: Value = serde_json::from_str(r#"{"output":{"choices":[]}}"#).unwrap();
        assert!(parse_sync(&v).unwrap_err().contains("没有图片结果"));
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
}
