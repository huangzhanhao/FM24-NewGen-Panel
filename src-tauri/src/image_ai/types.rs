//! AI 文生图的数据契约（L0）：IPC/落盘共用的 serde 类型、错误分类与 serde 辅助。
//!
//! 【分层】本层只放类型与纯函数，不碰文件系统、不碰网络、不做配置归并——
//! 上层（config/gallery/keys/prompt/validate/generate/gen_queue）依赖本层，本层不反向依赖任何兄弟模块。
//! 与前端 `types.ts` 一一对应，字段一律 camelCase、枚举一律小写/snake_case。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ===================== 数据契约（与前端 types.ts 一一对应，camelCase） ===============================================

// ---- 服务商类型 -----------------------------------------------------------------------------------------------------

/// 服务商类型：决定适配器。OpenAI 兼容服务与官方 OpenAI 请求/响应同构（差异仅在 base_url
/// 与模型名），全仓无一处按「是否官方」分流；兼容服务走 OpenAI 档、改 base_url 即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAI,       // OpenAI 官方 Images 协议（第三方 OpenAI 兼容服务同走此档，改 base_url 即可）
    Volcano,      // 火山方舟豆包 Seedream（同步 Bearer，响应与 OpenAI 同构）
    Dashscope,    // 阿里云百炼：千问 Qwen-Image / 万相 Wan / Z-Image（模型目录代码内置）
    Siliconflow,  // 硅基流动（同步 Bearer，images[].url，尺寸字段 image_size）
    Modelscope,   // 魔搭 ModelScope（强制异步：提交任务 → 轮询 /tasks/{id}）
    LocalComfyUI, // 本地 ComfyUI（http://127.0.0.1:8188，无鉴权，提交原生节点工作流）
}

impl ProviderKind {
    /// 该协议是否需要 API Key。本地 ComfyUI 无鉴权；后端两道 Key 门
    /// （generate_image / start_queue）与前端就绪度判据同用本口径。
    pub fn needs_key(self) -> bool {
        !matches!(self, ProviderKind::LocalComfyUI)
    }
}

// ---- 生成阶段（适配器上报、队列透传） -------------------------------------------------------------------------------

/// 生成阶段：适配器在执行链上按序上报（同步两段 Submitting→Downloading；异步三段
/// Submitting→Polling→Downloading），供队列把「当前项正在做什么」推给前端。
/// 【为什么放在契约层】阶段值直接进 `QueueProgress.stage`（IPC 字段）；定义在适配器族里会
/// 逼应用层（generate / gen_queue）反向依赖适配器，与 `types` 形成环。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GenStage {
    Submitting,
    Polling,
    Downloading,
}

impl GenStage {
    /// 前端契约字面量（对应 `QueueProgress.stage`）。
    pub fn as_str(self) -> &'static str {
        match self {
            GenStage::Submitting => "submitting",
            GenStage::Polling => "polling",
            GenStage::Downloading => "downloading",
        }
    }
}

// ---- 取消令牌（队列 → 执行链的单向信号） ----------------------------------------------------------------------------

/// 取消令牌：队列与执行链共享的「请停止」标志（`Arc<AtomicBool>`，单标志、无负载）。
///
/// 【语义：只拦还没提交的请求】派发前、重试退避结束后、发 HTTP 前各查一次；
/// 请求一旦发出就不再中断——已计费的调用等它返回，结果照常落盘并记画廊。
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// 请求取消（幂等）。
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// 复位（每次 `run_queue` 开跑前清上一次运行遗留的取消）。
    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }

    /// 是否已请求取消（各检查点读一次）。
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

// ---- 生成链错误分类：类别枚举、结构化错误与厂商错误归类 -------------------------------------------------------------

/// 生成链错误类别（snake_case 序列化，与前端 `GenErrorKind` 一一对应）：前端据此给出
/// 处置建议，并判断队列失败条目是否值得重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenErrorKind {
    NotConfigured,   // 服务商/模型/Key 等前置项缺省——用户补全即可，重试无意义
    Auth,            // 鉴权失败（401/403 或业务码 InvalidApiKey）
    RateLimited,     // 限流/配额（429 或业务码 Throttling/QuotaExceeded），错峰重试有意义
    InvalidParam,    // 参数不合法（尺寸/张数/提示词长度等）
    ContentRejected, // 服务商内容审核拦截
    Provider,        // 服务商协议/服务端错误（JSON 解析、结构缺失、5xx 等）
    Network,         // 网络传输失败（连接、发送、读体），可重试
    Io,              // 本地磁盘/目录 I/O
    Cancelled,       // 用户主动取消（队列自造，非服务商返回）
    Internal,        // 内部异常（队列重入拒绝、任务 panic 兜底等）
}

impl GenErrorKind {
    /// 类别默认可重试位；Provider 的 4xx/5xx 结论不同，由构造点显式指定，不查此表。
    fn default_retryable(self) -> bool {
        matches!(self, GenErrorKind::Network | GenErrorKind::RateLimited)
    }

    /// 用户可执行的处置建议；Cancelled/Internal 无动作可给。
    fn user_action(self) -> Option<&'static str> {
        Some(match self {
            GenErrorKind::Auth => "检查 API Key 是否正确或已过期",
            GenErrorKind::RateLimited => "稍后重试或降低并发",
            GenErrorKind::NotConfigured => "到服务商设置填写 Key/模型",
            GenErrorKind::InvalidParam => "按提示调整参数",
            GenErrorKind::ContentRejected => "修改提示词后重试",
            GenErrorKind::Network => "检查网络后重试",
            GenErrorKind::Provider => "稍后重试，持续失败请查看服务商状态",
            GenErrorKind::Io => "检查磁盘空间与目录权限",
            GenErrorKind::Cancelled | GenErrorKind::Internal => return None,
        })
    }
}

