//! AI 文生图：提示词模板 → 用户编辑确认 → 调用文生图 API → 会话临时图片 → 手动保存/保存并替换。
//!
//! 【与既有模块的复用关系，不另起一套】
//! - 命令统一写在 commands.rs，本模块只提供被命令调用的类型与函数；
//! - 出厂默认是 assets/ 三份 JSON（assets.rs include_str! 内嵌），用户可编辑副本在 data/；
//!   Config::builtin/load/ensure_user_files 仿 templates::RuleConfig；
//! - 原子写复用 json_store。
//! P2 起多家协议适配成「族」拆在 image_ai_providers/（openai 迁入 + volcano + dashscope），
//! 本模块只保留契约/配置/渲染与生成编排，生成时按 ProviderKind 经 registry 分发。

use crate::json_store::{self, LoadError};
use rand::Rng; // new_id() 用 rand::rng().random()（本项目既有模块同样显式导入，见 templates/mod.rs）
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

// ===================== 数据契约（与前端 types.ts 一一对应，camelCase）=====================

/// 服务商类型：决定适配器。
/// openai 兼容服务与官方 OpenAI 是**同一套 SDK**（请求/响应同构，
/// 差异仅在 base_url 与模型名），全仓没有任何一处需要按「是否官方」分流；
/// 用户自建兼容服务直接用 OpenAI 这一档、改 base_url 即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAI,
    Volcano,
    Dashscope,
    Siliconflow,
    Modelscope,
    LocalComfyUI,
}

impl ProviderKind {
    /// 该协议是否需要 API Key（会话内存）。本地 ComfyUI 无鉴权，Key 恒不需要——
    /// 后端两道 Key 门（generate_image / start_queue）与前端就绪度判据同用本口径。
    pub fn needs_key(self) -> bool {
        !matches!(self, ProviderKind::LocalComfyUI)
    }
}

/// 生成链错误类别（snake_case 序列化，与前端 `GenErrorKind` 一一对应）。
/// 分类的意义：前端据此给出可执行建议、并判断队列失败条目是否值得重试。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenErrorKind {
    /// 服务商/模型/Key 等前置项缺省——用户补全即可，重试无意义。
    NotConfigured,
    /// 鉴权失败（401/403 或业务码 InvalidApiKey）。
    Auth,
    /// 限流/配额（429 或业务码 Throttling/QuotaExceeded），错峰重试有意义。
    RateLimited,
    /// 参数不合法（尺寸/张数/提示词长度等）。
    InvalidParam,
    /// 服务商内容审核拦截。
    ContentRejected,
    /// 服务商协议/服务端错误（JSON 解析、结构缺失、5xx 等）。
    Provider,
    /// 网络传输失败（连接、发送、读体），可重试。
    Network,
    /// 本地磁盘/目录 I/O。
    Io,
    /// 用户主动取消（队列自造，非服务商返回）。
    Cancelled,
    /// 内部异常（队列重入拒绝、任务 panic 兜底等）。
    Internal,
}

impl GenErrorKind {
    /// 类别默认是否可重试。Provider 因 4xx/5xx 而同类别不同结论，由构造点显式指定，不查此表。
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

/// 生成链结构化错误：kind 供前端分类给建议，message 为既有中文文案（构造点不改文案），
/// retryable 决定队列条目是否展示「重试本条」，user_action 为用户可执行动作。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenError {
    pub kind: GenErrorKind,
    pub message: String,
    pub retryable: bool,
    pub user_action: Option<String>,
    /// 服务端 `Retry-After`（秒）：仅当 429 响应的该头可解析为整数秒时填；其余错误一律 None。
    /// 队列重试退避会优先采用它（见 `image_ai_queue::backoff_delay`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

impl ImageGenError {
    fn build(kind: GenErrorKind, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind,
            message: message.into(),
            retryable,
            user_action: kind.user_action().map(str::to_string),
            retry_after: None,
        }
    }

    /// 附加 `Retry-After`（秒）。仅在确实读到响应头时由适配器调用，其它错误保持 None。
    pub fn with_retry_after(mut self, secs: Option<u64>) -> Self {
        self.retry_after = secs;
        self
    }

    /// 按类别构造，retryable 取类别默认（仅 Network/RateLimited 为 true）。
    pub fn new(kind: GenErrorKind, message: impl Into<String>) -> Self {
        Self::build(kind, message, kind.default_retryable())
    }

    pub fn not_configured(message: impl Into<String>) -> Self {
        Self::new(GenErrorKind::NotConfigured, message)
    }

    pub fn invalid_param(message: impl Into<String>) -> Self {
        Self::new(GenErrorKind::InvalidParam, message)
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::new(GenErrorKind::Network, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(GenErrorKind::Io, message)
    }

    /// Provider 类显式指定可重试位（5xx=true，其余 4xx/解析失败=false）。
    pub fn provider(message: impl Into<String>, retryable: bool) -> Self {
        Self::build(GenErrorKind::Provider, message, retryable)
    }

    pub fn cancelled() -> Self {
        Self::new(GenErrorKind::Cancelled, "已取消")
    }

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
    // 内容审核两种码：DataInspectionFailed（输入/输出违规）与 IPInfringementSuspect（疑似侵权）。
    // 官方给的自救手段都在提示词侧（关掉 prompt_extend 重试、或改写涉及版权的角色名/作品名），
    // 归 ContentRejected 才能让用户看到「修改提示词后重试」，而不是无因由地重试整个请求。
    if c.contains("datainspection") || c.contains("data_inspection") || c.contains("ipinfringement")
    {
        return Some(GenErrorKind::ContentRejected);
    }
    None
}

/// HTTP 状态码 → 错误类别（必须在拼串前分流，status 是唯一稳定的分类依据）：
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

/// 厂商错误统一归类：业务 code 命中优先，否则回落 HTTP status。message 为既有中文文案。
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

/// 自由尺寸规则：约束**只对不在 `sizes` 预设里的自定义值**生效（见 `validate_model_caps`），
/// 每条 0 = 不检查。尺寸字符串为 `宽*高`/`宽x高` 形态；各模型的具体区间由内置目录逐模型声明
/// （见 `image_ai_providers::dashscope` 的各 `*_caps`），取值逐字对齐官方文档。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeRules {
    /// 单边下限像素（宽/高各自，0 = 不检查）。
    #[serde(default)]
    pub min_side: u32,
    /// 单边上限像素（0 = 不检查）。
    #[serde(default)]
    pub max_side: u32,
    /// 总像素（宽×高）下限（0 = 不检查）。
    #[serde(default)]
    pub total_min: u64,
    /// 总像素上限（0 = 不检查）。
    #[serde(default)]
    pub total_max: u64,
    /// 宽高比下限（短边/长边口径，如 1:4 = 0.25；0 = 不检查）。
    #[serde(default)]
    pub ratio_min: f32,
    /// 宽高比上限（如 4:1 = 4.0；0 = 不检查）。
    #[serde(default)]
    pub ratio_max: f32,
}

/// 模型能力集：UI 按此显隐参数，适配器按此决定是否传参。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub sizes: Vec<String>,
    #[serde(default = "one")]
    pub max_n: u32,
    #[serde(default)]
    pub supports_quality: bool,
    #[serde(default)]
    pub supports_seed: bool,
    #[serde(default)]
    pub supports_steps: bool,
    /// 采样步数上限（0 = 不校验；配合 `supports_steps`）。硅基流动 `num_inference_steps`（≤100）、魔搭 `steps`（≤100）用。
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
    #[serde(default)]
    pub supports_negative_prompt: bool,
    /// 是否可传 `response_format`。OpenAI 官方 gpt-image-1 系列**不支持**该参数
    /// （始终返回 base64，传了会 400），故出厂置 false；OpenAI 兼容的第三方服务多为默认返回 URL，
    /// 需要显式声明 b64_json 才能免去一次下载，故缺省为 true。
    #[serde(default = "yes")]
    pub supports_response_format: bool,
    /// 火山 Seedream `output_format`（png/jpeg）：仅 5.0 Pro/Flash/Lite 可设；
    /// 4.5/4.0 只回 jpeg、传参报错，故置 false 整条省略。OpenAI/百炼不用此参数。
    #[serde(default)]
    pub supports_output_format: bool,
    #[serde(default)]
    pub qualities: Vec<String>,
    /// 百炼 `prompt_extend`：大模型智能改写正向提示词（+3–4 秒；关闭可规避版权审核拦截）。
    #[serde(default)]
    pub supports_prompt_extend: bool,
    /// `prompt_extend` 的官方默认开关：千问/万相为开，z-image 为关（开启即思考模式、单价翻倍）。
    /// 前端切模型时按此初始化开关；缺省 true 取官方多数系列的口径（不发该参数的服务商读不到它）。
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
    /// `watermark` 的默认值（官方 false，各系列一致；仍按模型存放，便于日后分化）。
    #[serde(default)]
    pub watermark_default: bool,
    /// 请求体是否携带 `n` 参数：z-image 与火山 Seedream 全系列无此参数（固定 1 张）必须省略；
    /// 其余百炼模型为 true。缺省 true = OpenAI 协议默认发 n。
    #[serde(default = "yes")]
    pub supports_n: bool,
    /// 正向提示词字符/Token 上限（0 = 不展示计数、不校验；
    /// qwen3.0=4500/wan2.7=5000/wan2.6=2100/z-image=800）。
    #[serde(default)]
    pub max_prompt_chars: u32,
    /// 自由尺寸规则；`None` = **不做任何尺寸校验**（自定义宽高直接放行），`Some` = 只校验
    /// 不在 `sizes` 预设里的自定义值（预设值是官方给的，天然合法）。见 `validate_model_caps`。
    #[serde(default)]
    pub size_rules: Option<SizeRules>,
    /// 是否支持异步调用（提交任务 → 轮询 `/tasks/{task_id}` 取结果）。
    /// 这是**协议级**能力位（同一模型在同步/异步下请求体逐字相同，差异只在 endpoint 与
    /// 异步头），与其余能力位一样由编辑区勾选；当前由适配器按官方异步清单逐模型置位。
    #[serde(default)]
    pub supports_async: bool,
}

fn one() -> u32 {
    1
}

fn yes() -> bool {
    true
}

/// `prompt_extend_mode` 的官方默认档（serde 缺字段兜底，见 `Capabilities`）。
fn direct() -> String {
    "direct".into()
}

