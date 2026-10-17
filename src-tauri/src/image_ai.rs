//! AI 文生图：提示词模板 → 用户编辑确认 → 调用文生图 API → 会话临时图片 → 手动保存/保存并替换。
//!
//! 【与既有模块的复用关系，不另起一套】
//! - 命令统一写在 commands.rs，本模块只提供被命令调用的类型与函数；
//! - 出厂默认是 assets/ 两份 JSON（assets.rs include_str! 内嵌），用户可编辑副本在 data/；
//!   Config::builtin/load/ensure_user_files 仿 templates::RuleConfig；
//! - 原子写复用 json_store；
//! - 多家协议适配成「族」拆在 image_ai_providers/，本模块只保留契约/配置/渲染与生成编排，
//!   生成时按 ProviderKind 经 registry 分发。
//!
//! 【代码结构，自上而下】数据契约（ProviderKind/错误分类/Capabilities/各结构体）→ data/ 路径
//! 与小工具（new_id / now_ts）→ Config（双来源加载 + 内置服务商归并）→ CRUD 与准入校验 →
//! 模板渲染 → 词库/码值回落 → build_vars → cut_white_corners（四角裁圆角）→ generate 编排 →
//! 画廊记录与双淘汰闸门 → 单元测试。

use crate::json_store::{self, LoadError};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

// ===================== 数据契约（与前端 types.ts 一一对应，camelCase） ===========================

// ---- 服务商类型 -----------------------------------------------------------------------

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
    /// 该协议是否需要 API Key（仅存活于会话内存）。本地 ComfyUI 无鉴权；后端两道 Key 门
    /// （generate_image / start_queue）与前端就绪度判据同用本口径。
    pub fn needs_key(self) -> bool {
        !matches!(self, ProviderKind::LocalComfyUI)
    }
}

// ---- 生成链错误分类：类别枚举、结构化错误与厂商错误归类 ------------------------

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
    /// 队列重试退避优先采用（见 `image_ai_queue::backoff_delay`）。
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

// ---- 模型能力集：SizeRules（自由尺寸规则）/ Capabilities（UI 显隐与适配器发参依据） ---

/// 自由尺寸规则：只约束**不在 `sizes` 预设里的自定义值**（见 `validate_model_caps`），每条 0 = 不检查。
/// 尺寸字符串为 `宽*高`/`宽x高` 形态；各模型区间由内置目录逐模型声明，取值逐字对齐官方文档。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeRules {
    #[serde(default)]
    pub min_side: u32, // 单边下限像素（宽/高各自，0 = 不检查）
    #[serde(default)]
    pub max_side: u32, // 单边上限像素（0 = 不检查）
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
    /// `prompt_extend` 官方默认开关（千问/万相为开，z-image 为关）；前端切模型时据此初始化。
    /// 缺省 true 取官方多数系列口径（不发该参数的服务商读不到它）。
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

// ---- 配置文件结构：ModelInfo / ProviderConfig / GenSettings / ImageAiFile ------

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

/// 服务商配置（不含 API Key）。
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
    /// 批量生成队列的并发任务数
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

// ---- 提示词模板：DetailPools（码值映射 + 随机词库）/ PromptTemplate -------------------

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
    pub beard: Vec<String>, // 胡须候选
    #[serde(default)]
    pub expression: Vec<String>, // 表情候选
    #[serde(default)]
    pub lighting: Vec<String>, // 打光候选
    #[serde(default)]
    pub bg_tone: Vec<String>, // 背景候选（色调虚化或实景场景）
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
    pub id: String,
    pub name: String,
    pub body: String,
    #[serde(default)]
    pub negative: String,
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

// ---- 生成请求：GenRequest（含内部回填字段与能力快照访问器） ---------------------------

/// 一次生成请求。派生 Serialize：队列条目（image_ai_queue::QueueEntry 内嵌本类型）
/// 需经 IPC 回传快照。
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
    /// 尺寸字符串：sizes 预设值，或 `宽*高`/`宽x高` 形态的自定义值
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

// ---- 生成结果：GenImage / GenResult ---------------------------------------------------

/// 一张临时图片元信息。派生 Deserialize：队列条目经 IPC 往返（image_ai_queue::QueueEntry
/// 内嵌本类型）；前端提交的新条目 images 恒为空。
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

// ---- 画廊记录契约：GalleryRecord（持久记录）/ GalleryItem（查询期现算存在性） ---------

/// 一条画廊记录：持久保存在 `data_dir/gallery.json`，与 `.imgtmp` 图片字节是两条独立
/// 生命周期。除 id/ts 外字段均可缺省（兼容老文件与字段演进）；ts 为 now_ts 格式，
/// 字典序即时间序，淘汰与排序据此判新旧。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryRecord {
    /// 记录 id（= GenImage.id = .imgtmp 文件 stem）：画廊查询据此 stem 匹配图片是否仍在
    pub id: String,
    /// 写入时间（now_ts 格式，字典序即时间序）
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

/// 画廊查询返回项：记录本体 + **查询期现算**的存在性（绝不写回 gallery.json）。
/// path 为现算出的 `.imgtmp` 实际路径（扩展名随厂商返回格式变化），图片已被会话清理时为 None；
/// 前端据此渲染真图或占位。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryItem {
    #[serde(flatten)]
    pub record: GalleryRecord,
    pub image_available: bool,
    pub path: Option<String>,
}

// ===================== data/ 路径与通用小工具 ====================================================

/// 服务商与设置文件：`{data_dir}/image_ai.json`。
pub fn providers_path(d: &Path) -> PathBuf {
    d.join("image_ai.json")
}
/// 提示词模板文件：`{data_dir}/prompts/templates.json`。
pub fn templates_path(d: &Path) -> PathBuf {
    d.join("prompts").join("templates.json")
}
/// 会话临时图片目录：`{data_dir}/.imgtmp`，每次启动清空重建（见 `Config::prepare_session`）。
pub fn tmp_dir(d: &Path) -> PathBuf {
    d.join(".imgtmp")
}
/// 画廊记录文件：落在 data/ 根、**不在** `.imgtmp` 内——`.imgtmp` 启动即清，
/// 记录必须留在这里才能跨会话存活。
pub fn gallery_path(d: &Path) -> PathBuf {
    d.join("gallery.json")
}

/// 临时图片/画廊记录共用的随机 id：8 字节十六进制（16 字符）。
pub fn new_id() -> String {
    let bytes: [u8; 8] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 画廊记录时间戳：`%Y-%m-%d %H:%M:%S`（字典序即时间序）。
pub fn now_ts() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

// ===================== Config：内置默认 + 用户文件双来源（仿 templates::RuleConfig） =============

/// 生图模块生效配置的聚合（AppState 中以 Mutex 托管）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub providers: Vec<ProviderConfig>,
    pub settings: GenSettings,
    pub templates: Vec<PromptTemplate>,
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
        // 内置服务商：缺失整块注入，已存在只归位 kind；模型行/默认模型归用户所有
        //（可增删改、不被复活）。详见 reconcile_builtin_providers。
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

        // templates.json：缺失导出；存在则补入缺失的内置模板，「有模板但没写词库」的整份补上
        //（正文/负向归用户所有，不覆盖）。
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
            match tpls.iter_mut().find(|x| x.id == t.id) {
                // 已存在：只补**整份为空的**词库（否则新词库结构到不了老配置、随机维度
                // 静默退化）；用户改过的词库不动。
                Some(slot) if slot.pools.is_empty() && !t.pools.is_empty() => {
                    slot.pools = t.pools.clone();
                    tchanged = true;
                }
                Some(_) => {}
                None => {
                    tpls.push(t.clone());
                    tchanged = true;
                }
            }
        }
        if tchanged {
            json_store::save(&tp, &tpls)?;
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

    /// 持久化模板列表（模板 CRUD 命令收尾调用）。
    pub fn save_templates(&self, d: &Path) -> Result<(), String> {
        json_store::save(&templates_path(d), &self.templates)
    }

    /// 按 id 查服务商配置；不存在返回 None。
    pub fn provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// 查模型明细行；查不到时按「默认模型」兜底合成一行通用能力模型。
    /// 【为什么兜底】兼容服务可能只有 default_model 一个字符串（尺寸/能力都在明细行里），
    /// 死盯明细行会把填了默认模型的请求拦在「请先选择模型」外。合成行：不限张数、
    /// 高级参数全关；要精细控制就在 JSON 里补明细行，明细行优先。
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

// ---- 内置服务商归并（builtin/load/ensure 三处同口径） ---------------------------------

/// 把代码内置的服务商（百炼/火山/硅基流动/魔搭/本地 ComfyUI，目录不写 assets/image_ai.json）
/// 并入一份服务商列表（builtin/load/ensure 三处同口径）：
/// - 列表中没有：**整块补入**（新装/首次导入即得到完整目录）；
/// - 已有：**只归位 kind**；模型行/默认模型/名称/地址归用户所有，由「服务商设置」维护。
///   【刻意取舍，不是遗漏】代价是新内置模型不随版本推送、旧模型下线也不自动清理；
///   换来「对模型的增删改可保留」——改「只补缺失」的话，用户删掉的模型会在下次启动复活。
///
/// 返回是否需要落盘（内容已一致时不重写）。
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

// ===================== CRUD 辅助（命令层调用；校验失败返回中文错误） =============================

/// 新增或更新服务商（按 id 去重：同 id 整体替换、否则追加）；id/name trim 后为空则报中文错误。
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

/// 删除服务商。判据是 **id**（不是 kind）：「能否删」讲「是不是内置」，与 kind 讲的
/// 「用什么协议」是两个维度；前端删除按钮同用 id 判据（两处必须一致）。
/// openai 不可删：它是 default_provider_id 的兜底项。其余内置（如 dashscope）可删——
/// 下次启动 reconcile 会把内置目录整块重新注入，等于恢复默认模型行。
pub fn remove_provider(cfg: &mut Config, id: &str) -> Result<(), String> {
    if id == "openai" {
        return Err("内置服务商不可删除（可改名/改地址）".into());
    }
    cfg.providers.retain(|p| p.id != id);
    Ok(())
}

/// 新增或更新模板（按 id 去重：同 id 整体替换、否则追加）；id/name/body trim 后为空则报中文错误。
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

/// 删除模板（按 id）；"default" 为内置默认模板，受保护不可删。
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

// ===================== 模板渲染：{{key}} / {{key|默认值}} ========================================

/// 渲染结果（跨 IPC 返回给前端，故需 Serialize + camelCase）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderOutcome {
    /// 渲染后的文本（有默认值的占位符展开为默认值，命中 vars 的展开为变量值）。
    pub text: String,
    /// 既无 vars 值、又无默认值的占位符键名（无数据源时出现即需用户手工处理）。
    pub missing: Vec<String>,
}