/// 生成链结构化错误：kind 供前端分类给建议，message 为构造点提供的中文文案，
/// retryable 决定队列条目是否展示「重试本条」，user_action 为用户可执行动作。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenError {
    /// 错误类别：前端据此分类展示、给出处置建议
    pub kind: GenErrorKind,
    /// 中文错误文案：构造点原样保留，直接展示给用户
    pub message: String,
    /// 是否值得重试：队列据此决定失败条目是否展示「重试本条」
    pub retryable: bool,
    /// 用户可执行的处置建议：按 kind 自动带出，Cancelled/Internal 为 None
    pub user_action: Option<String>,
    /// 服务端 `Retry-After`（秒）：仅 429 响应的该头可解析为整数秒时填，其余一律 None；
    /// 队列重试退避优先采用（见 `image_ai::gen_queue::backoff_delay`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

impl ImageGenError {
    /// 统一构造：user_action 按 kind 自动带出，retry_after 初始为 None。
    fn build(kind: GenErrorKind, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind,
            message: message.into(),
            retryable,
            user_action: kind.user_action().map(str::to_string),
            retry_after: None,
        }
    }

    /// 附加 `Retry-After`（秒），仅适配器读到响应头时调用。
    pub fn with_retry_after(mut self, secs: Option<u64>) -> Self {
        self.retry_after = secs;
        self
    }

    /// 把文案里可能回显的 API Key 抹成 `[redacted]`。
    /// 【为什么需要】厂商 401 响应常把被拒的 Key 抄进 message，而文案会写进
    /// data/newgan.log；用户贴日志求助等于把 Key 公开。Key 含引号/反斜杠时日志里可能是
    /// JSON 转义形态，两种拼写都要换。
    pub fn scrub(&mut self, key: &str) {
        let key = key.trim();
        if key.is_empty() {
            return;
        }
        let escaped = serde_json::to_string(key)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| key.to_string());
        self.message = self.message.replace(key, "[redacted]");
        if escaped != key {
            self.message = self.message.replace(&escaped, "[redacted]");
        }
    }

    /// 按类别构造，retryable 取类别默认（仅 Network/RateLimited 为 true）。
    pub fn new(kind: GenErrorKind, message: impl Into<String>) -> Self {
        Self::build(kind, message, kind.default_retryable())
    }

    /// 构造 NotConfigured 类（前置项缺省，不可重试）。
    pub fn not_configured(message: impl Into<String>) -> Self {
        Self::new(GenErrorKind::NotConfigured, message)
    }

    /// 构造 InvalidParam 类（不可重试）。
    pub fn invalid_param(message: impl Into<String>) -> Self {
        Self::new(GenErrorKind::InvalidParam, message)
    }

    /// 构造 Network 类（默认可重试）。
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(GenErrorKind::Network, message)
    }

    /// 构造 Io 类（不可重试）。
    pub fn io(message: impl Into<String>) -> Self {
        Self::new(GenErrorKind::Io, message)
    }

    /// Provider 类显式指定可重试位（5xx=true，其余 4xx/解析失败=false）。
    pub fn provider(message: impl Into<String>, retryable: bool) -> Self {
        Self::build(GenErrorKind::Provider, message, retryable)
    }

    /// 构造 Cancelled 类，文案固定「已取消」（队列任务被用户取消时用）。
    pub fn cancelled() -> Self {
        Self::new(GenErrorKind::Cancelled, "已取消")
    }

    /// 构造 Internal 类（队列重入拒绝、任务 panic 兜底等内部异常，不可重试）。
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(GenErrorKind::Internal, message)
    }
}

/// 业务错误码 → 类别（优先于 HTTP status）：InvalidApiKey→Auth；
/// Throttling/Quota/限流→RateLimited；DataInspection/IPInfringement→ContentRejected；未命中返回 None。
pub fn kind_from_business_code(code: &str) -> Option<GenErrorKind> {
    let c = code.to_ascii_lowercase();
    if c.contains("invalidapikey") || c.contains("invalid_api_key") {
        return Some(GenErrorKind::Auth);
    }
    if c.contains("throttl")
        || c.contains("quota")
        || c.contains("ratelimit")
        || c.contains("rate_limit")
    {
        return Some(GenErrorKind::RateLimited);
    }
    // 审核类码（DataInspectionFailed 输入/输出违规、IPInfringementSuspect 疑似侵权）的自救
    // 手段都在提示词侧（关 prompt_extend 或改写版权角色名），归 ContentRejected 才能让
    // 用户看到「修改提示词后重试」而非无因由重试。
    if c.contains("datainspection") || c.contains("data_inspection") || c.contains("ipinfringement")
    {
        return Some(GenErrorKind::ContentRejected);
    }
    None
}

/// HTTP 状态码 → 错误类别（须在拼接文案前分流，status 是唯一稳定的分类依据）：
/// 401/403→Auth、429→RateLimited、400→InvalidParam、5xx→Provider(可重试)、其余→Provider(不可重试)。
pub fn from_http_status(status: u16, message: impl Into<String>) -> ImageGenError {
    match status {
        401 | 403 => ImageGenError::new(GenErrorKind::Auth, message),
        429 => ImageGenError::new(GenErrorKind::RateLimited, message),
        400 => ImageGenError::invalid_param(message),
        500..=599 => ImageGenError::provider(message, true),
        _ => ImageGenError::provider(message, false),
    }
}

/// 厂商错误统一归类：业务 code 命中优先，否则按 HTTP status 分流。
pub fn classify_provider_error(
    status: u16,
    code: Option<&str>,
    message: impl Into<String>,
) -> ImageGenError {
    if let Some(kind) = code.and_then(kind_from_business_code) {
        return ImageGenError::new(kind, message);
    }
    from_http_status(status, message)
}

// ---- 模型能力集：SizeRules（自由尺寸规则）/ Capabilities（UI 显隐与适配器发参依据） ---------------------------------

