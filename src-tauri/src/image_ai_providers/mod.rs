//! AI 文生图适配器族：按 [`crate::image_ai::ProviderKind`] 分发到具体厂商协议。
//!
//! 【族内分工】
//! - [`openai`]：OpenAI Images 协议。官方 OpenAI 与用户自填的 OpenAI 兼容服务同走这一档
//!   （含阿里云百炼的 `/compatible-mode/v1`，它额外接受一批平铺在顶层的百炼扩展字段）；
//! - [`volcano`]：火山方舟豆包 Seedream（5.0 Pro/Flash、4.0 三模型），同步 Bearer，
//!   响应形态与 OpenAI 同构，模型目录代码内置（缺失即整块注入）；
//! - [`dashscope`]：阿里云百炼（千问/万相/Z-Image 三系列），同步多模态消息式调用 +
//!   代码内置模型目录（模型能力不读 image_ai.json）；
//! - [`siliconflow`]：硅基流动（同步 Bearer），响应为 `images[].url`、尺寸字段是 `image_size`；
//! - [`modelscope`]：魔搭 ModelScope（**强制异步**：提交任务 → 轮询 `/tasks/{id}` 取 `output_images[]`）；
//! - [`comfyui`]：本地 ComfyUI（无鉴权），提交原生节点工作流到 `POST /prompt`，
//!   轮询 `/history/{prompt_id}` 后经 `/view` 取图；Key 恒不需要（`ProviderKind::needs_key`）。
//!
//! 【对象安全】trait 用「返回 `Pin<Box<dyn Future>>`」形式而非 `async fn`，
//! 故 [`for_kind`] 能返回 `Box<dyn ImageProvider>`。所有协议单测均为纯函数测试，不触网。

pub(crate) mod comfyui;
pub(crate) mod dashscope;
pub(crate) mod modelscope;
pub(crate) mod openai;
pub(crate) mod siliconflow;
pub(crate) mod volcano;

use crate::image_ai::types::{
    Capabilities, GenRequest, GenStage, ImageGenError, ProviderKind, SizeRules,
};
use std::future::Future;
use std::pin::Pin;

/// 适配器产出的单张图片：原始字节 + 建议文件后缀（不含点，如 "png"）。
pub struct FetchedImage {
    pub bytes: Vec<u8>,
    pub extension: String,
}

/// 读取 429 响应的 `Retry-After` 头（秒）。仅认整数秒形态；HTTP-date 形态不做解析
/// （那需要日期库，零新增依赖前提下返回 None，由退避兜底）。只在 429 分支调用。
pub(crate) fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// 一次生成的适配器结果：图片字节序列。
pub struct ProviderOutcome {
    pub images: Vec<FetchedImage>,
}

// ===================== 共用校验（三家共享的「按模型声明解释约束」） ==================================================
// 为什么放在族目录：张数/提示词/尺寸的解释口径由协议族共同拥有，留在应用层会让每个调用点
// 各自复刻一份；收口到 Provider 后，generate 与入队校验都只经 for_kind(...).validate。

/// 解析 宽×高（小写 `x`）/「宽*高」两种写法并按模型自由尺寸规则校验；0 值字段表示该约束不检查。
pub(crate) fn validate_size(size: &str, rules: &SizeRules) -> Result<(), ImageGenError> {
    let caps: Vec<u32> = size
        .split(['*', 'x'])
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .collect();
    let (w, h) = match caps.as_slice() {
        [w, h] if *w > 0 && *h > 0 => (*w, *h),
        _ => {
            return Err(ImageGenError::invalid_param(format!(
                "尺寸「{size}」格式不正确，应为 宽×高 或 宽*高（如 1280*1280）"
            )))
        }
    };
    if rules.min_side > 0 && (w < rules.min_side || h < rules.min_side) {
        return Err(ImageGenError::invalid_param(format!(
            "尺寸宽/高不能小于 {} 像素",
            rules.min_side
        )));
    }
    if rules.max_side > 0 && (w > rules.max_side || h > rules.max_side) {
        return Err(ImageGenError::invalid_param(format!(
            "尺寸宽/高不能超过 {} 像素",
            rules.max_side
        )));
    }
    let total = w as u64 * h as u64;
    if rules.total_min > 0 && total < rules.total_min {
        return Err(ImageGenError::invalid_param(format!(
            "总像素不能小于 {}（约 {}²）",
            rules.total_min,
            (rules.total_min as f64).sqrt() as u32
        )));
    }
    if rules.total_max > 0 && total > rules.total_max {
        return Err(ImageGenError::invalid_param(format!(
            "总像素不能超过 {}（约 {}²）",
            rules.total_max,
            (rules.total_max as f64).sqrt() as u32
        )));
    }
    if rules.ratio_min > 0.0 || rules.ratio_max > 0.0 {
        let ratio = w as f32 / h as f32;
        if (rules.ratio_min > 0.0 && ratio + 1e-4 < rules.ratio_min)
            || (rules.ratio_max > 0.0 && ratio - 1e-4 > rules.ratio_max)
        {
            return Err(ImageGenError::invalid_param(format!(
                "宽高比需在 {:.2}:1 至 {:.2}:1 之间（当前 {:.2}:1）",
                rules.ratio_min, rules.ratio_max, ratio
            )));
        }
    }
    Ok(())
}

