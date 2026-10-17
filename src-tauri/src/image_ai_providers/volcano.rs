//! 火山方舟（Ark）豆包 Seedream 适配器 + 内置模型目录（同步 Bearer）。
//!
//! Endpoint 与响应形态与 OpenAI Images 同构（`{base}/images/generations`、
//! `data[].b64_json|url`、`error.code/message`），故 endpoint 拼接与响应解析复用
//! [`super::openai`]；请求体由本模块按 Seedream 官方契约自行组装。
//!
//! 【内置目录】只接入当前在售的五个文生图模型（2026-10 官方口径，模型 ID 逐字照抄）：
//! - `doubao-seedream-5-0-pro-260628`（5.0 Pro）
//! - `doubao-seedream-5-0-flash-260915`（5.0 Flash，出厂默认）
//! - `doubao-seedream-5-0-260128`（5.0 Lite）
//! - `doubao-seedream-4-5-251128`（4.5）
//! - `doubao-seedream-4-0-250828`（4.0）
//!
//! 早期版本 assets 里出现过的 `seedream-5.0-flash` / `seedream-5.0-lite` / `seedream-4.0`
//! 是从未调通的猜测模型名；该目录现由代码内置、缺失即整块注入（见
//! [`crate::image_ai::reconcile_builtin_providers`]）。
//!
//! 【逐模型参数差异】（官方「图片生成 API」参数表，能力不声明就不发字段）：
//! | 参数 | 5.0 Pro/Flash | 5.0 Lite | 4.5 | 4.0 |
//! |---|---|---|---|---|
//! | size 档位 | 1K/1.5K/2K（默认 2K） | 2K/3K/4K | 2K/4K | 1K/2K/4K |
//! | 自定义像素总像素 | [921600(1280×720), 4624220] | [3686400(2560×1440), 16777216(4096²)] | 同 Lite | [921600, 16777216] |
//! | 宽高比 | 全系列 [1/16, 16] ||||
//! | output_format | png/jpeg | png/jpeg | **不可设，仅回 jpeg** | **不可设，仅回 jpeg** |
//! | sequential_image_generation | **不支持，传参报错** | disabled/auto（组图） | 同 Lite | 同 Lite |
//! | seed / n | **全系列均无此参数**（文生图固定单图）||||
//! | watermark | bool，全系列支持；官方 curl 示例均显式 false（服务端默认 true 带「AI生成」水印）||||
//! | response_format | url(默认,24h)/b64_json，全系列支持；适配器固定 b64_json 直返 ||||
//!
//! 本项目只做单轮文生图：`sequential_image_generation` 全系列不发送（Pro/Flash 发了直接报错；
//! Lite/4.5/4.0 缺省即单图）；Pro/Flash 专属的图层拆分/参考图/交互编辑，以及 Lite 的联网能力，
//! 均不在文生图场景接入。size 档位串与 `WxH` 像素串（小写 `x`，如 `2048x1024`）原样透传、
//! 不可混用。prompt 官方仅给软建议（中文 ≤300 字 / 英文 ≤600 词），非硬限，故不设字符上限拦截。

use super::{validate_model_caps, FetchedImage, GenStage, ImageProvider, ProviderOutcome};
use crate::image_ai::{
    classify_provider_error, from_http_status, Capabilities, GenRequest, ImageGenError, ModelInfo,
    ProviderConfig, ProviderKind, SizeRules,
};
use base64::Engine; // STANDARD.decode() 来自 Engine trait
use std::future::Future;
use std::pin::Pin;

/// 内置火山服务商的固定 ID（配置合并/迁移按此 ID 认领）。
pub(crate) const PROVIDER_ID: &str = "volcano";

/// 方舟华北 2（北京）公共基址；生图端点为 `{base}/images/generations`。
const DEFAULT_BASE: &str = "https://ark.cn-beijing.volces.com/api/v3";

/// 默认模型：Flash 档，速度快成本低，适合批量出脸。
const DEFAULT_MODEL: &str = "doubao-seedream-5-0-flash-260915";