/// 渲染模板正文：`{{key}}` 展开为 vars 中的值，`{{key|默认值}}` 在 vars 缺该键时展开为
/// 默认值；普通文本（含孤立 `{`）原样输出。
///
/// - `body`：模板正文，占位符的键名与默认值先 trim；
/// - `vars`：模板变量表（通常由 build_vars 构建）。
///
/// 既无变量值又无默认值的占位符展开为空串，键名去重后按首次出现顺序计入 missing，
/// 供前端提示手工补全。
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

// ===================== 词库与码值映射（内容在模板里，代码只留「出厂默认」的读取口） ==============

/// 出厂默认词库 = 内置默认模板（id = "default"）自带的一份，模板没写词库时逐组套用，
/// 避免随机/码值维度静默退化。内置 JSON 是编译期常量，缓存一次即可（用户改的是 data/ 副本）。
static BUILTIN_POOLS: LazyLock<DetailPools> = LazyLock::new(|| {
    Config::builtin()
        .templates
        .into_iter()
        .find(|t| t.id == "default")
        .map(|t| t.pools)
        .unwrap_or_default()
});

// ---- 基础名单肤色随机区间（按族裔的统计分位，build_vars 随机补足时用） ----------------

/// 肤色随机区间（族裔 0–10 各一项，取自真实导出名单的 5%~95% 分位）。肤色码
/// 0 = 最浅、19 = 最深；区间是统计值、个别赋值可能偏离，取用时上下各放 1 档余量
///（见 build_vars）。只在**基础名单**缺 `skinCode` 时使用，防「北欧人配最深肤色」类组合。
const SKIN_RANGE_BY_ETHNICITY: [(u32, u32); 11] = [
    (0, 6),   // 0 北欧/白人
    (2, 10),  // 1 地中海/西语裔
    (3, 13),  // 2 中东/北非
    (11, 18), // 3 非洲/加勒比
    (8, 16),  // 4 南亚
    (7, 14),  // 5 东南亚
    (7, 15),  // 6 太平洋岛民
    (2, 11),  // 7 美洲原住民
    (8, 17),  // 8 澳洲原住民
    (6, 16),  // 9 黑白混血
    (5, 8),   // 10 东亚
];

// ---- 随机选取与「模板优先、出厂兜底」的逐组回落工具 -----------------------------------

/// 从候选池随机取一项；调用方保证池非空（空池在 build_vars 直接跳过该变量，
/// 由模板 `{{key|默认值}}` 接管）。
fn pick<T: AsRef<str>>(pool: &[T]) -> &str {
    pool[rand::rng().random_range(0..pool.len())].as_ref()
}

/// 性别原文是否表示女性：FM 性别列是导出语言原文（男/女、Male/Female），判据走中英双语
///（与第 15 列表头识别同口径）；双语之外不识别，胡须照常随机。
fn is_female(gender: &str) -> bool {
    let g = gender.trim();
    g == "女" || g.eq_ignore_ascii_case("female")
}

/// 某一组候选池：模板里写了就用模板的，没写回落出厂默认。
fn pool_or_builtin<'a>(pool: &'a [String], fallback: &'a [String]) -> &'a [String] {
    if pool.is_empty() {
        fallback
    } else {
        pool
    }
}

/// 某一组码值映射：模板里写了就用模板的，没写回落出厂默认。
fn map_or_builtin<'a>(
    map: &'a HashMap<String, String>,
    fallback: &'a HashMap<String, String>,
) -> &'a HashMap<String, String> {
    if map.is_empty() {
        fallback
    } else {
        map
    }
}

/// 从发色映射里随机取一个**真实颜色**（排除「未知」这一档本身）。排序后再取，
/// 使结果不依赖 HashMap 迭代顺序。
fn random_hair_color(hair_colors: &HashMap<String, String>) -> String {
    let mut pool: Vec<&String> = hair_colors
        .iter()
        .filter(|(code, name)| code.as_str() != "1" && name.as_str() != "未知")
        .map(|(_, name)| name)
        .collect();
    pool.sort_unstable();
    if pool.is_empty() {
        return "黑色".into();
    }
    pick(&pool).to_string()
}

// ===================== 球员数据源 → 模板变量（与 render 同层的纯函数） ===========================

/// 从球员记录构建模板变量，并补足 RTF 未声明的维度。
///
/// # 参数
/// - `rec`：RTF 扫出的球员记录（基础名单 detailed=None，拓展名单 detailed=Some）；
/// - `tpl`：提示词模板，pools 里为空的分组逐组回落出厂默认。
///
/// 【缺字段不入 map vs 随机补足】码值类字段（`ethnicity`/`hairColor`/`skinCode`）为空
/// 或查不到该码时一律不入 map，留给模板 `{{key|默认值}}` 兜底——裸数字进提示词对 AI
/// 无意义，空串占位还会绕过默认值机制；`gender`/`age`/`height`/`skinCode` 在**基础名单**
/// （`detailed` 为 None）属结构性缺席，按约定从合理区间随机补足。
///
/// 【随机是常态】发型样式/胡须/表情/景别+动作（联动一掷）/打光/背景/球衣细节 RTF 本就不
/// 提供，每次构建重掷（同一球员每次生成换一套）；发色码 `1`（未知）同样随机取真实色名。
/// 词库与码值映射随**模板**传入（[`PromptTemplate::pools`]），未写逐组回落出厂默认。
///
/// 【身份二分】职员（拓展名单「偏好工作」有值）identity 直接渲染职位原文，服装/球衣细节/
/// 景别动作换职员池（`pools.staff`）；球员与基础名单走球员路径。
pub fn build_vars(
    rec: &crate::rtf_scanner::PlayerRecord,
    tpl: &PromptTemplate,
) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    vars.insert("uid".into(), rec.uid.clone());
    vars.insert("name".into(), rec.name.clone());
    vars.insert("primaryNat".into(), rec.primary_nat.clone());
    vars.insert("secNat".into(), rec.sec_nat.clone());
    // 国籍中文名（三字码反查）；查不到码则不入 map，由模板默认值兜底。
    if let Some(cn) = crate::assets::NAT_CN_BY_CODE.get(&rec.primary_nat) {
        vars.insert("nationality".into(), cn.clone());
    }
    let pools = &tpl.pools;
    // 身份：拓展名单「偏好工作」有值即职员 → 直接渲染职位原文；球员与基础名单
    //（无拓展字段，分辨不了）→ 称谓取 pools.identity["player"]。
    let is_staff = rec
        .detailed
        .as_ref()
        .is_some_and(|d| !d.preferred_job.is_empty());
    if is_staff {
        // 英文职位与中文性别/国籍直接拼接会出「中国男Performance Analyst」，故中英
        // 边界补空格；纯中文职位不受影响。
        let job = rec
            .detailed
            .as_ref()
            .unwrap()
            .preferred_job
            .trim()
            .to_string();
        let mut spaced = String::new();
        if job.starts_with(|c: char| c.is_ascii_alphabetic()) {
            spaced.push(' ');
        }
        spaced.push_str(&job);
        if job.ends_with(|c: char| c.is_ascii_alphabetic()) {
            spaced.push(' ');
        }
        vars.insert("identity".into(), spaced);
    } else {
        let identity_map = if pools.identity.is_empty() {
            &BUILTIN_POOLS.identity
        } else {
            &pools.identity
        };
        if let Some(v) = identity_map.get("player") {
            vars.insert("identity".into(), v.clone());
        }
    }
    if let Some(v) =
        map_or_builtin(&pools.ethnicity, &BUILTIN_POOLS.ethnicity).get(&rec.ethnicity_code)
    {
        vars.insert("ethnicity".into(), v.clone());
    }
    // 发色解析：映射值「未知」→ 随机真实色名；码查不到或光头（发长码 0）→ None。
    // 两处消费：独立变量 hairColor + 并入 hairStyle 出口（见发型段），光头自然无色。
    let hair_color_name: Option<String> = if !rec.hair_color.is_empty() && rec.hair_length != "0" {
        let hair_colors = map_or_builtin(&pools.hair_color, &BUILTIN_POOLS.hair_color);
        hair_colors.get(&rec.hair_color).map(|name| {
            if name == "未知" {
                random_hair_color(hair_colors)
            } else {
                name.clone()
            }
        })
    } else {
        None
    };
    if let Some(c) = &hair_color_name {
        vars.insert("hairColor".into(), c.clone());
    }

    let mut club = String::new();
    if let Some(d) = &rec.detailed {
        if !d.skin_code.is_empty() {
            if let Some(v) =
                map_or_builtin(&pools.skin_code, &BUILTIN_POOLS.skin_code).get(&d.skin_code)
            {
                vars.insert("skinCode".into(), v.clone());
            }
        }
        for (key, val) in [
            ("gender", &d.gender),
            ("age", &d.age),
            ("height", &d.height),
            ("ca", &d.ca),
            ("pa", &d.pa),
        ] {
            if !val.is_empty() {
                vars.insert(key.into(), val.clone());
            }
        }
        if !d.club.is_empty() {
            club = d.club.clone();
        }
        if !d.based.is_empty() {
            vars.insert("based".into(), d.based.clone());
        }
    }
    // 生涯句按身份组装整句（身份差异由变量承载，render 不加分支）：职员「任职于足球
    // 俱乐部」（总监/队医不「效力于联赛」）；球员拼 based，缺 based 时不入 map。
    let career = if is_staff {
        Some("任职于足球俱乐部".to_string())
    } else {
        rec.detailed
            .as_ref()
            .filter(|d| !d.based.is_empty())
            .map(|d| format!("效力于{}", d.based))
    };
    if let Some(c) = career {
        vars.insert("career".into(), c);
    }
    // 基础名单缺拓展字段：肤色按族裔合理区间随机，性别/年龄/身高取合理区间。
    if !vars.contains_key("skinCode") {
        let (lo, hi) = rec
            .ethnicity_code
            .parse::<usize>()
            .ok()
            .and_then(|e| SKIN_RANGE_BY_ETHNICITY.get(e).copied())
            .unwrap_or((0, 19));
        let code = rand::rng()
            .random_range(lo.saturating_sub(1)..=(hi + 1).min(19))
            .to_string();
        if let Some(v) = map_or_builtin(&pools.skin_code, &BUILTIN_POOLS.skin_code).get(&code) {
            vars.insert("skinCode".into(), v.clone());
        }
    }
    if !vars.contains_key("gender") {
        vars.insert(
            "gender".into(),
            if rand::rng().random_bool(0.85) {
                "男"
            } else {
                "女"
            }
            .into(),
        );
    }
    if !vars.contains_key("age") {
        vars.insert("age".into(), rand::rng().random_range(16..=35).to_string());
    }
    if !vars.contains_key("height") {
        vars.insert(
            "height".into(),
            format!("{} cm", rand::rng().random_range(168..=194)),
        );
    }
    if !club.is_empty() {
        vars.insert("club".into(), club.clone());
    }

    // 球衣归属：职员从职员池随机取便装/训练外套，池空不入 map；球员俱乐部优先，
    // 缺俱乐部退回国籍，两者都缺用通用描述。
    let jersey = if is_staff {
        let cand = pool_or_builtin(&pools.staff.jersey, &BUILTIN_POOLS.staff.jersey);
        (!cand.is_empty()).then(|| pick(cand).to_string())
    } else if !club.is_empty() {
        Some(format!("{club}的比赛球衣"))
    } else if let Some(cn) = vars.get("nationality").cloned() {
        Some(format!("{cn}国家队球衣"))
    } else {
        Some("深色比赛球衣，带队长袖标与徽章".into())
    };
    if let Some(j) = jersey {
        vars.insert("jersey".into(), j);
    }

    // 随机细节：词库来自模板（未写逐组回落出厂默认），每次构建重掷；池为空则不入
    // 变量，由模板 `{{key|默认值}}` 接管。
    let tiers = if pools.hair_style_by_length.is_empty() {
        &BUILTIN_POOLS.hair_style_by_length
    } else {
        &pools.hair_style_by_length
    };
    // 发长码查不到（脏数据）时依次回落到码 2 的档、再落到第一个档，不 panic。
    let style_pool = tiers
        .get(&rec.hair_length)
        .or_else(|| tiers.get("2"))
        .or_else(|| tiers.values().next());
    if let Some(p) = style_pool.filter(|p| !p.is_empty()) {
        let style = pick(p).to_string();
        vars.insert(
            "hairStyle".into(),
            // 发色并入发型出口：单一变量承载发色+剪法，光头（无色）不渲染发色。
            match &hair_color_name {
                Some(c) => format!("一头{c}，{style}"),
                None => style,
            },
        );
    }
    // 景别与动作联动随机：先选档、动作只从该档池内取（两变量不独立掷）；职员换职员池。
    let shots = if is_staff {
        if pools.staff.shots.is_empty() {
            &BUILTIN_POOLS.staff.shots
        } else {
            &pools.staff.shots
        }
    } else if pools.shots.is_empty() {
        &BUILTIN_POOLS.shots
    } else {
        &pools.shots
    };
    if !shots.is_empty() {
        let shot = &shots[rand::rng().random_range(0..shots.len())];
        vars.insert("shot".into(), shot.label.clone());
        if !shot.poses.is_empty() {
            vars.insert("pose".into(), pick(&shot.poses).to_string());
        }
    }
    // 球衣细节：职员换职员池；球员池在循环外选好，随其余四池一并随机。
    let (kit_pool, kit_fallback) = if is_staff {
        (&pools.staff.kit_detail, &BUILTIN_POOLS.staff.kit_detail)
    } else {
        (&pools.kit_detail, &BUILTIN_POOLS.kit_detail)
    };
    for (key, pool, fallback) in [
        ("beard", &pools.beard, &BUILTIN_POOLS.beard),
        ("expression", &pools.expression, &BUILTIN_POOLS.expression),
        ("lighting", &pools.lighting, &BUILTIN_POOLS.lighting),
        ("bgTone", &pools.bg_tone, &BUILTIN_POOLS.bg_tone),
        ("kitDetail", kit_pool, kit_fallback),
    ] {
        let cand = pool_or_builtin(pool, fallback);
        if !cand.is_empty() {
            vars.insert(key.into(), pick(cand).to_string());
        }
    }
    // 女性不随机胡须：移除该变量 → 模板默认值「干净无胡须」兜底（词库不分性别）；
    // gender 此时恒在 map。
    if vars.get("gender").is_some_and(|g| is_female(g)) {
        vars.remove("beard");
    }

    vars
}