/// 自由尺寸规则：只约束**不在 `sizes` 预设里的自定义值**（见 `validate_model_caps`），每条 0 = 不检查。
/// 尺寸字符串为 `宽x高`（小写 x）/ `宽*高` 形态；各模型区间由内置目录逐模型声明，取值逐字对齐官方文档。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeRules {
    #[serde(default)]
    pub min_side: u32,  // 单边下限像素（宽/高各自，0 = 不检查）
    #[serde(default)]
    pub max_side: u32,  // 单边上限像素（0 = 不检查）
    #[serde(default)]
    pub total_min: u64, // 总像素（宽×高）下限（0 = 不检查）
    #[serde(default)]
    pub total_max: u64, // 总像素上限（0 = 不检查）
    #[serde(default)]
    pub ratio_min: f32, // 宽高比下限（短边/长边口径，如 1:4 = 0.25；0 = 不检查）
    #[serde(default)]
    pub ratio_max: f32, // 宽高比上限（如 4:1 = 4.0；0 = 不检查）
}

/// 模型能力集：UI 按此显隐参数，适配器按此决定是否传参。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// 该模型支持的预设尺寸枚举（如 "1024x1024"、"4K"）；不在枚举内的自定义尺寸另由 size_rules 校验
    #[serde(default)]
    pub sizes: Vec<String>,
    /// 单次请求最大张数（0 = 不声明上限）；缺省 1
    #[serde(default = "one")]
    pub max_n: u32,
    /// 是否支持 quality（画质档）参数，可选值见 qualities
    #[serde(default)]
    pub supports_quality: bool,
    /// 是否支持 seed（随机种子）参数
    #[serde(default)]
    pub supports_seed: bool,
    /// 是否支持 steps（采样步数）参数
    #[serde(default)]
    pub supports_steps: bool,
    /// 采样步数上限（0 = 不校验）；硅基流动 `num_inference_steps` 与魔搭 `steps` 用（均 ≤100）。
    #[serde(default)]
    pub steps_max: u32,
    /// 采样步数默认值（模型声明；前端切模型时据此归位）。
    #[serde(default)]
    pub steps_default: u32,
    /// 引导系数（Classifier-Free Guidance）：硅基流动 `guidance_scale`、魔搭 `guidance`。
    #[serde(default)]
    pub supports_guidance: bool,
    /// 引导系数默认值（硅基流动 Kolors 7.5 / 魔搭 3.5）。
    #[serde(default)]
    pub guidance_default: f32,
    /// 引导系数上限（0 = 不校验）。
    #[serde(default)]
    pub guidance_max: f32,
    /// 是否支持 negative_prompt（反向提示词）参数
    #[serde(default)]
    pub supports_negative_prompt: bool,
    /// 是否可传 `response_format`。官方 gpt-image-1 系列不支持（始终返回 base64，传了会 400）
    /// 故出厂置 false；兼容服务多为默认返回 URL、需显式 b64_json 免下载，故缺省 true。
    #[serde(default = "yes")]
    pub supports_response_format: bool,
    /// 火山 Seedream `output_format`（png/jpeg）：仅 5.0 系列可设；4.5/4.0 只回 jpeg、
    /// 传参报错，故置 false 整条省略。OpenAI/百炼不用此参数。
    #[serde(default)]
    pub supports_output_format: bool,
    /// quality 参数可选值枚举（如 standard/high）；空 = 该模型不支持画质档
    #[serde(default)]
    pub qualities: Vec<String>,
    /// 百炼 `prompt_extend`：大模型智能改写正向提示词（+3–4 秒；关闭可规避版权审核拦截）。
    #[serde(default)]
    pub supports_prompt_extend: bool,
    /// `prompt_extend` 官方默认开关（千问与 wan2.6 为开，z-image 为关）；前端切模型时据此初始化。
    /// 缺省 true 取官方多数系列口径（不支持该参数的服务商读不到它）。
    #[serde(default = "yes")]
    pub prompt_extend_default: bool,
    /// 百炼 `prompt_extend_mode`：智能改写模式（`direct`=DPE / `agent`=APE）。
    /// `agent` 官方仅支持文生图，图生图传入会 400。
    #[serde(default)]
    pub supports_prompt_extend_mode: bool,
    /// `prompt_extend_mode` 的默认档（官方 `direct`）。
    #[serde(default = "direct")]
    pub prompt_extend_mode_default: String,
    /// 百炼 `enable_thinking`：思考模式，提升质量但增加耗时。
    /// 官方默认 true，且**仅当 `prompt_extend=true` 时生效**。
    #[serde(default)]
    pub supports_enable_thinking: bool,
    /// `enable_thinking` 的默认值（官方 true）。
    #[serde(default = "yes")]
    pub enable_thinking_default: bool,
    /// 百炼 `watermark`：右下角固定「AI生成」水印。
    #[serde(default)]
    pub supports_watermark: bool,
    /// `watermark` 的默认值（官方 false；按模型存放，便于日后分化）。
    #[serde(default)]
    pub watermark_default: bool,
    /// 请求体是否携带张数参数：z-image、火山 Seedream 全系列、硅基流动 Qwen-Image（无 `batch_size`）
    /// 与魔搭全系列均无此参数（出图固定 1 张）必须省略；其余模型为 true。
    /// 缺省 true = OpenAI 协议默认发 n。
    #[serde(default = "yes")]
    pub supports_n: bool,
    /// 正向提示词字符/Token 上限（0 = 不展示计数、不校验；
    /// qwen3.0=4500/wan2.7=5000/wan2.6=2100/z-image=800）。
    #[serde(default)]
    pub max_prompt_chars: u32,
    /// 自由尺寸规则；`None` = 不校验尺寸，`Some` = 只校验不在 `sizes` 预设里的自定义值
    /// （预设值是官方给的，天然合法）。见 `validate_model_caps`。
    #[serde(default)]
    pub size_rules: Option<SizeRules>,
    /// 是否支持异步调用（提交任务 → 轮询 `/tasks/{task_id}`）。协议级能力位（同步/异步
    /// 请求体逐字相同，差异只在 endpoint 与异步头），出厂按官方异步清单逐模型置位。
    #[serde(default)]
    pub supports_async: bool,
}

/// serde 缺省值：字段缺失时填 1（max_n、n 等）。
fn one() -> u32 {
    1
}

/// serde 缺省值：字段缺失时填 true（多数能力开关的缺省档）。
fn yes() -> bool {
    true
}