// ===================== 自定义像素总像素区间（官方逐模型口径）=====================

/// 下限：1280×720 = 921600（Pro/Flash 与 4.0）。
const TOTAL_MIN_720P: u64 = 1280 * 720;
/// Pro/Flash 上限：2048²×1.1025 = 4624220。
const TOTAL_MAX_PRO_FLASH: u64 = 4_624_220;
/// 下限：2560×1440 = 3686400（Lite 与 4.5）。
const TOTAL_MIN_2K: u64 = 2560 * 1440;
/// 上限：4096² = 16777216。
const TOTAL_MAX_4K: u64 = 4096 * 4096;

// ===================== 内置模型目录 =====================

/// 火山内置服务商：模型目录代码托管（不读 assets/image_ai.json），新装直接得到完整目录。
pub(crate) fn builtin_provider() -> ProviderConfig {
    ProviderConfig {
        id: PROVIDER_ID.into(),
        kind: ProviderKind::Volcano,
        name: "火山方舟 Seedream（豆包）".into(),
        base_url: DEFAULT_BASE.into(),
        default_model: DEFAULT_MODEL.into(),
        models: vec![
            ModelInfo {
                model: "doubao-seedream-5-0-pro-260628".into(),
                capabilities: seedream_caps(
                    &["1K", "1.5K", "2K"],
                    TOTAL_MIN_720P,
                    TOTAL_MAX_PRO_FLASH,
                    true,
                ),
            },
            ModelInfo {
                model: DEFAULT_MODEL.into(),
                capabilities: seedream_caps(
                    &["1K", "1.5K", "2K"],
                    TOTAL_MIN_720P,
                    TOTAL_MAX_PRO_FLASH,
                    true,
                ),
            },
            ModelInfo {
                model: "doubao-seedream-5-0-260128".into(),
                capabilities: seedream_caps(&["2K", "3K", "4K"], TOTAL_MIN_2K, TOTAL_MAX_4K, true),
            },
            ModelInfo {
                model: "doubao-seedream-4-5-251128".into(),
                capabilities: seedream_caps(&["2K", "4K"], TOTAL_MIN_2K, TOTAL_MAX_4K, false),
            },
            ModelInfo {
                model: "doubao-seedream-4-0-250828".into(),
                capabilities: seedream_caps(
                    &["1K", "2K", "4K"],
                    TOTAL_MIN_720P,
                    TOTAL_MAX_4K,
                    false,
                ),
            },
        ],
    }
}

/// Seedream 全系列共同基线 + 逐模型差异：
/// - 文生图固定单图：`max_n=1`、[`Capabilities::supports_n`]=false（请求体不发 n）；
/// - 全系列无 seed（不发）、无负向词/改写/质量档位、仅同步；
/// - 全系列支持 `response_format`（适配器固定 b64_json）与 `watermark`（默认关，显式发 false）；
/// - `output_format` 仅 Pro/Flash/Lite 可设（值取用户选择 png/jpeg，未选默认 png）；4.5/4.0 只回 jpeg、传参报错；
/// - 尺寸：`tiers` 为官方档位串，自定义像素按总像素区间 + 宽高比 [1/16, 16] 校验
///   （min/max_side 留 0 = 官方未给单边约束）。
fn seedream_caps(
    tiers: &[&str],
    total_min: u64,
    total_max: u64,
    supports_output_format: bool,
) -> Capabilities {
    Capabilities {
        sizes: tiers.iter().map(|s| (*s).to_string()).collect(),
        max_n: 1,
        // 注：Capabilities::default() 给 bool 的是 false；代码构造必须显式声明，
        // serde(default) 只对 JSON 缺字段反序列化生效。
        supports_n: false,
        supports_seed: false,
        supports_response_format: true,
        supports_watermark: true,
        watermark_default: false,
        supports_output_format,
        supports_async: false,
        // 官方仅「建议」中文 ≤300 字，非硬限：不拦截（0 = 不校验），避免把 API 本可接受的提示词挡在本地。
        max_prompt_chars: 0,
        size_rules: Some(SizeRules {
            total_min,
            total_max,
            ratio_min: 1.0 / 16.0,
            ratio_max: 16.0,
            ..Default::default()
        }),
        ..Default::default()
    }
}