/// 一个模型行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub model: String,
    #[serde(default)]
    pub capabilities: Capabilities,
}

/// 服务商配置（不含 API Key）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<ModelInfo>,
}

/// 生图设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenSettings {
    pub default_provider_id: String,
    /// 临时图片上限：仅由 `enforce_image_limit` 消费（决定 `.imgtmp` 保留几张图）。
    pub history_limit: usize,
    /// 画廊记录上限：仅由 `enforce_gallery_limit` 消费（决定 gallery.json 保留几行）。
    /// 与 history_limit 是两条独立闸门的上限，**互不换算**；旧用户文件没有该字段，故带缺省。
    #[serde(default = "gallery_limit")]
    pub gallery_record_limit: usize,
    pub concurrency: u32,
    pub naming_pattern: String,
}

/// `gallery_record_limit` 的缺省（出厂 100）：旧用户文件缺该字段时兜底，
/// 不能让一份没有它的设置整份解析失败。
fn gallery_limit() -> usize {
    100
}

impl Default for GenSettings {
    fn default() -> Self {
        Self {
            default_provider_id: "openai".into(),
            history_limit: 20,
            gallery_record_limit: 100,
            concurrency: 2,
            naming_pattern: "{uid}_{index}".into(),
        }
    }
}

/// data/image_ai.json 形状。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAiFile {
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub settings: GenSettings,
}

/// 提示词模板，body 支持 `{{变量|默认值}}`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplate {
    pub id: String,
    pub name: String,
    pub body: String,
    #[serde(default)]
    pub negative: String,
}

/// 码值转译表。注：肤色 0–19 的色卡来自 FM 内置**肤色**代码图鉴（仅作色卡参照）。
/// hairColor 不设字典——FM 发色码语义无权威来源，按「查不到的码退回数字原值」处理。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeDictionary {
    #[serde(default)]
    pub ethnicity: HashMap<String, String>,
    #[serde(default)]
    pub hair_length: HashMap<String, String>,
    #[serde(default)]
    pub skin_code: HashMap<String, String>,
}

/// 一次生成请求。
/// 【Serialize】批量生成队列的条目（image_ai_queue::QueueEntry 内嵌本类型）要经 IPC
/// 回传快照，序列化方向也需要——字段无新增，仅补派生。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenRequest {
    pub provider_id: String,
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: String,
    pub size: String,
    #[serde(default = "one")]
    pub n: u32,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub steps: Option<u32>,
    #[serde(default)]
    pub guidance: Option<f32>,
    #[serde(default)]
    pub uid: Option<String>,
    /// 球员名：仅用于画廊记录展示。前端单张取数据源球员名、批量取入队行名，
    /// 无数据源时缺省 None（记录里 name 为空）。
    #[serde(default)]
    pub name: Option<String>,
    /// 提示词模板 id：随请求带入以写入画廊记录；未选模板（或旧前端不传）时缺省 None。
    #[serde(default)]
    pub template_id: Option<String>,
    /// 调用方式：`Some("async")` = 走异步任务端点，None/缺省 = 同步。
    /// **这不是能力字段**：是否真走异步由 `generate` 依据模型 `supports_async` 门控后回填，
    /// 前端只表达「想用异步」的意图。
    #[serde(default)]
    pub call_mode: Option<String>,
    /// 万相智能改写开关（仅百炼适配器消费；None = 按官方默认 true）。
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
    /// 内部字段（前端不传，serde 缺省为空）：`generate` 解析出服务商后填入 base_url，
    /// 适配器据此拼 endpoint。它与能力无关，故独立于能力快照。
    #[serde(default)]
    pub base_url: String,
    /// 内部字段：`generate` 从模型能力集**整体**拷入的能力快照（`None`/缺省 = 未回填，
    /// 访问器 [`GenRequest::caps`] 按旧字段缺省语义兜底）。用快照而非逐字段摊平：
    /// 新增一个能力位时只需改 `Capabilities` 与消费方两处，不必再动本结构体与拷贝行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caps: Option<Capabilities>,
}

impl GenRequest {
    /// 能力快照访问器：`generate` 已回填时直接返回；缺省（单测直接构造、旧快照回放）时
    /// 走 [`default_caps`] 兜底。适配器一律经此读取，避免 `unwrap_or` 与默认值散落各处。
    pub fn caps(&self) -> Capabilities {
        self.caps.clone().unwrap_or_else(default_caps)
    }
}

/// 能力快照缺省时的兜底：**逐条等价**于改前 `GenRequest` 各能力字段的 serde 缺省。
/// 【为什么不能直接用 `Capabilities::default()`】其 `Default` 派生把 `supports_n` 置 false，
/// 而旧 `GenRequest.supports_n` 缺省为 true（`#[serde(default = "yes")]`）——语义不同。
fn default_caps() -> Capabilities {
    Capabilities {
        supports_n: true,
        prompt_extend_mode_default: "direct".into(),
        enable_thinking_default: true,
        watermark_default: false,
        ..Default::default()
    }
}

/// 一张临时图片元信息。
/// 【Deserialize】批量生成队列的条目经 IPC 往返（image_ai_queue::QueueEntry 内嵌本类型），
/// 反序列化方向也需要它——前端提交的新条目 images 恒为空，字段本身不产生新契约。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenImage {
    pub id: String,
    pub path: String,
    pub bytes: u64,
}

/// 生成命令返回。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenResult {
    pub images: Vec<GenImage>,
}

/// 一条画廊记录：持久保存在 `data_dir/gallery.json`（不随会话清理，与 `.imgtmp` 里的图片字节
/// 是两条独立生命周期）。除 id/ts 外的字段都可缺省，兼容旧文件与后续字段演进。
/// ts 为写入时间（now_ts 格式），该格式字典序即时间序，淘汰与排序据此判断新旧。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryRecord {
    pub id: String,
    pub ts: String,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: String,
    #[serde(default)]
    pub template_id: String,
}

/// 画廊查询返回项：记录本体 + **查询期现算**的图片存在性。存在性绝不写回 gallery.json。
/// path 是现算出的 `.imgtmp` 实际文件路径（扩展名随厂商返回格式变化），图片已被会话清理时为 None；
/// 前端据此决定渲染真图还是占位。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryItem {
    #[serde(flatten)]
    pub record: GalleryRecord,
    pub image_available: bool,
    pub path: Option<String>,
}

// ===================== data/ 路径（与既有路径工具同居一层风格）=====================

pub fn providers_path(d: &Path) -> PathBuf {
    d.join("image_ai.json")
}
pub fn templates_path(d: &Path) -> PathBuf {
    d.join("prompts").join("templates.json")
}
pub fn dictionary_path(d: &Path) -> PathBuf {
    d.join("prompts").join("code_translation.json")
}
pub fn tmp_dir(d: &Path) -> PathBuf {
    d.join(".imgtmp")
}
/// 画廊记录文件：落在 data/ 根、**不在** `.imgtmp` 内——`.imgtmp` 启动即清，
/// 记录必须留在这里才能跨会话存活。
pub fn gallery_path(d: &Path) -> PathBuf {
    d.join("gallery.json")
}

// ===================== Config：内置默认 + 用户文件双来源（仿 templates::RuleConfig）=====================

/// 生图模块生效配置的聚合（AppState 中以 Mutex 托管）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub providers: Vec<ProviderConfig>,
    pub settings: GenSettings,
    pub templates: Vec<PromptTemplate>,
    pub dictionary: CodeDictionary,
}

impl Config {
    /// 解析编译期内嵌的出厂默认。
    pub fn builtin() -> Self {
        let f: ImageAiFile =
            serde_json::from_str(crate::assets::IMAGE_AI_RAW).expect("内置 image_ai.json 格式错误");
        let mut providers = f.providers;
        reconcile_builtin_providers(&mut providers);
        Config {
            providers,
            settings: f.settings,
            templates: serde_json::from_str(crate::assets::IMAGE_AI_TEMPLATES_RAW)
                .expect("内置 image_ai_templates.json 格式错误"),
            dictionary: serde_json::from_str(crate::assets::IMAGE_AI_DICT_RAW)
                .expect("内置 image_ai_code_translation.json 格式错误"),
        }
    }

    /// 加载生效配置：用户文件存在则覆盖对应分组，缺失用内置默认，损坏报错（含路径）。
    pub fn load(d: &Path) -> Result<Self, String> {
        let mut cfg = Self::builtin();
        match json_store::load::<ImageAiFile>(&providers_path(d)) {
            Ok(mut f) => {
                reconcile_builtin_providers(&mut f.providers);
                cfg.providers = f.providers;
                cfg.settings = f.settings;
            }
            Err(LoadError::Missing) => {}
            Err(LoadError::Corrupt(e)) => {
                return Err(format!(
                    "生图配置无法解析：{}（{e}）",
                    providers_path(d).display()
                ))
            }
        }
        match json_store::load::<Vec<PromptTemplate>>(&templates_path(d)) {
            Ok(v) => cfg.templates = v,
            Err(LoadError::Missing) => {}
            Err(LoadError::Corrupt(e)) => {
                return Err(format!(
                    "模板无法解析：{}（{e}）",
                    templates_path(d).display()
                ))
            }
        }
        match json_store::load::<CodeDictionary>(&dictionary_path(d)) {
            Ok(v) => cfg.dictionary = v,
            Err(LoadError::Missing) => {}
            Err(LoadError::Corrupt(e)) => {
                return Err(format!(
                    "码值表无法解析：{}（{e}）",
                    dictionary_path(d).display()
                ))
            }
        }
        Ok(cfg)
    }