/// serde 缺省值：`prompt_extend_mode` 的官方默认档 direct。
fn direct() -> String {
    "direct".into()
}

// ---- 配置文件结构：ModelInfo / ProviderConfig / GenSettings / ImageAiFile -------------------------------------------

/// 模型明细行（模型名 + 能力集）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// 模型名：厂商 API 请求体 `model` 字段的原值（如 "gpt-image-1"）
    pub model: String,
    /// 该模型的能力集：UI 据此显隐参数、适配器据此决定发哪些字段
    #[serde(default)]
    pub capabilities: Capabilities,
}

/// 已加密的 API Key：`nonce` 与「密文‖tag」各自 base64。
/// 【为什么单独一个结构】明文只在 set_provider_key 与发请求时短暂存在；落盘与回前端
/// 都只经这个结构，避免有人把明文塞进 ProviderConfig。加解密见 keys.rs 的 seal_key / open_key。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedKey {
    /// GCM nonce 的 base64（每次封装随机生成）
    pub nonce: String,
    /// 「密文‖认证标签」的 base64
    pub data: String,
}

/// 服务商配置（不含明文 API Key）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// 服务商唯一标识：前端选择与增删改的判据；"openai" 为内置保留 id，不可删
    pub id: String,
    /// 协议种类：决定适配器分发；内置服务商此项恒由代码归位（见 reconcile_one_builtin）
    pub kind: ProviderKind,
    /// 界面显示名（用户可改）
    pub name: String,
    /// API 基址：适配器据此拼端点（本地 ComfyUI 形如 http://127.0.0.1:8188）
    pub base_url: String,
    /// 默认模型名；models 中无对应明细行时，Config::model 据此合成一行通用能力模型
    pub default_model: String,
    /// 模型明细行（尺寸/张数/能力位）；可为空——空时只有 default_model 可放行且高级能力全关
    #[serde(default)]
    pub models: Vec<ModelInfo>,
    /// 加密后的 API Key（每家一把，跨平台不可复用）；None = 未配置。
    /// 【绝不回前端】ProviderConfig 同时用于落盘与 IPC 返回，所以回前端前必须经
    /// [`Config::public_view`](crate::image_ai::Config::public_view) 清空，否则密文会随状态一起送到前端。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<SealedKey>,
}

/// 生图设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenSettings {
    /// 默认服务商 id；若指向已不存在的服务商，启动时归位为 "openai"
    pub default_provider_id: String,
    /// 临时图片上限：仅由 `enforce_image_limit` 消费（决定 `.imgtmp` 保留几张图）。
    pub history_limit: usize,
    /// 画廊记录上限：仅由 `enforce_gallery_limit` 消费（决定 gallery.json 保留几行）。
    /// 与 history_limit 是两条独立闸门的上限，**互不换算**；老文件缺该字段，带缺省兜底。
    #[serde(default = "gallery_limit")]
    pub gallery_record_limit: usize,
    /// 批量生成队列的并发任务数。**仅供文件读写保留，当前不参与逻辑**——队列一律串行，
    /// 见 `commands::start_queue` 的 `QUEUE_CONCURRENCY`（提交即计费，多线并行会在用户没点
    /// 确认时翻倍账单）。
    pub concurrency: u32,
    /// 保存图片的文件名模板（支持 {uid}/{index} 等占位）
    pub naming_pattern: String,
}

/// `gallery_record_limit` 缺省（出厂 100）：老文件缺该字段时兜底，避免整份设置解析失败。
fn gallery_limit() -> usize {
    100
}

impl Default for GenSettings {
    fn default() -> Self {
        Self {
            default_provider_id: "openai".into(),
            history_limit: 20,
            gallery_record_limit: 100,
            concurrency: 1,
            naming_pattern: "{uid}_{index}".into(),
        }
    }
}

/// data/image_ai.json 形状。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAiFile {
    /// 服务商列表（内置 + 用户新增/修改）
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    /// 全局生图设置
    #[serde(default)]
    pub settings: GenSettings,
}

// ---- 提示词模板：DetailPools（码值映射 + 随机词库）/ PromptTemplate -------------------------------------------------

/// 一档景别及其适配的动作池：景别与动作联动随机——「胸像特写」配「进球后滑跪」是矛盾画面。
/// label 渲染进 `{{shot}}`，poses 是该景别下可行的动作（`{{pose}}`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotPool {
    pub label: String,
    #[serde(default)]
    pub poses: Vec<String>,
}

/// 职员专用词库：服装与动作与球员分池——教练不穿比赛球衣、动作是场边指挥/观察。
/// 正文占位符与球员共用（`{{jersey}}`/`{{kitDetail}}`/`{{shot}}`/`{{pose}}`），build_vars 按身份选池。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StaffPools {
    #[serde(default)]
    pub jersey: Vec<String>,
    #[serde(default)]
    pub kit_detail: Vec<String>,
    #[serde(default)]
    pub shots: Vec<ShotPool>,
}

impl StaffPools {
    pub fn is_empty(&self) -> bool {
        self.jersey.is_empty() && self.kit_detail.is_empty() && self.shots.is_empty()
    }
}