// ===================== 请求体（纯函数，字段逐字照抄官方文档）=====================

/// 组装 Seedream 文生图请求体：
/// - 恒发：`model` / `prompt` / `size`（档位串或小写 `x` 像素串，原样透传不转换）；
/// - `response_format`：固定 `b64_json`（能力位门控，内置目录全开）；
/// - `watermark`：用户值优先，未置回落能力集声明的默认（内置 false；官方服务端默认 true）；
/// - `output_format`：仅 Pro/Flash/Lite 发（4.5/4.0 传参报错，整条省略），值取用户选择
///   （`png` / `jpeg`），未选或越界回落 `png`——见 [`super::output_format`]；
/// - **绝不发** `n` / `seed` / `sequential_image_generation`（全系列无 n/seed；
///   Pro/Flash 收到 sequential 直接报错，Lite 等缺省即单图）。
pub(crate) fn build_volcano_body(req: &GenRequest) -> serde_json::Value {
    use serde_json::json;
    let caps = req.caps();
    let mut v = json!({
        "model": req.model,
        "prompt": req.prompt,
        "size": req.size,
    });
    let obj = v.as_object_mut().unwrap();
    if caps.supports_response_format {
        obj.insert("response_format".into(), json!("b64_json"));
    }
    if caps.supports_watermark {
        obj.insert(
            "watermark".into(),
            json!(req.watermark.unwrap_or(caps.watermark_default)),
        );
    }
    if caps.supports_output_format {
        obj.insert("output_format".into(), json!(super::output_format(req)));
    }
    v
}

/// 落盘图片后缀：4.5/4.0 不接受 `output_format`、服务端只回 JPEG；其余模型随用户选择的格式。
pub(crate) fn image_extension(req: &GenRequest) -> &'static str {
    if req.caps().supports_output_format {
        super::output_format(req)
    } else {
        "jpeg"
    }
}