// ===================== 四角白底裁成圆角（生成图后处理） ==========================================

/// 把生成图四角的白底裁成圆角弧：模型收到「圆角方形构图」只会把圆角外画成白底
///（文生图产不出真 alpha），深色背景下白角扎眼。确定性后处理，与模型/服务商无关。
///
/// 做法：每角对近白（min 通道 ≥ 228——用 245 会漏抗锯齿过渡带、裁完沿弧留亮线）的
/// 连通像素做 flood fill，限本角象限内（白弧不过中线，兼作跑飞防护）。模型画的弧是
/// 有机形、按「边缘跨度」量不准，故对白区每个像素求「恰好覆盖它所需半径」
/// R = lx + ly + √(2·lx·ly)（弧方程解出：弧外 ⟺ 该式 < R），取最大值 + 3px 余量
///（1~2px 会露离散亮残点）作裁切半径；仅在角上 R×R 方框内把弧外置透明，**宽高不变**。
/// 半径 > 短边 1/4 视为大片白景物，跳过该角（裁错不可逆，宁可留白角）。
///
/// `Ok(None)` = 四角均无可裁白弧（按原图原格式落盘）；`Ok(Some)` = 裁角后 PNG（恒 RGBA）；
/// `Err` = 解码失败。
fn cut_white_corners(bytes: &[u8]) -> Result<Option<Vec<u8>>, String> {
    const EDGE_T: u8 = 228;
    const MARGIN: u32 = 3;
    let rgba = image::load_from_memory(bytes)
        .map_err(|e| format!("图片解码失败: {e}"))?
        .into_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let (wz, hz) = (w as usize, h as usize);
    let mut raw = rgba.into_raw();
    let white_at = |raw: &[u8], x: u32, y: u32| {
        let i = (y as usize * wz + x as usize) * 4;
        raw[i].min(raw[i + 1]).min(raw[i + 2]) >= EDGE_T
    };

    // 象限 = (起点, x 下界, x 上界, y 下界, y 上界)。
    let quadrants: [(u32, u32, u32, u32, u32, u32); 4] = [
        (0, 0, 0, w / 2, 0, h / 2),
        (w - 1, 0, w / 2, w, 0, h / 2),
        (0, h - 1, 0, w / 2, h / 2, h),
        (w - 1, h - 1, w / 2, w, h / 2, h),
    ];
    let mut visited = vec![false; wz * hz];
    let mut cut_any = false;
    for (sx, sy, x0, x1, y0, y1) in quadrants {
        if !white_at(&raw, sx, sy) {
            continue;
        }
        // flood fill 找齐本角白弧的连通区，同时取覆盖该区所需的最大弧半径。
        let mut need = 0f64;
        let mut stack = vec![(sx, sy)];
        visited[sy as usize * wz + sx as usize] = true;
        while let Some((x, y)) = stack.pop() {
            let lx = (if sx == 0 { x } else { w - 1 - x }) as f64;
            let ly = (if sy == 0 { y } else { h - 1 - y }) as f64;
            need = need.max(lx + ly + (2.0 * lx * ly).sqrt());
            for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let nx = x as i64 + dx;
                let ny = y as i64 + dy;
                if nx < x0 as i64 || nx >= x1 as i64 || ny < y0 as i64 || ny >= y1 as i64 {
                    continue;
                }
                let vi = ny as usize * wz + nx as usize;
                if visited[vi] || !white_at(&raw, nx as u32, ny as u32) {
                    continue;
                }
                visited[vi] = true;
                stack.push((nx as u32, ny as u32));
            }
        }
        let r = need.ceil() as u32 + MARGIN;
        if r > w.min(h) / 4 {
            continue;
        }
        // 只在本角 r×r 方框内工作：圆心 (r,r) 半径 r 的圆弧外（白角区）置透明。
        for ly in 0..r {
            let gy = if sy == 0 { ly } else { h - 1 - ly };
            for lx in 0..r {
                let gx = if sx == 0 { lx } else { w - 1 - lx };
                let ddx = (r - lx) as i64;
                let ddy = (r - ly) as i64;
                if ddx * ddx + ddy * ddy > (r as i64) * (r as i64) {
                    raw[(gy as usize * wz + gx as usize) * 4 + 3] = 0;
                }
            }
        }
        cut_any = true;
    }

    if !cut_any {
        return Ok(None);
    }
    let out = image::RgbaImage::from_raw(w, h, raw).ok_or("图片像素缓冲与尺寸不一致")?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(out)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| format!("PNG 重编码失败: {e}"))?;
    Ok(Some(png))
}

// ===================== 生成编排：准入校验 → 适配器取字节 → 临时文件 → 画廊记录 ===================