/// 模板自带的词库：随机细节候选池 + 码值转译映射（ethnicity/hairColor/skinCode）。
/// 挂在模板结构上而非写成代码常量——这些全是**内容**（中文措辞），须随模板落盘、即改即生效，
/// 与 body/negative 同等待遇。各池为空时逐组回落内置默认模板（见 build_vars），手建模板不写也能跑。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailPools {
    /// 发型样式候选，**键 = 发长码**（0=光头 … 5=极长发）：随机只换剪法，不改长度。
    /// BTreeMap：码为单字符数字，字典序即数值序，落盘键序天然稳定。
    #[serde(default)]
    pub hair_style_by_length: std::collections::BTreeMap<String, Vec<String>>,
    /// 族裔码 0–10 → 描述词。
    #[serde(default, serialize_with = "serialize_sorted_codes")]
    pub ethnicity: HashMap<String, String>,
    /// 发色码 1–19 → 色名；`1` = 未知（随机取一色，见 build_vars），兼任随机候选池
    /// （排除「未知」档本身）。
    #[serde(default, serialize_with = "serialize_sorted_codes")]
    pub hair_color: HashMap<String, String>,
    /// 肤色码 0–19 → 色卡描述（色卡来自 FM 内置肤色代码图鉴，仅作色卡参照）。
    #[serde(default, serialize_with = "serialize_sorted_codes")]
    pub skin_code: HashMap<String, String>,
    /// 身份称谓：键 `player` → 球员称谓（职员不用此表，直接渲染职位原文，见 build_vars）。
    /// BTreeMap 保持键落盘序稳定。
    #[serde(default)]
    pub identity: std::collections::BTreeMap<String, String>,
    /// 景别档位（胸像/半身/全身），每档自带动作池；随机先选档再从档内取动作。
    #[serde(default)]
    pub shots: Vec<ShotPool>,
    /// 职员专用池：服装与动作按身份换池，占位符与球员共用。
    #[serde(default)]
    pub staff: StaffPools,
    #[serde(default)]
    pub beard: Vec<String>,      // 胡须候选
    #[serde(default)]
    pub expression: Vec<String>, // 表情候选
    #[serde(default)]
    pub lighting: Vec<String>,   // 打光候选
    #[serde(default)]
    pub bg_tone: Vec<String>,    // 背景候选（色调虚化或实景场景）
    #[serde(default)]
    pub kit_detail: Vec<String>, // 球衣细节候选
}

/// 码值分组的稳定序列化：HashMap 的 serde 落盘键序随**进程级随机哈希种子**变化，
/// 同一文件每次启动写出的键序都不同。按键的数值序输出（非数字键按字典序排其后），
/// 文件才稳定、可 diff、可手改。
fn serialize_sorted_codes<S: serde::Serializer>(
    map: &HashMap<String, String>,
    s: S,
) -> Result<S::Ok, S::Error> {
    SortedCodes(map).serialize(s)
}

impl DetailPools {
    /// 所有分组皆空 = 这个模板没写词库（手建模板，或老配置尚未补齐）。
    /// 【刻意整份判据，不逐组补】ensure_user_files 只在整体为空时补内置词库——
    /// 逐组补会把用户刻意删空的池/码值组在重启时复活。
    pub fn is_empty(&self) -> bool {
        self.hair_style_by_length.is_empty()
            && self.ethnicity.is_empty()
            && self.hair_color.is_empty()
            && self.skin_code.is_empty()
            && self.identity.is_empty()
            && self.shots.is_empty()
            && self.staff.is_empty()
            && self.beard.is_empty()
            && self.expression.is_empty()
            && self.lighting.is_empty()
            && self.bg_tone.is_empty()
            && self.kit_detail.is_empty()
    }
}

/// 提示词模板，body 支持 `{{变量|默认值}}`；`pools` 携带该模板的码值映射与随机词库——
/// 内容随模板落盘、可编辑，空时逐组回落出厂默认模板（id = "default"）的同名结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplate {
    /// 模板唯一 id（`default` 为内置保留，不可删）
    pub id: String,
    /// 模板显示名（用户可改）
    pub name: String,
    /// 正向提示词正文（支持 `{{key}}` / `{{key|默认值}}`）
    pub body: String,
    /// 反向提示词（可为空；模型不支持时适配器整条省略）
    #[serde(default)]
    pub negative: String,
    /// 该模板自带的码值映射与随机词库
    #[serde(default)]
    pub pools: DetailPools,
}

/// 键序：纯数字码按**数值**升序（按字典序会让 "10" 排在 "2" 前面），非数字键排在其后按字典序。
fn sorted_keys(map: &HashMap<String, String>) -> Vec<&String> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by(|a, b| match (a.parse::<u32>(), b.parse::<u32>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        (Err(_), Err(_)) => a.cmp(b),
    });
    keys
}

/// 单个分组的稳定序列化包装。
struct SortedCodes<'a>(&'a HashMap<String, String>);

impl Serialize for SortedCodes<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for k in sorted_keys(self.0) {
            m.serialize_entry(k, &self.0[k])?;
        }
        m.end()
    }
}

// ---- 生成请求：GenRequest（含内部回填字段与能力快照访问器） ---------------------------------------------------------

/// 一行式人员摘要（日志用）：`李传斌(200204642) 中国/上海海港`；缺项自动省略，
/// 连姓名与 uid 都没有时返回 `无人员`（无数据源的单张生成）。
pub fn person_summary(req: &GenRequest) -> String {
    let name = req.name.as_deref().unwrap_or("").trim();
    let uid = req.uid.as_deref().unwrap_or("").trim();
    let head = match (name.is_empty(), uid.is_empty()) {
        (false, false) => format!("{name}({uid})"),
        (false, true) => name.to_string(),
        (true, false) => uid.to_string(),
        (true, true) => "无人员".to_string(),
    };
    let nat = req.nationality.as_deref().unwrap_or("").trim();
    let club = req.club.as_deref().unwrap_or("").trim();
    let extra: Vec<&str> = [nat, club].into_iter().filter(|s| !s.is_empty()).collect();
    if extra.is_empty() {
        head
    } else {
        format!("{head} {}", extra.join("/"))
    }
}

/// 一行式参数摘要（日志用）：只列非空项，默认值不占位，末尾带提示词字数。
/// 【为什么记字数而不记全文】提示词可能上千字，日志里刷屏；长度足以识别「模板没渲染出来」这类问题。
pub fn request_summary(req: &GenRequest) -> String {
    let mut parts = vec![
        format!("{}/{}", req.provider_id, req.model),
        req.size.clone(),
    ];
    if req.n != 1 {
        parts.push(format!("n={}", req.n));
    }
    if let Some(q) = req.quality.as_deref().filter(|s| !s.is_empty()) {
        parts.push(format!("quality={q}"));
    }
    if let Some(seed) = req.seed {
        parts.push(format!("seed={seed}"));
    }
    if let Some(steps) = req.steps {
        parts.push(format!("steps={steps}"));
    }
    if let Some(g) = req.guidance {
        parts.push(format!("guidance={g}"));
    }
    if req.call_mode.as_deref() == Some("async") {
        parts.push("async".into());
    }
    if req.output_format.as_deref() == Some("jpeg") {
        parts.push("jpeg".into());
    }
    parts.push(format!("提示词 {} 字", req.prompt.chars().count()));
    parts.join(" ")
}

