//! 百炼通义万相（DashScope）适配器：异步任务模式 submit → poll → fetch。
//!
//! - 提交：POST text2image/image-synthesis，必须带 header `X-DashScope-Async: enable`；
//! - 轮询：GET tasks/{task_id}，按 `output.task_status` 流转；
//! - 成功：`output.results[].url` **24h 有效，立即下载**为字节；
//! - 退避：3s 起步 ×1.5，总超时 180s。
//!
//! 全部协议拆为纯函数（body 组装 / task_id / 轮询解析），单测不触网。

use super::{FetchedImage, ImageProvider, ProviderOutcome};
use crate::image_ai::GenRequest;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// 提交 URL（异步文生图）。
const SUBMIT_URL: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/aigc/text2image/image-synthesis";

/// 轮询 URL：`/api/v1/tasks/{task_id}`。
pub(crate) fn task_url(task_id: &str) -> String {
    format!("https://dashscope.aliyuncs.com/api/v1/tasks/{task_id}")
}

/// 组装提交请求体：
/// `{"model","input":{"prompt"},"parameters":{"size"(x→*)、"n"、"seed"(Some 才带)、
/// "negative_prompt"(非空才带)、"prompt_extend":true}}`。
pub(crate) fn build_submit_body(req: &GenRequest) -> serde_json::Value {
    use serde_json::json;
    // 适配器把尺寸显示用的 'x' 转为厂商文档口径的 '*'（1024x1024 → 1024*1024）。
    let size = req.size.replace('x', "*");
    let mut params = json!({
        "size": size,
        "n": req.n,
        "prompt_extend": true, // 先由模型智能扩写提示词
    });
    let obj = params.as_object_mut().unwrap();
    if let Some(seed) = req.seed {
        obj.insert("seed".into(), json!(seed));
    }
    if !req.negative_prompt.trim().is_empty() {
        obj.insert("negative_prompt".into(), json!(req.negative_prompt));
    }
    json!({
        "model": req.model,
        "input": {"prompt": req.prompt},
        "parameters": params,
    })
}

