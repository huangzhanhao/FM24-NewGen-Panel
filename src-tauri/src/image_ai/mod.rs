//! AI 文生图：提示词模板 → 用户编辑确认 → 调用文生图 API → 会话临时图片 → 手动保存/保存并替换。
//!
//! 【与既有模块的复用关系，不另起一套】
//! - 命令统一写在 commands.rs，本模块只提供被命令调用的类型与函数；
//! - 出厂默认是 assets/ 两份 JSON（assets.rs include_str! 内嵌），用户可编辑副本在 data/；
//! - 原子写复用 json_store；
//! - 多家协议适配成「族」拆在 image_ai_providers/，本模块只保留契约/配置/渲染与生成编排，
//!   生成时按 ProviderKind 经 registry 分发。
//!
//! 【模块分层，自上而下单向依赖】
//! - [`types`]：L0 契约（IPC/落盘 serde 类型 + 错误分类），不依赖任何兄弟模块；
//! - [`config`]：Config 生命周期（双来源加载 + 内置服务商归并）与 CRUD、data/ 路径；
//! - [`keys`]：服务商 Key 的密文存储（指纹/主密钥/封装与解封）；
//! - [`gallery`]：画廊记录 + 两套独立淘汰闸门 + 查询（`GALLERY_IO` 串行化读改写）；
//! - [`prompt`]：模板渲染、词库回落与 [`build_vars`]；
//! - [`postprocess`]：落盘前的四角裁圆角；
//! - [`validate`]：两层准入校验（完整性 + 协议约束）；
//! - [`generate`]：一次生成的编排（校验 → 适配器 → 落盘 → 画廊记录）；
//! - [`gen_queue`]：批量生成队列（条目状态机 + 并发调度，独立子模块供命令层直接引用）。
//!
//! 【本文件职责】只有模块声明与**显式**重导出——零业务逻辑。显式（而非 glob）重导出是为了
//! 让 `crate::image_ai::X` 这一既有调用面在拆分后逐字不变：commands / lib / providers 一行不用改。

mod config;
mod gallery;
mod generate;
mod keys;
mod postprocess;
mod prompt;
// `types` 是 L0 契约层：适配器族（image_ai_providers）也直接引它，故对 crate 可见。
pub(crate) mod types;
mod validate;

pub mod gen_queue;

// 门面重导出面刻意宽于 crate 内部用量：它是 `crate::image_ai::X` 调用面的稳定契约（含仅由
// 单测/本模块消费的条目）。`mod image_ai` 私有，未使用会被 lint 判为 unused_imports，故放行。
#[allow(unused_imports)] // 见上：门面契约面宽于内部用量
pub use config::{
    providers_path, remove_provider, remove_template, templates_path, tmp_dir, upsert_provider,
    upsert_template, Config,
};

// ---- 画廊记录与双淘汰闸门 -------------------------------------------------------------------------------------------
#[allow(unused_imports)] // 见上：门面契约面宽于内部用量
pub use gallery::{
    enforce_gallery_limit, enforce_image_limit, gallery_path, load_gallery_items, now_ts,
    record_gallery, record_saved_image,
};

// ---- 生成编排 -------------------------------------------------------------------------------------------------------
pub use generate::{generate, new_id};
// 日志摘要：单张与队列提交时各打一行（人是谁 + 这条参数）
pub use types::{person_summary, request_summary};

// ---- Key 密文存储 ---------------------------------------------------------------------------------------------------
#[allow(unused_imports)] // open_key 由 load_stored_keys 内部与单测消费，仍属对外契约
pub use keys::{load_stored_keys, open_key, seal_key};

// ---- 模板渲染 -------------------------------------------------------------------------------------------------------
pub use prompt::{build_vars, render, RenderOutcome};

// ---- 准入校验 -------------------------------------------------------------------------------------------------------
pub use validate::validate_complete;

// ---- 数据契约（L0） -------------------------------------------------------------------------------------------------
#[allow(unused_imports)] // 见上：类型契约须完整导出，用量随上层变动
pub use types::{
    classify_provider_error, from_http_status, kind_from_business_code, CancelToken, Capabilities,
    DetailPools, GalleryItem, GalleryRecord, GenErrorKind, GenImage, GenRequest, GenResult,
    GenSettings, GenStage, ImageAiFile, ImageGenError, ModelInfo, PromptTemplate, ProviderConfig,
    ProviderKind, SealedKey, ShotPool, SizeRules, StaffPools,
};

// ---- crate 内部可见（非公开 API） -----------------------------------------------------------------------------------
// `validate_for_generate` 是 generate 与 gen_queue 的唯一准入入口，不对外暴露给命令层。
pub(crate) use validate::validate_for_generate;
// rtf_scanner 的性别归一化复用本模块的判据（保持单一来源）。
pub(crate) use prompt::is_female;

// ---- 测试夹具（providers/queue 单测共用同一份 GenRequest 基座） -----------------------------------------------------
#[cfg(test)]
pub(crate) use types::test_req_base;
