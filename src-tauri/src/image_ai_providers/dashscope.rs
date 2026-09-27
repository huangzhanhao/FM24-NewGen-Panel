//! 百炼 / DashScope 兼容平台适配器，请求写法逐字对齐官方 HTTP 模板：
//!
//! - **异步**（`wanx*` 系列、初代 `qwen-image`/`qwen-image-plus`）：
//!   `POST {base}/services/aigc/text2image/image-synthesis`，必带 header
//!   `X-DashScope-Async: enable` → 取 `output.task_id` → 每 **10s** 轮询
//!   `GET {base}/tasks/{task_id}`（官方建议间隔）→ `output.results[].url`。
//! - **同步**（`qwen-image-3.0/2.0/max` 等）：
//!   `POST {base}/services/aigc/multimodal-generation/generation`，
//!   输入为 `input.messages[user].content[text]` → `output.choices[].message.content[].image`。
//! - 两类结果 URL 均 **24h 有效，立即下载**为字节。
//!
//! `base` 默认阿里云直连地址，可由服务商 base_url 覆盖（千问 AI 平台等代理同样适用）。
//! 协议全部拆为纯函数（体组装 / 路由 / 解析），单测不触网。

use super::{FetchedImage, ImageProvider, ProviderOutcome};
use crate::image_ai::GenRequest;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// 阿里云直连默认基址（国内）。服务商配置了 base_url（如千问 AI 平台
/// `https://maas.qianwenaiapi.com/api/v1`）时以配置为准。
const DEFAULT_BASE: &str = "https://dashscope.aliyuncs.com/api/v1";

/// 解析基址：空白配置回落默认值，统一去掉结尾 '/'（官方 SDK base_http_api_url 口径）。
pub(crate) fn base(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    if b.is_empty() {
        DEFAULT_BASE.into()
    } else {
        b.to_string()
    }
}

/// 异步提交地址。
pub(crate) fn submit_url(base: &str) -> String {
    format!("{base}/services/aigc/text2image/image-synthesis")
}

/// 同步（多模态消息式）地址。
pub(crate) fn sync_url(base: &str) -> String {
    format!("{base}/services/aigc/multimodal-generation/generation")
}

/// 轮询地址：`{base}/tasks/{task_id}`。
pub(crate) fn task_url(base: &str, task_id: &str) -> String {
    format!("{base}/tasks/{task_id}")
}

/// 调用形态：官方按模型把文生图拆成两条接口。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CallMode {
    /// text2image 异步任务（submit → poll）。
    Async,
    /// multimodal-generation 同步消息式调用。
    Sync,
}

/// 按官方文档路由模型：wanx 全系列与初代 qwen-image/qwen-image-plus 只支持异步端点；
/// qwen-image-3.0/2.0/max 等其余图像模型走同步端点。
pub(crate) fn call_mode(model: &str) -> CallMode {
    match model {
        "qwen-image" | "qwen-image-plus" => CallMode::Async,
        m if m.starts_with("wanx") => CallMode::Async,
        _ => CallMode::Sync,
    }
}

/// 尺寸显示用的 'x' 统一转为厂商文档口径的 '*'（1024x1024 → 1024*1024；已是 '*' 不动）。
fn vendor_size(size: &str) -> String {
    size.trim().replace('x', "*")
}

/// 组装异步提交请求体——与官方 cURL 模板同字段：
/// `{"model","input":{"prompt"},"parameters":{"negative_prompt","size","n",
/// "prompt_extend":true,"watermark":false,["seed"]}}`。
pub(crate) fn build_async_body(req: &GenRequest) -> Value {
    use serde_json::json;
    let mut params = json!({
        "negative_prompt": req.negative_prompt,
        "size": vendor_size(&req.size),
        "n": req.n,
        "prompt_extend": true, // 先由模型智能扩写提示词（官方默认值）
        "watermark": false,
    });
    if let Some(seed) = req.seed {
        params["seed"] = json!(seed);
    }
    json!({
        "model": req.model,
        "input": {"prompt": req.prompt},
        "parameters": params,
    })
}