/// 按模型声明解释厂商无关的数值/尺寸约束（张数上限、提示词长度、seed 非负、自由尺寸规则）——多家共用。
/// `0` = 不声明该约束（放行）；`size_rules` 只对不在 `sizes` 枚举内的尺寸生效。
pub(crate) fn validate_model_caps(
    req: &GenRequest,
    caps: &Capabilities,
) -> Result<(), ImageGenError> {
    // seed 官方域一律非负（DashScope [0, 4294967295]、SiliconFlow [0, 9999999999]、ModelScope 同）：
    // 负值在本地拦下，避免直发厂商吃 400；上限各协议域不同，交服务端校验。
    if let Some(seed) = req.seed {
        if seed < 0 {
            return Err(ImageGenError::invalid_param("seed 不能为负数"));
        }
    }
    if caps.max_n > 0 && req.n > caps.max_n {
        return Err(ImageGenError::invalid_param(format!(
            "该模型单次最多 {} 张",
            caps.max_n
        )));
    }
    // 提示词长度：官方口径是「超长自动截断」，这里改为显式报错，避免用户以为全文生效。
    if caps.max_prompt_chars > 0 && req.prompt.chars().count() > caps.max_prompt_chars as usize {
        return Err(ImageGenError::invalid_param(format!(
            "提示词超出该模型 {} 字上限（当前 {} 字），请精简后再生成",
            caps.max_prompt_chars,
            req.prompt.chars().count()
        )));
    }
    // 采样步数与引导系数：只在该模型声明上限时校验（0 = 不声明、放行）。
    if caps.supports_steps && caps.steps_max > 0 {
        if let Some(s) = req.steps {
            if s > caps.steps_max {
                return Err(ImageGenError::invalid_param(format!(
                    "采样步数不能超过 {}",
                    caps.steps_max
                )));
            }
        }
    }
    if caps.supports_guidance && caps.guidance_max > 0.0 {
        if let Some(g) = req.guidance {
            if g > caps.guidance_max {
                return Err(ImageGenError::invalid_param(format!(
                    "引导系数不能超过 {}",
                    caps.guidance_max
                )));
            }
        }
    }
    if let Some(rules) = &caps.size_rules {
        if !caps.sizes.iter().any(|s| s == &req.size) {
            validate_size(&req.size, rules)?;
        }
    }
    Ok(())
}

// ===================== 共用请求参数（能力位驱动的可选生成参数） ======================================================

/// 按模型能力集组装可选生成参数；**不含 `size`**（分隔符按协议不同：DashScope `*`、
/// OpenAI 兼容 `x`），也不含各家专有的 `quality` / `response_format`。
/// DashScope 与百炼 OpenAI 兼容模式共用同一批字段，故在此单点收口；能力不声明就不发。
pub(crate) fn capability_params(req: &GenRequest) -> serde_json::Map<String, serde_json::Value> {
    use serde_json::{json, Map, Value};
    let caps = req.caps();
    let mut m: Map<String, Value> = Map::new();
    // z-image 没有 n 参数（固定 1 张），必须整个省略而不是发 1。
    if caps.supports_n {
        m.insert("n".into(), json!(req.n));
    }
    if caps.supports_negative_prompt {
        m.insert("negative_prompt".into(), json!(req.negative_prompt));
    }
    if caps.supports_prompt_extend {
        // None = 用户未置：回落**该模型能力集声明的**官方默认（数据单点在能力集，不按模型名推）。
        m.insert(
            "prompt_extend".into(),
            json!(req.prompt_extend.unwrap_or(caps.prompt_extend_default)),
        );
    }
    if caps.supports_prompt_extend_mode {
        // 用户未选则回流模型声明的默认档；两者都缺（空串）时兜官方 direct，绝不发空值。
        let chosen = req
            .prompt_extend_mode
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| caps.prompt_extend_mode_default.clone());
        m.insert(
            "prompt_extend_mode".into(),
            json!(if chosen.is_empty() {
                "direct"
            } else {
                chosen.as_str()
            }),
        );
    }
    if caps.supports_enable_thinking {
        // 官方那句「仅 prompt_extend=true 时生效」交给服务端：发了不生效 ≠ 发错，本层不做二次门控。
        m.insert(
            "enable_thinking".into(),
            json!(req.enable_thinking.unwrap_or(caps.enable_thinking_default)),
        );
    }
    if caps.supports_watermark {
        m.insert(
            "watermark".into(),
            json!(req.watermark.unwrap_or(caps.watermark_default)),
        );
    }
    // seed 只在用户填了才发：官方口径是「未传入时服务随机选择种子」。
    if caps.supports_seed {
        if let Some(seed) = req.seed {
            m.insert("seed".into(), json!(seed));
        }
    }
    m
}

