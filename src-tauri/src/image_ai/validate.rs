//! 生图准入校验的两层：厂商无关完整性（`validate_complete`）+ 协议约束（`validate_for_generate`）。

use super::config::Config;
use super::types::{GenRequest, ImageGenError};

/// 生成前的厂商无关完整性校验：提示词非空、张数 ≥ 1、服务商与模型存在。
/// 厂商约束（张数上限/提示词长度/尺寸规则）一律由 Provider 的 `validate` 解释，本层不再碰。
pub fn validate_complete(cfg: &Config, req: &GenRequest) -> Result<(), ImageGenError> {
    if req.prompt.trim().is_empty() {
        return Err(ImageGenError::invalid_param("提示词不能为空"));
    }
    if req.n < 1 {
        return Err(ImageGenError::invalid_param("张数至少为 1"));
    }
    cfg.provider(&req.provider_id)
        .ok_or_else(|| ImageGenError::not_configured("请先选择服务商"))?;
    cfg.model(&req.provider_id, &req.model).ok_or_else(|| {
        ImageGenError::not_configured("请先选择模型，或在服务商设置中为该服务添加模型")
    })?;
    Ok(())
}

/// 生图准入校验的**唯一入口**：解析服务商/模型，并按协议解释厂商约束（张数上限/提示词长度/
/// 尺寸规则）。`generate` 与回归测试都经此函数——删掉校验必然触发断言失败（结构性覆盖）。
pub(crate) fn validate_for_generate(cfg: &Config, req: &GenRequest) -> Result<(), ImageGenError> {
    let provider = cfg
        .provider(&req.provider_id)
        .cloned()
        .ok_or_else(|| ImageGenError::not_configured("请先选择服务商"))?;
    let model = cfg
        .model(&req.provider_id, &req.model)
        .ok_or_else(|| ImageGenError::not_configured("请先选择模型"))?;
    // 用探针副本补齐 base_url 与能力快照，使探针与 generate 实跑同形（validate 的 caps 走
    // 形参、真正读 req.caps() 的是适配器组装请求体，同形避免两套输入）。
    let mut probe = req.clone();
    probe.base_url = provider.base_url.clone();
    probe.caps = Some(model.capabilities.clone());
    crate::image_ai_providers::for_kind(provider.kind).validate(&probe, &model.capabilities)
}

// ===================== 单元测试：两层准入校验 ==================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_ai::types::GenErrorKind;

    /// 造一条合规请求：openai / gpt-image-1 / n=2 / 预设尺寸。
    fn gen_req() -> GenRequest {
        GenRequest {
            provider_id: "openai".into(),
            model: "gpt-image-1".into(),
            prompt: "一张脸".into(),
            size: "1024x1024".into(),
            n: 2,
            quality: Some("high".into()),
            ..crate::image_ai::test_req_base()
        }
    }

    #[test]
    fn validation_rejects_missing_fields_and_vendor_limits() {
        // 两层准入校验：validate_complete 管请求自身完整性（提示词/张数/服务商与模型存在）；
        // validate_for_generate 由 Provider 层解释厂商约束——结构性覆盖：generate 不再调用
        // 它、或它不再委派 for_kind(...).validate，断言即失败。
        let cfg = Config::builtin();
        let mut r = gen_req();
        r.prompt = " ".into();
        assert_eq!(
            validate_complete(&cfg, &r).unwrap_err().kind,
            GenErrorKind::InvalidParam
        );
        r = gen_req();
        r.provider_id = "nope".into();
        assert_eq!(
            validate_complete(&cfg, &r).unwrap_err().kind,
            GenErrorKind::NotConfigured
        );
        r = gen_req();
        r.model = "nope-model".into();
        assert_eq!(
            validate_complete(&cfg, &r).unwrap_err().kind,
            GenErrorKind::NotConfigured
        );
        r = gen_req();
        r.n = 0;
        assert_eq!(
            validate_complete(&cfg, &r).unwrap_err().kind,
            GenErrorKind::InvalidParam
        );
        // 厂商约束（张数上限/提示词长度/尺寸）不在此层：合规请求一律放行。
        assert!(validate_complete(&cfg, &gen_req()).is_ok());
        // 张数超上限（openai/gpt-image-1 maxN=10）→ InvalidParam；未知服务商 → NotConfigured。
        let mut r = gen_req();
        r.n = 99;
        assert_eq!(
            validate_for_generate(&cfg, &r).unwrap_err().kind,
            GenErrorKind::InvalidParam
        );
        let mut r = gen_req();
        r.provider_id = "nope".into();
        assert_eq!(
            validate_for_generate(&cfg, &r).unwrap_err().kind,
            GenErrorKind::NotConfigured
        );
        // 合规请求放行。
        assert!(validate_for_generate(&cfg, &gen_req()).is_ok());
    }
}