/// 执行一次生成：解析服务商/模型 → 唯一准入校验 → 适配器取回图片字节 → 四角裁圆角
/// → 落盘 `.imgtmp` → 写画廊记录并按上限裁剪。记录写入与图片裁剪失败只告警，不掩盖已成功落盘的生成。
///
/// # 参数
/// - `data_dir`：应用数据目录（`.imgtmp` 与 `gallery.json` 都在其下）；
/// - `cfg`：生效配置（服务商/模型与两道淘汰闸门上限取自此处）；
/// - `key`：API Key，仅存活于会话内存、不落盘（本地 ComfyUI 免鉴权，传空串）；
/// - `req`：生成请求；本函数回填内部字段 base_url、caps 并门控 call_mode；
/// - `progress`：阶段回调（提交/轮询/下载），适配器按序上报、本层只透传。
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

    // 调用方式门控：显式选异步且模型支持时才置 "async"，否则回落同步（校验与 HTTP 均据此分流）。
    req.call_mode =
        if req.call_mode.as_deref() == Some("async") && model.capabilities.supports_async {
            Some("async".into())
        } else {
            None
        };

    // 【唯一准入校验入口】厂商约束在此解释，失败即返回、绝不发 HTTP；抽成独立函数便于回归测试锚定「确实调用了校验」。
    validate_for_generate(cfg, &req)?;

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
    let fetched = adapter.generate(&client, key, &req, progress).await?;

    let tmp = tmp_dir(data_dir);
    fs::create_dir_all(&tmp).map_err(|e| ImageGenError::io(e.to_string()))?;
    let mut images = Vec::new();
    for img in fetched.images {
        let id = new_id();
        // 四角裁圆角；无可裁白弧或解码失败 → 原图原格式落盘（后处理不拖垮生成）。
        // 裁过的图恒为 PNG，故成功分支扩展名统一 png。
        let (bytes, ext) = match cut_white_corners(&img.bytes) {
            Ok(Some(png)) => (png, "png".to_string()),
            Ok(None) => (img.bytes, img.extension),
            Err(e) => {
                log::warn!(target: "image_ai", "四角裁角失败（按原图落盘）: {e}");
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

// ===================== 画廊记录（data_dir/gallery.json）+ 两套独立淘汰闸门 =======================
// 记录持久、图片仅存活于会话临时目录（.imgtmp，启动即清），生命周期独立，淘汰拆成两条互不
// 越界的闸门：记录闸门只删 gallery.json 的行、图片闸门只删 .imgtmp 的字节。禁止引入
//「删记录行 + 连带删图」的一体实现——那会让一条闸门越界影响另一条。
//
// 【并发】gallery.json 是「read → push → save」的非原子读改写：批量并发下各任务读到同一份
// 旧表再整体覆盖写，后写者会丢先写者刚追加的行（json_store::save 的 tmp+rename 只保单文件
// 不损坏，不防丢更新）。故用一把模块级互斥把 gallery.json 的 I/O 串行化；互斥只覆盖同步
// fs 段、绝不跨 await；不碰 gallery.json 的 enforce_image_limit 不参与加锁。
static GALLERY_IO: Mutex<()> = Mutex::new(());

/// 读画廊清单的**不加锁内核**：调用方必须已持有 GALLERY_IO（分开是为了让取锁方在自己
/// 临界区内直接调用，避免加锁函数互相调用造成重入死锁——std Mutex 不可重入）。
/// 缺失（首次写入前）按空表；损坏也按空表并告警——记录损坏不值得阻断生成，下次写入即重建。
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

/// 把本次生成的每张图记入画廊（每图一条），再按记录上限淘汰旧行。仅 generate() 成功
/// 路径调用（失败路径图片未落盘）。记录字段取自请求。
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
    // 裁剪另起一次加锁（不嵌套，避免 std Mutex 重入死锁）；只裁行、不碰图。
    enforce_gallery_limit(data_dir, limit)
}

/// 图片淘汰闸门：`.imgtmp` 内最多保留 mtime 最新的 limit 个文件，超出删除。
/// 【只碰图片字节】按 mtime 判新旧，不读不写 gallery.json——记录可多于图片（图片被删后
/// 记录仍在，查询期现算为「无图」）。limit==0 = 不裁剪（与记录闸门同口径，互不换算）。
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

/// 记录淘汰闸门：按 ts 保留最新的 limit 行，超出者从 gallery.json 删除。
/// 【只碰记录行】绝不删 `.imgtmp` 图片——图片存亡只由 enforce_image_limit 决定。
/// limit==0 = 不裁剪。
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
/// 【存在性只在查询期计算】绝不写回 gallery.json——一次查询就会污染持久清单，且「存在」
/// 在下次启动即失真。
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
    // 读清单也走同一把锁：gallery.json 的 I/O 收敛到单一串行点。save 是 tmp+rename 原子
    // 替换、读不会见到半截文件，加锁只为与写共用同一条串行纪律。
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
            size: "1024x1024".into(),
            n: 2,
            quality: Some("high".into()),
            // 单测直接构造：能力快照留空，由 caps() 兜底或由 validate_for_generate 回填。
            ..test_req_base()
        }
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
    fn builtin_template_pools_content_and_key_order() {
        // 内置默认模板的 pools 契约：码值映射抽查（族裔 0–10、肤色 0–19 覆盖全码段含两端）。
        let pools = &Config::builtin().templates[0].pools;
        assert_eq!(pools.ethnicity["10"], "东亚");
        assert_eq!(pools.skin_code["0"], "极浅暖棕（#fcecdc）");
        assert_eq!(pools.skin_code["19"], "近黑褐色（#5c2c1c）");
        assert_eq!(pools.hair_color.len(), 19);
        assert_eq!(pools.hair_color["1"], "未知");
        assert_eq!(pools.skin_code.len(), 20);

        // 码值分组是 HashMap，落盘顺序本会随进程随机（用户看到「字段顺序全变了」）；
        // serialize_with 稳定按数值序输出，否则手改的文件 diff 会一直很脏。
        let text = serde_json::to_string_pretty(&Config::builtin().templates).unwrap();
        let skin_at = text.find("\"skinCode\"").expect("应输出 skinCode 分组");
        let skin = &text[skin_at..];
        let k0 = skin.find("\"0\"").expect("应含码 0");
        let k2 = skin.find("\"2\"").expect("应含码 2");
        let k10 = skin.find("\"10\"").expect("应含码 10");
        assert!(
            k0 < k2 && k2 < k10,
            "键应按数值序（0 < 2 < 10），实际 0@{k0} 2@{k2} 10@{k10}"
        );
        // 顶层分组顺序同样稳定：ethnicity → hairColor → skinCode
        let eth = text.find("\"ethnicity\"").unwrap();
        let hair = text.find("\"hairColor\"").unwrap();
        assert!(eth < hair && hair < skin_at);
    }

    #[test]
    fn render_expands_defaults_and_preserves_edge_inputs() {
        // 展开：命中变量用变量值；缺变量有默认值用默认值、无默认值报缺失（去重、按首次出现序）。
        let r = render(
            "一名{{ethnicity}}球员，{{age}}岁",
            &vars(&[("ethnicity", "东亚"), ("age", "18")]),
        );
        assert_eq!(r.text, "一名东亚球员，18岁");
        assert!(r.missing.is_empty());
        let r = render("{{hairLength|短发}}的{{name|球员}}", &vars(&[]));
        assert_eq!(r.text, "短发的球员");
        assert_eq!(r.missing, vec!["hairLength", "name"]);
        let r = render("{{legacy}}{{legacy|旧}}", &vars(&[]));
        assert_eq!(r.text, "旧");
        assert_eq!(r.missing, vec!["legacy"], "同一缺失键只计一次");
        // 手写模板常带空格：占位符两侧空白不得导致查不中变量、或残留在展开结果里。
        let r = render("{{ age | 18 }}", &vars(&[]));
        assert_eq!(r.text, "18");
        assert_eq!(r.missing, vec!["age"]);
        let r = render("{{ age }}", &vars(&[("age", "24")]));
        assert_eq!(r.text, "24");
        assert!(r.missing.is_empty());
        // 半成品括号（编辑中被截断的 "{{"）与孤立 "}}"/"{" 一律按普通文本原样输出，不吞字符、不误报缺失。
        let r = render("只有 { 一个括号", &vars(&[]));
        assert_eq!(r.text, "只有 { 一个括号");
        let r = render("评分{{age 分", &vars(&[("age", "24")]));
        assert_eq!(r.text, "评分{{age 分");
        assert!(r.missing.is_empty());
        let r = render("a}}b", &vars(&[]));
        assert_eq!(r.text, "a}}b");
    }

    #[test]
    fn default_model_without_detail_row_is_served() {
        // 自建兼容服务只有 default_model、没有明细行：请求默认模型必须放行（合成通用能力）。
        let mut cfg = Config::builtin();
        upsert_provider(
            &mut cfg,
            ProviderConfig {
                id: "my-relay".into(),
                kind: ProviderKind::OpenAI, // 兼容服务与官方同档
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
        // default_model 为空：空模型名不得与空 default_model 互相匹配出「合成行」。
        upsert_provider(
            &mut cfg,
            ProviderConfig {
                id: "bare".into(),
                kind: ProviderKind::OpenAI,
                name: "空模型".into(),
                base_url: "https://x/compatible-mode/v1".into(),
                default_model: String::new(),
                models: vec![],
            },
        )
        .unwrap();
        assert!(cfg.model("bare", "").is_none());
    }

    #[test]
    fn new_id_and_now_ts_formats_are_stable() {
        // 两者都是持久化判据的一部分：id 作 .imgtmp 词干与画廊记录键（16 位 hex），
        // ts 作画廊排序键（定长 `%Y-%m-%d %H:%M:%S`，字典序即时间序）。
        let a = new_id();
        assert_ne!(a, new_id());
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "{a}");
        let ts = now_ts();
        assert_eq!(ts.len(), 19, "{ts}");
        assert_eq!(&ts[10..11], " ", "{ts}");
    }

    #[test]
    fn validation_rejects_missing_fields_and_vendor_limits() {
        // 两层准入校验：validate_complete 管请求自身完整性（提示词/张数/服务商与模型存在）；
        // validate_for_generate 是 generate 的准入校验函数，厂商约束由 Provider 层解释——
        // 结构性覆盖：若 generate 不再调用它、或它不再委派 for_kind(...).validate，断言会失败。
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
        // 各模型行能力快照：逐家抽查关键差异；新增/下线模型时这里的快照会强制失败，提醒同步审查。
        let caps = |p: &str, m: &str| cfg.model(p, m).unwrap().capabilities;
        // 本地 ComfyUI：免 Key（协议无鉴权），模型名 = checkpoint 文件名（占位行，归用户改）。
        let comfy = cfg.provider("localcomfyui").unwrap();
        assert_eq!(comfy.kind, ProviderKind::LocalComfyUI);
        assert!(!comfy.kind.needs_key());
        assert_eq!(comfy.base_url, "http://127.0.0.1:8188");
        // 出厂 gpt-image-1 必须关掉 response_format（官方不支持该参数，传了会 400）。
        assert!(!caps("openai", "gpt-image-1").supports_response_format);
        // 火山：默认模型与目录规模随出厂快照；能力抽查：单张、无 seed/n、有水印与 output_format。
        let volcano = cfg.provider("volcano").unwrap();
        assert_eq!(volcano.default_model, "doubao-seedream-5-0-flash-260915");
        assert_eq!(volcano.models.len(), 5);
        let flash = caps("volcano", "doubao-seedream-5-0-flash-260915");
        assert_eq!(flash.max_n, 1);
        assert!(!flash.supports_seed, "Seedream 全系列无 seed");
        assert!(!flash.supports_n, "Seedream 全系列无 n 参数");
        assert!(!flash.supports_quality);
        assert!(flash.supports_watermark);
        assert!(!flash.watermark_default);
        assert!(flash.supports_output_format, "Flash 可设 png/jpeg");
        // 4.5 只回 jpeg：不能发 output_format；旧出厂模型名不应出现在目录中。
        assert!(!caps("volcano", "doubao-seedream-4-5-251128").supports_output_format);
        assert!(cfg.model("volcano", "seedream-5.0-flash").is_none());
        // 百炼模型目录代码内置：三系列 6 个模型抽查关键差异。
        let qwen = caps("dashscope", "qwen-image-3.0-pro");
        assert_eq!(qwen.max_n, 6);
        assert!(qwen.supports_negative_prompt);
        assert!(qwen.prompt_extend_default);
        assert!(qwen.sizes.contains(&"2048*2048".to_string()));
        let wan27 = caps("dashscope", "wan2.7-image-pro");
        assert!(wan27.sizes.contains(&"4K".to_string()));
        assert!(!wan27.supports_negative_prompt);
        assert!(!wan27.supports_prompt_extend);
        let z = caps("dashscope", "z-image-turbo");
        assert_eq!(z.max_n, 1);
        assert!(!z.supports_n);
        assert!(!z.prompt_extend_default);
        // 下线模型（wanx2.1）不在目录中。
        assert!(cfg.model("dashscope", "wanx2.1-t2i-turbo").is_none());
        // 目录规模与默认模型（assets/image_ai.json 不含百炼块，由 reconcile 代码注入）：
        // 新增/下线模型时此快照会强制失败，提醒同步审查默认模型与模型总数。
        let ds = cfg.provider("dashscope").unwrap();
        assert_eq!(ds.default_model, "z-image-turbo");
        assert_eq!(ds.models.len(), 6);
        // 硅基流动：Qwen-Image 无 batch_size、字段名走 image_size；Kolors 有 batch_size + guidance。
        let sf_qwen = caps("siliconflow", "Qwen/Qwen-Image");
        assert!(!sf_qwen.supports_n, "Qwen-Image 无 batch_size");
        assert!(sf_qwen.supports_steps);
        assert!(!sf_qwen.supports_guidance);
        let sf_kolors = caps("siliconflow", "Kwai-Kolors/Kolors");
        assert!(sf_kolors.supports_n, "Kolors 有 batch_size");
        assert!(sf_kolors.supports_guidance);
        assert_eq!(sf_kolors.max_n, 4);
        // 魔搭：单张、无 n；支持 steps/guidance。
        let ms = caps("modelscope", "Qwen/Qwen-Image-2.1");
        assert_eq!(ms.max_n, 1);
        assert!(!ms.supports_n);
        assert!(ms.supports_steps && ms.supports_guidance);
    }

    #[test]
    fn reconcile_normalizes_kind_and_keeps_user_entries() {
        // kind 是唯一由代码接管的字段（决定适配器分发）：错值归位；模型行/默认模型/名称/
        // 地址归用户所有，绝不整表替换——否则用户删掉的模型会在启动时复活。
        // 第一阶段：两个内置服务商 kind 错值（百炼空模型行、火山用户自填行）→ 归位 + 注入 + 幂等。
        let mut providers = vec![
            ProviderConfig {
                id: "dashscope".into(),
                kind: ProviderKind::OpenAI, // 错值：必须归位
                name: "我的百炼".into(),
                base_url: "https://proxy.example.com/api/v1".into(),
                default_model: "my-own-model".into(),
                models: vec![],
            },
            ProviderConfig {
                id: "volcano".into(),
                kind: ProviderKind::OpenAI, // 错值：必须归位
                name: "我的火山".into(),
                base_url: "https://example.com".into(),
                default_model: "my-seedream".into(),
                models: vec![ModelInfo {
                    model: "my-seedream".into(),
                    capabilities: Capabilities::default(),
                }],
            },
        ];
        // 两处 kind 归位 + 缺失内置（siliconflow/modelscope/comfyui）整块注入 → 有变更。
        assert!(reconcile_builtin_providers(&mut providers));
        assert_eq!(providers.len(), 5);
        let ds = providers.iter().find(|p| p.id == "dashscope").unwrap();
        assert_eq!(ds.kind, ProviderKind::Dashscope);
        assert_eq!(ds.name, "我的百炼", "名称归用户所有");
        assert_eq!(ds.models.len(), 0, "kind 归位不得顺带补入内置模型行");
        assert_eq!(ds.default_model, "my-own-model", "默认模型不被校正");
        let vol = providers.iter().find(|p| p.id == "volcano").unwrap();
        assert_eq!(vol.kind, ProviderKind::Volcano);
        assert_eq!(vol.models.len(), 1, "用户模型不得被内置目录覆盖");
        assert!(vol.models.iter().any(|m| m.model == "my-seedream"));
        assert_eq!(vol.default_model, "my-seedream", "默认模型不被校正");
        assert_eq!(vol.base_url, "https://example.com", "地址不被接管");
        for id in ["siliconflow", "modelscope", "localcomfyui"] {
            assert!(
                providers.iter().any(|p| p.id == id),
                "缺失内置 {id} 应整块注入"
            );
        }
        assert!(
            !reconcile_builtin_providers(&mut providers),
            "归位/注入后幂等"
        );

        // 第二阶段：六个条目全为当前目录 → 零变更（不触发无谓写盘），用户定制字段全部保留。
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
        assert!(!reconcile_builtin_providers(&mut providers));
        let ds = providers.iter().find(|p| p.id == "dashscope").unwrap();
        assert_eq!(ds.name, "我的百炼代理");
        assert_eq!(ds.base_url, "https://proxy.example.com/api/v1");
        assert_eq!(ds.models.len(), 1, "模型行不被内置目录整表替换");
        assert_eq!(ds.default_model, "wanx2.1-t2i-turbo", "默认模型不再被校正");
    }

    #[test]
    fn ensure_upgrade_injects_missing_builtins_and_keeps_user_entries() {
        // 升级路径全场景：老用户文件只有 openai 与用户定制的 dashscope 条——ensure 须把
        // 缺失的内置服务商整块补入并落盘（否则界面上看不到它们），同时**逐字段保留**用户
        // 已有条目（模型行/默认模型/名称/地址归用户所有，不得用内置目录覆盖）。
        let root = tempfile::tempdir().unwrap();
        let existing = serde_json::json!({
            "providers": [
                {
                    "id": "openai",
                    "kind": "openai",
                    "name": "OpenAI",
                    "baseUrl": "https://api.openai.com/v1",
                    "defaultModel": "gpt-image-1",
                    "models": [{ "model": "gpt-image-1", "capabilities": {} }]
                },
                {
                    "id": "dashscope",
                    "kind": "dashscope",
                    "name": "百炼通义万相",
                    "baseUrl": "https://dashscope.aliyuncs.com/api/v1",
                    "defaultModel": "my-own-model",
                    "models": [{
                        "model": "my-own-model",
                        "capabilities": {}
                    }]
                }
            ],
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
        // 用户原有条目不被覆盖：openai 原样、dashscope 逐字段保留。
        assert_eq!(cfg.provider("openai").unwrap().default_model, "gpt-image-1");
        let ds = cfg.provider("dashscope").unwrap();
        assert_eq!(ds.name, "百炼通义万相");
        assert_eq!(ds.models.len(), 1, "不整表补入内置目录");
        assert_eq!(ds.models[0].model, "my-own-model");
        assert_eq!(ds.default_model, "my-own-model", "默认模型不被校正");
        // 缺失的内置服务商整块补入，目录细节可证（「正常加载」的证据）。
        assert!(cfg.provider("volcano").is_some());
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
        // 再次 ensure 幂等：不再产生写盘变更。
        let before = fs::read_to_string(providers_path(root.path())).unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        assert_eq!(
            before,
            fs::read_to_string(providers_path(root.path())).unwrap()
        );
    }

    #[test]
    fn config_load_missing_falls_back_and_corrupt_reports_path() {
        // 缺文件（首次运行）→ 整份回退出厂默认；损坏 → 绝不静默回退（否则用户手改的内容被
        // 无声清掉），load 与 ensure 都须带出文件路径（json_store 已备份 .corrupt 可找回）。
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let cfg = Config::load(d).unwrap();
        let builtin = Config::builtin();
        assert_eq!(cfg.providers.len(), builtin.providers.len());
        assert_eq!(cfg.templates.len(), builtin.templates.len());
        assert!(cfg.templates.iter().any(|t| t.id == "default"));

        fs::write(providers_path(d), "{ 这不是合法 JSON").unwrap();
        let err = Config::load(d).unwrap_err();
        assert!(err.contains("生图配置无法解析"), "{err}");
        assert!(err.contains("image_ai.json"), "{err}");
        let err = Config::ensure_user_files(d).unwrap_err();
        assert!(err.contains("image_ai.json"), "{err}");
        // 服务商文件合法后进入第二条解析路径：模板文件损坏同样报错带路径。
        Config::builtin().save_providers(d).unwrap();
        fs::create_dir_all(d.join("prompts")).unwrap(); // json_store::save 不建父目录
        fs::write(templates_path(d), "[ 坏掉的模板").unwrap();
        let err = Config::load(d).unwrap_err();
        assert!(err.contains("模板无法解析"), "{err}");
        assert!(err.contains("templates.json"), "{err}");
        let err = Config::ensure_user_files(d).unwrap_err();
        assert!(err.contains("templates.json"), "{err}");
    }

    #[test]
    fn crud_validates_trims_and_protects_builtins() {
        // 命令层 CRUD 的守卫文案与判据：id/name/body trim 后为空拒绝；删除判据是 **id**——
        // openai 是 default_provider_id 的兜底项不可删、default 模板不可删；其余可删
        //（dashscope 重启由 reconcile 复活）。模板落盘往返验证「改 → save → 重载」链路。
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let mut cfg = Config::builtin();

        let provider = |id: &str, name: &str| ProviderConfig {
            id: id.into(),
            kind: ProviderKind::OpenAI,
            name: name.into(),
            base_url: "https://example.com/v1".into(),
            default_model: "m".into(),
            models: vec![],
        };
        assert!(upsert_provider(&mut cfg, provider("  ", "x"))
            .unwrap_err()
            .contains("不能为空"));
        assert!(upsert_provider(&mut cfg, provider("mine", " "))
            .unwrap_err()
            .contains("不能为空"));
        // 新增：id/name 两处都 trim；同 id 再次 upsert 为整体替换而非追加。
        upsert_provider(&mut cfg, provider(" mine ", " A ")).unwrap();
        assert_eq!(cfg.provider("mine").unwrap().name, "A");
        let n = cfg.providers.len();
        upsert_provider(&mut cfg, provider("mine", "B")).unwrap();
        assert_eq!(cfg.providers.len(), n, "同 id 应替换而非追加");
        assert_eq!(cfg.provider("mine").unwrap().name, "B");
        // 删除：openai 受保护；其余（含内置服务商）可删。
        assert!(remove_provider(&mut cfg, "openai")
            .unwrap_err()
            .contains("不可删除"));
        remove_provider(&mut cfg, "dashscope").unwrap();
        assert!(cfg.provider("dashscope").is_none());
        remove_provider(&mut cfg, "mine").unwrap();
        assert!(cfg.provider("mine").is_none());

        let template = |id: &str, name: &str, body: &str| PromptTemplate {
            id: id.into(),
            name: name.into(),
            body: body.into(),
            negative: String::new(),
            pools: DetailPools::default(),
        };
        assert!(upsert_template(&mut cfg, template("t1", "", "b"))
            .unwrap_err()
            .contains("不能为空"));
        assert!(upsert_template(&mut cfg, template("  ", "n", "b"))
            .unwrap_err()
            .contains("不能为空"));
        assert!(upsert_template(&mut cfg, template("t1", "我的", "  "))
            .unwrap_err()
            .contains("不能为空"));
        upsert_template(&mut cfg, template("t1", " 我的 ", " 正文 ")).unwrap();
        let t = cfg.templates.iter().find(|t| t.id == "t1").unwrap();
        assert_eq!(t.name, "我的");
        assert_eq!(t.body, "正文");
        fs::create_dir_all(d.join("prompts")).unwrap(); // json_store::save 不建父目录
        cfg.save_templates(d).unwrap();
        let back = Config::load(d).unwrap();
        assert!(back
            .templates
            .iter()
            .any(|t| t.id == "t1" && t.body == "正文"));
        // default 受保护；用户模板可删。
        assert!(remove_template(&mut cfg, "default")
            .unwrap_err()
            .contains("不可删除"));
        remove_template(&mut cfg, "t1").unwrap();
        assert!(!cfg.templates.iter().any(|t| t.id == "t1"));
    }

    // ===================== 画廊记录 + 双淘汰闸门 =================================================

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

    /// 造一条指定秒数时间戳的画廊记录（"%H:%M:%S" 字典序即时间序，秒数递增即时间递增）。
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
        // 按 mtime 判新旧：删最旧、留最新；只碰图片字节、绝不触碰 gallery.json；
        // 文件数等于上限时早退。
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
        // 文件数已等于上限：再次执行早退，一张不删
        enforce_image_limit(dir.path(), 2).unwrap();
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[1])).exists());
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[2])).exists());

        // 同刻 tie-break：mtime 完全相同必须按路径名确定性地删「较小者」，否则该分支被改坏
        //（比较反转、丢掉 then_with）时结果会随目录遍历顺序漂移，测试却照过。
        let tie_dir = tempfile::tempdir().unwrap();
        let first = fake_image(tie_dir.path(), "tieA");
        let second = fake_image(tie_dir.path(), "tieB");
        set_mtime(&first, 2_000_000);
        set_mtime(&second, 2_000_000); // 同刻
        enforce_image_limit(tie_dir.path(), 1).unwrap();
        // 同刻按路径名升序：tieA.png < tieB.png → 删 tieA、留 tieB
        assert!(!first.exists());
        assert!(second.exists());
    }

    #[test]
    fn zero_limit_disables_both_gates() {
        // 两道闸门同一口径：limit==0 一律视为「不裁剪」。同一目录内同时验证，
        // 顺带锁定两闸互不越界（图片闸不碰记录、记录闸不碰图）。
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

        enforce_image_limit(dir.path(), 0).unwrap();
        enforce_gallery_limit(dir.path(), 0).unwrap();

        // 三张图全留（图片闸门早返回）
        for id in &ids {
            assert!(tmp_dir(dir.path()).join(format!("{}.png", id)).exists());
        }
        // 三行全留（记录闸门早返回）
        assert_eq!(load_gallery(dir.path()).len(), 3);
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
    fn prepare_session_wipes_tmp_dir_and_keeps_gallery() {
        // 会话清理只针对 .imgtmp：旧临时图片被清、目录被重建；持久画廊记录逐字节原样保留。
        let dir = tempfile::tempdir().unwrap();
        let stale = fake_image(dir.path(), "stale");
        let recs = vec![gallery_rec("a", 0), gallery_rec("b", 1)];
        json_store::save(&gallery_path(dir.path()), &recs).unwrap();
        let before = fs::read_to_string(gallery_path(dir.path())).unwrap();

        Config::prepare_session(dir.path()).unwrap();

        assert!(!stale.exists());
        assert!(tmp_dir(dir.path()).is_dir());
        assert_eq!(
            before,
            fs::read_to_string(gallery_path(dir.path())).unwrap()
        );
    }

    // ---- cut_white_corners：四角白底裁成圆角 ------------------------------------------

    /// 合成「内容铺满、四角圆弧外为白」：贴近模型实际输出的圆角构图（白弧只在四角）。
    fn rounded_white_corner_sample(size: u32, radius: u32) -> image::RgbaImage {
        let mut img = image::RgbaImage::from_pixel(size, size, image::Rgba([20, 30, 40, 255]));
        let rr = (radius as i64) * (radius as i64);
        for ly in 0..radius {
            for lx in 0..radius {
                let dx = (radius - lx) as i64;
                let dy = (radius - ly) as i64;
                if dx * dx + dy * dy <= rr {
                    continue; // 弧内是内容
                }
                let white = image::Rgba([255, 255, 255, 255]);
                img.put_pixel(lx, ly, white);
                img.put_pixel(size - 1 - lx, ly, white);
                img.put_pixel(lx, size - 1 - ly, white);
                img.put_pixel(size - 1 - lx, size - 1 - ly, white);
            }
        }
        img
    }

    fn encode_as(img: &image::RgbaImage, fmt: image::ImageFormat) -> Vec<u8> {
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img.clone())
            .write_to(&mut std::io::Cursor::new(&mut buf), fmt)
            .unwrap();
        buf
    }

    #[test]
    fn cut_white_corners_trims_arcs_and_skips_non_arcs() {
        // 成功路径：白弧外的角区置透明、宽高保持不变。
        // 100px 图、四角白弧半径 20：白区最远的边点 (19,0) 决定覆盖半径 19，加余量后裁切半径 22。
        let sample = rounded_white_corner_sample(100, 20);
        let png = cut_white_corners(&encode_as(&sample, image::ImageFormat::Png))
            .unwrap()
            .expect("白弧应触发裁角");
        let out = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (100, 100), "宽高必须保持不变");
        for (x, y) in [(0, 0), (99, 0), (0, 99), (99, 99)] {
            assert_eq!(out.get_pixel(x, y)[3], 0, "角 ({x},{y}) 弧外应透明");
        }
        // (19,0)/(20,0) 被裁、(23,0) 保留：把裁切余量锁在 2–4px 带内——余量小了沿弧留
        // 亮残点、大了多吃内容（(20,0) 正是被多吃 2px 的代价）。
        assert_eq!(out.get_pixel(19, 0)[3], 0);
        assert_eq!(out.get_pixel(20, 0)[3], 0);
        assert_eq!(out.get_pixel(23, 0)[3], 255);
        assert_eq!(out.get_pixel(50, 0)[3], 255);
        assert_eq!(*out.get_pixel(50, 50), image::Rgba([20, 30, 40, 255]));

        // 只有左上角有白弧：其余三角必须原样保留——防「一角触发、四角齐裁」类越界。
        let size = 100u32;
        let radius = 20u32;
        let mut img = image::RgbaImage::from_pixel(size, size, image::Rgba([20, 30, 40, 255]));
        let rr = (radius as i64) * (radius as i64);
        for ly in 0..radius {
            for lx in 0..radius {
                let dx = (radius - lx) as i64;
                let dy = (radius - ly) as i64;
                if dx * dx + dy * dy <= rr {
                    continue;
                }
                img.put_pixel(lx, ly, image::Rgba([255, 255, 255, 255]));
            }
        }
        let png = cut_white_corners(&encode_as(&img, image::ImageFormat::Png))
            .unwrap()
            .expect("左上角白弧应触发裁角");
        let out = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(out.dimensions(), (size, size));
        assert_eq!(out.get_pixel(0, 0)[3], 0, "有白弧的角应被裁");
        for (x, y) in [(size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
            assert_eq!(out.get_pixel(x, y)[3], 255, "角 ({x},{y}) 无白弧不得被裁");
        }

        // 三种「不裁」情形都返回 None，调用方按原图原格式落盘：纯色深图（四角无白弧）；
        // 深底中央封闭白块（白区不在角部，不该被波及）；整行白贴到顶角（场景级白色，
        // 不是模型画的角弧——裁错不可逆，宁可留白）。
        let plain = image::RgbaImage::from_pixel(32, 32, image::Rgba([10, 20, 30, 255]));
        assert!(
            cut_white_corners(&encode_as(&plain, image::ImageFormat::Png))
                .unwrap()
                .is_none()
        );
        let mut sealed = image::RgbaImage::from_pixel(64, 64, image::Rgba([15, 25, 35, 255]));
        for y in 26..38 {
            for x in 26..38 {
                sealed.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        assert!(
            cut_white_corners(&encode_as(&sealed, image::ImageFormat::Png))
                .unwrap()
                .is_none()
        );
        let mut scene = image::RgbaImage::from_pixel(64, 64, image::Rgba([15, 25, 35, 255]));
        for x in 0..64 {
            scene.put_pixel(x, 0, image::Rgba([255, 255, 255, 255]));
        }
        assert!(
            cut_white_corners(&encode_as(&scene, image::ImageFormat::Png))
                .unwrap()
                .is_none()
        );

        // 解码失败 → Err：generate 据此按原图原格式落盘，不拖垮成功生成。
        assert!(cut_white_corners(b"not an image").is_err());
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
    fn record_gallery_wires_requests_and_query_orders_newest_first() {
        // 画廊查询契约：缺清单按空表；请求字段落进记录；存在性逐条现算；结果最新在前。
        let dir = tempfile::tempdir().unwrap();
        assert!(load_gallery_items(dir.path()).is_empty());
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

        // 排序：a/b 的 ts 相同（同一秒写入）验不出顺序，另写三条不同 ts 的记录。
        json_store::save(
            &gallery_path(dir.path()),
            &[
                gallery_rec("old", 0),
                gallery_rec("new", 2),
                gallery_rec("mid", 1),
            ],
        )
        .unwrap();
        fake_image(dir.path(), "old"); // 只有旧图还在 .imgtmp（其余已被会话清理）
        let items = load_gallery_items(dir.path());
        let ids: Vec<&str> = items.iter().map(|i| i.record.id.as_str()).collect();
        assert_eq!(ids, ["new", "mid", "old"], "查询结果最新在前");
        assert!(!items[0].image_available, "无图记录应标记不可用");
        assert!(items[2].image_available && items[2].path.is_some());
        // 序列化契约：存在性字段与记录字段同层（flatten），前端按此读取。
        let j = serde_json::to_string(&items[2]).unwrap();
        assert!(j.contains(r#""imageAvailable":true"#), "{j}");
        assert!(j.contains(r#""id":"old""#), "{j}");
    }

    #[test]
    fn concurrent_record_gallery_keeps_all_rows() {
        // 回归 H1：非原子读改写（read→push→save）在并发（这里放大到 8 线程）下会丢更新；
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

    // ===================== build_vars：球员数据源 → 模板变量 =====================================

    /// 造一条可指定 detailed 的样本记录（各测试互不影响，每次现造）。
    fn sample_record(detailed: Option<DetailedInfo>) -> PlayerRecord {
        PlayerRecord {
            uid: "2000123456".into(),
            primary_nat: "CHN".into(),
            sec_nat: "SWE".into(),
            name: "测试球员".into(),
            hair_length: "2".into(),
            hair_color: "3".into(),
            ethnicity_code: "0".into(),
            detailed,
        }
    }

    /// 空模板（不写词库）：构造「回落出厂默认」路径的测试样本。
    fn blank_tpl() -> PromptTemplate {
        PromptTemplate {
            id: "t".into(),
            name: String::new(),
            body: String::new(),
            negative: String::new(),
            pools: DetailPools::default(),
        }
    }

    #[test]
    fn build_vars_expanded_roster_translates_codes() {
        // 拓展名单全字段：码值经模板映射转译，其余原文。
        let tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
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
            &tpl,
        );
        assert_eq!(vars["uid"], "2000123456");
        assert_eq!(vars["name"], "测试球员");
        assert_eq!(vars["primaryNat"], "CHN");
        assert_eq!(vars["secNat"], "SWE");
        assert_eq!(vars["nationality"], "中国"); // 三字码反查中文名
        assert_eq!(vars["ethnicity"], "北欧/白人"); // "0" 经模板映射转译
        assert!(
            !vars.contains_key("hairLength"),
            "发长译名不产出——词库分档已承载长度语义"
        );
        assert_eq!(vars["hairColor"], "金色"); // "3" 经模板映射转译
        assert!(
            vars["hairStyle"].starts_with("一头金色，"),
            "发色并入发型出口：有发为「一头{{色}}，{{剪法}}」，实际「{}」",
            vars["hairStyle"]
        );
        assert_eq!(vars["skinCode"], "红棕褐色（#b47454）"); // "12" → 色卡号文本
        assert_eq!(vars["gender"], "男");
        assert_eq!(vars["age"], "24");
        assert_eq!(vars["height"], "182");
        assert_eq!(vars["club"], "Test FC");
        assert_eq!(vars["jersey"], "Test FC的比赛球衣"); // 球衣：俱乐部优先
        assert_eq!(vars["ca"], "120");
        assert_eq!(vars["pa"], "150");
    }

    #[test]
    fn build_vars_basic_roster_randomizes_extended_fields() {
        // 基础名单 detailed=None：拓展字段本就不在 RTF 里，按约定随机补足而非空着；
        // RTF 同样不提供的随机细节（发型/景别+动作/胡须/表情/打光/背景/球衣细节）
        // 也必须存在（缺了会在模板里报未填充）——beard 例外：女性跳过胡须随机。
        let tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
        let vars = build_vars(&sample_record(None), &tpl);
        for key in ["skinCode", "gender", "age", "height"] {
            assert!(vars.contains_key(key), "基础名单应随机补足 {key}");
            assert!(!vars[key].is_empty(), "{key} 不得为空串");
        }
        for key in [
            "hairStyle",
            "shot",
            "expression",
            "pose",
            "lighting",
            "bgTone",
            "kitDetail",
        ] {
            assert!(vars.contains_key(key), "{key} 必须存在");
        }
        if vars["gender"] == "女" {
            assert!(!vars.contains_key("beard"), "女性不得随机胡须");
        } else {
            assert!(vars.contains_key("beard"), "男性必须随机胡须");
        }
        // 俱乐部不编造真实队名：缺省会退成国家队/通用球衣描述。
        assert!(!vars.contains_key("club"));
        assert!(!vars.contains_key("ca"));
        assert!(!vars.contains_key("pa"));
        assert_eq!(vars["jersey"], "中国国家队球衣");
        assert_eq!(vars["ethnicity"], "北欧/白人");
        assert_eq!(vars["identity"], "足球运动员"); // 基础名单无拓展字段，分辨不了职员

        // 球衣归属三级回落：俱乐部 → 国籍 → 通用描述；国籍码查不到中文名时
        // nationality 不入 map，球衣也不得编造队名。
        let mut rec = sample_record(None);
        rec.primary_nat = "ZZZ".into(); // 词表外的代码
        let vars = build_vars(&rec, &tpl);
        assert!(!vars.contains_key("nationality"));
        assert_eq!(vars["jersey"], "深色比赛球衣，带队长袖标与徽章");
    }

    #[test]
    fn build_vars_skips_empty_code_fields() {
        // 码值字段为空或映射查不到该码时一律不入 map（空串占位会让 render「命中变量」
        // 而不再展开默认值，裸数字对 AI 无意义）；skinCode/gender/age/height 属结构性
        // 缺席，会被随机补足——两条口径互不冲突。
        let tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
        let mut rec = PlayerRecord {
            uid: "u1".into(),
            primary_nat: "SWE".into(),
            sec_nat: String::new(),
            name: "n".into(),
            hair_length: String::new(),
            hair_color: String::new(),
            ethnicity_code: "99".into(), // 映射外的脏码：不入 map，由模板默认值兜底
            detailed: Some(DetailedInfo {
                skin_code: "99".into(),
                ..Default::default()
            }),
        };
        let vars = build_vars(&rec, &tpl);
        for key in ["ethnicity", "hairColor"] {
            assert!(!vars.contains_key(key), "空字段/映射外码 {key} 不应入 map");
        }
        assert!(
            !vars.contains_key("skinCode") || vars["skinCode"] != "99",
            "映射外的肤色码不得以裸数字入 map"
        );
        for key in ["gender", "age", "height"] {
            assert!(vars.contains_key(key), "{key} 应随机补足");
        }
        assert_eq!(vars["uid"], "u1");
        assert_eq!(vars["primaryNat"], "SWE");
        assert_eq!(vars["nationality"], "瑞典");
        // secNat 与 uid/name 同为「原文」字段：无条件入 map（空串也是其原文）。
        assert!(vars.contains_key("secNat"));
        // 空字段路径（码为空串）单独验证：同样不入 map。
        rec.ethnicity_code = String::new();
        rec.detailed = None;
        let vars = build_vars(&rec, &tpl);
        assert!(!vars.contains_key("ethnicity"));
        assert!(!vars.contains_key("hairColor"));
    }

    #[test]
    fn build_vars_unknown_hair_color_never_reaches_prompt() {
        // 发色码 1 = 未知：「未知」不得进提示词，改取映射内的真实色名；
        // 真实色名被用户删光（映射只剩「未知」档）时随机池为空 → 兜底「黑色」。
        let mut tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
        let mut rec = sample_record(None);
        rec.hair_color = "1".into();
        for _ in 0..50 {
            let vars = build_vars(&rec, &tpl);
            let got = &vars["hairColor"];
            assert_ne!(got, "未知");
            assert!(
                tpl.pools.hair_color.values().any(|v| v == got),
                "应是映射内的真实色名，实际 {got}"
            );
        }
        tpl.pools.hair_color = [("1".to_string(), "未知".to_string())]
            .into_iter()
            .collect();
        assert_eq!(build_vars(&rec, &tpl)["hairColor"], "黑色");
    }

    #[test]
    fn build_vars_hair_style_length_code_edges() {
        // 光头（发长码 0）不发色：避免「黑色光头」这类矛盾描述；长度语义由发型词库第 0 档承载。
        let tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
        let mut rec = sample_record(None);
        rec.hair_length = "0".into();
        let vars = build_vars(&rec, &tpl);
        assert!(!vars.contains_key("hairColor"));
        assert!(
            !vars["hairStyle"].starts_with("一头"),
            "光头发型不得带发色前缀"
        );
        // 光头档取词库原文（0 档可多候选）
        assert!(
            tpl.pools.hair_style_by_length["0"].contains(&vars["hairStyle"]),
            "光头发型「{}」应取自发长码 0 档词库",
            vars["hairStyle"]
        );

        // 发长码查不到（脏数据）→ 先落码 2 档；模板连码 2 档都没有 → 落 BTreeMap 第一档；
        // 命中的档位存在但为空 → 不入 map，交由模板默认值接管。
        let mut dirty = sample_record(None);
        dirty.hair_length = "9".into();
        dirty.hair_color = String::new(); // 隔离发色前缀：只验证剪法来源
        let vars = build_vars(&dirty, &tpl);
        assert!(
            tpl.pools.hair_style_by_length["2"].contains(&vars["hairStyle"]),
            "脏码应回落码 2 档，实际「{}」",
            vars["hairStyle"]
        );

        let mut other = tpl.clone();
        other.pools.hair_style_by_length = [("0".to_string(), vec!["第一档词".into()])]
            .into_iter()
            .collect();
        assert_eq!(
            build_vars(&dirty, &other)["hairStyle"],
            "第一档词",
            "码 2 档缺失时应落第一档"
        );

        let mut empty_tier = tpl.clone();
        empty_tier.pools.hair_style_by_length =
            [("2".to_string(), Vec::new())].into_iter().collect();
        let mut rec2 = sample_record(None);
        rec2.hair_length = "2".into();
        assert!(
            !build_vars(&rec2, &empty_tier).contains_key("hairStyle"),
            "档位为空时不由空串占位"
        );
    }

    #[test]
    fn build_vars_identifies_staff_by_preferred_job() {
        // 拓展名单第 15 列是「偏好工作」即职员 → identity 直接渲染职位原文（自由文本，
        // 不可靠细分，不做二次翻译）；球员（preferred_job 空）→ 取 identity 映射的 player 档。
        let tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
        let staff = sample_record(Some(DetailedInfo {
            preferred_job: "足球总监".into(),
            ..Default::default()
        }));
        assert_eq!(build_vars(&staff, &tpl)["identity"], "足球总监");
        let player = sample_record(Some(DetailedInfo {
            based: "英格兰".into(),
            ..Default::default()
        }));
        let player_vars = build_vars(&player, &tpl);
        assert_eq!(player_vars["identity"], "足球运动员");
        assert_eq!(player_vars["career"], "效力于英格兰");
        // 英文职位原文与中文性别/国籍拼接时补中英边界空格（纯中文职位不受影响）
        let en_staff = build_vars(
            &sample_record(Some(DetailedInfo {
                preferred_job: "Performance Analyst".into(),
                ..Default::default()
            })),
            &tpl,
        );
        assert_eq!(en_staff["identity"], " Performance Analyst ");
        // 基础名单无 based：career 不入 map，由模板默认值「效力于职业联赛」兜底
        assert!(!build_vars(&sample_record(None), &tpl).contains_key("career"));
    }

    #[test]
    fn build_vars_female_skips_beard() {
        // 女性不随机胡须（变量缺席 → 模板默认值「干净无胡须」兜底），
        // 男性照常随机；判据是中英双语原文（女/Female）。
        let tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
        for gender in ["女", "Female"] {
            let vars = build_vars(
                &sample_record(Some(DetailedInfo {
                    gender: gender.into(),
                    ..Default::default()
                })),
                &tpl,
            );
            assert!(
                !vars.contains_key("beard"),
                "性别原文「{gender}」不得随机胡须"
            );
        }
        for gender in ["男", "Male"] {
            let vars = build_vars(
                &sample_record(Some(DetailedInfo {
                    gender: gender.into(),
                    ..Default::default()
                })),
                &tpl,
            );
            assert!(
                vars.contains_key("beard"),
                "性别原文「{gender}」必须随机胡须"
            );
        }
    }

    #[test]
    fn build_vars_staff_uses_staff_pools() {
        // 职员的服装/球衣细节/景别动作全部取自职员池——即使记录带 club 也不拼
        // 「{club}的比赛球衣」；球员对照路径仍走球员拼装与球员池。
        let tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
        let staff = sample_record(Some(DetailedInfo {
            preferred_job: "足球总监".into(),
            club: "曼联".into(),
            ..Default::default()
        }));
        for _ in 0..50 {
            let vars = build_vars(&staff, &tpl);
            assert_eq!(vars["career"], "任职于足球俱乐部");
            assert!(
                tpl.pools.staff.jersey.contains(&vars["jersey"]),
                "职员球衣「{}」应取自职员池",
                vars["jersey"]
            );
            assert!(
                tpl.pools.staff.kit_detail.contains(&vars["kitDetail"]),
                "职员球衣细节「{}」应取自职员池",
                vars["kitDetail"]
            );
            let shot = tpl
                .pools
                .staff
                .shots
                .iter()
                .find(|s| s.label == vars["shot"])
                .expect("职员景别应取自职员档位");
            assert!(
                shot.poses.contains(&vars["pose"]),
                "职员动作「{}」应落在景别「{}」的动作池内",
                vars["pose"],
                shot.label
            );
        }
        // 球员对照：同样带 club，仍是球员拼装口径
        let player = sample_record(Some(DetailedInfo {
            club: "曼联".into(),
            ..Default::default()
        }));
        assert_eq!(build_vars(&player, &tpl)["jersey"], "曼联的比赛球衣");

        // 手建模板没写职员池（blank_tpl）：职员路径逐子组回落出厂职员池——
        // 否则教练/队医会被随机到球员的踢球动作与比赛球衣。
        let medic = sample_record(Some(DetailedInfo {
            preferred_job: "队医".into(),
            ..Default::default()
        }));
        let vars = build_vars(&medic, &blank_tpl());
        assert!(
            BUILTIN_POOLS.staff.jersey.contains(&vars["jersey"]),
            "职员球衣「{}」应取自出厂职员池",
            vars["jersey"]
        );
        assert!(BUILTIN_POOLS.staff.kit_detail.contains(&vars["kitDetail"]));
        let shot = BUILTIN_POOLS
            .staff
            .shots
            .iter()
            .find(|s| s.label == vars["shot"])
            .expect("职员景别应落在出厂职员档位内");
        assert!(shot.poses.contains(&vars["pose"]));
        assert_eq!(vars["career"], "任职于足球俱乐部");
        assert_eq!(vars["identity"], "队医");
    }

    #[test]
    fn build_vars_random_skin_stays_within_ethnicity_band() {
        // 基础名单随机补足的肤色必须落在该族裔的合理色带内（含 1 档容差），
        // 否则会出现「北欧人配最深肤色」这类组合。东亚数据分位 5–8 → 容差后 4–9。
        let tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
        let mut rec = sample_record(None);
        rec.ethnicity_code = "10".into();
        let allowed: Vec<String> = (4..=9)
            .filter_map(|c| tpl.pools.skin_code.get(&c.to_string()).cloned())
            .collect();
        assert!(!allowed.is_empty());
        for _ in 0..50 {
            let vars = build_vars(&rec, &tpl);
            assert!(
                allowed.contains(&vars["skinCode"]),
                "肤色「{}」越出东亚色带",
                vars["skinCode"]
            );
        }
    }

    #[test]
    fn ensure_user_files_backfills_empty_pools_into_existing_template() {
        // 老配置的模板整份没写词库（pools 全空）：启动时整块补入，
        // 否则码值/随机维度会静默退化。用户改过的词库不动——判据是「整份空才补」。
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        fs::create_dir_all(d.join("prompts")).unwrap(); // json_store::save 不建父目录
        let mut stale = Config::builtin().templates[0].clone();
        assert!(!stale.pools.is_empty());
        stale.pools = DetailPools::default();
        json_store::save(&templates_path(d), &[stale]).unwrap();

        Config::ensure_user_files(d).unwrap();
        let cfg = Config::load(d).unwrap();
        assert_eq!(
            cfg.templates[0].pools.hair_color.len(),
            19,
            "空的词库应整块补入（含码值映射）"
        );
        assert_eq!(cfg.templates[0].pools.skin_code.len(), 20);

        // 非空（用户改过）的词库不回冲：只改一个键值，重启后原样保留。
        let mut edited = cfg.templates[0].clone();
        edited
            .pools
            .hair_color
            .insert("1".into(), "用户自定".into());
        json_store::save(&templates_path(d), &[edited]).unwrap();
        Config::ensure_user_files(d).unwrap();
        let cfg = Config::load(d).unwrap();
        assert_eq!(
            cfg.templates[0].pools.hair_color["1"], "用户自定",
            "用户改过的词库不得被内置值回冲"
        );
    }

    #[test]
    fn ensure_fresh_install_exports_builtin_and_is_idempotent() {
        // 全新安装：data/ 下连 prompts 目录都没有 → 建目录并按出厂内容导出服务商与模板。
        // 这条路径保证「删掉整个 data/ 也能恢复出厂」；再次运行必须零写盘（幂等）。
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert!(!d.join("prompts").exists());

        Config::ensure_user_files(d).unwrap();
        assert!(d.join("prompts").is_dir(), "prompts 目录应被自动创建");
        assert!(providers_path(d).is_file());
        assert!(templates_path(d).is_file());

        // 模板逐字复用出厂默认（含码值映射）
        let cfg = Config::load(d).unwrap();
        let builtin = Config::builtin();
        assert_eq!(cfg.templates.len(), builtin.templates.len());
        assert_eq!(
            cfg.templates[0].body, builtin.templates[0].body,
            "模板应逐字复用出厂默认（含新字段拼接）"
        );
        assert_eq!(
            cfg.templates[0].pools.hair_color.len(),
            19,
            "模板应携带码值映射"
        );
        assert_eq!(cfg.templates[0].pools.skin_code.len(), 20);

        // 幂等：再次运行两份文件均逐字节原样（不产生任何写盘变更）。
        let pp_before = fs::read_to_string(providers_path(d)).unwrap();
        let tp_before = fs::read_to_string(templates_path(d)).unwrap();
        Config::ensure_user_files(d).unwrap();
        assert_eq!(pp_before, fs::read_to_string(providers_path(d)).unwrap());
        assert_eq!(tp_before, fs::read_to_string(templates_path(d)).unwrap());
    }

    #[test]
    fn build_vars_prefers_template_pools_and_falls_back_to_builtin() {
        // 内容随模板走：模板写了自己的词库/码值映射就必须用它；没写（blank_tpl，全空）则
        // 逐组回落出厂默认——两个维度（词库/码值）在两条路径上都不允许静默消失。
        // 模板优先路径：样本沿用 sample_record 的发长码 "2"/发色码 "3"/族裔码 "0"。
        let mut tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .unwrap();
        tpl.pools = DetailPools {
            hair_style_by_length: [("2".to_string(), vec!["自定发型".into()])]
                .into_iter()
                .collect(),
            ethnicity: [("0".to_string(), "自定义族裔描述".to_string())]
                .into_iter()
                .collect(),
            hair_color: [("3".to_string(), "自定义金色".to_string())]
                .into_iter()
                .collect(),
            skin_code: [("12".to_string(), "自定义肤色".to_string())]
                .into_iter()
                .collect(),
            identity: [("player".to_string(), "自定义身份".to_string())]
                .into_iter()
                .collect(),
            shots: vec![ShotPool {
                label: "自定义景别".into(),
                poses: vec!["自定动作".into()],
            }],
            beard: vec!["自定胡须".into()],
            expression: vec!["自定表情".into()],
            lighting: vec!["自定打光".into()],
            bg_tone: vec!["自定背景".into()],
            kit_detail: vec!["自定球衣".into()],
            staff: StaffPools::default(),
        };

        let vars = build_vars(
            &sample_record(Some(DetailedInfo {
                skin_code: "12".into(),
                gender: "男".into(), // 固定男性：女性会跳过胡须随机，beard 断言需要确定性
                ..Default::default()
            })),
            &tpl,
        );
        // 词库全部取自模板池子（景别与动作联动：动作必须落在选中档的池内；
        // 发色并入发型出口——「一头{色}，{剪法}」）
        assert_eq!(vars["hairStyle"], "一头自定义金色，自定发型");
        assert_eq!(vars["shot"], "自定义景别");
        assert_eq!(vars["pose"], "自定动作");
        assert_eq!(vars["beard"], "自定胡须");
        assert_eq!(vars["expression"], "自定表情");
        assert_eq!(vars["lighting"], "自定打光");
        assert_eq!(vars["bgTone"], "自定背景");
        assert_eq!(vars["kitDetail"], "自定球衣");
        // 码值映射全部取自模板映射
        assert_eq!(vars["ethnicity"], "自定义族裔描述");
        assert_eq!(vars["hairColor"], "自定义金色");
        assert_eq!(vars["skinCode"], "自定义肤色");

        // 回落路径：内置默认模板必须自带词库与码值映射（回落源非空是逐组回落的前提）。
        let builtin_tpl = Config::builtin()
            .templates
            .into_iter()
            .find(|t| t.id == "default")
            .expect("内置默认模板");
        assert!(
            !BUILTIN_POOLS.ethnicity.is_empty(),
            "内置默认模板必须自带码值映射"
        );
        assert!(!builtin_tpl.pools.is_empty(), "内置默认模板必须自带词库");

        let vars = build_vars(&sample_record(None), &blank_tpl());
        // 码值维度：族裔直接对照出厂映射；随机补足的肤色应落在出厂映射内
        assert_eq!(vars["ethnicity"], BUILTIN_POOLS.ethnicity["0"]);
        assert!(
            BUILTIN_POOLS
                .skin_code
                .values()
                .any(|v| *v == vars["skinCode"]),
            "随机补足的肤色「{}」应在出厂映射内",
            vars["skinCode"]
        );
        // 随机维度：四项细节都应取自出厂词库；动作应落在出厂景别档位内。
        // beard 单独断言：女性跳过胡须随机（变量缺席），男性应落在出厂词库。
        for (key, pool) in [
            ("expression", &builtin_tpl.pools.expression),
            ("lighting", &builtin_tpl.pools.lighting),
            ("bgTone", &builtin_tpl.pools.bg_tone),
            ("kitDetail", &builtin_tpl.pools.kit_detail),
        ] {
            assert!(pool.contains(&vars[key]), "「{}」不在出厂词库里", vars[key]);
        }
        if vars["gender"] == "女" {
            assert!(!vars.contains_key("beard"), "女性不得随机胡须");
        } else {
            assert!(
                builtin_tpl.pools.beard.contains(&vars["beard"]),
                "「{}」不在出厂词库里",
                vars["beard"]
            );
        }
        let shot = builtin_tpl
            .pools
            .shots
            .iter()
            .find(|s| s.label == vars["shot"])
            .expect("景别应在出厂档位内");
        assert!(
            shot.poses.iter().any(|p| *p == vars["pose"]),
            "动作「{}」应属于景别「{}」的档位池",
            vars["pose"],
            vars["shot"]
        );
        assert_eq!(vars["identity"], "足球运动员");
    }
}