/// 组装同步请求体——官方多模态模板：输入必须是单条 user 消息、content 内一个 text 对象；
/// `parameters` 含 negative_prompt/prompt_extend/watermark/size/n（seed 可选）。
pub(crate) fn build_sync_body(req: &GenRequest) -> Value {
    use serde_json::json;
    let mut params = json!({
        "negative_prompt": req.negative_prompt,
        "prompt_extend": true,
        "watermark": false,
        "size": vendor_size(&req.size),
        "n": req.n,
    });
    if let Some(seed) = req.seed {
        params["seed"] = json!(seed);
    }
    json!({
        "model": req.model,
        "input": {
            "messages": [
                {"role": "user", "content": [{"text": req.prompt}]}
            ]
        },
        "parameters": params,
    })
}

/// 提交响应取 `output.task_id`；失败（顶层 code/message）返回中文错误。
pub(crate) fn parse_task_id(resp: &Value) -> Result<String, String> {
    if let Some(tid) = resp
        .get("output")
        .and_then(|o| o.get("task_id"))
        .and_then(|t| t.as_str())
    {
        if !tid.is_empty() {
            return Ok(tid.to_string());
        }
    }
    Err(error_detail(resp, "提交失败，未取得 task_id"))
}

/// 一次轮询的解析结果。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PollResult {
    Pending,
    Running,
    Succeeded(Vec<String>),
    /// 失败描述（已拼入厂商 code 与 message）。
    Failed(String),
}