/// 请求体里的 `output_format` 取值（仅声明 `supports_output_format` 的模型用）：
/// `None` / 越界串 / 大小写不符一律回落 `png`；火山与硅基流动共用此白名单。
pub(crate) fn output_format(req: &GenRequest) -> &'static str {
    if req.output_format.as_deref() == Some("jpeg") {
        "jpeg"
    } else {
        "png"
    }
}

/// 统一适配器：一次 async 调用内含 POST → 解析 →（b64 解码 / URL 立即下载）；
/// 异步厂商在此调用内含 submit → poll → fetch 全流程。
/// `progress` 为阶段回调：适配器只上报、不感知去重（去重由队列调用方完成）。
/// 不接收取消令牌：取消只在 generate 发 HTTP 前检查一次，调用内没有中断点。
pub trait ImageProvider: Send + Sync {
    fn generate<'a>(
        &'a self,
        client: &'a reqwest::Client,
        key: &'a str,
        req: &'a GenRequest,
        progress: &'a (dyn Fn(GenStage) + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<ProviderOutcome, ImageGenError>> + Send + 'a>>;

    /// 发 HTTP 前的最后一道闸：厂商无关的数值/尺寸约束走 [`validate_model_caps`]，
    /// 各协议在此叠加自己的特有门控（如端点/基址要求）。
    fn validate(&self, req: &GenRequest, caps: &Capabilities) -> Result<(), ImageGenError>;
}

