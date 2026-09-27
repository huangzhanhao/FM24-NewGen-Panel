//! AI 文生图：提示词模板 → 用户编辑确认 → 调用文生图 API → 会话临时图片 → 手动保存/用量台账。
//!
//! 【与既有模块的复用关系，不另起一套】
//! - 命令统一写在 commands.rs，本模块只提供被命令调用的类型与函数；
//! - 出厂默认是 assets/ 三份 JSON（assets.rs include_str! 内嵌），用户可编辑副本在 data/；
//!   Config::builtin/load/ensure_user_files 仿 templates::RuleConfig；
//! - 原子写复用 json_store、分页复用 paging::PageResult。
//! P2 起多家协议适配成「族」拆在 image_ai_providers/（openai 迁入 + volcano + dashscope），
//! 本模块只保留契约/配置/渲染/台账与生成编排，生成时按 ProviderKind 经 registry 分发。

use crate::json_store::{self, LoadError};
use crate::paging::PageResult;
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
    /// 内部字段（前端不传，serde 缺省为空）：`generate` 解析出服务商后填入 base_url，
    /// 适配器据此拼 endpoint。
    #[serde(default)]
    pub base_url: String,
    /// 内部字段：`generate` 从模型能力集 `supports_response_format` 拷入，
    /// OpenAI 协议适配器据此决定是否传 response_format。
    #[serde(default)]
    pub supports_response_format: bool,
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
    // P2 起 openai/custom/volcano/dashscope 四家均已适配（image_ai_providers 族）。
    if provider.base_url.trim().is_empty()
        && matches!(provider.kind, ProviderKind::OpenAI | ProviderKind::Custom | ProviderKind::Volcano)
    {
        // dashscope endpoint 为固定常量，不需要 base_url；其余三家必须填写。
        return Err("请先在「服务商设置」中填写接口地址 Base URL".into());
    }
    let model = cfg.model(&req.provider_id, &req.model)
        .ok_or("请先选择模型，或在服务商设置中为该服务添加模型")?;
    if model.capabilities.max_n > 0 && req.n > model.capabilities.max_n {
        return Err(format!("该模型单次最多 {} 张", model.capabilities.max_n));
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
        let provider = cfg.provider(&req.provider_id).cloned().ok_or("请先选择服务商")?;
        let model = cfg.model(&req.provider_id, &req.model).cloned().ok_or("请先选择模型")?;
        (provider, model)
    };
    // 协议适配所需的服务商/能力信息随请求内部字段带入适配器（前端无需传）。
    req.base_url = provider.base_url.clone();
    req.supports_response_format = model.capabilities.supports_response_format;
    let record_id = new_id();
    let ts = now_ts();

    let outcome: Result<(Vec<GenImage>, f64, Option<RealUsage>), String> = async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(180))
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
        let unit = model.prices.get(&req.size).copied().unwrap_or(0.0);
        let cost = (unit * req.n as f64 * 1000.0).round() / 1000.0;
        Ok((images, cost, fetched.real_usage))
    }
    .await;

    match outcome {
        Ok((images, cost, real_usage)) => {
            let rec = UsageRecord {
                id: record_id.clone(),
                ts,
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                kind: provider.kind.as_str().into(),
                model: req.model.clone(),
                size: req.size.clone(),
                n: req.n,
                seed: req.seed,
                quality: req.quality,
                prompt: req.prompt.clone(),
                negative_prompt: req.negative_prompt.clone(),
                estimated_cost: cost,
                currency: model.currency.clone(),
                status: "success".into(),
                error: String::new(),
                real_usage,
            };
            append_usage(&usage_path(data_dir), &rec)?;
            Ok(GenResult { images, record_id, estimated_cost: cost, currency: model.currency })
        }
        Err(e) => {
            let rec = UsageRecord {
                id: record_id.clone(),
                ts,
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                kind: provider.kind.as_str().into(),
                model: req.model.clone(),
                size: req.size.clone(),
                n: req.n,
                seed: req.seed,
                quality: req.quality,
                prompt: req.prompt.clone(),
                negative_prompt: req.negative_prompt.clone(),
                estimated_cost: 0.0,
                currency: model.currency.clone(),
                status: "failed".into(),
                error: e.clone(),
                real_usage: None,
            };
            let _ = append_usage(&usage_path(data_dir), &rec);
            Err(e)
        }
    }
}