/// 解析轮询响应：task_status ∈ PENDING/RUNNING/SUCCEEDED/FAILED（另兼容 CANCELED/UNKNOWN）；
/// 成功取 results[].url，失败取 output.code/message；顶层 code 为接口级错误。
pub(crate) fn parse_poll(resp: &Value) -> Result<PollResult, String> {
    // 接口级错误（如鉴权失败）：code/message 在顶层。
    if let Some(code) = resp.get("code").and_then(|c| c.as_str()) {
        if !code.is_empty() {
            let msg = resp
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("未知错误");
            return Ok(PollResult::Failed(format!("{code}: {msg}")));
        }
    }
    let output = resp.get("output").ok_or("响应缺少 output 对象")?;
    let status = output
        .get("task_status")
        .and_then(|s| s.as_str())
        .ok_or("响应缺少 task_status")?;
    match status {
        "PENDING" | "UNKNOWN" => Ok(PollResult::Pending),
        "RUNNING" => Ok(PollResult::Running),
        "SUCCEEDED" => {
            let mut urls = Vec::new();
            if let Some(items) = output.get("results").and_then(|r| r.as_array()) {
                for item in items {
                    if let Some(url) = item.get("url").and_then(|u| u.as_str()) {
                        urls.push(url.to_string());
                    }
                }
            }
            if urls.is_empty() {
                return Err("任务成功但 results 中没有图片 URL".into());
            }
            Ok(PollResult::Succeeded(urls))
        }
        "FAILED" => Ok(PollResult::Failed(error_detail(output, "未知错误"))),
        // CANCELED 是终态：按失败上报，不再空转等待。
        "CANCELED" => Ok(PollResult::Failed("CANCELED: 任务已取消".into())),
        other => Err(format!("未知 task_status: {other}")),
    }
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
            let urls = match call_mode(&req.model) {
                // ---------- 异步：submit → poll ----------
                CallMode::Async => {
                    let resp = client
                        .post(submit_url(&base))
                        .bearer_auth(key)
                        .header("X-DashScope-Async", "enable") // 异步开关必须显式打开
                        .json(&build_async_body(req))
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
                            error_detail(&json, "提交失败")
                        ));
                    }
                    let task_id = parse_task_id(&json)?;

                    // 官方建议每 10s 轮询一次；总等待上限 180s。
                    let interval = Duration::from_secs(10);
                    let deadline = Instant::now() + Duration::from_secs(180);
                    loop {
                        let resp = client
                            .get(task_url(&base, &task_id))
                            .bearer_auth(key)
                            .send()
                            .await
                            .map_err(|e| format!("轮询请求失败: {e}"))?;
                        let poll_status = resp.status();
                        let json: Value = resp
                            .json()
                            .await
                            .map_err(|e| format!("轮询响应解析失败: {e}"))?;
                        if !poll_status.is_success() {
                            return Err(format!(
                                "HTTP {poll_status}：{}",
                                error_detail(&json, "轮询失败")
                            ));
                        }
                        match parse_poll(&json)? {
                            PollResult::Pending | PollResult::Running => {
                                if Instant::now() + interval > deadline {
                                    return Err(format!(
                                        "百炼任务轮询超时（超过 180s），task_id: {task_id}"
                                    ));
                                }
                                // tauri async_runtime 未直接再导出 sleep（无新依赖前提下），
                                // 在 blocking 线程池上等 10s，不阻塞异步 worker。
                                tauri::async_runtime::spawn_blocking(move || {
                                    std::thread::sleep(interval)
                                })
                                .await
                                .map_err(|e| format!("轮询等待失败: {e}"))?;
                            }
                            PollResult::Succeeded(urls) => break urls,
                            PollResult::Failed(msg) => return Err(format!("百炼任务失败：{msg}")),
                        }
                    }
                }
                // ---------- 同步：一次调用直接出图 ----------
                CallMode::Sync => {
                    let resp = client
                        .post(sync_url(&base))
                        .bearer_auth(key)
                        .json(&build_sync_body(req))
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
                    parse_sync(&json)?
                }
            };

            let images = download_all(client, urls).await?;
            Ok(ProviderOutcome {
                images,
                real_usage: None,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen_req() -> GenRequest {
        GenRequest {
            provider_id: "dashscope".into(),
            base_url: String::new(),
            model: "wanx2.1-t2i-turbo".into(),
            prompt: "一张脸".into(),
            negative_prompt: String::new(),
            size: "1024x1024".into(),
            n: 2,
            quality: None,
            seed: None,
            steps: None,
            uid: None,
            supports_response_format: false,
        }
    }

    #[test]
    fn base_falls_back_and_trims_slash() {
        assert_eq!(base("   "), DEFAULT_BASE);
        assert_eq!(
            base("https://maas.qianwenaiapi.com/api/v1/"),
            "https://maas.qianwenaiapi.com/api/v1"
        );
    }

    #[test]
    fn routes_models_per_official_endpoints() {
        assert_eq!(call_mode("wanx2.1-t2i-turbo"), CallMode::Async);
        assert_eq!(call_mode("qwen-image"), CallMode::Async);
        assert_eq!(call_mode("qwen-image-plus"), CallMode::Async);
        assert_eq!(call_mode("qwen-image-3.0-pro"), CallMode::Sync);
        assert_eq!(call_mode("qwen-image-2.0"), CallMode::Sync);
        assert_eq!(call_mode("qwen-image-max"), CallMode::Sync);
    }

    #[test]
    fn async_body_matches_official_template() {
        let b = build_async_body(&gen_req());
        assert_eq!(b["model"], "wanx2.1-t2i-turbo");
        assert_eq!(b["input"]["prompt"], "一张脸");
        // size：x → *
        assert_eq!(b["parameters"]["size"], "1024*1024");
        assert_eq!(b["parameters"]["n"], 2);
        assert_eq!(b["parameters"]["prompt_extend"], true);
        assert_eq!(b["parameters"]["watermark"], false);
        // 官方模板里 negative_prompt 恒在；空串即空串。
        assert_eq!(b["parameters"]["negative_prompt"], "");
        assert!(b["parameters"].get("seed").is_none());
    }

    #[test]
    fn async_body_keeps_star_size_and_includes_seed() {
        let mut r = gen_req();
        r.size = "1024*1024".into(); // 已是 * 形态：不得被破坏
        r.seed = Some(7);
        r.negative_prompt = "模糊".into();
        let b = build_async_body(&r);
        assert_eq!(b["parameters"]["size"], "1024*1024");
        assert_eq!(b["parameters"]["seed"], 7);
        assert_eq!(b["parameters"]["negative_prompt"], "模糊");
    }

    #[test]
    fn sync_body_uses_messages_shape() {
        let mut r = gen_req();
        r.model = "qwen-image-3.0".into();
        r.size = "350x350".into();
        let b = build_sync_body(&r);
        assert_eq!(b["model"], "qwen-image-3.0");
        assert_eq!(b["input"]["messages"][0]["content"][0]["text"], "一张脸");
        assert_eq!(b["input"]["messages"][0]["role"], "user");
        assert_eq!(b["parameters"]["size"], "350*350");
        assert_eq!(b["parameters"]["watermark"], false);
        assert_eq!(b["parameters"]["n"], 2);
    }

    #[test]
    fn submit_response_yields_task_id() {
        let v: Value =
            serde_json::from_str(r#"{"output":{"task_id":"task-abc","task_status":"PENDING"}}"#)
                .unwrap();
        assert_eq!(parse_task_id(&v).unwrap(), "task-abc");
    }

    #[test]
    fn submit_error_reports_code_and_message() {
        let v: Value = serde_json::from_str(
            r#"{"code":"InvalidApiKey","message":"Invalid API-key provided","request_id":"x"}"#,
        )
        .unwrap();
        assert_eq!(
            parse_task_id(&v).unwrap_err(),
            "InvalidApiKey: Invalid API-key provided"
        );
    }

    #[test]
    fn poll_statuses_including_unknown_and_canceled() {
        let v: Value =
            serde_json::from_str(r#"{"output":{"task_id":"t","task_status":"PENDING"}}"#).unwrap();
        assert_eq!(parse_poll(&v).unwrap(), PollResult::Pending);
        let v: Value =
            serde_json::from_str(r#"{"output":{"task_id":"t","task_status":"UNKNOWN"}}"#).unwrap();
        assert_eq!(parse_poll(&v).unwrap(), PollResult::Pending);
        let v: Value =
            serde_json::from_str(r#"{"output":{"task_id":"t","task_status":"CANCELED"}}"#).unwrap();
        assert_eq!(
            parse_poll(&v).unwrap(),
            PollResult::Failed("CANCELED: 任务已取消".into())
        );
    }

    #[test]
    fn poll_succeeded_extracts_result_urls() {
        let v: Value = serde_json::from_str(
            r#"{"output":{"task_status":"SUCCEEDED","results":[{"url":"https://r/1.png"},{"url":"https://r/2.png"}]}}"#,
        )
        .unwrap();
        match parse_poll(&v).unwrap() {
            PollResult::Succeeded(urls) => {
                assert_eq!(urls, vec!["https://r/1.png", "https://r/2.png"]);
            }
            other => panic!("应为 Succeeded，实际 {other:?}"),
        }
    }

    #[test]
    fn poll_failed_extracts_code_and_message() {
        let v: Value = serde_json::from_str(
            r#"{"output":{"task_status":"FAILED","code":"ContentFiltered","message":"内容审核不通过"}}"#,
        )
        .unwrap();
        assert_eq!(
            parse_poll(&v).unwrap(),
            PollResult::Failed("ContentFiltered: 内容审核不通过".into())
        );
    }

    #[test]
    fn poll_treats_top_level_code_as_failure() {
        let v: Value = serde_json::from_str(
            r#"{"code":"InvalidApiKey","message":"key bad","request_id":"r"}"#,
        )
        .unwrap();
        assert_eq!(
            parse_poll(&v).unwrap(),
            PollResult::Failed("InvalidApiKey: key bad".into())
        );
    }

    #[test]
    fn sync_response_extracts_image_urls() {
        let v: Value = serde_json::from_str(
            r#"{"output":{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":[{"image":"https://r/1.png"}]}}]}}"#,
        )
        .unwrap();
        assert_eq!(parse_sync(&v).unwrap(), vec!["https://r/1.png"]);
    }

    #[test]
    fn sync_error_uses_top_level_code() {
        let v: Value =
            serde_json::from_str(r#"{"code":"BadRequest","message":"size invalid"}"#).unwrap();
        assert_eq!(parse_sync(&v).unwrap_err(), "BadRequest: size invalid");
    }

    #[test]
    fn urls_resolve_from_given_base() {
        assert_eq!(
            submit_url("https://maas.qianwenaiapi.com/api/v1"),
            "https://maas.qianwenaiapi.com/api/v1/services/aigc/text2image/image-synthesis"
        );
        assert_eq!(
            task_url(DEFAULT_BASE, "task-abc"),
            "https://dashscope.aliyuncs.com/api/v1/tasks/task-abc"
        );
    }
}