    /// 首次运行导出用户文件（已存在不覆盖）；同时把新版本新增的内置服务商/模板补进用户文件。
    pub fn ensure_user_files(d: &Path) -> Result<(), String> {
        fs::create_dir_all(d.join("prompts")).map_err(|e| e.to_string())?;
        let builtin = Self::builtin();

        // image_ai.json：缺失整份导出；存在则补入缺失的内置服务商并校正默认服务商。
        let pp = providers_path(d);
        let mut file = match json_store::load::<ImageAiFile>(&pp) {
            Ok(f) => f,
            Err(LoadError::Missing) => {
                json_store::save(
                    &pp,
                    &ImageAiFile {
                        providers: builtin.providers.clone(),
                        settings: builtin.settings.clone(),
                    },
                )?;
                ImageAiFile {
                    providers: builtin.providers.clone(),
                    settings: builtin.settings.clone(),
                }
            }
            Err(LoadError::Corrupt(e)) => return Err(format!("{}（{e}）", pp.display())),
        };
        let mut changed = false;
        // 代码内置服务商（百炼/火山）：缺失整块注入，已存在只归位 kind；模型行与默认模型
        // 归用户所有（可增删改且不被复活），内置目录的更新不会自动推送到老配置。
        // 详见 reconcile_builtin_providers。
        if reconcile_builtin_providers(&mut file.providers) {
            changed = true;
        }
        for p in &builtin.providers {
            if !file.providers.iter().any(|x| x.id == p.id) {
                file.providers.push(p.clone());
                changed = true;
            }
        }
        if !file
            .providers
            .iter()
            .any(|p| p.id == file.settings.default_provider_id)
        {
            file.settings.default_provider_id = "openai".into();
            changed = true;
        }
        if changed {
            json_store::save(&pp, &file)?;
        }

        // templates.json：缺失导出；存在补入缺失的内置模板。
        let tp = templates_path(d);
        let mut tpls = match json_store::load::<Vec<PromptTemplate>>(&tp) {
            Ok(v) => v,
            Err(LoadError::Missing) => {
                json_store::save(&tp, &builtin.templates)?;
                builtin.templates.clone()
            }
            Err(LoadError::Corrupt(e)) => return Err(format!("{}（{e}）", tp.display())),
        };
        let mut tchanged = false;
        for t in &builtin.templates {
            if !tpls.iter().any(|x| x.id == t.id) {
                tpls.push(t.clone());
                tchanged = true;
            }
        }
        if tchanged {
            json_store::save(&tp, &tpls)?;
        }

        // 码值表：用户全权拥有，**仅缺失时**导出；已存在（含损坏）一律不覆盖、不合并。
        // 注意必须显式区分 Missing 与 Corrupt：用 `is_err()` 会把损坏的用户文件静默覆盖成出厂表。
        let dp = dictionary_path(d);
        match json_store::load::<CodeDictionary>(&dp) {
            Ok(_) => {}
            Err(LoadError::Missing) => json_store::save(&dp, &builtin.dictionary)?,
            Err(LoadError::Corrupt(e)) => {
                log::warn!(target: "image_ai", "码值表损坏，保留原文件不覆盖: {}（{e}）", dp.display());
            }
        }
        Ok(())
    }

    /// 启动会话准备：清空并重建临时图片目录（等价退出清理且抗崩溃）。
    pub fn prepare_session(d: &Path) -> Result<(), String> {
        let tmp = tmp_dir(d);
        if tmp.exists() {
            fs::remove_dir_all(&tmp).map_err(|e| format!("清理临时目录失败: {e}"))?;
        }
        fs::create_dir_all(&tmp).map_err(|e| e.to_string())
    }

    /// 持久化服务商与设置（CRUD 命令收尾调用）。
    pub fn save_providers(&self, d: &Path) -> Result<(), String> {
        json_store::save(
            &providers_path(d),
            &ImageAiFile {
                providers: self.providers.clone(),
                settings: self.settings.clone(),
            },
        )
    }

    pub fn save_templates(&self, d: &Path) -> Result<(), String> {
        json_store::save(&templates_path(d), &self.templates)
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// 查模型明细行；查不到时按「默认模型」兜底合成一行通用模型。
    /// 【为什么兜底】custom（OpenAI 兼容服务）服务商只有 default_model 一个字符串，
    /// 模型明细行（尺寸/能力）要手工编辑 image_ai.json 才有——若校验死盯明细行，
    /// 用户填了默认模型也会被「请先选择模型」拦住（实测反馈）。合成的行为：
    /// 不限张数（max_n=0）、不发 response_format/seed 等高级参数（能力默认 false）；
    /// 要精细控制就在 JSON 里补明细行，明细行优先。
    pub fn model(&self, provider_id: &str, model: &str) -> Option<ModelInfo> {
        let p = self.provider(provider_id)?;
        if let Some(m) = p.models.iter().find(|m| m.model == model) {
            return Some(m.clone());
        }
        if !p.default_model.is_empty() && p.default_model == model {
            return Some(ModelInfo {
                model: model.to_string(),
                capabilities: Capabilities::default(),
            });
        }
        None
    }
}

/// 把代码内置的服务商并入一份服务商列表（builtin/load/ensure 三处同口径）。
///
/// 当前有五个代码内置服务商：百炼 dashscope、火山 volcano、硅基流动 siliconflow、
/// 魔搭 modelscope、本地 ComfyUI，目录均不写 assets/image_ai.json。对每个内置服务商：
/// - 列表中没有：**整块补入内置服务商**（新装/首次导入即得到完整目录）；
/// - 已有：**只归位 kind，模型行/默认模型/名称/地址，
///   由「服务商设置」弹窗内的模型清单增删改。
///   【刻意取舍，不是遗漏】代价是新内置模型不随版本推送、旧模型下线也不自动清理；
///   换来的是「对模型的增删改可保留」——若改成「只补缺失」，用户删掉的内置模型
///   会在下次启动复活，与「可删」自相矛盾。
///
/// 返回是否发生需要落盘的变更（内容已一致时不重写，避免每次启动无谓写文件）。
fn reconcile_builtin_providers(providers: &mut Vec<ProviderConfig>) -> bool {
    [
        crate::image_ai_providers::dashscope::builtin_provider(),
        crate::image_ai_providers::volcano::builtin_provider(),
        crate::image_ai_providers::siliconflow::builtin_provider(),
        crate::image_ai_providers::modelscope::builtin_provider(),
        crate::image_ai_providers::comfyui::builtin_provider(),
    ]
    .into_iter()
    .fold(false, |changed, builtin| {
        changed | reconcile_one_builtin(providers, builtin)
    })
}

/// 单个内置服务商的归并：缺失整块注入；已存在只归位 kind。
fn reconcile_one_builtin(providers: &mut Vec<ProviderConfig>, builtin: ProviderConfig) -> bool {
    match providers.iter_mut().find(|p| p.id == builtin.id) {
        // kind 是代码级不变量（决定适配器分发），是唯一恒由代码接管的字段。
        Some(slot) if slot.kind != builtin.kind => {
            slot.kind = builtin.kind;
            true
        }
        Some(_) => false,
        None => {
            providers.push(builtin);
            true
        }
    }
}

// ===================== CRUD 辅助（命令层调用；校验失败返回中文错误）=====================

pub fn upsert_provider(cfg: &mut Config, mut p: ProviderConfig) -> Result<(), String> {
    p.id = p.id.trim().to_string();
    p.name = p.name.trim().to_string();
    if p.id.is_empty() || p.name.is_empty() {
        return Err("服务商 ID 与名称不能为空".into());
    }
    match cfg.providers.iter_mut().find(|x| x.id == p.id) {
        Some(slot) => *slot = p,
        None => cfg.providers.push(p),
    }
    Ok(())
}

/// 删除服务商。判据是 **id**（不是 kind）——「能否删」讲的是「是不是内置」，
/// 与 kind 讲的「用什么协议」是两个维度；前端删除按钮同用 id 判据（两处必须一致）。
/// openai 不可删：它是 settings.default_provider_id 的兜底项（见 load/ensure 的悬空回退）。
/// dashscope 可删：下次启动 `reconcile_builtin_providers` 会把内置目录整块重新注入，
/// 等于「恢复默认模型行」——这也是老配置同步内置目录变更的另一条路。
pub fn remove_provider(cfg: &mut Config, id: &str) -> Result<(), String> {
    if id == "openai" {
        return Err("内置服务商不可删除（可改名/改地址）".into());
    }
    cfg.providers.retain(|p| p.id != id);
    Ok(())
}

pub fn upsert_template(cfg: &mut Config, mut t: PromptTemplate) -> Result<(), String> {
    t.name = t.name.trim().to_string();
    t.body = t.body.trim().to_string();
    if t.id.trim().is_empty() || t.name.is_empty() || t.body.is_empty() {
        return Err("模板 ID、名称与内容不能为空".into());
    }
    match cfg.templates.iter_mut().find(|x| x.id == t.id) {
        Some(slot) => *slot = t,
        None => cfg.templates.push(t),
    }
    Ok(())
}

pub fn remove_template(cfg: &mut Config, id: &str) -> Result<(), String> {
    if id == "default" {
        return Err("默认模板不可删除".into());
    }
    cfg.templates.retain(|t| t.id != id);
    Ok(())
}

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

/// 生图准入校验的**唯一入口**：解析服务商/模型，并按协议解释厂商约束（张数上限/提示词长度/尺寸规则）。
/// `generate` 与回归测试都经此函数——这样「删掉准入校验」必然会有断言失败（结构性覆盖，而非靠人工 review）。
pub(crate) fn validate_for_generate(cfg: &Config, req: &GenRequest) -> Result<(), ImageGenError> {
    let provider = cfg
        .provider(&req.provider_id)
        .cloned()
        .ok_or_else(|| ImageGenError::not_configured("请先选择服务商"))?;
    let model = cfg
        .model(&req.provider_id, &req.model)
        .ok_or_else(|| ImageGenError::not_configured("请先选择模型"))?;
    // base_url 与能力快照都只在服务商配置里，用探针副本补齐，使探针与 generate 实跑的请求同形。
    // （真正读 req.caps() 的是适配器的请求体组装 build_body；validate 的 caps 走形参，
    //  但同形可避免探针与实跑出现两套输入。）
    let mut probe = req.clone();
    probe.base_url = provider.base_url.clone();
    probe.caps = Some(model.capabilities.clone());
    crate::image_ai_providers::for_kind(provider.kind).validate(&probe, &model.capabilities)
}
// ===================== 码值转译 =====================

#[derive(Debug, Clone, Copy)]
pub enum CodeKind {
    Ethnicity,
    HairLength,
    SkinCode,
}

impl CodeDictionary {
    pub fn translate(&self, kind: CodeKind, code: &str) -> String {
        let map = match kind {
            CodeKind::Ethnicity => &self.ethnicity,
            CodeKind::HairLength => &self.hair_length,
            CodeKind::SkinCode => &self.skin_code,
        };
        map.get(code).cloned().unwrap_or_else(|| code.to_string())
    }
}

// ===================== 模板渲染：{{key}} / {{key|默认值}} =====================

/// 渲染结果（跨 IPC 返回给前端，故需 Serialize + camelCase）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOutcome {
    /// 渲染后的文本（有默认值的占位符展开为默认值，命中 vars 的展开为变量值）。
    pub text: String,
    /// 既无 vars 值、又无默认值的占位符键名（P1 无数据源，出现即需用户手工处理）。
    pub missing: Vec<String>,
}