/// 唯一分发：按 ProviderKind 取适配器。
pub fn for_kind(kind: ProviderKind) -> Box<dyn ImageProvider> {
    match kind {
        ProviderKind::OpenAI => Box::new(openai::OpenAiProvider),
        ProviderKind::Volcano => Box::new(volcano::VolcanoProvider),
        ProviderKind::Dashscope => Box::new(dashscope::DashscopeProvider),
        ProviderKind::Siliconflow => Box::new(siliconflow::SiliconflowProvider),
        ProviderKind::Modelscope => Box::new(modelscope::ModelscopeProvider),
        ProviderKind::LocalComfyUI => Box::new(comfyui::ComfyUIProvider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_ai::types::GenErrorKind;

    fn gen_req() -> GenRequest {
        GenRequest {
            provider_id: "dashscope".into(),
            model: "z-image-turbo".into(),
            prompt: "一张脸".into(),
            size: "1024x1024".into(),
            // 校验用例显式传 caps 参数，请求侧快照留空即可。
            ..crate::image_ai::types::test_req_base()
        }
    }

    /// 只装配约束相关字段：其余走 `Default`（数值 0 = 不检查，与 serde 缺省不同，避免误带上限）。
    fn caps(max_n: u32, max_prompt_chars: u32, size_rules: Option<SizeRules>) -> Capabilities {
        Capabilities {
            max_n,
            max_prompt_chars,
            size_rules,
            ..Default::default()
        }
    }

    #[test]
    fn size_rules_match_wan25_26_official_constraints() {
        // 官方 2.5/2.6：总像素 [1280², 1440²]，宽高比 [1:4, 4:1]。
        let rules = SizeRules {
            total_min: 1280 * 1280,
            total_max: 1440 * 1440,
            ratio_min: 0.25,
            ratio_max: 4.0,
            ..Default::default()
        };
        assert!(validate_size("1280*1280", &rules).is_ok());
        assert!(validate_size("768x2700", &rules).is_ok()); // 文档举例的极端竖版（总像素恰为上限）
                                                            // 注：官方推荐档 960*1696 总像素略小于 1280²，属文档近似边界；它在预设枚举里走豁免，不经规则校验。
        assert!(validate_size("1024*1024", &rules).is_err()); // 总像素不足
        assert!(validate_size("1440*1441", &rules).is_err()); // 总像素超限
        assert!(validate_size("768*3100", &rules).is_err()); // 比例越界
        assert!(validate_size("abc", &rules).is_err()); // 格式错误
    }

    #[test]
    fn size_rules_side_bounds_for_wan22() {
        // 2.2 及更早：宽高均在 [512, 1440]。
        let rules = SizeRules {
            min_side: 512,
            max_side: 1440,
            ..Default::default()
        };
        assert!(validate_size("1024*1024", &rules).is_ok());
        assert!(validate_size("512*1440", &rules).is_ok());
        assert!(validate_size("500*1024", &rules).is_err());
        assert!(validate_size("1024*1441", &rules).is_err());
    }

    #[test]
    fn provider_validate_rejects_n_over_max_n() {
        let mut r = gen_req();
        r.n = 3;
        let err = for_kind(ProviderKind::Dashscope)
            .validate(&r, &caps(2, 0, None))
            .unwrap_err();
        assert_eq!(err.kind, GenErrorKind::InvalidParam);
        assert_eq!(err.message, "该模型单次最多 2 张");
    }

    #[test]
    fn provider_validate_rejects_prompt_over_limit() {
        let mut r = gen_req();
        r.prompt = "一二三四".into(); // 4 字
        let err = for_kind(ProviderKind::Dashscope)
            .validate(&r, &caps(0, 3, None))
            .unwrap_err();
        assert_eq!(err.kind, GenErrorKind::InvalidParam);
        assert!(err.message.contains("提示词超出该模型 3 字上限"));
    }

    #[test]
    fn provider_validate_rejects_size_rule_violation() {
        // sizes 为空 → 请求尺寸不在枚举内，size_rules 生效（此处总像素过低被拦）。
        let rules = SizeRules {
            total_min: 1280 * 1280,
            ..Default::default()
        };
        let mut r = gen_req();
        r.size = "256x256".into();
        let err = for_kind(ProviderKind::Dashscope)
            .validate(&r, &caps(0, 0, Some(rules)))
            .unwrap_err();
        assert_eq!(err.kind, GenErrorKind::InvalidParam);
        assert!(err.message.contains("总像素不能小于"));
    }

    #[test]
    fn provider_validate_rejects_negative_seed() {
        // 官方 seed 域一律非负：负值本地拦下，绝不直发厂商。
        let mut r = gen_req();
        r.seed = Some(-1);
        let err = for_kind(ProviderKind::Dashscope)
            .validate(&r, &caps(0, 0, None))
            .unwrap_err();
        assert_eq!(err.kind, GenErrorKind::InvalidParam);
        assert_eq!(err.message, "seed 不能为负数");
    }

    #[test]
    fn provider_validate_allows_when_max_n_zero() {
        // max_n == 0 表示模型不声明上限，任意张数放行。
        let mut r = gen_req();
        r.n = 999;
        assert!(for_kind(ProviderKind::Dashscope)
            .validate(&r, &caps(0, 0, None))
            .is_ok());
    }

    #[test]
    fn validate_requires_base_url_for_url_gated_kinds() {
        // OpenAI/Volcano/Siliconflow/Modelscope 四家共用同一道基址闸（文案逐字相同），
        // LocalComfyUI 是同闸不同文案；Dashscope 走百炼固定端点、没有这道闸，不入表。
        // 循环而非各模块各复制一份：新厂商接入时往表里加一行即可。
        for (kind, keyword) in [
            (ProviderKind::OpenAI, "Base URL"),
            (ProviderKind::Volcano, "Base URL"),
            (ProviderKind::Siliconflow, "Base URL"),
            (ProviderKind::Modelscope, "Base URL"),
            (ProviderKind::LocalComfyUI, "ComfyUI 服务地址"),
        ] {
            let err = for_kind(kind)
                .validate(&gen_req(), &caps(0, 0, None))
                .unwrap_err();
            let msg = err.message.clone();
            assert_eq!(err.kind, GenErrorKind::NotConfigured, "{kind:?}");
            assert!(msg.contains(keyword), "{kind:?}: {msg}");
        }
        // 非空基址时这道闸放行（以 OpenAI 为代表验证通过路径）。
        let mut ok = gen_req();
        ok.base_url = "https://api.openai.com/v1".into();
        assert!(for_kind(ProviderKind::OpenAI)
            .validate(&ok, &caps(0, 0, None))
            .is_ok());
    }
}