/// 提交响应取 `output.task_id`；失败（顶层 code/message）返回中文错误。
pub(crate) fn parse_task_id(resp: &serde_json::Value) -> Result<String, String> {
    if let Some(tid) = resp
        .get("output")
        .and_then(|o| o.get("task_id"))
        .and_then(|t| t.as_str())
    {
        if !tid.is_empty() {
            return Ok(tid.to_string());
        }
    }
    let code = resp.get("code").and_then(|c| c.as_str()).unwrap_or("SubmitError");
    let msg = resp
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("提交失败，未取得 task_id");
    Err(format!("{code}: {msg}"))
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

/// 解析轮询响应：task_status ∈ PENDING/RUNNING/SUCCEEDED/FAILED；
/// 成功取 results[].url，失败取 output.code/message；顶层 code 为接口级错误。
pub(crate) fn parse_poll(resp: &serde_json::Value) -> Result<PollResult, String> {
    // 接口级错误（如鉴权失败）：code/message 在顶层。
    if let Some(code) = resp.get("code").and_then(|c| c.as_str()) {
        if !code.is_empty() {
            let msg = resp.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误");
            return Ok(PollResult::Failed(format!("{code}: {msg}")));
        }
    }
    let output = resp.get("output").ok_or("响应缺少 output 对象")?;
    let status = output
        .get("task_status")
        .and_then(|s| s.as_str())
        .ok_or("响应缺少 task_status")?;
    match status {
        "PENDING" => Ok(PollResult::Pending),
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
        "FAILED" => {
            let code = output.get("code").and_then(|c| c.as_str()).unwrap_or("未知错误码");
            let msg = output.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误");
            Ok(PollResult::Failed(format!("{code}: {msg}")))
        }
        other => Err(format!("未知 task_status: {other}")),
    }
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
            // 1) submit：异步开关必须在 header 显式打开。
            let resp = client
                .post(SUBMIT_URL)
                .bearer_auth(key)
                .header("X-DashScope-Async", "enable")
                .json(&build_submit_body(req))
                .send()
                .await
                .map_err(|e| format!("请求发送失败: {e}"))?;
            let status = resp.status();
            let json: serde_json::Value =
                resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
            if !status.is_success() {
                let code = json.get("code").and_then(|c| c.as_str()).unwrap_or("SubmitError");
                let msg = json.get("message").and_then(|m| m.as_str()).unwrap_or("提交失败");
                return Err(format!("HTTP {status}：{code}: {msg}"));
            }
            let task_id = parse_task_id(&json)?;

            // 2) poll：3s 起步 ×1.5，总超时 180s。
            let deadline = Instant::now() + Duration::from_secs(180);
            let mut delay = Duration::from_secs(3);
            let urls = loop {
                let resp = client
                    .get(task_url(&task_id))
                    .bearer_auth(key)
                    .send()
                    .await
                    .map_err(|e| format!("轮询请求失败: {e}"))?;
                let poll_status = resp.status();
                let json: serde_json::Value =
                    resp.json().await.map_err(|e| format!("轮询响应解析失败: {e}"))?;
                if !poll_status.is_success() {
                    let code =
                        json.get("code").and_then(|c| c.as_str()).unwrap_or("PollError");
                    let msg =
                        json.get("message").and_then(|m| m.as_str()).unwrap_or("轮询失败");
                    return Err(format!("HTTP {poll_status}：{code}: {msg}"));
                }
                match parse_poll(&json)? {
                    PollResult::Pending | PollResult::Running => {
                        if Instant::now() + delay > deadline {
                            return Err(format!(
                                "百炼任务轮询超时（超过 180s），task_id: {task_id}"
                            ));
                        }
                        // tauri 2.11 的 async_runtime 未直接再导出 sleep（无新依赖前提下），
                        // 用 blocking 线程池上的 std sleep 等价实现异步等待，不阻塞 worker。
                        tauri::async_runtime::spawn_blocking(move || std::thread::sleep(delay))
                            .await
                            .map_err(|e| format!("轮询等待失败: {e}"))?;
                        delay = Duration::from_secs_f64(
                            (delay.as_secs_f64() * 1.5).min(180.0),
                        );
                    }
                    PollResult::Succeeded(urls) => break urls,
                    PollResult::Failed(msg) => {
                        return Err(format!("百炼任务失败：{msg}"));
                    }
                }
            };

            // 3) fetch：结果 URL 24h 有效，此刻立即下载。
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
                images.push(FetchedImage { bytes, extension: "png".into() });
            }
            Ok(ProviderOutcome { images, real_usage: None })
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
    fn body_converts_size_and_defaults_optional_fields() {
        let b = build_submit_body(&gen_req());
        assert_eq!(b["model"], "wanx2.1-t2i-turbo");
        assert_eq!(b["input"]["prompt"], "一张脸");
        // size：x → *
        assert_eq!(b["parameters"]["size"], "1024*1024");
        assert_eq!(b["parameters"]["n"], 2);
        assert_eq!(b["parameters"]["prompt_extend"], true);
        // seed / negative_prompt 缺省不带。
        assert!(b["parameters"].get("seed").is_none());
        assert!(b["parameters"].get("negative_prompt").is_none());
    }

    #[test]
    fn body_keeps_star_size_and_includes_seed_and_negative() {
        let mut r = gen_req();
        r.size = "1024*1024".into(); // UI 已可能直接传 * 形态：不得被破坏
        r.seed = Some(7);
        r.negative_prompt = "模糊".into();
        let b = build_submit_body(&r);
        assert_eq!(b["parameters"]["size"], "1024*1024");
        assert_eq!(b["parameters"]["seed"], 7);
        assert_eq!(b["parameters"]["negative_prompt"], "模糊");
    }

    #[test]
    fn body_omits_whitespace_negative_prompt() {
        let mut r = gen_req();
        r.negative_prompt = "   ".into();
        let b = build_submit_body(&r);
        assert!(b["parameters"].get("negative_prompt").is_none());
    }

    #[test]
    fn submit_response_yields_task_id() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"output":{"task_id":"task-abc","task_status":"PENDING"}}"#)
                .unwrap();
        assert_eq!(parse_task_id(&v).unwrap(), "task-abc");
    }

    #[test]
    fn submit_error_reports_code_and_message() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"code":"InvalidApiKey","message":"Invalid API-key provided","request_id":"x"}"#,
        )
        .unwrap();
        assert_eq!(
            parse_task_id(&v).unwrap_err(),
            "InvalidApiKey: Invalid API-key provided"
        );
    }

    #[test]
    fn poll_pending_and_running_parse() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"output":{"task_id":"t","task_status":"PENDING"}}"#)
                .unwrap();
        assert_eq!(parse_poll(&v).unwrap(), PollResult::Pending);
        let v: serde_json::Value =
            serde_json::from_str(r#"{"output":{"task_id":"t","task_status":"RUNNING"}}"#)
                .unwrap();
        assert_eq!(parse_poll(&v).unwrap(), PollResult::Running);
    }

    #[test]
    fn poll_succeeded_extracts_result_urls() {
        let v: serde_json::Value = serde_json::from_str(
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
        let v: serde_json::Value = serde_json::from_str(
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
        let v: serde_json::Value = serde_json::from_str(
            r#"{"code":"InvalidApiKey","message":"key bad","request_id":"r"}"#,
        )
        .unwrap();
        assert_eq!(
            parse_poll(&v).unwrap(),
            PollResult::Failed("InvalidApiKey: key bad".into())
        );
    }

    #[test]
    fn task_url_contains_id() {
        assert_eq!(
            task_url("task-abc"),
            "https://dashscope.aliyuncs.com/api/v1/tasks/task-abc"
        );
    }
}