pub fn render(body: &str, vars: &HashMap<String, String>) -> RenderOutcome {
    let mut out = String::new();
    let mut missing: Vec<String> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(rel) = body[i + 2..].find("}}") {
                let expr = body[i + 2..i + 2 + rel].trim();
                let (key, default) = match expr.split_once('|') {
                    Some((k, d)) => (k.trim(), d.trim().to_string()),
                    None => (expr, String::new()),
                };
                match vars.get(key) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push_str(&default);
                        if !missing.iter().any(|m| m == key) {
                            missing.push(key.to_string());
                        }
                    }
                }
                i = i + 2 + rel + 2;
                continue;
            }
        }
        let ch = body[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    RenderOutcome { text: out, missing }
}

// ===================== 球员数据源 → 模板变量（P3：与 render 同层的纯函数） =====================

/// 从球员记录构建模板变量。**缺失字段一律不入 map**：render 对查不到的键会展开
/// `{{key|默认值}}` 的默认值兜底，模板作者得以按变量是否可用分别写默认文案；
/// 若用空串占位反而会「渲染成功但内容为空」，绕过了默认值机制。
/// 码值转译口径（对齐计划表）：ethnicity/hairLength/skinCode 经码值字典转译
/// （如 "0" → "北欧/白人"、"12" → 色卡号文本）；hairColor 不过字典——FM 发色码
/// 语义无权威来源，色卡即原文（见 CodeDictionary 上的说明）；其余字段一律原文。
/// 基础名单 `detailed` 为 None：gender/skinCode 等拓展字段全部缺席但绝不 panic。
pub fn build_vars(
    rec: &crate::rtf_scanner::PlayerRecord,
    dict: &CodeDictionary,
) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    vars.insert("uid".into(), rec.uid.clone());
    vars.insert("name".into(), rec.name.clone());
    vars.insert("primaryNat".into(), rec.primary_nat.clone());
    vars.insert("secNat".into(), rec.sec_nat.clone());
    if !rec.ethnicity_code.is_empty() {
        vars.insert(
            "ethnicity".into(),
            dict.translate(CodeKind::Ethnicity, &rec.ethnicity_code),
        );
    }
    if !rec.hair_length.is_empty() {
        vars.insert(
            "hairLength".into(),
            dict.translate(CodeKind::HairLength, &rec.hair_length),
        );
    }
    if !rec.hair_color.is_empty() {
        vars.insert("hairColor".into(), rec.hair_color.clone());
    }
    if let Some(d) = &rec.detailed {
        if !d.skin_code.is_empty() {
            vars.insert(
                "skinCode".into(),
                dict.translate(CodeKind::SkinCode, &d.skin_code),
            );
        }
        for (key, val) in [
            ("gender", &d.gender),
            ("age", &d.age),
            ("height", &d.height),
            ("club", &d.club),
            ("ca", &d.ca),
            ("pa", &d.pa),
        ] {
            if !val.is_empty() {
                vars.insert(key.into(), val.clone());
            }
        }
    }
    vars
}

// ===================== 生成编排：适配器 → 字节 → 临时文件 → 估价 =====================

pub fn new_id() -> String {
    let bytes: [u8; 8] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now_ts() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 执行一次生成并写台账（成功/失败都写）。失败台账写入失败不掩盖原始错误。
pub async fn generate(
    data_dir: &Path,
    cfg: &Config,
    key: &str,
    mut req: GenRequest,
    progress: &(dyn Fn(crate::image_ai_providers::GenStage) + Sync),
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

    // 调用方式门控：仅当用户显式选异步、且该模型声明支持异步时才置 "async"，否则一律回落同步
    // （校验与 HTTP 都据此分流，见 DashscopeProvider::generate）。
    req.call_mode =
        if req.call_mode.as_deref() == Some("async") && model.capabilities.supports_async {
            Some("async".into())
        } else {
            None
        };

    // 【唯一准入校验入口】厂商约束（张数上限/提示词长度/尺寸规则）在此解释；失败即返回，绝不发出 HTTP。
    // 抽成 validate_for_generate 便于回归测试锚定「确实调用了校验」。
    validate_for_generate(cfg, &req)?;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // 百炼三系列文生图统一为同步长连接（千问 3.0 多图/思考模式官方建议客户端超时 600s 起），
        // 该 client 四家共用，放宽上限不影响 OpenAI/火山的正常返回（它们本就秒级完成）。
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| ImageGenError::network(format!("HTTP 客户端初始化失败: {e}")))?;
    let adapter = crate::image_ai_providers::for_kind(provider.kind);
    // 适配器内部完成 POST/解析/（异步厂商的 submit→poll→fetch），图片已为字节；
    // 执行阶段（提交/轮询/下载）由适配器经 progress 回调上报，本层只透传。
    let fetched = adapter.generate(&client, key, &req, progress).await?;

    let tmp = tmp_dir(data_dir);
    fs::create_dir_all(&tmp).map_err(|e| ImageGenError::io(e.to_string()))?;
    let mut images = Vec::new();
    for img in fetched.images {
        let id = new_id();
        let path = tmp.join(format!("{id}.{}", img.extension));
        fs::write(&path, &img.bytes)
            .map_err(|e| ImageGenError::io(format!("临时图片写入失败: {e}")))?;
        images.push(GenImage {
            id,
            path: path.to_string_lossy().to_string(),
            bytes: img.bytes.len() as u64,
        });
    }
    // 记录持久化与图片淘汰是两条互不越界的闸门：任一失败都不应掩盖已成功的生成（图片已落盘）。
    if let Err(e) = record_gallery(data_dir, &images, &req, cfg.settings.gallery_record_limit) {
        log::warn!(target: "image_ai", "写入画廊记录失败（不影响本次生成）: {e}");
    }
    if let Err(e) = enforce_image_limit(data_dir, cfg.settings.history_limit) {
        log::warn!(target: "image_ai", "裁剪临时图片失败（不影响本次生成）: {e}");
    }
    Ok(GenResult { images })
}

// ===================== 画廊记录（data_dir/gallery.json）+ 两套独立淘汰闸门 =====================
// 记录持久、图片仅存活于会话临时目录（.imgtmp，启动即清）——两条生命周期各自独立，
// 淘汰因此拆成两条互不越界的闸门：记录闸门只删 gallery.json 的行、图片闸门只删 .imgtmp 的字节。
// 任何「删记录行 + 连带删图」的一体实现都会让一条闸门的动作越界影响另一条，禁止再引入。
//
// 【并发】gallery.json 的写入是「read → push → save」的非原子读改写：批量生成默认并发 2，
// 多个任务各读到同一份旧表再整体覆盖写，后写者会静默丢掉先写者刚追加的行（json_store::save
// 的 tmp+rename 只保证单文件不损坏，不防丢更新）。故用一把模块级互斥把 gallery.json 的全部 I/O
// 串行化。互斥只覆盖同步 fs 段、绝不跨 await；不碰 gallery.json 的 enforce_image_limit 不参与加锁。
static GALLERY_IO: Mutex<()> = Mutex::new(());

/// 读画廊清单的**不加锁内核**：调用方必须已持有 GALLERY_IO。
/// 把「加锁入口 / 不加锁内核」分开，是为了让取锁的调用方在自己的临界区内直接调它，
/// 避免「加锁函数互相调用」造成的重入死锁（std Mutex 不可重入）。
/// 缺失（首次写入前）按空表；损坏同样按空表并告警——记录损坏不值得阻断生成，下次写入即重建。
fn read_gallery_inner(path: &Path) -> Vec<GalleryRecord> {
    match json_store::load::<Vec<GalleryRecord>>(path) {
        Ok(v) => v,
        Err(LoadError::Missing) => Vec::new(),
        Err(LoadError::Corrupt(e)) => {
            log::warn!(target: "image_ai", "画廊记录损坏，按空记录重建: {}（{e}）", path.display());
            Vec::new()
        }
    }
}

/// 把本次生成的每张图记入画廊（每图一条），再按记录上限淘汰旧行。
/// 仅 generate() 成功路径调用——失败路径图片未落盘，不产生记录。
/// 记录字段取自请求（model/prompt/negative/uid/name/template_id）。
pub fn record_gallery(
    data_dir: &Path,
    images: &[GenImage],
    req: &GenRequest,
    limit: usize,
) -> Result<(), String> {
    let path = gallery_path(data_dir);
    let ts = now_ts();
    {
        // 「读—改—写」在同一临界区内完成：并发调用被串行化，杜绝后写者覆盖前写者的丢更新。
        let _guard = GALLERY_IO.lock().unwrap();
        let mut recs = read_gallery_inner(&path);
        for img in images {
            recs.push(GalleryRecord {
                id: img.id.clone(),
                ts: ts.clone(),
                uid: req.uid.clone(),
                name: req.name.clone(),
                model: req.model.clone(),
                prompt: req.prompt.clone(),
                negative_prompt: req.negative_prompt.clone(),
                template_id: req.template_id.clone().unwrap_or_default(),
            });
        }
        json_store::save(&path, &recs)?;
    }
    // 裁剪另起一次加锁（与上面的写不嵌套，避免 std Mutex 重入死锁）：只裁行、不碰图。
    enforce_gallery_limit(data_dir, limit)
}