/// 一次生成请求。队列条目（`image_ai::gen_queue::QueueEntry` 内嵌本类型）经 IPC 双向往返：
/// 前端提交时反序列化，后端回传快照时序列化。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenRequest {
    /// 目标服务商 id（对应 ProviderConfig.id）
    pub provider_id: String,
    /// 模型名（对应 ModelInfo.model，或该服务商的 default_model）
    pub model: String,
    /// 正向提示词最终文本：模板渲染并经用户编辑确认后的全文
    pub prompt: String,
    /// 反向提示词：模型不支持时适配器整条省略
    #[serde(default)]
    pub negative_prompt: String,
    /// 尺寸字符串：sizes 预设值，或 `宽x高`（小写 x）/ `宽*高` 形态的自定义值
    pub size: String,
    /// 单次生成张数（缺省 1；上限由模型 max_n 校验）
    #[serde(default = "one")]
    pub n: u32,
    /// 画质档：取模型 qualities 的声明值；None = 不传该参数
    #[serde(default)]
    pub quality: Option<String>,
    /// 随机种子：仅模型 supports_seed 且用户填了才发（非负）
    #[serde(default)]
    pub seed: Option<i64>,
    /// 采样步数：仅模型 supports_steps 时消费
    #[serde(default)]
    pub steps: Option<u32>,
    /// 引导系数（CFG）：仅模型 supports_guidance 时消费
    #[serde(default)]
    pub guidance: Option<f32>,
    /// 球员 UID：写入画廊记录用于关联；无数据源时为 None
    #[serde(default)]
    pub uid: Option<String>,
    /// 球员名：仅用于画廊记录展示；无数据源时 None（记录里 name 为空）。
    #[serde(default)]
    pub name: Option<String>,
    /// 提示词模板 id：随请求写入画廊记录；未选模板时 None。
    #[serde(default)]
    pub template_id: Option<String>,
    /// 国籍三字码：**只用于日志与记录展示**，任何适配器都不发送；未知时 None。
    #[serde(default)]
    pub nationality: Option<String>,
    /// 俱乐部：**只用于日志与记录展示**，任何适配器都不发送；基础名单无此列时 None。
    #[serde(default)]
    pub club: Option<String>,
    /// 调用方式：`Some("async")` = 走异步任务端点，None = 同步。**这不是能力字段**：
    /// 前端只表达「想用异步」的意图，是否真走异步由 `generate` 依据 `supports_async` 门控回填。
    #[serde(default)]
    pub call_mode: Option<String>,
    /// 百炼智能改写开关（仅百炼适配器消费；None = 按官方默认 true）。
    #[serde(default)]
    pub prompt_extend: Option<bool>,
    /// 「AI生成」水印开关（百炼与火山 Seedream 适配器消费；None = 按模型声明的官方默认 false）。
    #[serde(default)]
    pub watermark: Option<bool>,
    /// 百炼 `prompt_extend_mode`（仅百炼适配器消费；None = 按该模型默认档 direct）。
    #[serde(default)]
    pub prompt_extend_mode: Option<String>,
    /// 百炼 `enable_thinking`（仅百炼适配器消费；None = 按该模型默认值 true）。
    #[serde(default)]
    pub enable_thinking: Option<bool>,
    /// 输出图片格式 `output_format`（png/jpeg）：仅声明 `supports_output_format` 的模型（火山
    /// Seedream 5.0 系列、硅基流动全模型）消费，None = 适配器默认 png。取值由适配器白名单校验，
    /// 前端选到的越界值不会直发厂商。
    #[serde(default)]
    pub output_format: Option<String>,
    /// 内部字段（前端不传）：`generate` 解析服务商后填入，适配器据此拼 endpoint；
    /// 与能力无关，故独立于能力快照。
    #[serde(default)]
    pub base_url: String,
    /// 内部字段：`generate` 从模型能力集**整体**拷入的能力快照（`None` = 未回填，访问器
    /// [`GenRequest::caps`] 按缺省语义兜底）。用快照而非逐字段摊平：新增能力位只改
    /// `Capabilities` 与消费方，不必动本结构体与拷贝行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<Capabilities>,
}

impl GenRequest {
    /// 能力快照访问器：已回填时直接返回，缺省（单测直接构造、老快照回放）时走
    /// [`default_caps`] 兜底。适配器一律经此读取，避免默认值散落各处。
    pub fn caps(&self) -> Capabilities {
        self.caps.clone().unwrap_or_else(default_caps)
    }
}

/// 测试夹具基座：用例可用 `GenRequest { n: 2, ..test_req_base() }` 只覆盖关心的字段；
/// GenRequest 增删字段时只有这里要改。
#[cfg(test)]
pub(crate) fn test_req_base() -> GenRequest {
    GenRequest {
        provider_id: String::new(),
        base_url: String::new(),
        model: String::new(),
        prompt: String::new(),
        negative_prompt: String::new(),
        size: String::new(),
        n: 1,
        quality: None,
        seed: None,
        steps: None,
        guidance: None,
        uid: None,
        name: None,
        template_id: None,
        nationality: None,
        club: None,
        call_mode: None,
        prompt_extend: None,
        watermark: None,
        prompt_extend_mode: None,
        enable_thinking: None,
        output_format: None,
        caps: None,
    }
}

/// 能力快照缺省时的兜底：**逐条等价**于 GenRequest 各能力字段的 serde 缺省。
/// 【为什么不能直接用 `Capabilities::default()`】其 `Default` 派生把 `supports_n` 置 false，
/// 而请求侧缺省为 true（`#[serde(default = "yes")]`）——语义不同。
fn default_caps() -> Capabilities {
    Capabilities {
        supports_n: true,
        prompt_extend_mode_default: "direct".into(),
        enable_thinking_default: true,
        watermark_default: false,
        ..Default::default()
    }
}

