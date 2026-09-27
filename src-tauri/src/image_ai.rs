//! AI 文生图：提示词模板 → 用户编辑确认 → 调用文生图 API → 会话临时图片 → 手动保存/用量台账。
//!
//! 【与既有模块的复用关系，不另起一套】
//! - 命令统一写在 commands.rs，本模块只提供被命令调用的类型与函数；
//! - 出厂默认是 assets/ 三份 JSON（assets.rs include_str! 内嵌），用户可编辑副本在 data/；
//!   Config::builtin/load/ensure_user_files 仿 templates::RuleConfig；
//! - 原子写复用 json_store、分页复用 paging::PageResult。
//! 【扩展点】P2 多家适配器成「族」时，按 templates/ 目录先例把协议适配拆为 image_ai_providers/。

use crate::json_store::{self, LoadError};
use crate::paging::PageResult;
use base64::Engine;
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

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::OpenAI => "openai",
            ProviderKind::Volcano => "volcano",
            ProviderKind::Dashscope => "dashscope",
            ProviderKind::Custom => "custom",
        }
    }
}

/// 模型能力集：UI 按此显隐参数，适配器按此决定是否传参。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}

fn one() -> u32 {
    1
}

fn yes() -> bool {
    true
}

/// 一个模型行；prices 的 key 为尺寸标签、value 为单张估算单价。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub model: String,
    pub label: String,
    pub currency: String,
    #[serde(default)]
    pub prices: HashMap<String, f64>,
    #[serde(default)]
    pub capabilities: Capabilities,
}

/// 服务商配置（不含 API Key）。
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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
}

/// 一张临时图片元信息。
#[derive(Debug, Clone, Serialize)]
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
    pub record_id: String,
    pub estimated_cost: f64,
    pub currency: String,
}

/// 厂商真实 token 用量。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RealUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// 一条用量台账。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub id: String,
    pub ts: String,
    pub provider_id: String,
    pub provider_name: String,
    pub kind: String,
    pub model: String,
    pub size: String,
    pub n: u32,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub quality: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: String,
    pub estimated_cost: f64,
    pub currency: String,
    pub status: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub real_usage: Option<RealUsage>,
}

/// 台账分页查询。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuery {
    pub offset: usize,
    pub limit: usize,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub date: String,
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
pub fn usage_path(d: &Path) -> PathBuf {
    d.join("usage").join("usage.jsonl")
}
pub fn tmp_dir(d: &Path) -> PathBuf {
    d.join(".imgtmp")
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
        let f: ImageAiFile = serde_json::from_str(crate::assets::IMAGE_AI_RAW)
            .expect("内置 image_ai.json 格式错误");
        Config {
            providers: f.providers,
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
            Ok(f) => {
                cfg.providers = f.providers;
                cfg.settings = f.settings;
            }
            Err(LoadError::Missing) => {}
            Err(LoadError::Corrupt(e)) => {
                return Err(format!("生图配置无法解析：{}（{e}）", providers_path(d).display()))
            }
        }
        match json_store::load::<Vec<PromptTemplate>>(&templates_path(d)) {
            Ok(v) => cfg.templates = v,
            Err(LoadError::Missing) => {}
            Err(LoadError::Corrupt(e)) => {
                return Err(format!("模板无法解析：{}（{e}）", templates_path(d).display()))
            }
        }
        match json_store::load::<CodeDictionary>(&dictionary_path(d)) {
            Ok(v) => cfg.dictionary = v,
            Err(LoadError::Missing) => {}
            Err(LoadError::Corrupt(e)) => {
                return Err(format!("码值表无法解析：{}（{e}）", dictionary_path(d).display()))
            }
        }
        Ok(cfg)
    }

    /// 首次运行导出用户文件（已存在不覆盖）；同时把新版本新增的内置服务商/模板补进用户文件。
    pub fn ensure_user_files(d: &Path) -> Result<(), String> {
        fs::create_dir_all(d.join("prompts")).map_err(|e| e.to_string())?;
        fs::create_dir_all(d.join("usage")).map_err(|e| e.to_string())?;
        let builtin = Self::builtin();

        // image_ai.json：缺失整份导出；存在则补入缺失的内置服务商并校正默认服务商。
        let pp = providers_path(d);
        let mut file = match json_store::load::<ImageAiFile>(&pp) {
            Ok(f) => f,
            Err(LoadError::Missing) => {
                json_store::save(&pp, &ImageAiFile {
                    providers: builtin.providers.clone(),
                    settings: builtin.settings.clone(),
                })?;
                ImageAiFile { providers: builtin.providers.clone(), settings: builtin.settings.clone() }
            }
            Err(LoadError::Corrupt(e)) => {
                return Err(format!("{}（{e}）", pp.display()))
            }
        };
        let mut changed = false;
        for p in &builtin.providers {
            if !file.providers.iter().any(|x| x.id == p.id) {
                file.providers.push(p.clone());
                changed = true;
            }
        }
        if !file.providers.iter().any(|p| p.id == file.settings.default_provider_id) {
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
        json_store::save(&providers_path(d), &ImageAiFile {
            providers: self.providers.clone(),
            settings: self.settings.clone(),
        })
    }

    pub fn save_templates(&self, d: &Path) -> Result<(), String> {
        json_store::save(&templates_path(d), &self.templates)
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn model(&self, provider_id: &str, model: &str) -> Option<&ModelInfo> {
        self.provider(provider_id)?.models.iter().find(|m| m.model == model)
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
    if !matches!(provider.kind, ProviderKind::OpenAI | ProviderKind::Custom) {
        return Err("该服务商适配将在后续版本提供，当前请使用 OpenAI 或自定义兼容服务".into());
    }
    if provider.base_url.trim().is_empty() {
        return Err("请先在「服务商设置」中填写接口地址 Base URL".into());
    }
    let model = cfg.model(&req.provider_id, &req.model)
        .ok_or("请先选择模型，或在服务商设置中为该服务添加模型")?;
    if model.capabilities.max_n > 0 && req.n > model.capabilities.max_n {
        return Err(format!("该模型单次最多 {} 张", model.capabilities.max_n));
    }
    Ok(())
}