/// 图片淘汰闸门：`.imgtmp` 内最多保留修改时间最新的 limit 个文件，超出者删除。
/// 【只碰图片字节】按文件自身 mtime 判新旧，**不读也不写 gallery.json**——这才与记录闸门解耦：
/// 记录可以多于图片（图片被删后对应记录仍在，查询期现算为「无图」并显示占位）。
/// limit==0 视为「不裁剪」（与记录闸门同口径，两闸门互不换算）。
pub fn enforce_image_limit(data_dir: &Path, limit: usize) -> Result<(), String> {
    if limit == 0 {
        return Ok(());
    }
    let Ok(rd) = fs::read_dir(tmp_dir(data_dir)) else {
        return Ok(());
    };
    let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        // 取不到 mtime 的文件不参与裁剪：宁可少删，不可误删新旧不明的图片。
        if let Ok(m) = entry.metadata().and_then(|md| md.modified()) {
            files.push((p, m));
        }
    }
    if files.len() <= limit {
        return Ok(());
    }
    // mtime 升序（旧在前）；同刻按路径名兜底，确保裁剪结果确定。
    files.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    for (p, _) in files.iter().take(files.len() - limit) {
        if let Err(e) = fs::remove_file(p) {
            log::warn!(target: "image_ai", "清理过期临时图片失败 {}: {e}", p.display());
        }
    }
    Ok(())
}

/// 记录淘汰闸门：按 ts 升序保留最新的 limit 行，超出者从 gallery.json 删除。
/// 【只碰记录行】绝不删除 `.imgtmp` 的图片——图片存亡只由 enforce_image_limit 决定。
/// limit==0 视为「不裁剪」。
pub fn enforce_gallery_limit(data_dir: &Path, limit: usize) -> Result<(), String> {
    if limit == 0 {
        return Ok(());
    }
    // 「读—裁—写」整段持锁：与 record_gallery 的追加写共用同一把锁，两处不嵌套（防重入死锁）。
    let _guard = GALLERY_IO.lock().unwrap();
    let path = gallery_path(data_dir);
    let mut recs = read_gallery_inner(&path);
    if recs.len() <= limit {
        return Ok(());
    }
    // ts 为 "%Y-%m-%d %H:%M:%S"，字典序即时间序；升序后旧的聚在前，裁剪结果确定。
    recs.sort_by(|a, b| a.ts.cmp(&b.ts));
    recs.drain(..recs.len() - limit);
    json_store::save(&path, &recs)
}

