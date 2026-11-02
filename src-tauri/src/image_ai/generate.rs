//! 生成编排：准入校验 → 适配器取字节 → 四角裁圆角 → 临时文件落盘 → 画廊记录。

use rand::Rng;
use std::fs;
use std::path::Path;
use std::time::Duration;

use super::config::{tmp_dir, Config};
use super::gallery::{enforce_image_limit, record_gallery};
use super::postprocess::round_corners;
use super::types::{CancelToken, GenImage, GenRequest, GenResult, GenStage, ImageGenError};
use super::validate::validate_for_generate;

/// 临时图片/画廊记录共用的随机 id：8 字节十六进制（16 字符）。
pub fn new_id() -> String {
    let bytes: [u8; 8] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 执行一次生成：解析服务商/模型 → 唯一准入校验 → 适配器取回图片字节 → 四角裁圆角（恒 PNG）
/// → 落盘 `.imgtmp` → 写画廊记录并按上限裁剪。记录写入与图片裁剪失败只告警，不掩盖已成功落盘的生成。
///
/// # 参数
/// - `data_dir`：应用数据目录（`.imgtmp` 及其中的 `gallery.json` 都在其下）；
/// - `cfg`：生效配置（服务商/模型与两道淘汰闸门上限取自此处）；
/// - `key`：API Key 明文，只在本会话内存（密文随服务商行落盘；本地 ComfyUI 免鉴权，传空串）；
/// - `req`：生成请求；本函数回填内部字段 base_url、caps 并门控 call_mode；
/// - `progress`：阶段回调（提交/轮询/下载），适配器按序上报、本层只透传；
/// - `cancel`：取消令牌，**只用于拦「尚未提交」的请求**——本层在发 HTTP 之前查一次，
///   命中即返回 `Cancelled`、一个请求都不发。
///
/// 【取消粒度：已提交必等结果】请求一旦发出（服务端已计费），本轮不再中断：等适配器返回、
/// 结果照常落盘并记入画廊。半路丢弃只会得到「钱花了、图也没留下」——比不做更糟。
pub async fn generate(
    data_dir: &Path,
    cfg: &Config,
    key: &str,
    mut req: GenRequest,
    progress: &(dyn Fn(GenStage) + Sync),
    cancel: &CancelToken,
) -> Result<GenResult, ImageGenError> {
    let (provider, model) = {
        let provider = cfg
            .provider(&req.provider_id)
            .cloned()
            .ok_or_else(|| ImageGenError::not_configured("请先选择服务商"))?;
        let model = cfg
            .model(&req.provider_id, &req.model)
            .ok_or_else(|| ImageGenError::not_configured("请先选择模型"))?;
        (provider, model)
    };
    // 协议适配所需的服务商/能力信息随请求内部字段带入适配器（前端无需传）。
    req.base_url = provider.base_url.clone();
    // 能力位整体拷一份快照：新增能力位只改 Capabilities 与消费方，这里不再逐行维护。
    req.caps = Some(model.capabilities.clone());

    // 调用方式门控：显式选异步且模型支持时才置 "async"，否则回落同步（校验与 HTTP 均据此分流）。
    req.call_mode =
        if req.call_mode.as_deref() == Some("async") && model.capabilities.supports_async {
            Some("async".into())
        } else {
            None
        };

    // 【唯一准入校验入口】厂商约束在此解释，失败即返回、绝不发 HTTP；抽成独立函数便于回归测试锚定「确实调用了校验」。
    validate_for_generate(cfg, &req)?;
    // 取消的唯一检查点（本层）：此刻一个请求都还没发出，取消 = 直接不发、零计费。
    if cancel.is_cancelled() {
        return Err(ImageGenError::cancelled());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // 百炼同步长连接（千问 3.0 多图/思考模式官方建议客户端超时 600s 起）；client 各家共用，
        // 放宽上限不影响本就秒级完成的 OpenAI/火山。
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| ImageGenError::network(format!("HTTP 客户端初始化失败: {e}")))?;
    let adapter = crate::image_ai_providers::for_kind(provider.kind);
    // 适配器内部完成 POST/解析/（异步厂商的 submit→poll→fetch）并回传图片字节；
    // 执行阶段经 progress 回调上报，本层只透传。
    // 【拿到返回值即视为「已提交且已完成」】此处不再查取消：请求已计费，结果必须落盘并记画廊。
    let fetched = adapter.generate(&client, key, &req, progress).await?;

    let tmp = tmp_dir(data_dir);
    fs::create_dir_all(&tmp).map_err(|e| ImageGenError::io(e.to_string()))?;
    let mut images = Vec::new();
    for img in fetched.images {
        let id = new_id();
        // 四角统一裁圆角；解码/编码失败 → 原图原格式落盘（后处理不拖垮生成）。
        // 成功分支恒为 PNG（圆角需要 alpha），故扩展名统一 png。
        let (bytes, ext) = match round_corners(&img.bytes) {
            Ok(png) => (png, "png".to_string()),
            Err(e) => {
                log::warn!(target: "image_ai", "裁角失败（按原图落盘）: {e}");
                (img.bytes, img.extension)
            }
        };
        let path = tmp.join(format!("{id}.{ext}"));
        fs::write(&path, &bytes)
            .map_err(|e| ImageGenError::io(format!("临时图片写入失败: {e}")))?;
        images.push(GenImage {
            id,
            path: path.to_string_lossy().to_string(),
            bytes: bytes.len() as u64,
        });
    }
    // 两条闸门互不越界，失败只告警——不掩盖已成功落盘的生成。
    if let Err(e) = record_gallery(data_dir, &images, &req, cfg.settings.gallery_record_limit) {
        log::warn!(target: "image_ai", "写入画廊记录失败（不影响本次生成）: {e}");
    }
    if let Err(e) = enforce_image_limit(data_dir, cfg.settings.history_limit) {
        log::warn!(target: "image_ai", "裁剪临时图片失败（不影响本次生成）: {e}");
    }
    Ok(GenResult { images })
}

// ===================== 单元测试：取消分界点 ====================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_ai::types::GenErrorKind;

    /// 未提交即取消：发 HTTP 前那次检查命中 ⇒ 直接返回 `Cancelled`，**零计费、零落盘**。
    /// 这是「取消不花钱」的唯一分界点——请求一旦发出即已计费，本次调用必须等结果，
    /// 故本用例锁住「没发出去」这一侧。
    ///
    /// 【用 openai/gpt-image-1】内置合规模型能干净地过 `validate_for_generate`，让用例只
    /// 考察取消那一跳；`key` 传空串即可——本路径走不到发请求那一步（也因此不触网）。
    #[test]
    fn generate_cancelled_before_submit_returns_cancelled_without_touching_disk() {
        // TempDir 必须先绑定到变量再使用（临时目录随句柄 drop 删除）。
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::builtin();
        let req = GenRequest {
            provider_id: "openai".into(),
            model: "gpt-image-1".into(),
            prompt: "一张脸".into(),
            size: "1024x1024".into(),
            ..crate::image_ai::test_req_base()
        };
        // 先置位再调用：模拟「用户在提交前点了取消」。
        let cancel = CancelToken::new();
        cancel.cancel();

        // crate 无 tokio 直接依赖，异步测试统一走 tauri 的 async_runtime。
        let res = tauri::async_runtime::block_on(generate(
            dir.path(),
            &cfg,
            "",
            req,
            &|_stage: GenStage| {},
            &cancel,
        ));

        let err = res.expect_err("已取消的请求不该返回成功结果");
        assert_eq!(err.kind, GenErrorKind::Cancelled);

        // 【怎么证明一个请求都没发出去】走磁盘痕迹反证：gallery.json 只在拿到图片字节后才写
        //（record_gallery），.imgtmp 也在 `adapter.generate(...)` 返回后才 create_dir_all。
        // 故「无 gallery.json 且 .imgtmp 不存在/为空」⇒ 执行链没越过 generate 的成功分支，
        // 也不可能发生过 HTTP 提交（本用例全程无网）。
        assert!(
            !crate::image_ai::gallery_path(dir.path()).exists(),
            "未提交即取消不该留下画廊记录"
        );
        let tmp = tmp_dir(dir.path());
        assert!(
            !tmp.exists() || fs::read_dir(&tmp).unwrap().next().is_none(),
            "未提交即取消不该落任何临时图片"
        );
    }
}