/// 提取火山错误描述：`error.code` 与 `error.message` 同时存在时拼成 `code: message`。
pub(crate) fn error_message(resp: &serde_json::Value) -> Option<String> {
    let err = resp.get("error")?;
    let message = err
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("未知错误");
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
        progress: &'a (dyn Fn(GenStage) + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, ImageGenError>> + Send + 'a>> {
        Box::pin(async move {
            // 同步链：提交 → 下载。
            progress(GenStage::Submitting);
            let url = super::openai::endpoint(&req.base_url);
            let resp = client
                .post(&url)
                .bearer_auth(key)
                .json(&build_volcano_body(req))
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
            // 先取火山 error.code/message，再复用 OpenAI 的 data 解析（成功路径）。
            if !status.is_success() {
                if let Some(msg) = error_message(&json) {
                    let code = json
                        .get("error")
                        .and_then(|e| e.get("code"))
                        .and_then(|c| c.as_str());
                    return Err(classify_provider_error(
                        status.as_u16(),
                        code,
                        format!("HTTP {status}：{msg}"),
                    )
                    .with_retry_after(retry_after));
                }
            }
            let payloads = super::openai::parse_images(&json).map_err(|e| {
                from_http_status(status.as_u16(), format!("HTTP {status}：{}", e.message))
                    .with_retry_after(retry_after)
            })?;

            progress(GenStage::Downloading);
            // 后缀逐请求恒定（由模型能力决定），在循环外算一次。
            let extension = image_extension(req);
            let mut images = Vec::with_capacity(payloads.len());
            for payload in payloads {
                let bytes = match payload {
                    super::openai::ImagePayload::B64(b64) => {
                        base64::engine::general_purpose::STANDARD
                            .decode(b64.trim())
                            .map_err(|e| {
                                ImageGenError::provider(format!("图片 base64 解码失败: {e}"), false)
                            })?
                    }
                    super::openai::ImagePayload::Url(u) => client
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
                };
                images.push(FetchedImage {
                    bytes,
                    extension: extension.into(),
                });
            }
            Ok(ProviderOutcome { images })
        })
    }

    /// 与 OpenAI 同构：端点由 base_url 拼出，空基址是本协议特有的门控。
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

    // ===================== 内置目录 =====================

    #[test]
    fn catalog_lists_five_current_models() {
        let p = builtin_provider();
        assert_eq!(p.id, PROVIDER_ID);
        assert_eq!(p.kind, ProviderKind::Volcano);
        assert_eq!(p.base_url, DEFAULT_BASE);
        assert_eq!(p.default_model, "doubao-seedream-5-0-flash-260915");
        let ids: Vec<&str> = p.models.iter().map(|m| m.model.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "doubao-seedream-5-0-pro-260628",
                "doubao-seedream-5-0-flash-260915",
                "doubao-seedream-5-0-260128",
                "doubao-seedream-4-5-251128",
                "doubao-seedream-4-0-250828",
            ]
        );
        assert!(ids.contains(&p.default_model.as_str()));
        // 早期的点分猜测模型名不得出现在内置目录里。
        for old in ["seedream-5.0-flash", "seedream-5.0-lite", "seedream-4.0"] {
            assert!(!ids.contains(&old), "{old} 是失效的旧猜测名");
        }
    }

    #[test]
    fn catalog_common_caps_match_official_spec() {
        let p = builtin_provider();
        for m in &p.models {
            let c = &m.capabilities;
            assert_eq!(c.max_n, 1, "{} 文生图固定单图", m.model);
            assert!(!c.supports_n, "{} 无 n 参数", m.model);
            assert!(!c.supports_seed, "{} 无 seed 参数", m.model);
            assert!(
                c.supports_response_format,
                "{} 支持 response_format",
                m.model
            );
            assert!(c.supports_watermark, "{} 支持 watermark", m.model);
            assert!(!c.watermark_default, "{} 水印默认关", m.model);
            assert!(!c.supports_quality, "{} 无质量档位", m.model);
            assert!(!c.supports_negative_prompt, "{} 无负向词", m.model);
            assert!(!c.supports_prompt_extend, "{} 无智能改写", m.model);
            assert!(!c.supports_async, "{} 仅同步", m.model);
            assert_eq!(c.max_prompt_chars, 0, "{} 提示词只有软建议不硬拦", m.model);
            let r = c.size_rules.as_ref().unwrap();
            assert_eq!(r.ratio_min, 0.0625, "{} 宽高比下限 1/16", m.model);
            assert_eq!(r.ratio_max, 16.0, "{} 宽高比上限 16", m.model);
            assert_eq!(
                (r.min_side, r.max_side),
                (0, 0),
                "{} 官方无单边约束",
                m.model
            );
        }
    }

    #[test]
    fn catalog_size_tiers_pixel_bounds_and_output_format() {
        let pro = caps_of("doubao-seedream-5-0-pro-260628");
        assert_eq!(pro.sizes, vec!["1K", "1.5K", "2K"]);
        assert_eq!(pro.size_rules.as_ref().unwrap().total_min, 921_600);
        assert_eq!(pro.size_rules.as_ref().unwrap().total_max, 4_624_220);
        assert!(pro.supports_output_format);
        // Flash 与 Pro 同档。
        let flash = caps_of("doubao-seedream-5-0-flash-260915");
        assert_eq!(flash.sizes, vec!["1K", "1.5K", "2K"]);
        assert_eq!(flash.size_rules, pro.size_rules);
        assert!(flash.supports_output_format);

        let lite = caps_of("doubao-seedream-5-0-260128");
        assert_eq!(lite.sizes, vec!["2K", "3K", "4K"]);
        let r = lite.size_rules.as_ref().unwrap();
        assert_eq!(r.total_min, 3_686_400);
        assert_eq!(r.total_max, 16_777_216);
        assert!(lite.supports_output_format, "Lite 可设 png/jpeg");

        let v45 = caps_of("doubao-seedream-4-5-251128");
        assert_eq!(v45.sizes, vec!["2K", "4K"]);
        assert_eq!(v45.size_rules, lite.size_rules);
        assert!(!v45.supports_output_format, "4.5 仅回 jpeg、不可设格式");

        let v40 = caps_of("doubao-seedream-4-0-250828");
        assert_eq!(v40.sizes, vec!["1K", "2K", "4K"]);
        let r = v40.size_rules.as_ref().unwrap();
        assert_eq!(r.total_min, 921_600);
        assert_eq!(r.total_max, 16_777_216);
        assert!(!v40.supports_output_format, "4.0 仅回 jpeg、不可设格式");
    }

    // ===================== 请求体组装 =====================

    #[test]
    fn body_pro_flash_carries_exactly_official_fields() {
        for id in [
            "doubao-seedream-5-0-pro-260628",
            "doubao-seedream-5-0-flash-260915",
        ] {
            let b = build_volcano_body(&req_for(id));
            assert_eq!(b["model"], id);
            assert_eq!(b["prompt"], "一张东亚球员的脸");
            // 档位串原样透传（不做 x→* 转换，那是 dashscope 的规则）。
            assert_eq!(b["size"], "1K");
            assert_eq!(b["response_format"], "b64_json");
            assert_eq!(b["watermark"], false, "水印默认关、显式发送 false");
            assert_eq!(b["output_format"], "png");
            // 文档里没有 / 传了会报错的字段必须缺席。
            for forbidden in [
                "n",
                "seed",
                "sequential_image_generation",
                "negative_prompt",
            ] {
                assert!(b.get(forbidden).is_none(), "{id} 不应出现 {forbidden}");
            }
            assert_eq!(image_extension(&req_for(id)), "png");
        }
    }

    #[test]
    fn body_lite_uses_2k_png_and_passes_custom_pixels_verbatim() {
        let mut r = req_for("doubao-seedream-5-0-260128");
        assert_eq!(build_volcano_body(&r)["size"], "2K");
        assert_eq!(build_volcano_body(&r)["output_format"], "png");
        // 自定义像素串：小写 x 原样发（官方契约就是小写 x）。
        r.size = "2048x1024".into();
        assert_eq!(build_volcano_body(&r)["size"], "2048x1024");
        assert_eq!(image_extension(&r), "png");
    }

    #[test]
    fn body_45_40_omit_output_format_and_report_jpeg() {
        for id in ["doubao-seedream-4-5-251128", "doubao-seedream-4-0-250828"] {
            let b = build_volcano_body(&req_for(id));
            assert!(
                b.get("output_format").is_none(),
                "{id} 不接受 output_format，整条省略"
            );
            assert_eq!(b["response_format"], "b64_json");
            assert_eq!(b["watermark"], false);
            for forbidden in ["n", "seed", "sequential_image_generation"] {
                assert!(b.get(forbidden).is_none(), "{id} 不应出现 {forbidden}");
            }
            assert_eq!(image_extension(&req_for(id)), "jpeg", "{id} 只回 JPEG");
        }
    }

    #[test]
    fn body_ignores_seed_and_n_even_when_caller_sets_them() {
        // 防御性：GenRequest 仍带 seed/n 字段（其他协议在用），火山适配器必须无视它们，
        // 绝不能因为调用方残留值把陌生字段发给方舟。
        let mut r = req_for("doubao-seedream-5-0-flash-260915");
        r.seed = Some(42);
        r.n = 9;
        let b = build_volcano_body(&r);
        assert!(b.get("seed").is_none());
        assert!(b.get("n").is_none());
    }

    #[test]
    fn body_output_format_follows_user_choice_and_falls_back_to_png() {
        let mut r = req_for("doubao-seedream-5-0-flash-260915");
        // 用户选 jpeg：请求体与落盘后缀都跟着走（此前适配器写死 png，勾了能力位也无处可选）。
        r.output_format = Some("jpeg".into());
        assert_eq!(build_volcano_body(&r)["output_format"], "jpeg");
        assert_eq!(image_extension(&r), "jpeg");
        // 未选回落 png（出厂口径不变）；越界值走白名单，不直发厂商。
        r.output_format = None;
        assert_eq!(build_volcano_body(&r)["output_format"], "png");
        assert_eq!(image_extension(&r), "png");
        r.output_format = Some("webp".into());
        assert_eq!(build_volcano_body(&r)["output_format"], "png");
        // 4.5/4.0 不可设：即便传了 jpeg 也整条省略、后缀恒 jpeg（服务端只回 jpeg）。
        for id in ["doubao-seedream-4-5-251128", "doubao-seedream-4-0-250828"] {
            let mut old = req_for(id);
            old.output_format = Some("jpeg".into());
            assert!(build_volcano_body(&old).get("output_format").is_none());
            assert_eq!(image_extension(&old), "jpeg", "{id}");
        }
    }

    #[test]
    fn body_honors_explicit_watermark_override() {
        let mut r = req_for("doubao-seedream-5-0-flash-260915");
        r.watermark = Some(true);
        assert_eq!(build_volcano_body(&r)["watermark"], true);
        r.watermark = Some(false);
        assert_eq!(build_volcano_body(&r)["watermark"], false);
    }

    // ===================== 尺寸规则边界 =====================

    #[test]
    fn pro_flash_size_rules_guard_custom_pixels() {
        let caps = caps_of("doubao-seedream-5-0-flash-260915");
        let check = |size: &str| {
            let mut r = req_for("doubao-seedream-5-0-flash-260915");
            r.size = size.into();
            validate_model_caps(&r, &caps)
        };
        assert!(check("1K").is_ok(), "档位串豁免像素校验");
        assert!(check("1.5K").is_ok());
        assert!(check("1280x720").is_ok(), "总像素下限 921600");
        assert!(check("1280x719").is_err(), "低于下限");
        assert!(check("2150x2150").is_ok(), "4_622_500 ≤ 4_624_220");
        assert!(check("2151x2151").is_err(), "4_626_801 超 Pro/Flash 上限");
        // 宽高比 [1/16, 16]：用落在总像素区间内的极端长条验边界——
        // 3840×240 总像素恰为下限 921600、比例恰为 16:1，放行；
        // 3856×240 总像素仍在区间内但比例 16.07:1，只撞比例闸拦下。
        assert!(check("3840x240").is_ok());
        assert!(check("3856x240").is_err(), "比例 16.07:1 越界");
        assert!(check("240x3856").is_err(), "竖版比例越界");
    }

    #[test]
    fn lite_and_45_size_rules_start_at_1440p() {
        let caps = caps_of("doubao-seedream-5-0-260128");
        let check = |size: &str| {
            let mut r = req_for("doubao-seedream-5-0-260128");
            r.size = size.into();
            validate_model_caps(&r, &caps)
        };
        assert!(check("2K").is_ok());
        assert!(check("2560x1440").is_ok(), "Lite/4.5 下限 3686400");
        assert!(check("2559x1440").is_err(), "3_684_960 低于下限");
        assert!(check("4096x4096").is_ok(), "4096² 上限");
    }

    #[test]
    fn model_40_accepts_720p_to_4k() {
        let caps = caps_of("doubao-seedream-4-0-250828");
        let check = |size: &str| {
            let mut r = req_for("doubao-seedream-4-0-250828");
            r.size = size.into();
            validate_model_caps(&r, &caps)
        };
        assert!(check("1K").is_ok());
        assert!(check("1280x720").is_ok(), "4.0 下限同 720P");
        assert!(check("4096x4096").is_ok(), "4.0 上限 4096²");
        assert!(check("4097x4096").is_err());
    }

    // ===================== 错误解析 =====================

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