// ---- 生成结果：GenImage / GenResult ---------------------------------------------------------------------------------

/// 一张临时图片元信息。随队列条目经 IPC 往返（`image_ai::gen_queue::QueueEntry` 内嵌本类型）；
/// 前端提交的新条目 images 恒为空。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenImage {
    /// 图片 id（16 位随机十六进制）：同时是 .imgtmp 文件名 stem 与画廊记录 id
    pub id: String,
    /// 临时图片绝对路径（`.imgtmp/{id}.{扩展名}`，扩展名随厂商返回格式而定）
    pub path: String,
    /// 图片字节数
    pub bytes: u64,
}

/// 生成命令返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenResult {
    pub images: Vec<GenImage>,
}

// ---- 画廊记录契约：GalleryRecord（持久记录）/ GalleryItem（查询期现算存在性） ---------------------------------------

/// 一条画廊记录：持久保存在 `data_dir/.imgtmp/gallery.json`（启动清理只删图片、保留本文件），
/// 与图片字节是两条独立生命周期。除 id/ts 外字段均可缺省（兼容老文件与字段演进）；
/// ts 为 now_ts 格式，字典序即时间序，淘汰与排序据此判新旧。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryRecord {
    /// 记录 id（= GenImage.id = .imgtmp 文件 stem）：画廊查询据此 stem 匹配图片是否仍在
    pub id: String,
    /// 写入时间（now_ts 格式，字典序即时间序）
    pub ts: String,
    /// 球员 UID（可缺省）
    #[serde(default)]
    pub uid: Option<String>,
    /// 球员名（可缺省）
    #[serde(default)]
    pub name: Option<String>,
    /// 生成时使用的模型名（老记录可为空串）
    #[serde(default)]
    pub model: String,
    /// 正向提示词全文（老记录可为空串）
    #[serde(default)]
    pub prompt: String,
    /// 反向提示词（可缺省）
    #[serde(default)]
    pub negative_prompt: String,
    /// 提示词模板 id；空串 = 未选模板
    #[serde(default)]
    pub template_id: String,
    /// 「另存为 / 保存并替换」把该图落盘到的绝对路径；从未保存过为 None（不写进 JSON）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_path: Option<String>,
    /// 保存落点的文件名（含扩展名，如 `2002046242_1.png`）；从未保存过为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_name: Option<String>,
}

/// 画廊查询返回项：记录本体 + **查询期现算**的存在性（绝不写回 gallery.json）。
/// path 为现算出的 `.imgtmp` 实际路径（扩展名随厂商返回格式变化），图片已被会话清理时为 None；
/// 前端据此渲染真图或占位。记录里的 `savedPath`/`savedName` 则是保存动作落下的持久字段。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryItem {
    /// 持久记录本体
    #[serde(flatten)]
    pub record: GalleryRecord,
    /// 图片是否仍在 `.imgtmp`（查询期现算，不持久化）
    pub image_available: bool,
    /// 现算出的临时图片绝对路径；图片已被清理时为 None
    pub path: Option<String>,
}

