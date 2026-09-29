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
use std::time::Duration;

// ===================== 数据契约（与前端 types.ts 一一对应，camelCase）=====================

/// 服务商类型：决定适配器。custom = 用户自填 OpenAI 兼容服务。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAI,
    Volcano,
    Dashscope,
    Custom,
}

/// 自由尺寸规则（万相 2.5/2.6 等无固定尺寸枚举的模型）：0 表示该条约束不检查。
/// 尺寸字符串仍为 `宽*高`/`宽x高` 形态，约束含义逐字对齐官方文档：
/// 2.5/2.6 为总像素 [1280², 1440²]、宽高比 [1:4, 4:1]；2.2 及更早为单边 [512, 1440]。
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
    #[serde(default)]
    pub supports_negative_prompt: bool,
    /// 是否可传 `response_format`。OpenAI 官方 gpt-image-1 系列**不支持**该参数
    /// （始终返回 base64，传了会 400），故出厂置 false；OpenAI 兼容的第三方服务多为默认返回 URL，
    /// 需要显式声明 b64_json 才能免去一次下载，故缺省为 true。
    #[serde(default = "yes")]
    pub supports_response_format: bool,
    #[serde(default)]
    pub qualities: Vec<String>,
    /// 百炼 `prompt_extend`：大模型智能改写正向提示词（+3–4 秒；关闭可规避版权审核拦截）。
    #[serde(default)]
    pub supports_prompt_extend: bool,
    /// `prompt_extend` 的官方默认开关：千问/万相为开，z-image 为关（开启即思考模式、单价翻倍）。
    /// 前端切模型时按此初始化开关；缺省 true 兼容旧配置。
    #[serde(default = "yes")]
    pub prompt_extend_default: bool,
    /// 百炼 `watermark`：右下角固定「AI生成」水印。
    #[serde(default)]
    pub supports_watermark: bool,
    /// 请求体是否携带 `n` 参数：z-image 无此参数（固定 1 张）必须省略；其余百炼模型为 true。
    /// 缺省 true 兼容 OpenAI/火山与旧配置（它们一律发 n）。
    #[serde(default = "yes")]
    pub supports_n: bool,
    /// 正向提示词字符/Token 上限（0 = 不展示计数、不校验；
    /// qwen3.0=4500/qwen2.0=1300/wan2.7=5000/wan2.6=2100/z-image=800）。
    #[serde(default)]
    pub max_prompt_chars: u32,
    /// 自由尺寸规则；None = 仅允许 sizes 枚举值（自定义走软警告，不做硬校验）。
    #[serde(default)]
    pub size_rules: Option<SizeRules>,
}

fn one() -> u32 {
    1
}

fn yes() -> bool {
    true
}

/// 一个模型行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub model: String,
    pub label: String,
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
    pub history_limit: usize,
    pub concurrency: u32,
    pub naming_pattern: String,
}

impl Default for GenSettings {
    fn default() -> Self {
        Self {
            default_provider_id: "openai".into(),
            history_limit: 20,
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
    pub uid: Option<String>,
    /// 万相智能改写开关（仅百炼适配器消费；None = 按官方默认 true）。
    #[serde(default)]
    pub prompt_extend: Option<bool>,
    /// 万相「AI生成」水印开关（仅百炼适配器消费；None = 按官方默认 false）。
    #[serde(default)]
    pub watermark: Option<bool>,
    /// 内部字段（前端不传，serde 缺省为空）：`generate` 解析出服务商后填入 base_url，
    /// 适配器据此拼 endpoint。
    #[serde(default)]
    pub base_url: String,
    /// 内部字段：`generate` 从模型能力集 `supports_response_format` 拷入，
    /// OpenAI 协议适配器据此决定是否传 response_format。
    #[serde(default)]
    pub supports_response_format: bool,
    /// 内部字段：`generate` 从模型能力集拷入，百炼适配器据此决定是否发送对应参数
    /// （OpenAI 官方端点不认这些字段，门控为 false 时一律不带，避免 400）。
    #[serde(default)]
    pub supports_negative_prompt: bool,
    #[serde(default)]
    pub supports_prompt_extend: bool,
    #[serde(default)]
    pub supports_watermark: bool,
    #[serde(default)]
    pub supports_seed: bool,
    /// 内部字段：`generate` 从模型能力集拷入，百炼适配器据此决定请求体是否携带 n
    /// （z-image 无 n 参数，必须省略）。
    #[serde(default = "yes")]
    pub supports_n: bool,
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

/// 一条会话历史索引：指向 `.imgtmp` 里的一张临时图（文件名为 `{id}.{扩展名}`）。
/// ts 为写入时间（now_ts 格式），该格式字典序即时间序，LRU 裁剪据此判断新旧。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub ts: String,
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
pub fn history_path(d: &Path) -> PathBuf {
    tmp_dir(d).join("history.json")
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
        // 百炼模型目录代码托管：旧用户文件里的 dashscope 块在此迁移（模型行整表替换，
        // 仅保留显示名/base_url），新模型随版本自动推送。
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
                label: model.to_string(),
                capabilities: Capabilities::default(),
            });
        }
        None
    }
}