// ===================== 用量台账 JSONL =====================

pub fn append_usage(path: &Path, rec: &UsageRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let line = serde_json::to_string(rec).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("台账打开失败: {e}"))?;
    writeln!(f, "{line}").map_err(|e| e.to_string())
}

pub fn query_usage(path: &Path, q: &UsageQuery) -> Result<PageResult<UsageRecord>, String> {
    if !path.is_file() {
        return Ok(PageResult { total: 0, items: vec![] });
    }
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut recs: Vec<UsageRecord> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(r) = serde_json::from_str::<UsageRecord>(line) else { continue };
        if (!q.provider_id.is_empty() && r.provider_id != q.provider_id)
            || (!q.model.is_empty() && r.model != q.model)
            || (!q.status.is_empty() && r.status != q.status)
            || (!q.date.is_empty() && !r.ts.starts_with(&q.date))
        {
            continue;
        }
        recs.push(r);
    }
    let total = recs.len();
    recs.reverse();
    let items = recs.into_iter().skip(q.offset).take(q.limit).collect();
    Ok(PageResult { total, items })
}
#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
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
            base_url: String::new(),
            supports_response_format: false,
        }
    }

    #[test]
    fn provider_kind_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&ProviderKind::Dashscope).unwrap(), r#""dashscope""#);
        assert_eq!(serde_json::from_str::<ProviderKind>(r#""custom""#).unwrap(), ProviderKind::Custom);
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
        assert_eq!(before, fs::read_to_string(providers_path(root.path())).unwrap());
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
        assert_eq!(Config::load(root.path()).unwrap().providers[0].name, "MyOpenAI");
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
        let r = render("一名{{ethnicity}}球员，{{age}}岁",
                      &vars(&[("ethnicity", "东亚"), ("age", "18")]));
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
    fn ids_are_unique() {
        assert_ne!(new_id(), new_id());
        assert_eq!(new_id().len(), 16);
    }

    #[test]
    fn append_and_query_usage() {
        let p = tempfile::tempdir().unwrap().path().join("usage.jsonl");
        let mut rec = UsageRecord {
            id: "a".into(), ts: "2026-09-26 10:00:00".into(), provider_id: "openai".into(),
            provider_name: "OpenAI".into(), kind: "openai".into(), model: "gpt-image-1".into(),
            size: "1024x1024".into(), n: 1, seed: None, quality: None, prompt: "p".into(),
            negative_prompt: String::new(), estimated_cost: 0.04, currency: "USD".into(),
            status: "success".into(), error: String::new(), real_usage: None,
        };
        append_usage(&p, &rec).unwrap();
        rec.id = "b".into();
        rec.ts = "2026-09-27 09:00:00".into();
        rec.status = "failed".into();
        append_usage(&p, &rec).unwrap();
        let q = UsageQuery {
            offset: 0, limit: 10, provider_id: String::new(), model: String::new(),
            status: String::new(), date: String::new(),
        };
        let page = query_usage(&p, &q).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items[0].id, "b"); // 最新在前
    }

    #[test]
    fn query_usage_filters_and_skips_corrupt_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("usage.jsonl");
        fs::write(&p, "not json\n").unwrap();
        let q = UsageQuery {
            offset: 0, limit: 10, provider_id: String::new(), model: String::new(),
            status: String::new(), date: String::new(),
        };
        assert_eq!(query_usage(&p, &q).unwrap().total, 0); // 坏行不报错
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
    fn builtin_has_four_providers_p2() {
        let cfg = Config::builtin();
        let ids: Vec<&str> = cfg.providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["openai", "custom", "volcano", "dashscope"]);
        // 火山能力抽查：单张、支持 seed、不支持 quality
        let flash = cfg.model("volcano", "seedream-5.0-flash").unwrap();
        assert_eq!(flash.capabilities.max_n, 1);
        assert!(flash.capabilities.supports_seed);
        assert!(!flash.capabilities.supports_quality);
        assert_eq!(flash.prices["1024x1024"], 0.12);
        // 百炼能力抽查：n=4、负向提示词、size 用 * 形态
        let turbo = cfg.model("dashscope", "wanx2.1-t2i-turbo").unwrap();
        assert_eq!(turbo.capabilities.max_n, 4);
        assert!(turbo.capabilities.supports_negative_prompt);
        assert!(turbo.capabilities.sizes.contains(&"1024*1024".to_string()));
    }
}