// ===================== 单元测试：serde 契约 / 错误分类 / 错误序列化 ==================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一条请求样本：只覆盖关心的字段，其余取 `test_req_base` 缺省。
    fn gen_req() -> GenRequest {
        GenRequest {
            provider_id: "openai".into(),
            model: "gpt-image-1".into(),
            prompt: "一张脸".into(),
            size: "1024x1024".into(),
            n: 2,
            quality: Some("high".into()),
            // 单测直接构造：能力快照留空，由 caps() 兜底或由 validate_for_generate 回填。
            ..test_req_base()
        }
    }

    /// 日志摘要：缺项自动省略、默认值不占位、提示词按字数计。
    #[test]
    fn request_and_person_summary_omit_defaults() {
        let mut req = test_req_base();
        req.provider_id = "modelscope".into();
        req.model = "Qwen/Qwen-Image-2.1".into();
        req.prompt = "一二三".into();
        req.size = "1024x1024".into();
        req.uid = Some("200204642".into());
        req.name = Some("李传斌".into());
        req.nationality = Some("CHN".into());
        req.club = Some("上海海港".into());

        // n=1/无 seed/steps 等默认值不出现，末尾只报提示词字数
        assert_eq!(
            request_summary(&req),
            "modelscope/Qwen/Qwen-Image-2.1 1024x1024 提示词 3 字"
        );
        assert_eq!(person_summary(&req), "李传斌(200204642) CHN/上海海港");

        // 有值项按固定顺序追加；提示词字数按字符（不是字节）计
        req.prompt = "这是一段较长的提示词".into();
        req.n = 2;
        req.seed = Some(7);
        req.steps = Some(30);
        req.call_mode = Some("async".into());
        req.output_format = Some("jpeg".into());
        assert_eq!(
            request_summary(&req),
            "modelscope/Qwen/Qwen-Image-2.1 1024x1024 n=2 seed=7 steps=30 async jpeg 提示词 10 字"
        );

        // 无人员（无数据源单张）：回落文案；国籍/俱乐部缺失时不拼多余分隔符
        req.uid = None;
        req.name = None;
        req.nationality = None;
        req.club = None;
        assert_eq!(person_summary(&req), "无人员");
        req.name = Some("王丙".into());
        assert_eq!(person_summary(&req), "王丙");
    }

    #[test]
    fn serde_contract_kind_and_request_defaults() {
        // 与前端 types.ts 一一对应的序列化契约：kind 必须小写；n 缺省 1；
        // caps 缺省时走 default_caps() 兜底。
        assert_eq!(
            serde_json::to_string(&ProviderKind::OpenAI).unwrap(),
            r#""openai""#
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Volcano).unwrap(),
            r#""volcano""#
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Dashscope).unwrap(),
            r#""dashscope""#
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Siliconflow).unwrap(),
            r#""siliconflow""#
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Modelscope).unwrap(),
            r#""modelscope""#
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::LocalComfyUI).unwrap(),
            r#""localcomfyui""#
        );
        // 兼容服务与官方同档；「custom」不是合法 kind
        assert!(serde_json::from_str::<ProviderKind>(r#""custom""#).is_err());

        let v: GenRequest = serde_json::from_str(
            r#"{"providerId":"openai","model":"gpt-image-1","prompt":"x","size":"1024x1024"}"#,
        )
        .unwrap();
        assert_eq!(v.n, 1);

        // caps() 的兜底须与请求侧各能力字段的 serde 缺省**逐条等价**；访问器只认 Option，
        // 与 caps 如何变成 None 无关——两条路径（直接构造 / 老快照反序列化）同例覆盖。
        let r = gen_req(); // 路径一：直接构造，caps = None
        assert!(r.caps.is_none());
        let c = r.caps();
        assert!(!c.supports_response_format); // 缺省 false
        assert!(!c.supports_negative_prompt);
        assert!(!c.supports_prompt_extend);
        assert!(!c.supports_watermark);
        assert!(!c.supports_seed);
        assert!(c.supports_n); // 缺省 true
        assert!(!c.supports_prompt_extend_mode);
        assert_eq!(c.prompt_extend_mode_default, "direct");
        assert!(!c.supports_enable_thinking);
        assert!(c.enable_thinking_default);
        assert!(!c.watermark_default);
        // 对照：Capabilities::default() 的 supports_n 为 false，故兜底不能省。
        assert!(!Capabilities::default().supports_n);

        // 路径二：老快照（JSON 无 caps 字段）反序列化同样落 None、给同一兜底。
        let v: GenRequest = serde_json::from_str(
            r#"{"providerId":"dashscope","model":"z-image-turbo","prompt":"x","size":"1024x1024"}"#,
        )
        .unwrap();
        assert!(v.caps.is_none());
        assert!(v.caps().supports_n && v.caps().enable_thinking_default);
        assert_eq!(v.caps().prompt_extend_mode_default, "direct");
    }

    #[test]
    fn error_kind_classification() {
        // HTTP 状态 → 类别：429→RateLimited(可重试)；401/403→Auth 且不可重试
        //（Key 错误重试无意义，先修配置）；400→InvalidParam；5xx→Provider(可重试)；
        // 其余→Provider(不可重试)。
        let e = from_http_status(429, "HTTP 429：限流");
        assert_eq!(e.kind, GenErrorKind::RateLimited);
        assert!(e.retryable);
        for s in [401u16, 403] {
            let e = from_http_status(s, "HTTP：鉴权失败");
            assert_eq!(e.kind, GenErrorKind::Auth);
            assert!(!e.retryable);
        }
        assert_eq!(from_http_status(400, "x").kind, GenErrorKind::InvalidParam);
        let p5 = from_http_status(503, "x");
        assert_eq!(p5.kind, GenErrorKind::Provider);
        assert!(p5.retryable);
        let p4 = from_http_status(418, "x");
        assert_eq!(p4.kind, GenErrorKind::Provider);
        assert!(!p4.retryable);

        // 厂商码大小写/分隔符写法不一，匹配须大小写无关；审核两种码归同一类。
        for code in ["InvalidApiKey", "INVALID_API_KEY"] {
            assert_eq!(
                kind_from_business_code(code),
                Some(GenErrorKind::Auth),
                "{code}"
            );
        }
        for code in ["Throttling", "QuotaExceeded", "rate_limit"] {
            assert_eq!(
                kind_from_business_code(code),
                Some(GenErrorKind::RateLimited),
                "{code}"
            );
        }
        for code in ["DataInspectionFailed", "IPInfringementSuspect"] {
            assert_eq!(
                kind_from_business_code(code),
                Some(GenErrorKind::ContentRejected),
                "{code}"
            );
        }
        assert_eq!(kind_from_business_code("SomethingElse"), None);
        // 业务码优先于 status：即便 status 是 500，Throttling 仍归 RateLimited；
        // 未命中或没给码 → 回落 status。
        let e = classify_provider_error(500, Some("Throttling"), "x");
        assert_eq!(e.kind, GenErrorKind::RateLimited);
        assert_eq!(
            classify_provider_error(200, Some("SomethingElse"), "x").kind,
            GenErrorKind::Provider
        );
        assert!(classify_provider_error(503, None, "x").retryable);

        // 类别默认可重试位：Network/RateLimited 可重试，其余不可（Provider 由构造点显式指定）；
        // 建议随类别给出，Cancelled/Internal 无建议。
        assert!(ImageGenError::network("x").retryable);
        assert!(!ImageGenError::io("x").retryable);
        assert!(!ImageGenError::not_configured("x").retryable);
        assert!(from_http_status(429, "x").user_action.is_some());
        assert!(ImageGenError::cancelled().user_action.is_none());
        assert!(ImageGenError::internal("x").user_action.is_none());
    }

    #[test]
    fn gen_error_serializes_kind_snake_case_and_message() {
        // 前端按 snake_case 的 kind 判断重试与建议，序列化形态是契约的一部分。
        let e = ImageGenError::provider("HTTP 500：服务商错误", true);
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""kind":"provider""#), "{s}");
        assert!(s.contains(r#""message":"HTTP 500：服务商错误""#), "{s}");
        assert!(s.contains(r#""retryable":true"#), "{s}");
        assert!(s.contains(r#""userAction":"#), "{s}");
        let back: ImageGenError = serde_json::from_str(&s).unwrap();
        assert_eq!(back.kind, GenErrorKind::Provider);
        assert!(back.retryable);
        // retryAfter 仅在适配器读到 Retry-After 响应头时出现（None 不序列化，队列按缺省处理）。
        let e = from_http_status(429, "限流").with_retry_after(Some(7));
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""retryAfter":7"#), "{s}");
        let back: ImageGenError = serde_json::from_str(&s).unwrap();
        assert_eq!(back.retry_after, Some(7));
        let s = serde_json::to_string(&ImageGenError::provider("x", false)).unwrap();
        assert!(!s.contains("retryAfter"), "{s}");
    }
}
