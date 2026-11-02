//! 生图模块的生效配置：出厂默认 + 用户文件双来源（仿 templates::RuleConfig）。
//!
//! 【数据落点】服务商与设置在 `{data_dir}/image_ai.json`，提示词模板在
//! `{data_dir}/prompts/templates.json`，两者都由 assets 里的出厂 JSON 兜底。
//! 【内置服务商】dashscope/volcano/siliconflow/modelscope/comfyui 的目录写在代码里
//! （`image_ai_providers::*::builtin_provider`），不写 assets——builtin/load/ensure
//! 三处同口径归并，详见 [`reconcile_builtin_providers`]。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use crate::json_store::{self, LoadError};

use super::types::{
    Capabilities, GenSettings, ImageAiFile, ModelInfo, PromptTemplate, ProviderConfig,
};

// ===================== data/ 路径 ================================================================

/// 服务商与设置文件：`{data_dir}/image_ai.json`。
pub fn providers_path(d: &Path) -> PathBuf {
    d.join("image_ai.json")
}
/// 提示词模板文件：`{data_dir}/prompts/templates.json`。
pub fn templates_path(d: &Path) -> PathBuf {
    d.join("prompts").join("templates.json")
}
/// 会话临时图片目录：`{data_dir}/.imgtmp`，每次启动清理其中的图片（保留 gallery.json，见
/// `Config::prepare_session`）。
pub fn tmp_dir(d: &Path) -> PathBuf {
    d.join(".imgtmp")
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

    /// 启动会话准备：清空上一次会话的临时图片并补建目录。
    /// 【为什么不再整目录 remove_dir_all】`gallery.json` 住在 `.imgtmp`（聚合布局）且跨会话保留，
    /// 整目录删除会抹掉全部画廊历史；故逐项清理，只放行记录文件（含 `.corrupt` 备份）。
    /// 【注意】放行判定走 [`super::gallery::is_gallery_file`]，不要另写一份：以后再有需要
    /// 跨会话保留的文件只改那一处，漏改这里就会被静默删除。
    pub fn prepare_session(d: &Path) -> Result<(), String> {
        let tmp = tmp_dir(d);
        fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let entries = fs::read_dir(&tmp).map_err(|e| format!("清理临时目录失败: {e}"))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("清理临时目录失败: {e}"))?;
            if super::gallery::is_gallery_file(&entry.file_name()) {
                continue;
            }
            let p = entry.path();
            let removed = if p.is_dir() {
                fs::remove_dir_all(&p)
            } else {
                fs::remove_file(&p)
            };
            removed.map_err(|e| format!("清理临时文件失败（{}）: {e}", p.display()))?;
        }
        Ok(())
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

    /// 回前端前的视图：抹掉全部 Key 密文。
    /// 【必须走这里】有多个命令会整体返回 Config；前端只认 keysSet 布尔，
    /// 不需要也不该拿到密文。逐处清空必然漏，收口成一个方法。
    pub fn public_view(&self) -> Self {
        let mut out = self.clone();
        for p in &mut out.providers {
            p.api_key = None;
        }
        out
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
        Some(slot) => {
            // 前端不持有密文，回传的 api_key 恒为 None：**必须保留存量**，否则用户改一下
            // 服务商名称就把 Key 静默抹掉且无任何报错。清空只走 set_provider_key（传空串）。
            if p.api_key.is_none() {
                p.api_key = slot.api_key.take();
            }
            *slot = p;
        }
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

// ===================== 单元测试：出厂目录 / 归并 / 加载导出 / CRUD ==============================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_ai::types::{DetailPools, ProviderKind, SealedKey};

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
                api_key: None,
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
                api_key: None,
            },
        )
        .unwrap();
        assert!(cfg.model("bare", "").is_none());
    }

    #[test]
    fn builtin_has_seven_providers() {
        // openai 与 sensenova 来自 assets JSON（都是 OpenAI 图像协议，零适配器代码）；
        // dashscope/volcano/siliconflow/modelscope/comfyui 由 reconcile 顺序注入。
        let cfg = Config::builtin();
        let ids: Vec<&str> = cfg.providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "openai",
                "sensenova",
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
        // 火山：默认模型 + 能力抽查（单张、无 seed/n、有水印与 output_format）；目录条目数不锁，
        // 上架新模型时不该逼着改这里。
        let volcano = cfg.provider("volcano").unwrap();
        assert_eq!(volcano.default_model, "doubao-seedream-5-0-flash-260915");
        let flash = caps("volcano", "doubao-seedream-5-0-flash-260915");
        assert_eq!(flash.max_n, 1);
        assert!(!flash.supports_seed, "Seedream 全系列无 seed");
        assert!(!flash.supports_n, "Seedream 全系列无 n 参数");
        assert!(!flash.supports_quality);
        assert!(flash.supports_watermark);
        assert!(!flash.watermark_default);
        assert!(flash.supports_output_format, "Flash 可设 png/jpeg");
        // 4.0 只回 jpeg：不能发 output_format；旧出厂模型名不应出现在目录中。
        assert!(!caps("volcano", "doubao-seedream-4-0-20260415").supports_output_format);
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
                api_key: None,
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
                api_key: None,
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
                api_key: None,
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
                api_key: None,
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
        assert_eq!(sf.models.len(), 4, "Qwen-Image + Kolors + Z-Image ×2");
        let ms = cfg.provider("modelscope").expect("魔搭必须被补入");
        assert_eq!(ms.kind, ProviderKind::Modelscope);
        assert_eq!(ms.base_url, "https://api-inference.modelscope.cn/v1");
        assert_eq!(ms.default_model, "Qwen/Qwen-Image-2.1");
        // 再次 ensure 幂等：不再产生写盘变更。
        let before = fs::read_to_string(providers_path(root.path())).unwrap();
        Config::ensure_user_files(root.path()).unwrap();
        assert_eq!(
            before,
            fs::read_to_string(providers_path(root.path())).unwrap()
        );
    }

    /// 前端不持有密文，保存服务商时 api_key 恒为 None：守卫必须保留存量，
    /// 否则「改个名字」就把 Key 静默抹掉。显式传入密文（内部导入）时以传入为准。
    #[test]
    fn upsert_keeps_stored_key_when_incoming_has_none() {
        let mut cfg = Config::builtin();
        let sealed = SealedKey {
            nonce: "nonce-1".into(),
            data: "data-1".into(),
        };
        cfg.providers
            .iter_mut()
            .find(|p| p.id == "openai")
            .unwrap()
            .api_key = Some(sealed.clone());

        let incoming = ProviderConfig {
            id: "openai".into(),
            kind: ProviderKind::OpenAI,
            name: "改过名的 OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            default_model: "gpt-image-1".into(),
            models: vec![],
            api_key: None,
        };
        upsert_provider(&mut cfg, incoming).unwrap();
        let kept = cfg.provider("openai").unwrap();
        assert_eq!(kept.name, "改过名的 OpenAI");
        assert_eq!(kept.api_key, Some(sealed), "改名不得抹掉已存的 Key");

        let replaced = SealedKey {
            nonce: "nonce-2".into(),
            data: "data-2".into(),
        };
        let mut with_key = kept.clone();
        with_key.api_key = Some(replaced.clone());
        upsert_provider(&mut cfg, with_key).unwrap();
        assert_eq!(
            cfg.provider("openai").unwrap().api_key,
            Some(replaced),
            "显式传入的密文应覆盖存量"
        );
    }

    /// 回前端的视图必须抹掉密文；原对象保持不变（落盘仍要写出密文）。
    #[test]
    fn public_view_strips_sealed_keys() {
        let mut cfg = Config::builtin();
        cfg.providers[0].api_key = Some(SealedKey {
            nonce: "n".into(),
            data: "d".into(),
        });
        let view = cfg.public_view();
        assert!(
            view.providers.iter().all(|p| p.api_key.is_none()),
            "回前端不得带密文"
        );
        assert!(cfg.providers[0].api_key.is_some(), "原对象不受影响");
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
            api_key: None,
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
}