/// 组装画廊查询结果（最新在前）：逐条现算图片是否仍在 `.imgtmp`。
/// 【存在性只在查询期计算】绝不把结果写回 gallery.json——否则一次查询就污染持久清单，
/// 且图片的临时性会让写回的「存在」在下次启动即刻失真。
pub fn load_gallery_items(data_dir: &Path) -> Vec<GalleryItem> {
    // .imgtmp 只扫一次建 stem→路径 表：避免按记录逐条重扫目录（记录上限可达上百条）。
    let mut by_stem: HashMap<String, PathBuf> = HashMap::new();
    if let Ok(rd) = fs::read_dir(tmp_dir(data_dir)) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    by_stem.insert(stem.to_string(), p);
                }
            }
        }
    }
    // 读清单也走同一把锁：把 gallery.json 的全部 I/O 收敛到单一串行点。save 是 tmp+rename 原子
    // 替换、读本身不会见到半截文件，此处加锁只为与写共用同一条串行纪律（不拆分两套同步点）。
    let mut recs = {
        let _guard = GALLERY_IO.lock().unwrap();
        read_gallery_inner(&gallery_path(data_dir))
    };
    recs.sort_by(|a, b| b.ts.cmp(&a.ts));
    recs.into_iter()
        .map(|record| {
            let path = by_stem.get(&record.id).cloned();
            GalleryItem {
                image_available: path.is_some(),
                path: path.map(|p| p.to_string_lossy().to_string()),
                record,
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtf_scanner::{DetailedInfo, PlayerRecord}; // build_vars 单测的样本类型

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn gen_req() -> GenRequest {
        GenRequest {
            provider_id: "openai".into(),
            model: "gpt-image-1".into(),
            prompt: "一张脸".into(),
            negative_prompt: String::new(),
            size: "1024x1024".into(),
            n: 2,
            quality: Some("high".into()),
            seed: None,
            steps: None,
            guidance: None,
            uid: None,
            name: None,
            template_id: None,
            call_mode: None,
            prompt_extend: None,
            watermark: None,
            prompt_extend_mode: None,
            enable_thinking: None,
            base_url: String::new(),
            // 单测直接构造：能力快照留空，由 caps() 兜底或由 validate_for_generate 回填。
            caps: None,
        }
    }

    #[test]
    fn provider_kind_serializes_lowercase() {
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
        // 已合并：兼容服务与官方同档，不存在 custom 变体
        assert!(serde_json::from_str::<ProviderKind>(r#""custom""#).is_err());
    }

    #[test]
    fn gen_request_defaults_n_to_one() {
        let v: GenRequest = serde_json::from_str(
            r#"{"providerId":"openai","model":"gpt-image-1","prompt":"x","size":"1024x1024"}"#,
        )
        .unwrap();
        assert_eq!(v.n, 1);
    }

    #[test]
    fn caps_none_falls_back_to_legacy_serde_defaults() {
        // 快照缺省时 caps() 的兜底必须与改前 GenRequest 各能力字段的 serde 缺省**逐条等价**。
        let r = gen_req(); // 直接构造：caps = None
        assert!(r.caps.is_none());
        let c = r.caps();
        assert!(!c.supports_response_format); // 旧字段缺省 false
        assert!(!c.supports_negative_prompt);
        assert!(!c.supports_prompt_extend);
        assert!(!c.supports_watermark);
        assert!(!c.supports_seed);
        assert!(c.supports_n); // 旧缺省 true
        assert!(!c.supports_prompt_extend_mode);
        assert_eq!(c.prompt_extend_mode_default, "direct");
        assert!(!c.supports_enable_thinking);
        assert!(c.enable_thinking_default);
        assert!(!c.watermark_default);
        // 对照：Capabilities::default() 的 supports_n 为 false，故兜底不能省。
        assert!(!Capabilities::default().supports_n);
    }

    #[test]
    fn caps_none_from_legacy_snapshot_uses_fallback() {
        // 旧快照（JSON 无 caps 字段）反序列化后 caps==None，访问器同样给旧语义。
        let v: GenRequest = serde_json::from_str(
            r#"{"providerId":"dashscope","model":"z-image-turbo","prompt":"x","size":"1024x1024"}"#,
        )
        .unwrap();
        assert!(v.caps.is_none());
        assert!(v.caps().supports_n && v.caps().enable_thinking_default);
        assert_eq!(v.caps().prompt_extend_mode_default, "direct");
    }

    #[test]
    fn ensure_creates_files_and_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        assert!(providers_path(root.path()).is_file());
        assert!(templates_path(root.path()).is_file());
        assert!(dictionary_path(root.path()).is_file());
        let before = fs::read_to_string(providers_path(root.path())).unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        assert_eq!(
            before,
            fs::read_to_string(providers_path(root.path())).unwrap()
        );
    }

    #[test]
    fn ensure_injects_siliconflow_and_modelscope_into_existing_file() {
        // 升级路径：老用户文件里没有 siliconflow / modelscope —— 启动时 ensure_user_files
        // 必须把两个服务商整块补入并落盘，否则界面上看不到它们（「正常加载」的可证证据）。
        let root = tempfile::tempdir().unwrap();
        let existing = serde_json::json!({
            "providers": [{
                "id": "openai",
                "kind": "openai",
                "name": "OpenAI",
                "baseUrl": "https://api.openai.com/v1",
                "defaultModel": "gpt-image-1",
                "models": [{ "model": "gpt-image-1", "capabilities": {} }]
            }],
            "settings": {
                "defaultProviderId": "openai",
                "historyLimit": 20,
                "concurrency": 2,
                "namingPattern": "{uid}_{index}"
            }
        });
        fs::write(providers_path(root.path()), existing.to_string()).unwrap();

        Config::ensure_user_files(root.path()).unwrap();

        let cfg = Config::load(root.path()).unwrap();
        let sf = cfg.provider("siliconflow").expect("硅基流动必须被补入");
        assert_eq!(sf.kind, ProviderKind::Siliconflow);
        assert_eq!(sf.base_url, "https://api.siliconflow.cn/v1");
        assert_eq!(sf.default_model, "Qwen/Qwen-Image");
        assert_eq!(sf.models.len(), 2);
        let ms = cfg.provider("modelscope").expect("魔搭必须被补入");
        assert_eq!(ms.kind, ProviderKind::Modelscope);
        assert_eq!(ms.base_url, "https://api-inference.modelscope.cn/v1");
        assert_eq!(ms.default_model, "Qwen/Qwen-Image-2.1");
        assert_eq!(ms.models.len(), 3);
        // 用户原有的 openai 条不被覆盖。
        assert_eq!(cfg.provider("openai").unwrap().default_model, "gpt-image-1");
    }

    #[test]
    fn prepare_session_wipes_tmp_dir() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp_dir(root.path())).unwrap();
        fs::write(tmp_dir(root.path()).join("x.png"), b"x").unwrap();
        Config::prepare_session(root.path()).unwrap();
        assert!(!tmp_dir(root.path()).join("x.png").exists());
    }

    #[test]
    fn user_edits_are_kept() {
        let root = tempfile::tempdir().unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        let mut cfg = Config::load(root.path()).unwrap();
        cfg.providers[0].name = "MyOpenAI".into();
        cfg.save_providers(root.path()).unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        assert_eq!(
            Config::load(root.path()).unwrap().providers[0].name,
            "MyOpenAI"
        );
    }

    #[test]
    fn corrupt_dictionary_is_not_overwritten() {
        // ensure_user_files 必须显式区分 Missing/Corrupt：损坏的用户码值表不得被静默覆盖。
        let root = tempfile::tempdir().unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        let dp = dictionary_path(root.path());
        fs::write(&dp, b"{ not json").unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        assert_eq!(fs::read_to_string(&dp).unwrap(), "{ not json");
    }

    #[test]
    fn translates_known_and_unknown_code() {
        let d = Config::builtin().dictionary;
        assert_eq!(d.translate(CodeKind::Ethnicity, "10"), "东亚");
        // skinCode 0-19 覆盖肤色色卡：抽查最浅与最深。
        assert_eq!(d.translate(CodeKind::SkinCode, "0"), "极浅暖棕（#fcecdc）");
        assert_eq!(d.translate(CodeKind::SkinCode, "19"), "近黑褐色（#5c2c1c）");
        // 超出 0-19 的码回退数字原值。
        assert_eq!(d.translate(CodeKind::SkinCode, "20"), "20");
    }

    #[test]
    fn render_replaces_and_reports_missing() {
        let r = render(
            "一名{{ethnicity}}球员，{{age}}岁",
            &vars(&[("ethnicity", "东亚"), ("age", "18")]),
        );
        assert_eq!(r.text, "一名东亚球员，18岁");
        assert!(r.missing.is_empty());
        let r = render("{{hairLength|短发}}的{{name|球员}}", &vars(&[]));
        assert_eq!(r.text, "短发的球员");
        assert_eq!(r.missing, vec!["hairLength", "name"]);
        let r = render("只有 { 一个括号", &vars(&[]));
        assert_eq!(r.text, "只有 { 一个括号");
    }

    #[test]
    fn builtin_openai_disables_response_format() {
        // 出厂 gpt-image-1 必须关掉该能力。
        let cfg = Config::builtin();
        let openai = cfg.model("openai", "gpt-image-1").unwrap();
        assert!(!openai.capabilities.supports_response_format);
    }

    #[test]
    fn default_model_without_detail_row_is_served() {
        // 自建 OpenAI 兼容服务只有 default_model 字符串、没有模型明细行：
        // 请求默认模型必须放行（合成通用能力），这是用户实测被「请先选择模型」拦住的根因。
        let mut cfg = Config::builtin();
        upsert_provider(
            &mut cfg,
            ProviderConfig {
                id: "my-relay".into(),
                kind: ProviderKind::OpenAI, // 兼容服务与官方同档（Custom 已合并）
                name: "中转".into(),
                base_url: "https://x/compatible-mode/v1".into(),
                default_model: "qwen-image-3.0".into(),
                models: vec![], // 明细行缺失：正是用户文件的真实形状
            },
        )
        .unwrap();
        let m = cfg.model("my-relay", "qwen-image-3.0").unwrap();
        assert_eq!(m.model, "qwen-image-3.0");
        assert_eq!(m.capabilities.max_n, 0, "0 = 不限张数");
        // 请求的不是默认模型：仍然拒绝（提示用户去设置）。
        assert!(cfg.model("my-relay", "other-model").is_none());
    }

    #[test]
    fn ids_are_unique() {
        assert_ne!(new_id(), new_id());
        assert_eq!(new_id().len(), 16);
    }

    #[test]
    fn validate_complete_rejects_missing_fields_and_bad_counts() {
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
    }

    #[test]
    fn validate_for_generate_rejects_over_limit_and_unknown_provider() {
        // 结构性覆盖：断言 generate 的准入校验函数确实落到了 Provider 层并返回厂商约束错误——
        // 若 generate 不再调用校验、或该函数不再委派 for_kind(...).validate，这些断言会失败。
        let cfg = Config::builtin();
        // 已知服务商/模型但张数超上限（openai/gpt-image-1 maxN=10）→ InvalidParam。
        let mut r = gen_req();
        r.n = 99;
        assert_eq!(
            validate_for_generate(&cfg, &r).unwrap_err().kind,
            GenErrorKind::InvalidParam
        );
        // 未知服务商 → NotConfigured。
        let mut r = gen_req();
        r.provider_id = "nope".into();
        assert_eq!(
            validate_for_generate(&cfg, &r).unwrap_err().kind,
            GenErrorKind::NotConfigured
        );
        // 合规请求放行。
        assert!(validate_for_generate(&cfg, &gen_req()).is_ok());
    }

    #[test]
    fn gen_error_classifies_http_status_and_business_code() {
        // 429 → RateLimited 且可重试
        let e = from_http_status(429, "HTTP 429：限流");
        assert_eq!(e.kind, GenErrorKind::RateLimited);
        assert!(e.retryable);
        // 401/403 → Auth 且不可重试（Key 错误重试无意义，先修配置）
        for s in [401u16, 403] {
            let e = from_http_status(s, "HTTP：鉴权失败");
            assert_eq!(e.kind, GenErrorKind::Auth);
            assert!(!e.retryable);
        }
        // 400 → InvalidParam
        assert_eq!(from_http_status(400, "x").kind, GenErrorKind::InvalidParam);
        // 5xx → Provider(可重试)；其余 4xx → Provider(不可重试)
        let p5 = from_http_status(503, "x");
        assert_eq!(p5.kind, GenErrorKind::Provider);
        assert!(p5.retryable);
        let p4 = from_http_status(418, "x");
        assert_eq!(p4.kind, GenErrorKind::Provider);
        assert!(!p4.retryable);
        // 业务码优先于 status：InvalidApiKey→Auth、Throttling→RateLimited、DataInspection→ContentRejected
        assert_eq!(
            classify_provider_error(200, Some("InvalidApiKey"), "x").kind,
            GenErrorKind::Auth
        );
        assert_eq!(
            classify_provider_error(200, Some("Throttling"), "x").kind,
            GenErrorKind::RateLimited
        );
        assert_eq!(
            classify_provider_error(200, Some("DataInspection"), "x").kind,
            GenErrorKind::ContentRejected
        );
        // 内容审核的另一个码（官方与 DataInspectionFailed 并列给出）：自救手段同在提示词侧。
        assert_eq!(
            classify_provider_error(200, Some("IPInfringementSuspect"), "x").kind,
            GenErrorKind::ContentRejected
        );
        // 建议随类别给出；Cancelled/Internal 无建议
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
    }

    #[test]
    fn builtin_has_six_providers() {
        // openai 来自 assets JSON；dashscope/volcano/siliconflow/modelscope/comfyui 由 reconcile 顺序注入。
        let cfg = Config::builtin();
        let ids: Vec<&str> = cfg.providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "openai",
                "dashscope",
                "volcano",
                "siliconflow",
                "modelscope",
                "localcomfyui"
            ]
        );
        // 本地 ComfyUI：免 Key（协议无鉴权），模型名 = checkpoint 文件名（占位行，归用户改）。
        let comfy = cfg.provider("localcomfyui").unwrap();
        assert_eq!(comfy.kind, ProviderKind::LocalComfyUI);
        assert!(!comfy.kind.needs_key());
        assert_eq!(comfy.base_url, "http://127.0.0.1:8188");
        // 火山能力抽查：新模型 ID、单张、无 seed/n、有水印与 output_format。
        let flash = cfg
            .model("volcano", "doubao-seedream-5-0-flash-260915")
            .unwrap();
        assert_eq!(flash.capabilities.max_n, 1);
        assert!(!flash.capabilities.supports_seed, "Seedream 全系列无 seed");
        assert!(!flash.capabilities.supports_n, "Seedream 全系列无 n 参数");
        assert!(!flash.capabilities.supports_quality);
        assert!(flash.capabilities.supports_watermark);
        assert!(!flash.capabilities.watermark_default);
        assert!(
            flash.capabilities.supports_output_format,
            "Flash 可设 png/jpeg"
        );
        // 4.5 只回 jpeg：不能发 output_format。
        let v45 = cfg.model("volcano", "doubao-seedream-4-5-251128").unwrap();
        assert!(!v45.capabilities.supports_output_format);
        // 旧出厂模型名不得再查得到。
        assert!(cfg.model("volcano", "seedream-5.0-flash").is_none());
        // 百炼模型目录代码内置：三系列 6 个模型抽查关键差异。
        let qwen = cfg.model("dashscope", "qwen-image-3.0-pro").unwrap();
        assert_eq!(qwen.capabilities.max_n, 6);
        assert!(qwen.capabilities.supports_negative_prompt);
        assert!(qwen.capabilities.prompt_extend_default);
        assert!(qwen.capabilities.sizes.contains(&"2048*2048".to_string()));
        let wan27 = cfg.model("dashscope", "wan2.7-image-pro").unwrap();
        assert!(wan27.capabilities.sizes.contains(&"4K".to_string()));
        assert!(!wan27.capabilities.supports_negative_prompt);
        assert!(!wan27.capabilities.supports_prompt_extend);
        let z = cfg.model("dashscope", "z-image-turbo").unwrap();
        assert_eq!(z.capabilities.max_n, 1);
        assert!(!z.capabilities.supports_n);
        assert!(!z.capabilities.prompt_extend_default);
        // 旧模型（wanx2.1）已下线：目录中查不到。
        assert!(cfg.model("dashscope", "wanx2.1-t2i-turbo").is_none());

        // 硅基流动：Qwen-Image 无 batch_size、字段名走 image_size；Kolors 有 batch_size + guidance。
        let sf_qwen = cfg.model("siliconflow", "Qwen/Qwen-Image").unwrap();
        assert!(!sf_qwen.capabilities.supports_n, "Qwen-Image 无 batch_size");
        assert!(sf_qwen.capabilities.supports_steps);
        assert!(!sf_qwen.capabilities.supports_guidance);
        let sf_kolors = cfg.model("siliconflow", "Kwai-Kolors/Kolors").unwrap();
        assert!(sf_kolors.capabilities.supports_n, "Kolors 有 batch_size");
        assert!(sf_kolors.capabilities.supports_guidance);
        assert_eq!(sf_kolors.capabilities.max_n, 4);

        // 魔搭：单张、无 n；支持 steps/guidance。
        let ms = cfg.model("modelscope", "Qwen/Qwen-Image-2.1").unwrap();
        assert_eq!(ms.capabilities.max_n, 1);
        assert!(!ms.capabilities.supports_n);
        assert!(ms.capabilities.supports_steps && ms.capabilities.supports_guidance);
    }

    #[test]
    fn dashscope_catalog_is_injected_even_when_json_has_no_entry() {
        // assets/image_ai.json 已不含 dashscope 块：builtin 仍须注入代码内置目录。
        let cfg = Config::builtin();
        let p = cfg.provider("dashscope").expect("内置百炼服务商必须存在");
        assert_eq!(p.default_model, "z-image-turbo");
        assert_eq!(p.models.len(), 6);
    }

    #[test]
    fn reconcile_leaves_existing_dashscope_entry_to_user() {
        // 既有 dashscope 条：模型行与默认模型**归用户所有**（弹窗内可增删改），
        // reconcile 只做 kind 归位，绝不整表替换 —— 否则用户删掉的模型会在启动时复活。
        // 列表同时带上已是新目录的 volcano：排除「火山注入」对变更返回值的干扰，
        // 单用例锚定「百炼用户条 kind 正确时零变更（不触发无谓写盘）」这条口径。
        let mut providers = vec![
            ProviderConfig {
                id: "openai".into(),
                kind: ProviderKind::OpenAI,
                name: "OpenAI".into(),
                base_url: "https://api.openai.com/v1".into(),
                default_model: "gpt-image-1".into(),
                models: vec![],
            },
            ProviderConfig {
                id: "dashscope".into(),
                kind: ProviderKind::Dashscope,
                name: "我的百炼代理".into(),
                base_url: "https://proxy.example.com/api/v1".into(),
                default_model: "wanx2.1-t2i-turbo".into(),
                models: vec![ModelInfo {
                    model: "wanx2.1-t2i-turbo".into(),
                    capabilities: Capabilities::default(),
                }],
            },
            crate::image_ai_providers::volcano::builtin_provider(),
            crate::image_ai_providers::siliconflow::builtin_provider(),
            crate::image_ai_providers::modelscope::builtin_provider(),
            crate::image_ai_providers::comfyui::builtin_provider(),
        ];
        // kind 已正确、其余内置项也都是新目录 → 无任何变更（不触发无谓写盘）。
        assert!(!reconcile_builtin_providers(&mut providers));
        let ds = providers.iter().find(|p| p.id == "dashscope").unwrap();
        // 用户定制全部保留：名称 / 地址 / 模型行 / 默认模型
        assert_eq!(ds.name, "我的百炼代理");
        assert_eq!(ds.base_url, "https://proxy.example.com/api/v1");
        assert_eq!(ds.models.len(), 1, "模型行不被内置目录整表替换");
        assert_eq!(ds.default_model, "wanx2.1-t2i-turbo", "默认模型不再被校正");
    }

    #[test]
    fn reconcile_normalizes_dashscope_kind_only() {
        // kind 是唯一仍由代码接管的字段（决定适配器分发）；归位时不得顺带补模型行。
        let mut providers = vec![ProviderConfig {
            id: "dashscope".into(),
            kind: ProviderKind::OpenAI, // 错值：必须归位
            name: "百炼".into(),
            base_url: String::new(),
            default_model: "qwen-image-3.0".into(),
            models: vec![],
        }];
        // 首次调用：dashscope kind 归位 + 其余内置（volcano/siliconflow/modelscope）缺失被注入，
        // 故整体为「有变更」。
        assert!(reconcile_builtin_providers(&mut providers));
        let ds = providers.iter().find(|p| p.id == "dashscope").unwrap();
        assert_eq!(ds.kind, ProviderKind::Dashscope);
        assert_eq!(ds.models.len(), 0, "kind 归位不得顺带补入内置模型行");
        // 缺失的 volcano 已整块注入新目录。
        let vol = providers.iter().find(|p| p.id == "volcano").unwrap();
        assert_eq!(vol.default_model, "doubao-seedream-5-0-flash-260915");
        assert_eq!(vol.models.len(), 5);
        assert!(
            !reconcile_builtin_providers(&mut providers),
            "归位/注入后幂等"
        );
    }

    #[test]
    fn reconcile_leaves_existing_volcano_entry_to_user() {
        // 用户已有的火山块（哪怕模型是自己填的或旧的）：已存在就只归位 kind，
        // 模型行/默认模型/名称/地址一律不接管（可增删改且不被覆盖），与百炼同口径。
        let user_entry = ProviderConfig {
            id: "volcano".into(),
            kind: ProviderKind::OpenAI, // 错值：仍须归位
            name: "我的火山".into(),
            base_url: "https://example.com".into(),
            default_model: "my-seedream".into(),
            models: vec![ModelInfo {
                model: "my-seedream".into(),
                capabilities: Capabilities::default(),
            }],
        };
        let mut providers = vec![user_entry];
        assert!(
            reconcile_builtin_providers(&mut providers),
            "kind 归位算变更"
        );
        let vol = providers.iter().find(|x| x.id == "volcano").unwrap();
        assert_eq!(vol.kind, ProviderKind::Volcano);
        assert_eq!(vol.models.len(), 1, "用户模型不得被内置目录覆盖");
        assert!(vol.models.iter().any(|m| m.model == "my-seedream"));
        assert_eq!(vol.default_model, "my-seedream", "默认模型不被校正");
        assert_eq!(vol.base_url, "https://example.com", "地址不被接管");
        assert!(!reconcile_builtin_providers(&mut providers), "归位后幂等");
    }

    #[test]
    fn volcano_catalog_is_injected_even_when_json_has_no_entry() {
        // assets/image_ai.json 已不含 volcano 块：builtin 仍须注入代码内置的五模型目录。
        let cfg = Config::builtin();
        let p = cfg.provider("volcano").expect("内置火山服务商必须存在");
        assert_eq!(p.kind, ProviderKind::Volcano);
        assert_eq!(p.default_model, "doubao-seedream-5-0-flash-260915");
        assert_eq!(p.models.len(), 5);
    }

    #[test]
    fn ensure_keeps_existing_dashscope_entry_untouched() {
        // 既有 dashscope 条的模型行归用户所有：ensure 不得用内置目录覆盖它，
        // 否则用户在弹窗里增删改的模型会被启动流程抹掉。
        let root = tempfile::tempdir().unwrap();
        let existing = serde_json::json!({
            "providers": [{
                "id": "dashscope",
                "kind": "dashscope",
                "name": "百炼通义万相",
                "baseUrl": "https://dashscope.aliyuncs.com/api/v1",
                "defaultModel": "my-own-model",
                "models": [{
                    "model": "my-own-model",
                    "capabilities": {}
                }]
            }],
            "settings": {
                "defaultProviderId": "dashscope",
                "historyLimit": 20,
                "concurrency": 2,
                "namingPattern": "{uid}_{index}"
            }
        });
        fs::write(providers_path(root.path()), existing.to_string()).unwrap();

        Config::ensure_user_files(root.path()).unwrap();

        let cfg = Config::load(root.path()).unwrap();
        let ds = cfg.provider("dashscope").unwrap();
        assert_eq!(ds.name, "百炼通义万相");
        assert_eq!(ds.models.len(), 1, "不整表补入内置目录");
        assert_eq!(ds.models[0].model, "my-own-model");
        assert_eq!(ds.default_model, "my-own-model", "默认模型不被校正");
        // 缺失的内置服务商仍会补齐（openai / volcano），dashscope 不在其中。
        assert!(cfg.provider("openai").is_some());
        assert!(cfg.provider("volcano").is_some());
        // 再次 ensure 幂等：不再产生写盘变更。
        let before = fs::read_to_string(providers_path(root.path())).unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        assert_eq!(
            before,
            fs::read_to_string(providers_path(root.path())).unwrap()
        );
    }

    // ===================== 画廊记录 + 双淘汰闸门 =====================

    /// 在 .imgtmp 造一张假图文件（图片闸门按文件本身处理，扩展名任意）。TempDir 必须先绑定再使用。
    fn fake_image(root: &Path, id: &str) -> PathBuf {
        fs::create_dir_all(tmp_dir(root)).unwrap();
        let p = tmp_dir(root).join(format!("{id}.png"));
        fs::write(&p, b"img").unwrap();
        p
    }

    /// 把文件 mtime 设为给定秒数：图片闸门按 mtime 判新旧，测试必须显式控制顺序，
    /// 不能依赖「写入先后」在文件系统时间粒度下是否可分。
    fn set_mtime(p: &Path, secs: u64) {
        fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    /// 造一条指定秒数时间戳的画面记录（"%H:%M:%S" 字典序即时间序，秒数递增即时间递增）。
    fn gallery_rec(id: &str, sec: u32) -> GalleryRecord {
        GalleryRecord {
            id: id.into(),
            ts: format!("2026-09-26 10:00:{sec:02}"),
            uid: Some("u1".into()),
            name: Some("甲".into()),
            model: "m1".into(),
            prompt: "p".into(),
            negative_prompt: "n".into(),
            template_id: String::new(),
        }
    }

    /// 测试里读回画廊清单（LoadError 未实现 Debug，不能对 json_store::load 直接 unwrap）。
    fn load_gallery(root: &Path) -> Vec<GalleryRecord> {
        serde_json::from_str(&fs::read_to_string(gallery_path(root)).unwrap()).unwrap()
    }

    #[test]
    fn enforce_image_limit_deletes_only_images() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..3).map(|_| new_id()).collect();
        for (i, id) in ids.iter().enumerate() {
            let p = fake_image(dir.path(), id);
            set_mtime(&p, 1_000_000 + i as u64); // ids[0] 最旧
        }
        let recs: Vec<GalleryRecord> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| gallery_rec(id, i as u32))
            .collect();
        json_store::save(&gallery_path(dir.path()), &recs).unwrap();
        let before = fs::read_to_string(gallery_path(dir.path())).unwrap();

        enforce_image_limit(dir.path(), 2).unwrap();

        // 最旧 1 张图被删，最新 2 张保留
        assert!(!tmp_dir(dir.path()).join(format!("{}.png", ids[0])).exists());
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[1])).exists());
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[2])).exists());
        // 记录行一字未动：图片闸门绝不触碰 gallery.json
        assert_eq!(
            before,
            fs::read_to_string(gallery_path(dir.path())).unwrap()
        );
        assert_eq!(load_gallery(dir.path()).len(), 3);
    }

    #[test]
    fn enforce_image_limit_tie_breaks_by_path_name() {
        // 两文件 mtime 完全相同：覆盖排序的 tie-break 分支。必须按路径名确定性地删「较小者」，
        // 否则该分支被改坏（比较反转、丢掉 then_with）时结果会随目录遍历顺序漂移，测试却照过。
        let dir = tempfile::tempdir().unwrap();
        let first = fake_image(dir.path(), "tieA");
        let second = fake_image(dir.path(), "tieB");
        set_mtime(&first, 2_000_000);
        set_mtime(&second, 2_000_000); // 同刻

        enforce_image_limit(dir.path(), 1).unwrap();

        // 同刻按路径名升序：tieA.png < tieB.png → 删 tieA、留 tieB
        assert!(!first.exists());
        assert!(second.exists());
    }

    #[test]
    fn enforce_image_limit_zero_means_no_trim() {
        // limit==0 视为「不裁剪」：三张图全留（图片闸门早返回）
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..3).map(|_| new_id()).collect();
        for id in &ids {
            fake_image(dir.path(), id);
        }
        enforce_image_limit(dir.path(), 0).unwrap();
        for id in &ids {
            assert!(tmp_dir(dir.path()).join(format!("{}.png", id)).exists());
        }
    }

    #[test]
    fn enforce_gallery_limit_deletes_only_rows() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = (0..3).map(|_| new_id()).collect();
        for id in &ids {
            fake_image(dir.path(), id);
        }
        let recs: Vec<GalleryRecord> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| gallery_rec(id, i as u32))
            .collect();
        json_store::save(&gallery_path(dir.path()), &recs).unwrap();

        enforce_gallery_limit(dir.path(), 2).unwrap();

        // 断言完整的保留集合（ts 升序 = 由旧到新）：只查首元素会漏判「保留最旧而非最新」这类排序错误。
        let kept: Vec<String> = load_gallery(dir.path()).into_iter().map(|r| r.id).collect();
        assert_eq!(kept, vec![ids[1].clone(), ids[2].clone()]);
        // 三张图一个不少：记录闸门绝不删图
        for id in &ids {
            assert!(tmp_dir(dir.path()).join(format!("{}.png", id)).exists());
        }
    }

    #[test]
    fn enforce_gallery_limit_zero_means_no_trim() {
        // limit==0 视为「不裁剪」：三行全留（记录闸门早返回，与图片闸门同口径、互不换算）
        let dir = tempfile::tempdir().unwrap();
        let recs = vec![
            gallery_rec("a", 0),
            gallery_rec("b", 1),
            gallery_rec("c", 2),
        ];
        json_store::save(&gallery_path(dir.path()), &recs).unwrap();
        enforce_gallery_limit(dir.path(), 0).unwrap();
        assert_eq!(load_gallery(dir.path()).len(), 3);
    }

    #[test]
    fn prepare_session_keeps_gallery() {
        let dir = tempfile::tempdir().unwrap();
        let recs = vec![gallery_rec("a", 0), gallery_rec("b", 1)];
        json_store::save(&gallery_path(dir.path()), &recs).unwrap();
        let before = fs::read_to_string(gallery_path(dir.path())).unwrap();

        Config::prepare_session(dir.path()).unwrap();

        // 记录文件原样保留、且临时目录被重建：会话清理只针对 .imgtmp
        assert_eq!(
            before,
            fs::read_to_string(gallery_path(dir.path())).unwrap()
        );
        assert!(tmp_dir(dir.path()).is_dir());
    }

    #[test]
    fn gallery_record_round_trips_all_fields() {
        let r = gallery_rec("id1", 0);
        let s = serde_json::to_string(&r).unwrap();
        for key in [
            "\"id\"",
            "\"ts\"",
            "\"uid\"",
            "\"name\"",
            "\"model\"",
            "\"prompt\"",
            "\"negativePrompt\"",
            "\"templateId\"",
        ] {
            assert!(s.contains(key), "序列化缺少字段 {key}: {s}");
        }
        let back: GalleryRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(back.id, r.id);
        assert_eq!(back.ts, r.ts);
        assert_eq!(back.uid, r.uid);
        assert_eq!(back.name, r.name);
        assert_eq!(back.model, r.model);
        assert_eq!(back.prompt, r.prompt);
        assert_eq!(back.negative_prompt, r.negative_prompt);
        assert_eq!(back.template_id, r.template_id);
        // 除 id/ts 外均可缺省：精简输入（旧文件）也能解析
        let sparse: GalleryRecord = serde_json::from_str(r#"{"id":"x","ts":"t"}"#).unwrap();
        assert_eq!(sparse.uid, None);
        assert!(sparse.model.is_empty());
        assert!(sparse.template_id.is_empty());
    }

    #[test]
    fn record_gallery_wires_request_and_reports_availability() {
        let dir = tempfile::tempdir().unwrap();
        let images = vec![
            GenImage {
                id: "a".into(),
                path: String::new(),
                bytes: 0,
            },
            GenImage {
                id: "b".into(),
                path: String::new(),
                bytes: 0,
            },
        ];
        let mut req = gen_req();
        req.uid = Some("u9".into());
        req.name = Some("乙".into());
        req.template_id = Some("tpl9".into());
        record_gallery(dir.path(), &images, &req, 100).unwrap();
        fake_image(dir.path(), "a"); // 只留 a 的图：b 的图不存在，查询期应现算为「无图」

        let items = load_gallery_items(dir.path());
        assert_eq!(items.len(), 2);
        let a = items.iter().find(|i| i.record.id == "a").unwrap();
        let b = items.iter().find(|i| i.record.id == "b").unwrap();
        assert!(a.image_available);
        assert!(a.path.is_some());
        assert!(!b.image_available);
        assert!(b.path.is_none());
        // 请求字段落进记录（含模板 id）
        assert_eq!(a.record.uid.as_deref(), Some("u9"));
        assert_eq!(a.record.name.as_deref(), Some("乙"));
        assert_eq!(a.record.model, "gpt-image-1");
        assert_eq!(a.record.prompt, "一张脸");
        assert_eq!(a.record.template_id, "tpl9");
        // 存在性只活在查询期：落盘的 gallery.json 必须**不含** imageAvailable / path 键，
        // 否则一旦有人误把 GalleryItem 写回文件，读回时会被当成持久字段而掩盖。
        let raw = fs::read_to_string(gallery_path(dir.path())).unwrap();
        assert!(
            !raw.contains("imageAvailable"),
            "落盘不应含存在性字段: {raw}"
        );
        assert!(!raw.contains("\"path\""), "落盘不应含图片路径字段: {raw}");
    }

    #[test]
    fn concurrent_record_gallery_keeps_all_rows() {
        // 回归 H1：gallery.json 的「read→push→save」若非原子，批量并发（默认 2，这里放大到 8）
        // 会各自读到同一份旧表再整体覆盖写，后写者丢掉先写者刚追加的行。8 线程各写一条，
        // 断言最终 8 条一条不少——无锁实现下大概率只剩 1~3 条而失败。
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let n = 8usize;
        let mut handles = Vec::new();
        for i in 0..n {
            let d = data_dir.clone();
            handles.push(std::thread::spawn(move || {
                let mut req = gen_req();
                req.template_id = Some(format!("t{i}"));
                let images = vec![GenImage {
                    id: format!("img{i}"),
                    path: String::new(),
                    bytes: 0,
                }];
                record_gallery(&d, &images, &req, 1000).unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let mut ids: Vec<String> = load_gallery(dir.path()).into_iter().map(|r| r.id).collect();
        ids.sort();
        let expect: Vec<String> = (0..n).map(|i| format!("img{i}")).collect();
        assert_eq!(ids, expect, "并发写入不得丢更新");
    }

    // ===================== build_vars：球员数据源 → 模板变量 =====================

    /// 造一条可指定 detailed 的样本记录（各测试互不影响，每次现造）。
    fn sample_record(detailed: Option<DetailedInfo>) -> PlayerRecord {
        PlayerRecord {
            uid: "2000123456".into(),
            primary_nat: "CHN".into(),
            sec_nat: "SWE".into(),
            name: "测试球员".into(),
            hair_length: "2".into(),
            hair_color: "FF88CC".into(), // 发色无字典：任意原文都应原样保留
            ethnicity_code: "0".into(),
            detailed,
        }
    }

    #[test]
    fn build_vars_expanded_roster_translates_codes() {
        // 拓展名单全字段：码值经字典转译，其余原文；hairColor 原文不过字典。
        let cfg = Config::builtin();
        let vars = build_vars(
            &sample_record(Some(DetailedInfo {
                skin_code: "12".into(),
                gender: "男".into(),
                age: "24".into(),
                height: "182".into(),
                club: "Test FC".into(),
                ca: "120".into(),
                pa: "150".into(),
                ..Default::default()
            })),
            &cfg.dictionary,
        );
        assert_eq!(vars["uid"], "2000123456");
        assert_eq!(vars["name"], "测试球员");
        assert_eq!(vars["primaryNat"], "CHN");
        assert_eq!(vars["secNat"], "SWE");
        assert_eq!(vars["ethnicity"], "北欧/白人"); // "0" 经字典转译
        assert_eq!(vars["hairLength"], "短发"); // "2" 经字典转译
        assert_eq!(vars["hairColor"], "FF88CC"); // 字典里没有发色表：原样保留
        assert_eq!(vars["skinCode"], "红棕褐色（#b47454）"); // "12" → 色卡号文本
        assert_eq!(vars["gender"], "男");
        assert_eq!(vars["age"], "24");
        assert_eq!(vars["height"], "182");
        assert_eq!(vars["club"], "Test FC");
        assert_eq!(vars["ca"], "120");
        assert_eq!(vars["pa"], "150");
    }

    #[test]
    fn build_vars_basic_roster_omits_missing() {
        // 基础名单 detailed=None：拓展字段缺席（模板 {{key|默认值}} 兜底），且绝不 panic。
        let cfg = Config::builtin();
        let vars = build_vars(&sample_record(None), &cfg.dictionary);
        for key in ["skinCode", "gender", "age", "height", "club", "ca", "pa"] {
            assert!(!vars.contains_key(key), "{key} 不应出现在基础名单变量中");
        }
        assert_eq!(vars["uid"], "2000123456");
        assert_eq!(vars["name"], "测试球员");
        assert_eq!(vars["primaryNat"], "CHN");
        assert_eq!(vars["ethnicity"], "北欧/白人");
    }

    #[test]
    fn build_vars_skips_empty_fields() {
        // 空字段一律不入 map（包括 detailed 为 Some 但字段空串）：
        // 若空串占位，render 会「命中变量」而不再展开默认值。
        let cfg = Config::builtin();
        let vars = build_vars(
            &PlayerRecord {
                uid: "u1".into(),
                primary_nat: "SWE".into(),
                sec_nat: String::new(),
                name: "n".into(),
                hair_length: String::new(),
                hair_color: String::new(),
                ethnicity_code: String::new(),
                detailed: Some(DetailedInfo {
                    skin_code: String::new(),
                    ..Default::default()
                }),
            },
            &cfg.dictionary,
        );
        for key in ["ethnicity", "hairLength", "hairColor", "skinCode"] {
            assert!(!vars.contains_key(key), "空字段 {key} 不应入 map");
        }
        // secNat 与 uid/name 同为「原文」字段：无条件入 map（空串也是其原文）。
        assert!(vars.contains_key("secNat"));
        assert_eq!(vars["uid"], "u1");
        assert_eq!(vars["primaryNat"], "SWE");
    }
}