/// 把代码内置的百炼服务商目录并入一份服务商列表（builtin/load/ensure 三处同口径）。
///
/// 百炼模型目录代码托管（不写 assets/image_ai.json），故：
/// - 列表中没有 dashscope：整块补入内置服务商；
/// - 已有（含旧版本写入的 wanx/wan2.x 旧模型行）：**只保留用户可编辑的显示名与 base_url**
///   （base_url 可能指向代理平台，绝不能丢），kind 归位、模型行整表替换、
///   默认模型失效（旧模型已下线）时校正为内置默认。
///
/// 返回是否发生需要落盘的变更（内容已一致时不重写，避免每次启动无谓写文件）。
fn reconcile_builtin_providers(providers: &mut Vec<ProviderConfig>) -> bool {
    let builtin = crate::image_ai_providers::dashscope::builtin_provider();
    match providers
        .iter_mut()
        .find(|p| p.id == crate::image_ai_providers::dashscope::PROVIDER_ID)
    {
        Some(slot) => {
            let mut changed = false;
            if slot.kind != ProviderKind::Dashscope {
                slot.kind = ProviderKind::Dashscope;
                changed = true;
            }
            if slot.models != builtin.models {
                slot.models = builtin.models.clone();
                changed = true;
            }
            let default_valid = slot.models.iter().any(|m| m.model == slot.default_model);
            if !default_valid {
                slot.default_model = builtin.default_model.clone();
                changed = true;
            }
            // name / base_url 刻意不动：用户改名或指向代理平台是合法定制。
            changed
        }
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

pub fn remove_provider(cfg: &mut Config, id: &str) -> Result<(), String> {
    if id == "openai" || id == "custom" {
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

/// 生成前的统一校验（服务商/类型/地址/模型/张数/提示词）。
pub fn validate_request(cfg: &Config, req: &GenRequest) -> Result<(), String> {
    if req.prompt.trim().is_empty() {
        return Err("提示词不能为空".into());
    }
    if req.n < 1 {
        return Err("张数至少为 1".into());
    }
    let provider = cfg.provider(&req.provider_id).ok_or("请先选择服务商")?;
    // P2 起 openai/custom/volcano/dashscope 四家均已适配（image_ai_providers 族）。
    if provider.base_url.trim().is_empty()
        && matches!(
            provider.kind,
            ProviderKind::OpenAI | ProviderKind::Custom | ProviderKind::Volcano
        )
    {
        // dashscope endpoint 为固定常量，不需要 base_url；其余三家必须填写。
        return Err("请先在「服务商设置」中填写接口地址 Base URL".into());
    }
    let model = cfg
        .model(&req.provider_id, &req.model)
        .ok_or("请先选择模型，或在服务商设置中为该服务添加模型")?;
    if model.capabilities.max_n > 0 && req.n > model.capabilities.max_n {
        return Err(format!("该模型单次最多 {} 张", model.capabilities.max_n));
    }
    // 提示词长度：官方口径是「超长自动截断」，这里改为显式报错，避免用户以为全文生效。
    if model.capabilities.max_prompt_chars > 0
        && req.prompt.chars().count() > model.capabilities.max_prompt_chars as usize
    {
        return Err(format!(
            "提示词超出该模型 {} 字上限（当前 {} 字），请精简后再生成",
            model.capabilities.max_prompt_chars,
            req.prompt.chars().count()
        ));
    }
    // 自由尺寸模型（万相 2.5/2.6、2.2）：尺寸不在预设枚举时按规则硬校验。
    if let Some(rules) = &model.capabilities.size_rules {
        if !model.capabilities.sizes.iter().any(|s| s == &req.size) {
            validate_size(&req.size, rules)?;
        }
    }
    Ok(())
}

/// 解析 `宽*高`/`宽x高` 尺寸并按模型自由尺寸规则校验；0 值字段表示该约束不检查。
pub fn validate_size(size: &str, rules: &SizeRules) -> Result<(), String> {
    let caps: Vec<u32> = size
        .split(['*', 'x'])
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .collect();
    let (w, h) = match caps.as_slice() {
        [w, h] if *w > 0 && *h > 0 => (*w, *h),
        _ => {
            return Err(format!(
                "尺寸「{size}」格式不正确，应为 宽*高（如 1280*1280）"
            ))
        }
    };
    if rules.min_side > 0 && (w < rules.min_side || h < rules.min_side) {
        return Err(format!("尺寸宽/高不能小于 {} 像素", rules.min_side));
    }
    if rules.max_side > 0 && (w > rules.max_side || h > rules.max_side) {
        return Err(format!("尺寸宽/高不能超过 {} 像素", rules.max_side));
    }
    let total = w as u64 * h as u64;
    if rules.total_min > 0 && total < rules.total_min {
        return Err(format!(
            "总像素不能小于 {}（约 {}²）",
            rules.total_min,
            (rules.total_min as f64).sqrt() as u32
        ));
    }
    if rules.total_max > 0 && total > rules.total_max {
        return Err(format!(
            "总像素不能超过 {}（约 {}²）",
            rules.total_max,
            (rules.total_max as f64).sqrt() as u32
        ));
    }
    if rules.ratio_min > 0.0 || rules.ratio_max > 0.0 {
        let ratio = w as f32 / h as f32;
        if (rules.ratio_min > 0.0 && ratio + 1e-4 < rules.ratio_min)
            || (rules.ratio_max > 0.0 && ratio - 1e-4 > rules.ratio_max)
        {
            return Err(format!(
                "宽高比需在 {:.2}:1 至 {:.2}:1 之间（当前 {:.2}:1）",
                rules.ratio_min, rules.ratio_max, ratio
            ));
        }
    }
    Ok(())
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
) -> Result<GenResult, String> {
    let (provider, model) = {
        let provider = cfg
            .provider(&req.provider_id)
            .cloned()
            .ok_or("请先选择服务商")?;
        let model = cfg
            .model(&req.provider_id, &req.model)
            .ok_or("请先选择模型")?;
        (provider, model)
    };
    // 协议适配所需的服务商/能力信息随请求内部字段带入适配器（前端无需传）。
    req.base_url = provider.base_url.clone();
    req.supports_response_format = model.capabilities.supports_response_format;
    req.supports_negative_prompt = model.capabilities.supports_negative_prompt;
    req.supports_prompt_extend = model.capabilities.supports_prompt_extend;
    req.supports_watermark = model.capabilities.supports_watermark;
    req.supports_seed = model.capabilities.supports_seed;
    req.supports_n = model.capabilities.supports_n;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // 百炼三系列文生图统一为同步长连接（千问 3.0 多图/思考模式官方建议客户端超时 600s 起），
        // 该 client 四家共用，放宽上限不影响 OpenAI/火山的正常返回（它们本就秒级完成）。
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))?;
    let adapter = crate::image_ai_providers::for_kind(provider.kind);
    // 适配器内部完成 POST/解析/（异步厂商的 submit→poll→fetch），图片已为字节。
    let fetched = adapter.generate(&client, key, &req).await?;

    let tmp = tmp_dir(data_dir);
    fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let mut images = Vec::new();
    for img in fetched.images {
        let id = new_id();
        let path = tmp.join(format!("{id}.{}", img.extension));
        fs::write(&path, &img.bytes).map_err(|e| format!("临时图片写入失败: {e}"))?;
        images.push(GenImage {
            id,
            path: path.to_string_lossy().to_string(),
            bytes: img.bytes.len() as u64,
        });
    }
    // 会话历史只是临时图索引：记录/裁剪失败不应掩盖已成功的生成（图片已落盘）。
    if let Err(e) = record_history(data_dir, &images, cfg.settings.history_limit) {
        log::warn!(target: "image_ai", "记录会话历史失败（不影响本次生成）: {e}");
    }
    Ok(GenResult { images })
}

// ===================== 会话历史：LRU 裁剪（清单在 .imgtmp/history.json，随会话清理）=====================

/// 读历史清单。缺失（首次生成）按空表；损坏同样按空表并告警——历史只是临时图索引，
/// 不值得为它阻断生成（坏数据跳过的宽容口径），下次保存即完成重建。
fn read_history(path: &Path) -> Vec<HistoryEntry> {
    match json_store::load::<Vec<HistoryEntry>>(path) {
        Ok(v) => v,
        Err(LoadError::Missing) => Vec::new(),
        Err(LoadError::Corrupt(e)) => {
            log::warn!(target: "image_ai", "生图历史损坏，按空历史重建: {}（{e}）", path.display());
            Vec::new()
        }
    }
}

/// 把本次生成的每张图记入历史（每图一条：裁剪粒度是图片文件，而非一次生成），
/// 再按上限裁剪。仅 generate() 成功路径调用——失败路径图片未落盘，不产生历史。
pub fn record_history(data_dir: &Path, images: &[GenImage], limit: usize) -> Result<(), String> {
    // 目录可能不存在（极端情况下被外部清走）：自建父目录，不让索引写盘因目录缺失而失败。
    fs::create_dir_all(tmp_dir(data_dir)).map_err(|e| e.to_string())?;
    let path = history_path(data_dir);
    let ts = now_ts();
    let mut hist = read_history(&path);
    for img in images {
        hist.push(HistoryEntry {
            id: img.id.clone(),
            ts: ts.clone(),
        });
    }
    json_store::save(&path, &hist)?;
    enforce_history_limit(data_dir, limit)
}

/// 按写入时间倒序保留最新 limit 条历史，超出的条目连同 `.imgtmp` 中的图片文件一并删除。
/// limit==0 视为「不裁剪」：与 P1 的「无上限」语义对齐（出厂默认 20，0 是用户显式关闭裁剪的开关，
/// 若按「全删」解释会在用户改配置的瞬间清空全部会话图片，代价不对称）。
pub fn enforce_history_limit(data_dir: &Path, limit: usize) -> Result<(), String> {
    if limit == 0 {
        return Ok(());
    }
    let path = history_path(data_dir);
    let mut hist = read_history(&path);
    if hist.len() <= limit {
        return Ok(());
    }
    // ts 为 "%Y-%m-%d %H:%M:%S"，字典序即时间序；按升序稳定排序后旧的聚在前，
    // 同一秒内先写入的条目排前（先写入=更旧），与正常追加顺序一致，裁剪结果确定。
    hist.sort_by(|a, b| a.ts.cmp(&b.ts));
    let excess: Vec<HistoryEntry> = hist.drain(..hist.len() - limit).collect();
    let tmp = tmp_dir(data_dir);
    for e in &excess {
        // 图片名 `{id}.{扩展名}`，扩展名随厂商返回格式变化，只能按文件主干匹配；
        // 对应文件可能已被清走（prepare_session/手动清理），找不到即跳过；删除失败仅告警不阻断。
        let Ok(rd) = fs::read_dir(&tmp) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file() && p.file_stem().map(|s| s == e.id.as_str()).unwrap_or(false) {
                if let Err(err) = fs::remove_file(&p) {
                    log::warn!(target: "image_ai", "清理过期临时图片失败 {}: {err}", p.display());
                }
            }
        }
    }
    json_store::save(&path, &hist)
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
            uid: None,
            prompt_extend: None,
            watermark: None,
            base_url: String::new(),
            supports_response_format: false,
            supports_negative_prompt: false,
            supports_prompt_extend: false,
            supports_watermark: false,
            supports_seed: false,
            supports_n: true,
        }
    }

    #[test]
    fn provider_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&ProviderKind::Dashscope).unwrap(),
            r#""dashscope""#
        );
        assert_eq!(
            serde_json::from_str::<ProviderKind>(r#""custom""#).unwrap(),
            ProviderKind::Custom
        );
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
        // 出厂 gpt-image-1 必须关掉该能力，custom 保持开启。
        let cfg = Config::builtin();
        let openai = cfg.model("openai", "gpt-image-1").unwrap();
        assert!(!openai.capabilities.supports_response_format);
    }

    #[test]
    fn default_model_without_detail_row_is_served() {
        // custom 服务商只有 default_model 字符串、没有模型明细行：
        // 请求默认模型必须放行（合成通用能力），这是用户实测被「请先选择模型」拦住的根因。
        let mut cfg = Config::builtin();
        upsert_provider(
            &mut cfg,
            ProviderConfig {
                id: "my-relay".into(),
                kind: ProviderKind::Custom,
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
    fn validate_request_rules() {
        let cfg = Config::builtin();
        let mut r = gen_req();
        r.prompt = " ".into();
        assert!(validate_request(&cfg, &r).is_err());
        r = gen_req();
        r.provider_id = "nope".into();
        assert!(validate_request(&cfg, &r).is_err());
        r = gen_req();
        r.n = 99;
        assert!(validate_request(&cfg, &r).is_err());
        assert!(validate_request(&cfg, &gen_req()).is_ok());
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
    fn builtin_has_four_providers_p2() {
        let cfg = Config::builtin();
        let ids: Vec<&str> = cfg.providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["openai", "custom", "volcano", "dashscope"]);
        // 火山能力抽查：单张、支持 seed、不支持 quality
        let flash = cfg.model("volcano", "seedream-5.0-flash").unwrap();
        assert_eq!(flash.capabilities.max_n, 1);
        assert!(flash.capabilities.supports_seed);
        assert!(!flash.capabilities.supports_quality);
        // 百炼模型目录代码内置：三系列 8 个模型抽查关键差异。
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
    }

    #[test]
    fn dashscope_catalog_is_injected_even_when_json_has_no_entry() {
        // assets/image_ai.json 已不含 dashscope 块：builtin 仍须注入代码内置目录。
        let cfg = Config::builtin();
        let p = cfg.provider("dashscope").expect("内置百炼服务商必须存在");
        assert_eq!(p.default_model, "qwen-image-3.0-pro");
        assert_eq!(p.models.len(), 8);
    }

    #[test]
    fn reconcile_migrates_legacy_dashscope_block_keeping_user_edits() {
        // 模拟旧版本用户文件：dashscope 块带旧模型行、自定义显示名与代理 base_url、
        // 已失效的默认模型。
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
                default_model: "wanx2.1-t2i-turbo".into(), // 已下线 → 须校正
                models: vec![ModelInfo {
                    model: "wanx2.1-t2i-turbo".into(),
                    label: "旧模型".into(),
                    capabilities: Capabilities::default(),
                }],
            },
        ];
        assert!(reconcile_builtin_providers(&mut providers));
        let ds = providers.iter().find(|p| p.id == "dashscope").unwrap();
        // 用户定制保留
        assert_eq!(ds.name, "我的百炼代理");
        assert_eq!(ds.base_url, "https://proxy.example.com/api/v1");
        // 模型目录与默认模型被代码接管
        assert_eq!(ds.models.len(), 8);
        assert!(ds.models.iter().all(|m| m.model != "wanx2.1-t2i-turbo"));
        assert_eq!(ds.default_model, "qwen-image-3.0-pro");
        // 再跑一次：内容已一致，不产生变更（不触发无谓写盘）。
        assert!(!reconcile_builtin_providers(&mut providers));
        // 用户把默认模型改成新目录内的 wan2.7：不应被校正。
        providers[1].default_model = "wan2.7-image".into();
        assert!(!reconcile_builtin_providers(&mut providers));
        assert_eq!(providers[1].default_model, "wan2.7-image");
    }

    #[test]
    fn ensure_migrates_legacy_dashscope_user_file_end_to_end() {
        // 旧版本用户文件：只有旧 dashscope 服务商、默认模型已下线。ensure 必须就地迁移
        // 并把迁移结果写回（load 与磁盘两边都是 8 模型新目录）。
        let root = tempfile::tempdir().unwrap();
        let legacy = serde_json::json!({
            "providers": [{
                "id": "dashscope",
                "kind": "dashscope",
                "name": "百炼通义万相",
                "baseUrl": "https://dashscope.aliyuncs.com/api/v1",
                "defaultModel": "wanx2.1-t2i-turbo",
                "models": []
            }],
            "settings": {
                "defaultProviderId": "dashscope",
                "historyLimit": 20,
                "concurrency": 2,
                "namingPattern": "{uid}_{index}"
            }
        });
        fs::write(providers_path(root.path()), legacy.to_string()).unwrap();

        Config::ensure_user_files(root.path()).unwrap();

        let cfg = Config::load(root.path()).unwrap();
        assert_eq!(cfg.provider("dashscope").unwrap().models.len(), 8);
        assert_eq!(
            cfg.provider("dashscope").unwrap().default_model,
            "qwen-image-3.0-pro"
        );
        // 再次 ensure 幂等：迁移已落盘，不重复产生写盘变更。
        let before = fs::read_to_string(providers_path(root.path())).unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        assert_eq!(
            before,
            fs::read_to_string(providers_path(root.path())).unwrap()
        );
    }

    // ===================== 会话历史 LRU =====================

    /// 在 .imgtmp 造一张假图文件（裁剪按文件主干匹配，扩展名随意）。TempDir 必须先绑定再使用。
    fn fake_image(root: &Path, id: &str) -> PathBuf {
        fs::create_dir_all(tmp_dir(root)).unwrap();
        let p = tmp_dir(root).join(format!("{id}.png"));
        fs::write(&p, b"img").unwrap();
        p
    }

    /// 造一条指定秒数的时间戳条目（"%H:%M:%S" 字典序即时间序，秒数递增即时间递增）。
    fn hist_entry(id: &str, sec: u32) -> HistoryEntry {
        HistoryEntry {
            id: id.into(),
            ts: format!("2026-09-26 10:00:{sec:02}"),
        }
    }

    /// 测试里读回历史清单（LoadError 未实现 Debug，不能对 json_store::load 直接 unwrap）。
    fn load_history(root: &Path) -> Vec<HistoryEntry> {
        serde_json::from_str(&fs::read_to_string(history_path(root)).unwrap()).unwrap()
    }

    #[test]
    fn enforce_keeps_newest_and_deletes_oldest_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp_dir(dir.path())).unwrap();
        let ids: Vec<String> = (0..7).map(|_| new_id()).collect();
        let hist: Vec<HistoryEntry> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| hist_entry(id, i as u32))
            .collect();
        json_store::save(&history_path(dir.path()), &hist).unwrap();
        for id in &ids {
            fake_image(dir.path(), id);
        }

        enforce_history_limit(dir.path(), 5).unwrap();

        // 最早 2 张连文件一起删，最新 5 张保留
        assert!(!tmp_dir(dir.path()).join(format!("{}.png", ids[0])).exists());
        assert!(!tmp_dir(dir.path()).join(format!("{}.png", ids[1])).exists());
        for id in &ids[2..] {
            assert!(tmp_dir(dir.path()).join(format!("{}.png", id)).exists());
        }
        let kept: Vec<HistoryEntry> = load_history(dir.path());
        assert_eq!(kept.len(), 5);
        assert_eq!(kept[0].id, ids[2]); // 清单保持时间升序：最旧在先
        assert_eq!(kept[4].id, ids[6]);
    }

    #[test]
    fn corrupt_history_rebuilds_and_trims() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp_dir(dir.path())).unwrap();
        fs::write(history_path(dir.path()), "not json").unwrap();
        // 损坏不报错，按空历史处理（空表不触发裁剪）
        enforce_history_limit(dir.path(), 2).unwrap();

        // 随后记录与裁剪恢复正常：3 张图 limit=2 → 最旧一张连文件一起删
        let ids: Vec<String> = (0..3).map(|_| new_id()).collect();
        let images: Vec<GenImage> = ids
            .iter()
            .map(|id| GenImage {
                id: id.clone(),
                path: String::new(),
                bytes: 0,
            })
            .collect();
        for id in &ids {
            fake_image(dir.path(), id);
        }
        record_history(dir.path(), &images, 2).unwrap();
        assert!(!tmp_dir(dir.path()).join(format!("{}.png", ids[0])).exists());
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[1])).exists());
        assert!(tmp_dir(dir.path()).join(format!("{}.png", ids[2])).exists());
        let kept: Vec<HistoryEntry> = load_history(dir.path());
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn trim_skips_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp_dir(dir.path())).unwrap();
        let hist: Vec<HistoryEntry> = (0..3)
            .map(|i| hist_entry(&format!("ghost{i}"), i as u32))
            .collect();
        json_store::save(&history_path(dir.path()), &hist).unwrap();
        // 对应文件全都不存在，裁剪不应报错
        enforce_history_limit(dir.path(), 1).unwrap();
        let kept: Vec<HistoryEntry> = load_history(dir.path());
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "ghost2");
    }

    #[test]
    fn zero_limit_means_no_trim() {
        // limit==0 与 P1「无上限」语义对齐：不裁剪、不动文件
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp_dir(dir.path())).unwrap();
        let ids: Vec<String> = (0..3).map(|_| new_id()).collect();
        let hist: Vec<HistoryEntry> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| hist_entry(id, i as u32))
            .collect();
        json_store::save(&history_path(dir.path()), &hist).unwrap();
        for id in &ids {
            fake_image(dir.path(), id);
        }
        enforce_history_limit(dir.path(), 0).unwrap();
        for id in &ids {
            assert!(tmp_dir(dir.path()).join(format!("{}.png", id)).exists());
        }
        let kept: Vec<HistoryEntry> = load_history(dir.path());
        assert_eq!(kept.len(), 3);
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
