# AI 文生图模块 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在「名单预览」同级新增「AI 生图」Tab，提供「提示词模板 → 手动编辑 → 调用文生图 API → 保存/重生成/批量队列 → 用量台账」完整链路。

**Architecture:** 新功能**融入既有文件**：Rust 侧按 rtf_scanner.rs / replace_flow.rs 先例新增**单文件** `src-tauri/src/image_ai.rs`（契约 + 装载 + 纯逻辑 + 生成编排 + 台账），命令统一加在既有 `commands.rs`，出厂默认放入既有 `assets/` 机制；前端仅在确有新 UI 时新建组件并复用 TablePager / 全局弹窗表格类 / icons / dialog 助手。复用 Tauri 自带 `tauri::async_runtime`，不引 tokio；HTTP 只用 reqwest(rustls)；Key 仅存会话内存不落盘；用量 JSONL 追加，零数据库依赖。

**Tech Stack:** Rust: reqwest 0.12 (json + rustls-tls)、base64 0.22，其余复用现有 serde/serde_json/chrono/rand/json_store/paging/paths。前端: Svelte 5 runes、纯 CSS scoped、零新 npm 依赖。

---

## 0. 阅读指引与分期

按 writing-plans 的 scope check，三期各自产出可工作的软件，**本文件对 P1 给出任务级完整代码；P2/P3 给出文件地图、协议契约与验收标准，在对应阶段启动时用 writing-plans 展开为同级详细计划**。

| 阶段 | 交付 | 状态 |
|---|---|---|
| P1 MVP | 模板（无数据源，默认值占位）+ 模板渲染（Rust `render_prompt`）+ 提示词编辑 + OpenAI/自定义兼容接口 + 单轮多张 + 临时目录 + 手动保存 + JSONL 台账 + Key 会话内存 | 本文详化 |
| P2 | 火山/百炼适配器（成族时按 templates/ 先例拆目录）+ 能力集驱动 UI + seed 等高级参数 + LRU 历史 + 名单勾选批量队列（并发+事件） | 蓝图 |
| P3 | 上游全打通：名单行/单人微调一键送生图、变量自动填充、保存并一键替换头像 | 蓝图 |

## 1. 需求决策记录（2026-09-27 澄清结论）

1. **用量台账存储**：JSONL 追加 `data/usage/usage.jsonl`，不引入 SQLite。
2. **API Key**：**完全不落盘**。服务商配置持久化到 `data/image_ai.json`，其中不含 Key；每次启动后用户输入，Key 仅存后端内存（HashMap），UI 掩码不回显；关闭进程即失效。
3. **码值转译**：内置转译 + 用户可编辑。种族码 0–10（权威语义）、头发长度码 0–5、**肤色码 0–19** 内置默认表导出 `data/prompts/code_translation.json`，查不到的码退回数字原值。肤色 0–19 色卡取自 `docs/FM NewGAN种族代码示例.html`——该文档明确 0–19 是 FM 内置**肤色**代码（不是发色码），并声明仅作色卡参照、不关联 NewGAN 分类。**`hairColor` 不设字典**：FM 发色码语义尚无权威来源，按退回数字原值处理，待有来源再补。
4. **与替换流程衔接**：提供「保存并替换该 UID」（P3，图片先复制进当前头像包，因 `replace_single_face` 要求图片在包内）。
5. **批量队列来源**：名单预览表格勾选 + 手动「加入队列」（P2）。
6. **预置服务商**：OpenAI、火山方舟即梦 Seedream、阿里百炼通义万相。
7. **会话历史**：默认 LRU 20 条（可调 5–50），图片在响应后**立即**下载到 `data/.imgtmp`；启动时清空（抗崩溃）。**分期**：P1 只实现「响应后立即下载 + 启动清空」，结果区不做条数上限；LRU 上限与「可调 5–50」的设置项在 **P2** 交付（`settings.historyLimit` 字段 P1 先随文件落盘、暂不消费）。
8. **命名规则**：默认 `{uid}_{index}`，保存对话框可改名，命名模板用户可配置（`{uid} {index} {timestamp} {name}`）。**分期**：`{uid}` 依赖名单数据源，**P3** 交付（`settings.namingPattern` 字段 P1 先随文件落盘、暂不消费）。
9. **并发**：`settings.concurrency` 只服务 P2 的批量队列，**P2** 交付（P1 先随文件落盘、暂不消费）。

## 2. 复用优先原则（用户明确要求，不做独立一套）

### Rust：一切融入既有结构

| 既有资产 | 复用方式 |
|---|---|
| `commands.rs`（唯一命令层 + AppState） | 新命令全部加在此文件；AppState 就地加 `image` / `image_keys` 两字段 |
| `assets.rs` + `src-tauri/assets/*.json` | 三份出厂默认 JSON 放 assets/，assets.rs 加 `include_str!` 常量；不在 Rust 代码里硬编码默认表 |
| `templates/mod.rs`（builtin/load/ensure_user_files 双来源范式） | `image_ai::Config` 照搬此结构；原子写复用 `json_store`，分页复用 `paging::PageResult`，路径处理复用 `paths` |
| rtf_scanner.rs / replace_flow.rs（功能模块单文件先例） | 新功能 = 单文件 `src-tauri/src/image_ai.rs`；只有 P2 多家适配器成「族」时才按 templates/ 先例拆 `image_ai_providers/` 目录 |
| `rtf_scanner.rs`（PlayerRecord）、`xml_parser.rs`（single_replacement）、`logger.rs` | P3 数据源与保存替换直接调用 |

### 前端：能复用的组件一个都不重写

| 既有资产 | 复用方式 |
|---|---|
| `services/api.ts`、`services/images.ts` | 就地扩展；结果图仍走 read_image → blob |
| `components/results/TablePager.svelte` | 用量台账分页直接使用（bind:page / totalPages） |
| `ui/dialog.ts`（showError）、`ui/icons.ts` | 所有错误弹窗走 showError；新增图标（image/download/refresh）补进 icons.ts；禁止 emoji 与散落内联图标 |
| 全局类 `.modal/.modal-backdrop`、`.btn(.primary/.danger/.big)`、`.seg-tabs/.seg3`、`.row`、`.empty`、`.chip`、`.switch`、`.input-wrap`、`.muted/.small`、`.spacer` | 对话框/表单/结果区全部复用，scoped CSS 只写布局，色值只用设计令牌。**唯一新增**：`button.btn.sm`（紧凑按钮规格，Task 7 写入 app.css）——原先两处对话框各自 scoped 定义，属重复，提到全局 |
| `components/shared/AvatarThumb.svelte`、`avatarBlobs.svelte.ts` | P3 数据源球员头像复用 |
| stores 一域一文件（app/log/ui） | 新增 `stores/imageai.svelte.ts` 与就近测试；resultsView 仍登记在 app store |

视觉约束：任何新增色值双主题补齐；动画 ≤200ms、位移 ≤8px。

### Svelte 5 语法硬规则（P1 实测踩坑，重建代码必须遵守）

1. **each 回退数组**：`{#each maybe ?? [] as item (item.id)}`。写成 `?? [] (item)` 会被编译器解析为「调用空数组 `[] (item)`」，报 not callable。
2. **组件内可变状态用 runes**：`let open = $state(false)`；普通 `let` 的重新赋值不触发更新（non_reactive_update）。
3. **弹窗遮罩关闭**：遮罩 `onclick={(e) => { if (e.target === e.currentTarget) open = false; }}`，内部弹窗不放 stopPropagation——后者会触发 a11y click/key 诊断。
4. **label 必须 for/id 关联控件**，否则 a11y_label_has_associated_control。
5. `*.svelte` 有通配模块声明：组件文件缺失不会报模块解析错误，故验收以 svelte-check 诊断为准。

### 命名规范（本条踩过坑，务必区分两类名字）

| 类别 | 规则 | 例 |
|---|---|---|
| **外部协议字段名**（请求/响应体、Header、事件名） | **逐字照抄厂商文档，永不改写**。不得因为「项目 Rust 用 snake_case / 前端用 camelCase」就去归一化或重排 | `sequential_image_generation`、`response_format`、`b64_json`、`negative_prompt`、`prompt_extend`、`X-DashScope-Async`、`task_status`。**曾把 `sequential_image_generation` 误写成 `sequential_image_aieration`，就是过度套用命名风格的后果** |
| **我们自己的标识符** | 跟随各自生态既有习惯：Rust 模块/函数/字段 `snake_case`（`image_ai.rs`、`supports_response_format`）；跨 IPC 的 serde 字段统一 `#[serde(rename_all = "camelCase")]`，与 `types.ts` 对齐（`supportsResponseFormat`）；Svelte 组件 `PascalCase.svelte`；store 文件名跟随 `app.svelte.ts` / `avatarBlobs.svelte.ts` 既有写法 | `image_ai.rs`、`ImageAiPanel.svelte`、`imageai.svelte.ts` |

## 3. P1 文件地图（最小集）

### Rust

| 文件 | 动作 | 职责 |
|---|---|---|
| `src-tauri/assets/image_ai.json` | 新建（数据文件） | 出厂服务商 + 设置（ImageAiFile 形状） |
| `src-tauri/assets/image_ai_templates.json` | 新建（数据文件） | 出厂提示词模板 |
| `src-tauri/assets/image_ai_code_translation.json` | 新建（数据文件） | 出厂码值转译表 |
| `src-tauri/src/assets.rs` | 修改 | 加三个 `include_str!` 常量 |
| `src-tauri/src/image_ai.rs` | 新建（唯一新模块） | 契约 + Config 装载/CRUD + 码值转译 + 模板渲染 + OpenAI 协议纯函数 + 生成编排 + 台账 + 单测 |
| `src-tauri/src/commands.rs` | 修改 | AppState 加两字段；追加 10 个命令 |
| `src-tauri/src/lib.rs` | 修改 | `mod image_ai`；setup 会话准备/ensure；命令登记 |
| `src-tauri/Cargo.toml` | 修改 | +reqwest、+base64 |

### 前端

| 文件 | 动作 | 职责 |
|---|---|---|
| `src-svelte/lib/types.ts` | 修改 | 追加契约接口 |
| `src-svelte/lib/services/api.ts` | 修改 | 追加 10 个命令封装 |
| `src-svelte/lib/text.ts` | 修改 | 追加 `t.gen.*` |
| `src-svelte/lib/stores/imageai.svelte.ts` | 新建 | 工作台状态与动作 |
| `src-svelte/lib/stores/imageai.store.test.ts` | 新建 | store 单测（就近） |
| `src-svelte/lib/ui/icons.ts` | 修改 | 补 image/download/refresh 三图标 |
| `src-svelte/app.css` | 修改 | 新增 `button.btn.sm` 紧凑按钮全局规格（原来在两个对话框里各写一遍） |
| `src-svelte/lib/components/imageai/ImageAiPanel.svelte` | 新建 | Tab 内容：工作台/台账子分段 + 工作台全部 UI（模板/提示词/参数/结果区） |
| `src-svelte/lib/components/imageai/UsageLedger.svelte` | 新建 | 台账筛选/表格，分页用 TablePager |
| `src-svelte/lib/components/imageai/ProviderDialog.svelte` | 新建 | 服务商 CRUD + 掩码 Key 输入 |
| `src-svelte/lib/components/imageai/TemplateDialog.svelte` | 新建 | 模板 CRUD |
| `src-svelte/lib/stores/app.svelte.ts` | 修改 | resultsView 联合加 `'generate'` |
| `src-svelte/lib/components/results/ResultsSection.svelte` | 修改 | 加第 4 个 Tab 并挂 ImageAiPanel |

---

## 4. P1 详细任务

### Task 1: 依赖与出厂默认数据

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/assets/image_ai.json`
- Create: `src-tauri/assets/image_ai_templates.json`
- Create: `src-tauri/assets/image_ai_code_translation.json`
- Modify: `src-tauri/src/assets.rs`

- [ ] **Step 1: 添加依赖**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 中 `log = "0.4"` 之后追加：

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
base64 = "0.22"
```

- [ ] **Step 2: 创建出厂服务商配置**

创建 `src-tauri/assets/image_ai.json`：

```json
{
  "providers": [
    {
      "id": "openai",
      "kind": "openai",
      "name": "OpenAI",
      "baseUrl": "https://api.openai.com/v1",
      "defaultModel": "gpt-image-1",
      "models": [
        {
          "model": "gpt-image-1",
          "label": "gpt-image-1",
          "currency": "USD",
          "prices": { "1024x1024": 0.042, "1024x1536": 0.055, "1536x1024": 0.055 },
          "capabilities": {
            "sizes": ["1024x1024", "1024x1536", "1536x1024"],
            "maxN": 10,
            "supportsQuality": true,
            "supportsSeed": false,
            "supportsSteps": false,
            "supportsNegativePrompt": false,
            "supportsResponseFormat": false,
            "qualities": ["low", "medium", "high", "auto"]
          }
        }
      ]
    },
    {
      "id": "custom",
      "kind": "custom",
      "name": "自定义 OpenAI 兼容服务",
      "baseUrl": "",
      "defaultModel": "",
      "models": []
    }
  ],
  "settings": {
    "defaultProviderId": "openai",
    "historyLimit": 20,
    "concurrency": 2,
    "namingPattern": "{uid}_{index}"
  }
}
```

单价为 2026-09 参考口径、用户可改；dall-e-3 已停用故不预置。

- [ ] **Step 3: 创建出厂模板**

创建 `src-tauri/assets/image_ai_templates.json`：

```json
[
  {
    "id": "default",
    "name": "随机人写实肖像（默认）",
    "body": "一名{{gender|年轻}}足球运动员的正面肖像照，{{ethnicity|白人}}外貌，年龄约{{age|18}}岁，{{hairLength|短发}}，{{skinCode|浅肤色}}，面部光影自然，浅色纯色背景，正面平视镜头，写实摄影风格，高细节",
    "negative": "卡通，动漫，畸形，模糊，低分辨率，多余肢体"
  }
]
```

- [ ] **Step 4: 创建出厂码值转译表**

创建 `src-tauri/assets/image_ai_code_translation.json`：

```json
{
  "ethnicity": {
    "0": "北欧/白人",
    "1": "地中海/西语裔",
    "2": "中东/北非",
    "3": "非洲/加勒比",
    "4": "南亚",
    "5": "东南亚",
    "6": "太平洋岛民",
    "7": "美洲原住民",
    "8": "澳洲原住民",
    "9": "黑白混血",
    "10": "东亚"
  },
  "hairLength": {
    "0": "光头",
    "1": "极短发",
    "2": "短发",
    "3": "中等长度",
    "4": "长发",
    "5": "极长发"
  },
  "skinCode": {
    "0": "极浅暖棕（#fcecdc）",
    "1": "浅暖棕（#f4d4bc）",
    "2": "浅金棕（#ecccac）",
    "3": "浅棕（#e4c4a4）",
    "4": "浅褐色（#dcbca4）",
    "5": "浅红棕（#dcac94）",
    "6": "金棕色（#dcb48c）",
    "7": "暖橙棕（#e4ac84）",
    "8": "古铜金色（#dca46c）",
    "9": "灰玫棕色（#c49c84）",
    "10": "金棕褐色（#c4946c）",
    "11": "棕褐色（#c49474）",
    "12": "红棕褐色（#b47454）",
    "13": "深棕褐色（#a4644c）",
    "14": "深红棕色（#945444）",
    "15": "深栗色（#844c34）",
    "16": "深焦棕色（#7c442c）",
    "17": "深红褐色（#743c34）",
    "18": "深棕黑色（#6c3c2c）",
    "19": "近黑褐色（#5c2c1c）"
  }
}
```

`skinCode` 的 0–19 色卡取自 `docs/FM NewGAN种族代码示例.html`——该文档明确 0–19 是 **FM 内置肤色代码**（20 项，每码一张色卡，0 最浅、19 最深），且声明「不把内置肤色代码关联到 NewGAN 分类，仅作色卡参照」；此处同样只作色卡参照，hex 为色块代表色。`hairColor` **不设字典**：FM 发色码语义尚无权威来源，按「查不到的码退回数字原值」处理（`PlayerRecord.hair_color` 实测为数字码，如 `"12"`）。因此**出厂默认模板刻意不使用 `{{hairColor}}`**——它在 P3 接入数据源后会把数字码（如 `12`）直接写进提示词；默认模板改用有字典的 `{{skinCode|浅肤色}}`。`hairColor` 仍保留在 `VARIABLE_HINTS` 中，供明确知道该码含义的用户自行追加。用户可在导出的 `data/prompts/code_translation.json` 中修改/补表。

- [ ] **Step 5: assets.rs 加常量**

在 `src-tauri/src/assets.rs` 中 `pub const STANDARD3_RAW` 一行之后加：

```rust
/// AI 文生图出厂服务商与设置（image_ai::Config::builtin 解析；首次启动导出 data/image_ai.json）。
pub const IMAGE_AI_RAW: &str = include_str!("../assets/image_ai.json");
/// AI 文生图出厂提示词模板（导出 data/prompts/templates.json）。
pub const IMAGE_AI_TEMPLATES_RAW: &str = include_str!("../assets/image_ai_templates.json");
/// AI 文生图出厂码值转译表（导出 data/prompts/code_translation.json）。
pub const IMAGE_AI_DICT_RAW: &str = include_str!("../assets/image_ai_code_translation.json");
```

- [ ] **Step 6: 验证编译**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: 通过（新常量尚无人引用，无 warning）

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/assets/image_ai.json src-tauri/assets/image_ai_templates.json src-tauri/assets/image_ai_code_translation.json src-tauri/src/assets.rs
git commit -m "feat(image_ai): add reqwest/base64 and builtin asset defaults"
```

### Task 2: image_ai.rs —— 契约与 Config 装载

**Files:**
- Create: `src-tauri/src/image_ai.rs`

- [ ] **Step 1: 创建文件（契约 + 路径 + Config + CRUD，测试在 Task 3 末尾统一追加）**

```rust
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
```

- [ ] **Step 2: 验证编译**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: 通过；仅出现若干 unused-import 警告（`base64` / `Duration` / `rand::Rng` / `PageResult`，它们在 Task 3 才被用到，补齐后消失）。

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/image_ai.rs
git commit -m "feat(image_ai): add contracts, config load/ensure and CRUD helpers"
```

### Task 3: image_ai.rs —— 转译、渲染、协议、生成、台账与全部单测

**Files:**
- Modify: `src-tauri/src/image_ai.rs`

- [ ] **Step 1: 在文件末尾追加转译与模板渲染**

```rust
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

// ===================== OpenAI Images 协议纯函数 =====================

pub fn endpoint(base_url: &str) -> String {
    format!("{}/images/generations", base_url.trim_end_matches('/'))
}

/// 组装 OpenAI Images 请求体。
/// `supports_response_format` 来自模型能力集（`ModelInfo.capabilities`）：
/// OpenAI 官方 gpt-image-1 系列**不支持** `response_format`（始终返回 base64，传了即 400），
/// 出厂该能力为 false；OpenAI 兼容的第三方服务多默认返回 URL，需显式声明 b64_json 免去一次下载。
pub fn build_body(req: &GenRequest, supports_response_format: bool) -> serde_json::Value {
    use serde_json::json;
    let mut v = json!({
        "model": req.model,
        "prompt": req.prompt,
        "n": req.n,
        "size": req.size,
    });
    let obj = v.as_object_mut().unwrap();
    if supports_response_format {
        obj.insert("response_format".into(), json!("b64_json"));
    }
    if let Some(q) = &req.quality {
        if q != "auto" {
            obj.insert("quality".into(), json!(q));
        }
    }
    // OpenAI 不支持 seed，故此处不插入；支持 seed 的适配器在 P2 各自追加。
    v
}

#[derive(Debug)] // parse_images 的 Result 需 Debug 才能 unwrap_err
pub enum ImagePayload {
    B64(String),
    Url(String),
}

pub fn parse_images(resp: &serde_json::Value) -> Result<Vec<ImagePayload>, String> {
    if let Some(err) = resp.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(format!("服务商返回错误：{msg}"));
    }
    let items = resp.get("data").and_then(|d| d.as_array()).ok_or("响应缺少 data 数组")?;
    let mut out = Vec::new();
    for item in items {
        if let Some(b64) = item.get("b64_json").and_then(|x| x.as_str()) {
            out.push(ImagePayload::B64(b64.to_string()));
        } else if let Some(url) = item.get("url").and_then(|x| x.as_str()) {
            out.push(ImagePayload::Url(url.to_string()));
        } else {
            return Err("响应中的图片既无 b64_json 也无 url".into());
        }
    }
    if out.is_empty() {
        return Err("data 数组为空".into());
    }
    Ok(out)
}

pub fn parse_real_usage(resp: &serde_json::Value) -> Option<RealUsage> {
    let u = resp.get("usage")?;
    Some(RealUsage {
        input_tokens: u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
    })
}

// ===================== 生成编排：HTTP → 字节 → 临时文件 → 估价 =====================

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
    req: GenRequest,
) -> Result<GenResult, String> {
    let (provider, model) = {
        let provider = cfg.provider(&req.provider_id).cloned().ok_or("请先选择服务商")?;
        let model = cfg.model(&req.provider_id, &req.model).cloned().ok_or("请先选择模型")?;
        (provider, model)
    };
    let record_id = new_id();
    let ts = now_ts();

    let outcome: Result<(Vec<GenImage>, f64, Option<RealUsage>), String> = async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))?;
        let url = endpoint(&provider.base_url);
        let resp = client
            .post(&url)
            .bearer_auth(key)
            .json(&build_body(&req, model.capabilities.supports_response_format))
            .send()
            .await
            .map_err(|e| format!("请求发送失败: {e}"))?;
        let status = resp.status();
        let json: serde_json::Value =
            resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
        let payloads = parse_images(&json).map_err(|e| format!("HTTP {status}：{e}"))?;
        let real_usage = parse_real_usage(&json);

        let tmp = tmp_dir(data_dir);
        fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let mut images = Vec::new();
        for payload in payloads {
            let bytes = match payload {
                ImagePayload::B64(b64) => base64::engine::general_purpose::STANDARD
                    .decode(b64.trim())
                    .map_err(|e| format!("图片 base64 解码失败: {e}"))?,
                ImagePayload::Url(u) => {
                    // URL 有时效：此刻立即下载，不懒加载。
                    client
                        .get(&u)
                        .send()
                        .await
                        .map_err(|e| format!("图片下载失败: {e}"))?
                        .error_for_status()
                        .map_err(|e| format!("图片下载失败: {e}"))?
                        .bytes()
                        .await
                        .map_err(|e| format!("图片读取失败: {e}"))?
                        .to_vec()
                }
            };
            let id = new_id();
            let path = tmp.join(format!("{id}.png"));
            fs::write(&path, &bytes).map_err(|e| format!("临时图片写入失败: {e}"))?;
            images.push(GenImage { id, path: path.to_string_lossy().to_string(), bytes: bytes.len() as u64 });
        }
        let unit = model.prices.get(&req.size).copied().unwrap_or(0.0);
        let cost = (unit * req.n as f64 * 1000.0).round() / 1000.0;
        Ok((images, cost, real_usage))
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
```

- [ ] **Step 2: 追加全部单元测试**

```rust
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
    fn build_body_quality_and_response_format_rules() {
        // 能力集允许时才带 response_format。
        let b = build_body(&gen_req(), true);
        assert_eq!(b["quality"], "high");
        assert_eq!(b["response_format"], "b64_json");
        // gpt-image-1 能力为 false：整条字段不得出现（否则官方 API 返回 400）。
        let b = build_body(&gen_req(), false);
        assert!(b.get("response_format").is_none());
        let mut r = gen_req();
        r.quality = Some("auto".into());
        assert!(build_body(&r, true).get("quality").is_none());
    }

    #[test]
    fn builtin_openai_disables_response_format() {
        // 出厂 gpt-image-1 必须关掉该能力，custom 保持开启。
        let cfg = Config::builtin();
        let openai = cfg.model("openai", "gpt-image-1").unwrap();
        assert!(!openai.capabilities.supports_response_format);
    }

    #[test]
    fn parse_images_accepts_b64_url_and_error() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"data":[{"b64_json":"QUJD"},{"url":"https://x/a.png"}]}"#,
        )
        .unwrap();
        assert_eq!(parse_images(&v).unwrap().len(), 2);
        let v: serde_json::Value = serde_json::from_str(r#"{"error":{"message":"bad key"}}"#).unwrap();
        assert_eq!(parse_images(&v).unwrap_err(), "服务商返回错误：bad key");
    }

    #[test]
    fn parse_real_usage_tokens() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"usage":{"input_tokens":120,"output_tokens":2899}}"#,
        )
        .unwrap();
        assert_eq!(parse_real_usage(&v).unwrap().output_tokens, 2899);
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
        // TempDir 必须先绑定：临时值在语句末即 Drop（Windows 随后 write 报 NotFound）。
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
}
```

- [ ] **Step 3: 运行全部 image_ai 测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml image_ai`
Expected: 16 PASS

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/image_ai.rs
git commit -m "feat(image_ai): add translator, renderer, protocol, generate and usage ledger"
```

### Task 4: 接入既有 commands.rs 与 lib.rs

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: commands.rs 顶部与 AppState 加字段**

在 `src-tauri/src/commands.rs` 顶部 use 区（`use crate::templates...` 一行之后）加：

```rust
use crate::image_ai::{self, Config};
use crate::json_store;                 // save_generated_image 用其 copy_atomic（既有代码亦用全限定写法）
use std::collections::HashMap;          // AppState.image_keys；commands.rs 原本没有这条 use
```

AppState 结构体中在 `pub data_dir: PathBuf,` 之前加（**类型必须是 `Arc<Mutex<..>>`**，与同类字段 `rtf_cache`/`snapshot_cache` 一致，且与 lib.rs 构造处的 `Arc::new(Mutex::new(..))` 匹配）：

```rust
    pub image: Arc<Mutex<Config>>,                            // 生图生效配置（image_ai.json + 默认双来源）
    pub image_keys: Arc<Mutex<HashMap<String, String>>>,      // 会话内 API Key（不落盘）
```

`Arc` / `Mutex` 已在 commands.rs 的 `use std::sync::{Arc, Mutex};` 中。**注意**：commands.rs 顶部原本并**没有** `use std::collections`（既有代码全部写全限定 `std::collections::HashMap`），因此上面那条 `use std::collections::HashMap;` 必须显式补齐，不能省略。

- [ ] **Step 2: 在 commands.rs 末尾追加命令**

```rust
// ===================== AI 文生图命令（实现统一在 image_ai；与既有命令同居本文件）=====================

#[tauri::command]
pub fn get_image_ai_state(state: State<'_, AppState>) -> Result<Config, String> {
    Ok(state.image.lock().unwrap().clone())
}

/// 渲染 `{{变量|默认值}}` 模板文本（P1 无数据源，vars 恒为空 → 全部取默认值）。
/// 入参刻意用**文本**而非模板 ID：选中模板时对模板 body 渲染一次，生成前对用户编辑后的文本再渲染一次，
/// 这样用户的手工编辑不会被模板原文覆盖。返回 `missing` 非空表示存在「既无默认值、又无数据源」的占位符。
#[tauri::command]
pub fn render_prompt(
    body: String,
    vars: Option<HashMap<String, String>>,
) -> Result<image_ai::RenderOutcome, String> {
    Ok(image_ai::render(&body, &vars.unwrap_or_default()))
}

#[tauri::command]
pub fn save_provider(
    state: State<'_, AppState>,
    provider: image_ai::ProviderConfig,
) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::upsert_provider(&mut cfg, provider)?;
    cfg.save_providers(&state.data_dir)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub fn delete_provider(state: State<'_, AppState>, id: String) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::remove_provider(&mut cfg, &id)?;
    cfg.save_providers(&state.data_dir)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub fn set_provider_key(state: State<'_, AppState>, id: String, key: String) -> Result<(), String> {
    let mut keys = state.image_keys.lock().unwrap();
    if key.trim().is_empty() {
        keys.remove(&id);
    } else {
        keys.insert(id, key);
    }
    Ok(())
}

#[tauri::command]
pub fn save_prompt_template(
    state: State<'_, AppState>,
    template: image_ai::PromptTemplate,
) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::upsert_template(&mut cfg, template)?;
    cfg.save_templates(&state.data_dir)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub fn delete_prompt_template(state: State<'_, AppState>, id: String) -> Result<Config, String> {
    let mut cfg = state.image.lock().unwrap();
    image_ai::remove_template(&mut cfg, &id)?;
    cfg.save_templates(&state.data_dir)?;
    Ok(cfg.clone())
}

#[tauri::command]
pub async fn generate_image(
    state: State<'_, AppState>,
    req: image_ai::GenRequest,
) -> Result<image_ai::GenResult, String> {
    // await 前把配置 clone 出锁，不跨 await 持锁。
    let (cfg, key) = {
        let cfg = state.image.lock().unwrap();
        image_ai::validate_request(&cfg, &req)?;
        let key = state
            .image_keys
            .lock()
            .unwrap()
            .get(&req.provider_id)
            .cloned()
            .ok_or("请先输入 API Key（仅本会话记忆，不会保存）")?;
        (cfg.clone(), key)
    };
    image_ai::generate(&state.data_dir, &cfg, &key, req).await
}

#[tauri::command]
pub fn save_generated_image(
    state: State<'_, AppState>,
    image_id: String,
    target_path: String,
) -> Result<String, String> {
    let src = image_ai::tmp_dir(&state.data_dir).join(format!("{image_id}.png"));
    if !src.is_file() {
        return Err("源图片已失效（临时目录已清理）".into());
    }
    json_store::copy_atomic(&src, Path::new(&target_path))?;
    Ok(target_path)
}

#[tauri::command]
pub fn query_usage(
    state: State<'_, AppState>,
    query: image_ai::UsageQuery,
) -> Result<PageResult<image_ai::UsageRecord>, String> {
    image_ai::query_usage(&image_ai::usage_path(&state.data_dir), &query)
}
```

- [ ] **Step 3: lib.rs 登记模块、装配与命令**

(a) 在 `src-tauri/src/lib.rs` 的 `mod facepack_scanner;` 之后加：

```rust
mod image_ai; // AI 文生图（契约/模板渲染/协议适配/生成编排/用量台账；命令在 commands.rs）
```

(b) setup 中在 `migrate_legacy_user_data(&data_dir);` 之后加：

```rust
            // AI 文生图：清理上次临时图片、导出/补齐配置文件。失败不阻断启动。
            if let Err(e) = image_ai::Config::prepare_session(&data_dir) {
                log::warn!(target: "image_ai", "临时目录清理失败: {e}");
            }
            if let Err(e) = image_ai::Config::ensure_user_files(&data_dir) {
                log::warn!(target: "image_ai", "生图配置初始化失败: {e}");
            }
```

(c) AppState 构建处在 `snapshot_cache: ...,` 一行之后加：

```rust
                image: Arc::new(Mutex::new(
                    image_ai::Config::load(&data_dir).unwrap_or_else(|e| {
                        log::warn!(target: "image_ai", "读取生图配置失败，使用出厂默认: {e}");
                        image_ai::Config::builtin()
                    }),
                )),
                image_keys: Arc::new(Mutex::new(HashMap::new())),
```

lib.rs 顶部若无 `use std::collections::HashMap;` 则在 `use std::sync::{Arc, Mutex};` 之前加。

(d) invoke_handler 中 `commands::query_logs` 后补逗号并登记：

```rust
            commands::get_image_ai_state,
            commands::render_prompt,
            commands::save_provider,
            commands::delete_provider,
            commands::set_provider_key,
            commands::save_prompt_template,
            commands::delete_prompt_template,
            commands::generate_image,
            commands::save_generated_image,
            commands::query_usage
```

- [ ] **Step 4: 验证编译**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: 通过无新增 warning

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(image_ai): wire commands, AppState and startup hooks"
```

### Task 5: 前端契约、API 封装与文案

**Files:**
- Modify: `src-svelte/lib/types.ts`
- Modify: `src-svelte/lib/services/api.ts`
- Modify: `src-svelte/lib/text.ts`

- [ ] **Step 1: types.ts 追加**

```ts
export type ProviderKindCode = 'openai' | 'volcano' | 'dashscope' | 'custom';

export interface Capabilities {
  sizes: string[]; maxN: number;
  supportsQuality: boolean; supportsSeed: boolean; supportsSteps: boolean;
  supportsNegativePrompt: boolean; supportsResponseFormat: boolean; qualities: string[];
}

export interface ModelInfo {
  model: string; label: string; currency: string;
  prices: Record<string, number>; capabilities: Capabilities;
}

export interface ProviderConfig {
  id: string; kind: ProviderKindCode; name: string;
  baseUrl: string; defaultModel: string; models: ModelInfo[];
}

/** 生图设置。historyLimit/concurrency/namingPattern 在 P1 只随文件读写、不参与逻辑（分别 P2/P2/P3 生效）。 */
export interface GenSettings {
  defaultProviderId: string; historyLimit: number; concurrency: number; namingPattern: string;
}

export interface PromptTemplate { id: string; name: string; body: string; negative: string; }

export interface ImageConfig {
  providers: ProviderConfig[]; settings: GenSettings;
  templates: PromptTemplate[];
  dictionary: Record<string, Record<string, string>>;
}

export interface GenRequest {
  providerId: string; model: string; prompt: string; negativePrompt: string;
  size: string; n: number; quality: string | null; seed: number | null;
  /** P2 生效（steps 依赖支持该参数的适配器）；字段先落下，P1 恒不传。 */
  steps?: number | null;
  /** P3 生效（uid 依赖名单数据源）；字段先落下，P1 恒不传。 */
  uid?: string | null;
}

/** render_prompt 的返回：渲染后文本 + 无值且无默认值的占位符键名。 */
export interface RenderOutcome { text: string; missing: string[]; }

export interface GenImage { id: string; path: string; bytes: number; }

export interface GenResult {
  images: GenImage[]; recordId: string; estimatedCost: number; currency: string;
}

export interface GeneratedImage { id: string; path: string; url: string; }

export interface RealUsage { inputTokens: number; outputTokens: number; }

export interface UsageRecord {
  id: string; ts: string; providerId: string; providerName: string; kind: string;
  model: string; size: string; n: number; seed: number | null; quality: string | null;
  prompt: string; negativePrompt: string; estimatedCost: number; currency: string;
  status: 'success' | 'failed'; error: string; realUsage: RealUsage | null;
}

export interface UsageQuery {
  offset: number; limit: number; providerId: string; model: string;
  status: string; date: string;
}
```

- [ ] **Step 2: api.ts 追加**

`api.ts` 的 import type 列表加入 `GenRequest, GenResult, GeneratedImage, ImageConfig, PromptTemplate, ProviderConfig, RenderOutcome, UsageQuery, UsageRecord`；对象末尾（queryLogs 后加逗号）追加：

```ts
  getImageAiState: () => invoke<ImageConfig>('get_image_ai_state'),
  saveProvider: (provider: ProviderConfig) => invoke<ImageConfig>('save_provider', { provider }),
  deleteProvider: (id: string) => invoke<ImageConfig>('delete_provider', { id }),
  setProviderKey: (id: string, key: string) => invoke<void>('set_provider_key', { id, key }),
  savePromptTemplate: (template: PromptTemplate) =>
    invoke<ImageConfig>('save_prompt_template', { template }),
  deletePromptTemplate: (id: string) => invoke<ImageConfig>('delete_prompt_template', { id }),
  /** P1 无数据源：vars 不传（Rust 侧 `Option<HashMap>` → null 即 None），占位符全部取默认值。 */
  renderPrompt: (body: string, vars?: Record<string, string>) =>
    invoke<RenderOutcome>('render_prompt', { body, vars: vars ?? null }),
  generateImage: (req: GenRequest) => invoke<GenResult>('generate_image', { req }),
  saveGeneratedImage: (imageId: string, targetPath: string) =>
    invoke<string>('save_generated_image', { imageId, targetPath }),
  queryUsage: (query: UsageQuery) => invoke<PageResult<UsageRecord>>('query_usage', { query }),
```

- [ ] **Step 3: text.ts 追加 t.gen**

在 t 对象内 `setup` 段之后加（**键以 Task 7/8 组件实际引用为准**，不声明无引用的死键；协议值如 `status` 的 `'success'/'failed'`、kind 的 `'openai'/'custom'` 不入此表）：

```ts
  gen: {
    // 结果区第 4 个 Tab
    tabGenerate: 'AI 生图',
    // 工作台顶部与面板
    provider: '服务商',
    model: '模型',
    providerSettings: '服务商设置',
    tabWorkbench: '生成工作台',
    tabUsage: '用量台账',
    template: '提示词模板',
    manageTemplate: '管理模板',
    variables: '可用变量（点击追加占位符）',
    prompt: '提示词（可手动编辑）',
    toggleNegative: '负向提示词',
    negative: '负向提示词（模型不支持时仅记录不发送）',
    size: '尺寸',
    n: '张数',
    quality: '质量',
    seed: 'Seed',
    seedPlaceholder: '留空=随机',
    generate: '生成图片',
    generating: '生成中…',
    empty: '填写提示词后点「生成图片」，结果会显示在这里；图片仅存于会话临时目录，手动保存才会落盘。',
    save: '保存',
    saved: '图片已保存',
    removeImageTitle: '移除',
    needModel: '请先选择模型',
    /** 生成前渲染发现「既无默认值、又无数据源」的占位符时抛出（P1 无数据源，出现即需用户手工替换）。 */
    missingPlaceholders: (keys: string[]) => `占位符未填充：${keys.join('、')}`,
    // 服务商设置
    keyLabel: 'API Key（仅本会话记忆，不落盘）',
    keySetHint: '已输入，如需更换请重新填写',
    baseUrl: '接口地址 Base URL',
    kind: '类型',
    providerIdLabel: 'ID',
    providerIdPlaceholder: '留空自动生成',
    providerIdFixedNote: 'ID 是服务商的唯一标识，创建后不可修改；如需换 ID 请新建服务商。',
    providerNameLabel: '名称',
    providerModelLabel: '默认模型',
    modelRowsNote: '模型明细（尺寸/价格/能力）请在 data/image_ai.json 中编辑。',
    addProvider: '新建服务商',
    providerSaved: '服务商已保存',
    deleteProviderConfirm: (name: string) => `删除服务商「${name}」？`,
    // 模板管理
    templateManageTitle: '提示词模板',
    templateNameLabel: '模板名称',
    templateBodyLabel: '模板正文（占位符写法 {{变量|默认值}}）',
    templateNegativeLabel: '负向提示词',
    addTemplate: '新建模板',
    deleteTemplateConfirm: (name: string) => `删除模板「${name}」？`,
    // 通用
    edit: '编辑',
    remove: '删除',
    back: '返回',
    done: '完成',
    // 用量台账
    filterAllProviders: '全部服务商',
    filterAllStatus: '全部状态',
    filterModelPlaceholder: '按模型筛选…',
    filterDatePlaceholder: '按日期筛选（YYYY-MM-DD）…',
    statusSuccess: '成功',
    statusFailed: '失败',
    colTs: '时间',
    colModel: '模型',
    colSize: '尺寸',
    colN: '张数',
    colSeed: 'Seed',
    colPrompt: '提示词 / 错误',
    colCost: '估算费用',
    colStatus: '状态',
    ledgerTotal: '本页估算合计',
    count: (total: number) => `共 ${total} 条`,
  },
```

- [ ] **Step 4: Commit**

```bash
git add src-svelte/lib/types.ts src-svelte/lib/services/api.ts src-svelte/lib/text.ts
git commit -m "feat(image_ai): add frontend contracts, api wrappers and texts"
```

### Task 6: 图标与工作台 store

**Files:**
- Modify: `src-svelte/lib/ui/icons.ts`
- Create: `src-svelte/lib/stores/imageai.svelte.ts`
- Create: `src-svelte/lib/stores/imageai.store.test.ts`

- [ ] **Step 1: icons.ts 补三个图标**

在 `icons` 对象中 `sun` 项之后加（注意给 sun 行补逗号）：

```ts
  image: {
    body: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-5-5L5 21" />',
  },
  download: {
    body: '<path d="M12 3v12m0 0 4-4m-4 4-4-4"/><path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" />',
  },
  refresh: { body: '<path d="M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6" />' },
```

- [ ] **Step 2: 创建 image store**

创建 `src-svelte/lib/stores/imageai.svelte.ts`：

```ts
/** AI 生图工作台状态（runes）。配置来自后端双来源文件，Key 不经前端持久化。 */
import { api } from '../services/api';
import { loadImageBlobUrl } from '../services/images';
import { t } from '../text';
import type { GeneratedImage, ImageConfig } from '../types';

/** 模板变量说明（P3 起由上游自动填充；P1 仅展示与追加占位符）。
    `skinCode` 对应 RTF 的 FM 内置肤色码（0–19，见 FM NewGAN种族代码示例.html）；
    `hairColor` 对应名单中的发色字段——两者是独立字段，勿混用。 */
export const VARIABLE_HINTS: ReadonlyArray<readonly [string, string]> = [
  ['name', '姓名'], ['ethnicity', '种族'], ['hairLength', '发长'], ['hairColor', '发色'],
  ['skinCode', '肤色'],
  ['gender', '性别'], ['age', '年龄'], ['primaryNat', '国籍'], ['secNat', '第二国籍'],
  ['height', '身高'], ['club', '俱乐部'], ['ca', 'CA'], ['pa', 'PA'], ['uid', 'UID'],
];

export const imageaiStore = $state({
  config: null as ImageConfig | null,
  providerId: 'openai',
  model: '',
  templateId: 'default',
  prompt: '',
  negative: '',
  size: '1024x1024',
  n: 1,
  quality: 'auto',
  seed: '',
  generating: false,
  results: [] as GeneratedImage[],

  get provider() {
    return this.config?.providers.find((p) => p.id === this.providerId) ?? null;
  },
  get modelInfo() {
    return this.provider?.models.find((m) => m.model === this.model) ?? null;
  },
});

export async function loadImageState() {
  imageaiStore.config = await api.getImageAiState();
  selectProvider(imageaiStore.config.settings.defaultProviderId || 'openai');
  const tpl =
    imageaiStore.config.templates.find((x) => x.id === imageaiStore.templateId) ??
    imageaiStore.config.templates[0];
  if (tpl) await applyTemplate(tpl.id);
}

export function selectProvider(id: string) {
  imageaiStore.providerId = id;
  imageaiStore.model = imageaiStore.provider?.defaultModel ?? '';
  imageaiStore.size = imageaiStore.modelInfo?.capabilities.sizes[0] ?? '1024x1024';
  imageaiStore.n = 1;
  imageaiStore.seed = '';
  imageaiStore.quality = 'auto';
}

export function selectModel(model: string) {
  imageaiStore.model = model;
  const caps = imageaiStore.modelInfo?.capabilities;
  if (caps && !caps.sizes.includes(imageaiStore.size)) imageaiStore.size = caps.sizes[0] ?? imageaiStore.size;
  if (caps && imageaiStore.n > caps.maxN) imageaiStore.n = caps.maxN;
}

/** 选模板：回填负向文案，并把正文经后端渲染一次——`{{变量|默认值}}` 展开成可读文本，
    避免占位符原文直接送进 API。渲染交给 Rust（与生成前渲染同一实现），失败向上抛给调用方弹窗。 */
export async function applyTemplate(id: string) {
  const tpl = imageaiStore.config?.templates.find((x) => x.id === id);
  if (!tpl) return;
  imageaiStore.templateId = id;
  imageaiStore.negative = tpl.negative;
  try {
    imageaiStore.prompt = (await api.renderPrompt(tpl.body)).text;
  } catch (e) {
    // 渲染失败：正文退回模板原文（至少与刚选中的模板一致，不留上一个模板的残文），错误仍向上抛。
    imageaiStore.prompt = tpl.body;
    throw e;
  }
}

export function appendVariable(key: string) {
  imageaiStore.prompt = `${imageaiStore.prompt}{{${key}}}`;
}

export async function generate() {
  if (!imageaiStore.model) throw new Error(t.gen.needModel);
  imageaiStore.generating = true;
  try {
    // 生成前再渲染一次：用户可能手工改写过文本，模板原文不得覆盖编辑结果。
    // 入参刻意用「当前文本」而非模板 ID，正是为了让手工编辑生效。
    const rendered = await api.renderPrompt(imageaiStore.prompt);
    if (rendered.missing.length > 0) throw new Error(t.gen.missingPlaceholders(rendered.missing));
    imageaiStore.prompt = rendered.text; // 回写：所见即所发
    const res = await api.generateImage({
      providerId: imageaiStore.providerId,
      model: imageaiStore.model,
      prompt: rendered.text,
      negativePrompt: imageaiStore.negative,
      size: imageaiStore.size,
      n: imageaiStore.n,
      quality: imageaiStore.quality === 'auto' ? null : imageaiStore.quality,
      seed: imageaiStore.seed === '' ? null : Number(imageaiStore.seed),
    });
    const imgs: GeneratedImage[] = [];
    for (const im of res.images) {
      imgs.push({ id: im.id, path: im.path, url: await loadImageBlobUrl(im.path) });
    }
    imageaiStore.results = [...imgs, ...imageaiStore.results];
  } finally {
    imageaiStore.generating = false;
  }
}

export function removeResult(id: string) {
  const hit = imageaiStore.results.find((r) => r.id === id);
  if (hit) URL.revokeObjectURL(hit.url);
  imageaiStore.results = imageaiStore.results.filter((r) => r.id !== id);
}
```

- [ ] **Step 3: 写 store 测试**

创建 `src-svelte/lib/stores/imageai.store.test.ts`：

```ts
import { beforeEach, describe, expect, it, vi } from 'vitest';

// store 经 api 调后端做模板渲染；单测里整模块替身化，避免触碰 Tauri invoke。
// 替身实现刻意保留「展开默认值」语义，好让断言覆盖 render_prompt 的接线效果。
vi.mock('../services/api', () => ({
  api: {
    renderPrompt: vi.fn(async (body: string) => ({
      text: body.replace(/\{\{(\w+)\|([^}]*)\}\}/g, '$2'),
      missing: [],
    })),
  },
}));

import { api } from '../services/api';
import { applyTemplate, imageaiStore, selectProvider } from './imageai.svelte';
import type { ImageConfig } from '../types';

const cfg: ImageConfig = {
  providers: [
    {
      id: 'p1', kind: 'custom', name: 'P1', baseUrl: 'http://x/v1', defaultModel: 'm2',
      models: [
        {
          model: 'm2', label: 'M2', currency: 'CNY', prices: {},
          capabilities: {
            sizes: ['768x768'], maxN: 4, supportsQuality: false, supportsSeed: true,
            supportsSteps: false, supportsNegativePrompt: true, supportsResponseFormat: true,
            qualities: [],
          },
        },
      ],
    },
  ],
  settings: { defaultProviderId: 'p1', historyLimit: 20, concurrency: 2, namingPattern: '{uid}_{index}' },
  templates: [],
  dictionary: {},
};

describe('image store', () => {
  beforeEach(() => {
    vi.mocked(api.renderPrompt).mockClear();
  });

  it('selectProvider resets model-dependent fields', () => {
    imageaiStore.config = cfg;
    imageaiStore.size = 'old';
    imageaiStore.n = 9;
    selectProvider('p1');
    expect(imageaiStore.model).toBe('m2');
    expect(imageaiStore.size).toBe('768x768');
    expect(imageaiStore.n).toBe(1);
  });

  it('applyTemplate renders placeholders and fills negative', async () => {
    imageaiStore.config = {
      ...cfg,
      templates: [{ id: 't1', name: 'T1', body: '年轻{{gender|男性}}球员', negative: '负向B' }],
    };
    await applyTemplate('t1');
    // 占位符原文不得留在框里——这是 render_prompt 接线的核心断言。
    expect(imageaiStore.prompt).toBe('年轻男性球员');
    expect(imageaiStore.negative).toBe('负向B');
    expect(api.renderPrompt).toHaveBeenCalledWith('年轻{{gender|男性}}球员');
  });

  it('applyTemplate falls back to raw body but still rejects when render fails', async () => {
    imageaiStore.config = { ...cfg, templates: [{ id: 't1', name: 'T1', body: '__FAIL__', negative: '' }] };
    imageaiStore.prompt = '上一个模板的残文';
    vi.mocked(api.renderPrompt).mockRejectedValueOnce(new Error('boom'));
    await expect(applyTemplate('t1')).rejects.toThrow('boom');
    expect(imageaiStore.prompt).toBe('__FAIL__'); // 退回原文，不留残文
  });
});
```

- [ ] **Step 4: 运行测试**

Run: `pnpm test`
Expected: 新增 3 PASS，无回归

- [ ] **Step 5: Commit**

```bash
git add src-svelte/lib/ui/icons.ts src-svelte/lib/stores/imageai.svelte.ts src-svelte/lib/stores/imageai.store.test.ts
git commit -m "feat(image_ai): add icons and workbench store"
```

### Task 7: ImageAiPanel 与 UsageLedger

**Files:**
- Modify: `src-svelte/app.css`
- Create: `src-svelte/lib/components/imageai/ImageAiPanel.svelte`
- Create: `src-svelte/lib/components/imageai/UsageLedger.svelte`

- [ ] **Step 1: app.css 补紧凑按钮规格**

`src-svelte/app.css` 中 `button.btn.big`（约 L434）之后加：

```css
button.btn.sm { padding: 3px 8px; font-size: 12px; }
```

- [ ] **Step 2: 创建 ImageAiPanel.svelte**

```svelte
<!-- AI 生图 Tab 内容：顶部「服务商/模型」条 + 子分段（工作台/台账）。
     工作台左栏：模板/提示词/参数/生成；右栏：结果区。复用全局类与 icons，不重写控件。 -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { message, save } from '@tauri-apps/plugin-dialog';
  import { api } from '../../services/api';
  import UsageLedger from './UsageLedger.svelte';
  import ProviderDialog from './ProviderDialog.svelte';
  import TemplateDialog from './TemplateDialog.svelte';
  import { t } from '../../text';
  import { showError } from '../../ui/dialog';
  import { icons } from '../../ui/icons';
  import {
    VARIABLE_HINTS, appendVariable, applyTemplate, generate, imageaiStore,
    loadImageState, removeResult, selectModel, selectProvider,
  } from '../../stores/imageai.svelte';

  let view = $state<'workbench' | 'usage'>('workbench');
  let providerOpen = $state(false);
  let templateOpen = $state(false);
  let showNeg = true;

  // 首次进入懒加载配置；失败统一走错误弹窗。
  onMount(() => {
    if (!imageaiStore.config) void loadImageState().catch(showError);
  });

  async function onGenerate() {
    try {
      await generate();
    } catch (e) {
      await showError(e);
    }
  }

  async function onSave(imgId: string) {
    const target = await save({
      defaultPath: `${imgId}.png`,
      filters: [{ name: 'PNG', extensions: ['png'] }],
    });
    if (!target) return;
    try {
      await api.saveGeneratedImage(imgId, target);
      await message(t.gen.saved, { title: t.dialog.info });
    } catch (e) {
      await showError(e);
    }
  }
</script>

<div class="panel">
  <div class="bar">
    <span class="bar-label">{t.gen.provider}</span>
    <select value={imageaiStore.providerId} onchange={(e) => selectProvider(e.currentTarget.value)}>
      {#each imageaiStore.config?.providers ?? [] as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
    <span class="bar-label">{t.gen.model}</span>
    <select value={imageaiStore.model} onchange={(e) => selectModel(e.currentTarget.value)}>
      {#each imageaiStore.provider?.models ?? [] as m (m.model)}
        <option value={m.model}>{m.label}</option>
      {/each}
    </select>
    <button class="btn" onclick={() => (providerOpen = true)}>{t.gen.providerSettings}</button>
    <span class="spacer"></span>
    <div class="seg-tabs inner">
      <button class:active={view === 'workbench'} onclick={() => (view = 'workbench')}>{t.gen.tabWorkbench}</button>
      <button class:active={view === 'usage'} onclick={() => (view = 'usage')}>{t.gen.tabUsage}</button>
    </div>
  </div>

  {#if view === 'workbench'}
    <div class="wb-grid">
      <div class="wb-left">
        <!-- 模板 -->
        <div class="block">
          <div class="block-head">
            <span>{t.gen.template}</span>
            <button class="btn sm" onclick={() => (templateOpen = true)}>{t.gen.manageTemplate}</button>
          </div>
          <select value={imageaiStore.templateId} onchange={(e) => void applyTemplate(e.currentTarget.value).catch(showError)}>
            {#each imageaiStore.config?.templates ?? [] as tpl (tpl.id)}
              <option value={tpl.id}>{tpl.name}</option>
            {/each}
          </select>
          <p class="hint">{t.gen.variables}</p>
          <div class="chips">
            {#each VARIABLE_HINTS as [key, label] (key)}
              <button class="chip" onclick={() => appendVariable(key)}>{label}</button>
            {/each}
          </div>
        </div>

        <!-- 提示词 -->
        <div class="block">
          <label class="field-label" for="ig-prompt">{t.gen.prompt}</label>
          <textarea id="ig-prompt" rows="7" bind:value={imageaiStore.prompt}></textarea>
          <button class="neg-toggle" onclick={() => (showNeg = !showNeg)}>
            <svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html (showNeg ? icons.chevronUp : icons.chevronDown).body}</svg>
            {t.gen.toggleNegative}
          </button>
          {#if showNeg}
            <textarea rows="2" placeholder={t.gen.negative} bind:value={imageaiStore.negative}></textarea>
          {/if}
        </div>

        <!-- 参数：能力集驱动 -->
        <div class="block">
          <div class="frow">
            <label for="ig-size">{t.gen.size}</label>
            <select id="ig-size" bind:value={imageaiStore.size}>
              {#each imageaiStore.modelInfo?.capabilities.sizes ?? [] as s}
                <option value={s}>{s}</option>
              {/each}
            </select>
          </div>
          <div class="frow">
            <label for="ig-n">{t.gen.n}</label>
            <input id="ig-n" type="number" min="1" max={imageaiStore.modelInfo?.capabilities.maxN ?? 1} bind:value={imageaiStore.n} />
          </div>
          {#if imageaiStore.modelInfo?.capabilities.supportsQuality}
            <div class="frow">
              <label for="ig-quality">{t.gen.quality}</label>
              <select id="ig-quality" bind:value={imageaiStore.quality}>
                {#each ['auto', ...imageaiStore.modelInfo?.capabilities.qualities.filter((q) => q !== 'auto')] as q}
                  <option value={q}>{q}</option>
                {/each}
              </select>
            </div>
          {/if}
          {#if imageaiStore.modelInfo?.capabilities.supportsSeed}
            <div class="frow">
              <label for="ig-seed">{t.gen.seed}</label>
              <input id="ig-seed" type="number" placeholder={t.gen.seedPlaceholder} bind:value={imageaiStore.seed} />
            </div>
          {/if}
        </div>

        <button class="btn primary" disabled={imageaiStore.generating} onclick={onGenerate}>
          {imageaiStore.generating ? t.gen.generating : t.gen.generate}
        </button>
      </div>

      <div class="wb-right">
        {#if imageaiStore.results.length === 0}
          <div class="empty">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">{@html icons.image.body}</svg>
            <p>{t.gen.empty}</p>
          </div>
        {:else}
          <div class="gallery">
            {#each imageaiStore.results as img (img.id)}
              <div class="img-card">
                <img src={img.url} alt="generated" />
                <div class="actions">
                  <button class="btn sm" onclick={() => onSave(img.id)}>
                    <svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.download.body}</svg>
                    {t.gen.save}
                  </button>
                  <button class="btn sm" onclick={onGenerate}>
                    <svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.refresh.body}</svg>
                  </button>
                  <button class="icon-btn" title={t.gen.removeImageTitle} onclick={() => removeResult(img.id)}>
                    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">{@html icons.x.body}</svg>
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {:else}
    <UsageLedger />
  {/if}
</div>

<ProviderDialog bind:open={providerOpen} />
<TemplateDialog bind:open={templateOpen} />

<style>
  /* 父级 .tab-pane.active 是 display:block（app.css L284），故这里不能用 flex:1 撑高，改 height:100%。 */
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 10px; }
  .bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .bar-label { color: var(--text-2); font-size: 12.5px; }
  /* 内嵌变体：.seg-tabs 全局只有 ::before 绿色标条 + 9px 下边距（没有 border），
     所以这里要显式去掉标条与下边距，并缩小字号以适配顶栏单行。 */
  .seg-tabs.inner { margin: 0; }
  .seg-tabs.inner::before { display: none; }
  .seg-tabs.inner button { font-size: 12.5px; padding-bottom: 2px; margin-right: 12px; }
  .wb-grid { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(300px, 360px) minmax(0, 1fr); gap: 14px; }
  .wb-left { display: flex; flex-direction: column; gap: 10px; overflow-y: auto; padding-right: 2px; }
  .wb-right { min-height: 0; display: flex; }
  /* .empty 是全局类（app.css L765），但没有独立 .grow；这里让它填满右栏。 */
  .wb-right .empty { flex: 1; }
  .block-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 6px; font-size: 12.5px; color: var(--text-2); }
  .hint { margin: 8px 0 6px; color: var(--text-3); font-size: 12px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .field-label { display: block; font-size: 12.5px; color: var(--text-2); margin-bottom: 6px; }
  textarea { width: 100%; resize: vertical; }
  .neg-toggle { display: inline-flex; align-items: center; gap: 5px; margin: 6px 0; background: none; border: none; color: var(--text-3); font-size: 12px; cursor: pointer; padding: 0; }
  .neg-toggle:hover { color: var(--accent); }
  .frow { margin: 6px 0; display: flex; align-items: center; gap: 8px; }
  .frow label { width: 48px; color: var(--text-2); font-size: 12.5px; text-align: right; }
  .frow input { width: 110px; }
  .wb-left .btn.primary { padding: 10px; font-size: 14px; }
  .gallery { flex: 1; display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 10px; align-content: start; overflow-y: auto; }
  .img-card { border: 1px solid var(--border); border-radius: var(--radius-md); overflow: hidden; background: var(--bg-surface); display: flex; flex-direction: column; }
  .img-card img { width: 100%; aspect-ratio: 1; object-fit: cover; display: block; }
  .actions { display: flex; gap: 6px; padding: 6px; align-items: center; }
  .actions .icon-btn { margin-left: auto; }
  .ic { width: 13px; height: 13px; }
</style>
```

- [ ] **Step 3: 创建 UsageLedger.svelte（分页复用 TablePager）**

```svelte
<!-- 用量台账：筛选 + 表格 + TablePager（与名单/快照同款分页器）+ 本页合计。
     切走即卸载，再次进入自动重载。 -->
<script lang="ts">
  import { api } from '../../services/api';
  import TablePager from '../results/TablePager.svelte';
  import { imageaiStore } from '../../stores/imageai.svelte';
  import { showError } from '../../ui/dialog';
  import { t } from '../../text';
  import type { UsageRecord } from '../../types';

  const PAGE_SIZE = 20;
  let page = $state(0);
  let total = $state(0);
  let items = $state<UsageRecord[]>([]);
  let providerId = $state('');
  let model = $state('');
  let status = $state('');
  let date = $state('');

  const totalPages = $derived(Math.max(1, Math.ceil(total / PAGE_SIZE)));
  const sumCost = $derived(items.reduce((a, r) => a + r.estimatedCost, 0));
  const currency = $derived(items.find((r) => r.currency)?.currency ?? '');

  async function reload() {
    try {
      const p = await api.queryUsage({
        offset: page * PAGE_SIZE, limit: PAGE_SIZE,
        providerId, model, status, date,
      });
      total = p.total;
      items = p.items;
    } catch (e) {
      await showError(e); // 统一错误弹窗，避免 $effect 里静默失败
    }
  }

  // 首次进入与翻页/筛选变化均重载。
  $effect(() => { void page; void providerId; void model; void status; void date; reload(); });
</script>

<div class="ledger">
  <div class="filters">
    <select bind:value={providerId} onchange={() => (page = 0)}>
      <option value="">{t.gen.filterAllProviders}</option>
      {#each imageaiStore.config?.providers ?? [] as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
    <input placeholder={t.gen.filterModelPlaceholder} bind:value={model} onchange={() => (page = 0)} />
    <select bind:value={status} onchange={() => (page = 0)}>
      <option value="">{t.gen.filterAllStatus}</option>
      <option value="success">{t.gen.statusSuccess}</option>
      <option value="failed">{t.gen.statusFailed}</option>
    </select>
    <input placeholder={t.gen.filterDatePlaceholder} bind:value={date} onchange={() => (page = 0)} />
  </div>

  <div class="table-scroll">
    <table class="grid">
      <thead>
        <tr>
          <th>{t.gen.colTs}</th><th>{t.gen.provider}</th><th>{t.gen.colModel}</th>
          <th>{t.gen.colSize}</th><th>{t.gen.colN}</th>
          <th>{t.gen.colSeed}</th><th>{t.gen.colPrompt}</th>
          <th>{t.gen.colCost}</th><th>{t.gen.colStatus}</th>
        </tr>
      </thead>
      <tbody>
        {#each items as r (r.id)}
          <tr>
            <td>{r.ts}</td><td>{r.providerName}</td><td>{r.model}</td>
            <td class="c">{r.size}</td><td class="c">{r.n}</td>
            <td class="c">{r.seed ?? ''}</td>
            <td class="prompt-cell" title={r.prompt}>{r.status === 'failed' ? r.error : r.prompt}</td>
            <td class="c">{r.estimatedCost} {r.currency}</td>
            <td class="c {r.status}">{r.status === 'success' ? t.gen.statusSuccess : t.gen.statusFailed}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>

  <div class="foot">
    <span class="muted small">{t.gen.ledgerTotal}：{sumCost.toFixed(3)} {currency} · {t.gen.count(total)}</span>
    <TablePager bind:page {totalPages} />
  </div>
</div>

<style>
  /* 父级 .panel 是 flex 列且高度确定，故用 flex:1（不是 height:100%——那会连 .bar 一起算，溢出面板）。 */
  .ledger { display: flex; flex-direction: column; flex: 1; min-height: 0; gap: 8px; }
  .filters { display: flex; gap: 8px; flex-wrap: wrap; }
  .filters input { width: 150px; }
  .table-scroll { flex: 1; min-height: 0; overflow: auto; }
  .c { text-align: center; }
  .prompt-cell { max-width: 280px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .success { color: var(--accent); }
  .failed { color: var(--danger); }
  .foot { display: flex; justify-content: space-between; align-items: center; gap: 10px; }
</style>
```

- [ ] **Step 4: 类型检查**

Run: `pnpm check`
Expected: 无错误（未使用变量警告需清零）

- [ ] **Step 5: Commit**

```bash
git add src-svelte/app.css src-svelte/lib/components/imageai
git commit -m "feat(image_ai): add panel, workbench and ledger with shared pager"
```

### Task 8: ProviderDialog 与 TemplateDialog

**Files:**
- Create: `src-svelte/lib/components/imageai/ProviderDialog.svelte`
- Create: `src-svelte/lib/components/imageai/TemplateDialog.svelte`

- [ ] **Step 1: ProviderDialog.svelte**

```svelte
<!-- 服务商设置：列表/新建/编辑/删除；Key 掩码输入仅发会话内存。复用全局 modal 类。 -->
<script lang="ts">
  import { confirm, message } from '@tauri-apps/plugin-dialog';
  import { api } from '../../services/api';
  import { t } from '../../text';
  import { imageaiStore } from '../../stores/imageai.svelte';
  import { showError } from '../../ui/dialog';
  import type { ProviderConfig } from '../../types';

  let { open = $bindable(false) }: { open?: boolean } = $props();
  let editing = $state<ProviderConfig | null>(null);
  let keyInput = $state('');

  function blank(): ProviderConfig {
    return { id: '', kind: 'custom', name: '', baseUrl: '', defaultModel: '', models: [] };
  }

  async function onSave() {
    if (!editing) return;
    try {
      if (!editing.id) editing.id = `p-${Date.now()}`;
      if (!editing.defaultModel && editing.models[0]) editing.defaultModel = editing.models[0].model;
      imageaiStore.config = await api.saveProvider($state.snapshot(editing));
      if (keyInput) await api.setProviderKey(editing.id, keyInput);
      await message(t.gen.providerSaved, { title: t.dialog.info });
      editing = null;
      keyInput = '';
    } catch (e) {
      await showError(e); // 保存/写 Key 失败必须弹窗，不能静默
    }
  }

  async function onDelete(p: ProviderConfig) {
    if (!(await confirm(t.gen.deleteProviderConfirm(p.name), { title: t.dialog.warning }))) return;
    try {
      imageaiStore.config = await api.deleteProvider(p.id);
    } catch (e) {
      await showError(e);
    }
  }
</script>

{#if open}
  <div class="modal-backdrop" onclick={(e) => { if (e.target === e.currentTarget) open = false; }} role="presentation">
    <div class="modal" role="dialog" aria-modal="true">
      <div class="modal-title">{t.gen.providerSettings}</div>
      <div class="modal-body">
        {#if !editing}
          <div class="p-list">
            {#each imageaiStore.config?.providers ?? [] as p (p.id)
              <div class="p-row">
                <span>{p.name} <em>{p.kind}</em></span>
                <span class="btns">
                  <button class="btn sm" onclick={() => (editing = $state.snapshot(p))}>{t.gen.edit}</button>
                  {#if p.kind !== 'openai' && p.kind !== 'custom'}
                    <button class="btn sm danger" onclick={() => onDelete(p)}>{t.gen.remove}</button>
                  {/if}
                </span>
              </div>
            {/each}
          </div>
        {:else}
          <div class="form">
            <!-- ID 是 upsert_provider 的匹配键：编辑态只读，否则改 ID 会「新增一条、旧条目仍在」（Key 也留在旧 id 上）。 -->
            <div class="f-row"><label for="pv-id">{t.gen.providerIdLabel}</label><input id="pv-id" readonly={editing.id !== ''} bind:value={editing.id} placeholder={t.gen.providerIdPlaceholder} /></div>
            <div class="f-row"><label for="pv-kind">{t.gen.kind}</label>
              <!-- 联合字面量类型：不用 bind:value（svelte-check 会报类型不符），改显式赋值 -->
              <select id="pv-kind" value={editing.kind} onchange={(e) => (editing!.kind = e.currentTarget.value as ProviderConfig['kind'])}>
                <option value="custom">custom</option>
                <option value="openai">openai</option>
                <option value="volcano" disabled>volcano（P2）</option>
                <option value="dashscope" disabled>dashscope（P2）</option>
              </select>
            </div>
            <div class="f-row"><label for="pv-name">{t.gen.providerNameLabel}</label><input id="pv-name" bind:value={editing.name} /></div>
            <div class="f-row"><label for="pv-url">{t.gen.baseUrl}</label><input id="pv-url" bind:value={editing.baseUrl} placeholder="https://example.com/v1" /></div>
            <div class="f-row"><label for="pv-model">{t.gen.providerModelLabel}</label><input id="pv-model" bind:value={editing.defaultModel} /></div>
            <div class="f-row"><label for="pv-key">{t.gen.keyLabel}</label><input id="pv-key" type="password" bind:value={keyInput} placeholder={t.gen.keySetHint} /></div>
            <p class="note">{t.gen.providerIdFixedNote}</p>
            <p class="note">{t.gen.modelRowsNote}</p>
          </div>
        {/if}
      </div>
      <div class="modal-foot">
        {#if editing}
          <button class="btn" onclick={() => (editing = null)}>{t.gen.back}</button>
          <button class="btn primary" onclick={onSave}>{t.gen.save}</button>
        {:else}
          <button class="btn" onclick={() => (editing = blank())}>{t.gen.addProvider}</button>
          <button class="btn primary" onclick={() => (open = false)}>{t.gen.done}</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* 只写布局：紧凑按钮尺寸是控件规格，已提到 app.css 的 button.btn.sm（§2 的复用原则） */
  .p-list { display: flex; flex-direction: column; gap: 8px; }
  .p-row { display: flex; justify-content: space-between; align-items: center; font-size: 13px; }
  .p-row em { color: var(--text-3); font-style: normal; font-size: 12px; margin-left: 6px; }
  .btns { display: flex; gap: 6px; }
  .form { display: flex; flex-direction: column; gap: 8px; }
  .f-row { display: flex; align-items: center; gap: 8px; }
  .f-row label { width: 150px; text-align: right; color: var(--text-2); font-size: 12.5px; flex-shrink: 0; }
  .f-row input, .f-row select { flex: 1; min-width: 0; }
  .note { color: var(--text-3); font-size: 12px; }
  .modal-foot { display: flex; justify-content: flex-end; gap: 8px; padding-top: 10px; }
</style>
```

- [ ] **Step 2: TemplateDialog.svelte**

```svelte
<!-- 提示词模板 CRUD，复用全局 modal 类。 -->
<script lang="ts">
  import { confirm } from '@tauri-apps/plugin-dialog';
  import { api } from '../../services/api';
  import { imageaiStore } from '../../stores/imageai.svelte';
  import { showError } from '../../ui/dialog';
  import { t } from '../../text';
  import type { PromptTemplate } from '../../types';

  let { open = $bindable(false) }: { open?: boolean } = $props();
  let editing = $state<PromptTemplate | null>(null);

  function blank(): PromptTemplate {
    return { id: '', name: '', body: '', negative: '' };
  }

  async function onSave() {
    if (!editing) return;
    try {
      if (!editing.id) editing.id = `t-${Date.now()}`;
      imageaiStore.config = await api.savePromptTemplate($state.snapshot(editing));
      editing = null;
    } catch (e) {
      await showError(e);
    }
  }

  // 形参不要叫 t：会遮蔽 text.ts 的 t 对象。
  async function onDelete(tpl: PromptTemplate) {
    if (!(await confirm(t.gen.deleteTemplateConfirm(tpl.name), { title: t.dialog.warning }))) return;
    try {
      imageaiStore.config = await api.deletePromptTemplate(tpl.id);
    } catch (e) {
      await showError(e);
    }
  }
</script>

{#if open}
  <div class="modal-backdrop" onclick={(e) => { if (e.target === e.currentTarget) open = false; }} role="presentation">
    <div class="modal" role="dialog" aria-modal="true">
      <div class="modal-title">{t.gen.templateManageTitle}</div>
      <div class="modal-body">
        {#if !editing}
          {#each imageaiStore.config?.templates ?? [] as tpl (tpl.id)}
            <div class="t-row">
              <span>{tpl.name}</span>
              <span class="btns">
                <button class="btn sm" onclick={() => (editing = $state.snapshot(tpl))}>{t.gen.edit}</button>
                {#if tpl.id !== 'default'}
                  <button class="btn sm danger" onclick={() => onDelete(tpl)}>{t.gen.remove}</button>
                {/if}
              </span>
            </div>
          {/each}
        {:else}
          <label for="tpl-name">{t.gen.templateNameLabel}</label>
          <input id="tpl-name" bind:value={editing.name} />
          <label for="tpl-body">{t.gen.templateBodyLabel}</label>
          <textarea id="tpl-body" rows="8" bind:value={editing.body}></textarea>
          <label for="tpl-neg">{t.gen.templateNegativeLabel}</label>
          <textarea id="tpl-neg" rows="2" bind:value={editing.negative}></textarea>
        {/if}
      </div>
      <div class="modal-foot">
        {#if editing}
          <button class="btn" onclick={() => (editing = null)}>{t.gen.back}</button>
          <button class="btn primary" onclick={onSave}>{t.gen.save}</button>
        {:else}
          <button class="btn" onclick={() => (editing = blank())}>{t.gen.addTemplate}</button>
          <button class="btn primary" onclick={() => (open = false)}>{t.gen.done}</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* 只写布局；紧凑按钮尺寸走 app.css 的 button.btn.sm */
  .t-row { display: flex; justify-content: space-between; align-items: center; padding: 4px 0; font-size: 13px; }
  .btns { display: flex; gap: 6px; }
  label { display: block; margin: 8px 0 4px; color: var(--text-2); font-size: 12.5px; }
  .modal-foot { display: flex; justify-content: flex-end; gap: 8px; padding-top: 10px; }
</style>
```

- [ ] **Step 3: 类型检查**

Run: `pnpm check`
Expected: 无错误

- [ ] **Step 4: Commit**

```bash
git add src-svelte/lib/components/imageai/ProviderDialog.svelte src-svelte/lib/components/imageai/TemplateDialog.svelte
git commit -m "feat(image_ai): add provider and template dialogs"
```

### Task 9: Tab 接入与端到端验证

**Files:**
- Modify: `src-svelte/lib/stores/app.svelte.ts`
- Modify: `src-svelte/lib/components/results/ResultsSection.svelte`

- [ ] **Step 1: resultsView 类型扩展**

`src-svelte/lib/stores/app.svelte.ts` 中：

```ts
resultsView: 'table' as 'table' | 'viewer' | 'snapshot' | 'generate',
```

- [ ] **Step 2: ResultsSection 挂第 4 个 Tab**

script 中加：

```svelte
  import ImageAiPanel from '../imageai/ImageAiPanel.svelte';
```

seg-tabs 中在 snapshot 按钮后加（文案走 `t.gen.tabGenerate`，与其余三个 Tab 的 `t.results.*` 一致）：

```svelte
    <button class:active={view === 'generate'} onclick={() => (appStore.resultsView = 'generate')}>{t.gen.tabGenerate}</button>
```

card-body 末尾加：

```svelte
    <div class="tab-pane" class:active={view === 'generate'}>
      <ImageAiPanel />
    </div>
```

- [ ] **Step 3: 全量检查与测试**

Run: `pnpm check && pnpm test`
Expected: 全部通过

- [ ] **Step 4: 手工端到端验证清单**

Run: `pnpm tauri dev`，逐项验证：

1. 结果区出现「AI 生图」第 4 个 Tab，默认仍停名单预览。
2. 工作台显示 OpenAI / gpt-image-1；未输入 Key 直接生成 → 中文报错弹窗。
3. 服务商设置 → Key 掩码输入保存；重开弹窗 Key 框空白（不回显）。
4. 选模板 → 提示词自动填充（无数据源走默认值）→ 手动改写 → 生成。
5. 结果卡出图；保存对话框保存的文件可打开且内容一致。
6. 重新生成产生新卡、原卡保留；移除按钮回收该图。
7. 台账：成功/失败各一条，字段齐全；筛选与 TablePager 翻页有效；重启后台账仍在。
8. 重启应用：`.imgtmp` 已清空、Key 需重输、配置与模板改动保留。
9. 深/浅主题切换无破损；窗口拖到 860px 宽不溢出。

- [ ] **Step 5: Commit**

```bash
git add src-svelte/lib/stores/app.svelte.ts src-svelte/lib/components/results/ResultsSection.svelte
git commit -m "feat(image_ai): integrate AI generation tab into results section"
```

---

## 5. P2 蓝图（P1 验收通过后用 writing-plans 展开）

### 5.1 新增/修改文件（仍遵守复用原则）

Rust：
- `src-tauri/assets/image_ai.json`：追加火山、百炼两行服务商/模型（出厂资产更新；用户文件经 `ensure_user_files` 自动补入缺失内置项，用户改名/单价保留）。新模型行的 `capabilities` 必须写全（含 P1 新增的 `supportsResponseFormat`），缺字段会走 serde 缺省值而非报错，容易悄悄发错参数。
- 适配器成「族」→ 按 templates/ 先例新建目录 `src-tauri/src/image_ai_providers/`：`mod.rs`（`ImageProvider` trait：submit/poll/fetch + registry 分发 + 共享 HTTP 工具）、`openai.rs`、`volcano.rs`、`dashscope.rs` 各一个文件。`image_ai.rs` 只保留契约/配置/渲染/台账，协议函数移入族内。
- **P1 协议单测随迁**：Task 3 里的 `build_body_*` / `builtin_openai_disables_response_format` / `parses_*` 等纯函数单测，拆族时整组搬到 `image_ai_providers/openai.rs` 的 `mod tests`——连同被测函数一起走，不留「测 A 文件里的函数已被搬走」的悬空测试。P1 阶段它们仍在 `image_ai.rs`。
- 批量队列按 replace_flow.rs 单文件先例新建 `src-tauri/src/image_ai_queue.rs`（Vec 状态 + 并发计数，不引 tokio Semaphore；命令仍在 commands.rs）。
- LRU 历史作为 `image_ai.rs` 内新增函数（同模块扩展，不另建文件）。

前端：
- 新增 `components/imageai/QueuePanel.svelte`；ImageAiPanel 子分段加「队列」。
- 修改既有 `components/results/PlayersTable.svelte`：加复选列与「加入队列」工具条（复用其既有工具条/分页模式，不重写表格）。

### 5.2 协议契约（2026-09 官方文档口径）

> 本节所有字段名均为**厂商文档逐字原文**（多为 snake_case），不得按本项目风格改写或归一化——见 §2「命名规范」。

**火山方舟 Seedream（同步 Bearer）**
- `POST {base}/images/generations`，base 默认 `https://ark.cn-beijing.volces.com/api/v3`
- Body：`{model, prompt, size, response_format, sequential_image_generation:"disabled", watermark, seed}`
- 200：`data[].b64_json|url`；错误 `{error:{code,message}}`
- 默认模型行：seedream-5-0-flash 0.12 元/张、5-0-lite 0.22、4-0 0.20；能力 maxN=1、supportsSeed、quality=false
- `response_format` 是否可传：**照 P1 的 `supportsResponseFormat` 能力标志填**（P1 已把该参数改为能力驱动，不再无条件发送）。落地前按各家当期文档核对一次再写进出厂 JSON——本计划不预设结论。

**百炼通义万相（异步任务）**
- 提交：`POST https://dashscope.aliyuncs.com/api/v1/services/aigc/text2image/image-synthesis`，Header `Authorization: Bearer` + **`X-DashScope-Async: enable`**
- Body：`{"model":"wanx2.1-t2i-turbo","input":{"prompt":"..."},"parameters":{"size":"1024*1024","n":1,"seed":1,"negative_prompt":"...","prompt_extend":true}}`
- 提交响应：`output.task_id`
- 轮询：`GET /api/v1/tasks/{task_id}`；`output.task_status ∈ PENDING/RUNNING/SUCCEEDED/FAILED`；成功取 `output.results[].url`（**24h 有效立即下载**）；失败取 `output.code/message`
- 退避：3s 起步 ×1.5、总超时 180s
- 默认行：wanx2.1-t2i-turbo 0.14、plus 0.20；能力 maxN=4、seed/negative 支持；适配器把 size 的 `x` 转 `*`

### 5.3 队列契约

`QueueEntry { id, uid, name, model, size, n, seed, prompt, negativePrompt, status: queued|running|success|failed, images, error }`
事件：`gen:queue:progress {status,current,total,uid}`、`gen:queue:finished {finished,failed,total}`
命令（加在 commands.rs）：add_to_queue、remove_from_queue、clear_queue（确认框）、start_queue、cancel_queue

### 5.4 验收标准

1. 三家各成功一张；不支持的参数 UI 不出现、不发送。
2. 异步任务断网/审核失败有清晰错误、写失败台账且不计费。
3. LRU：limit=5 连生 7 次，最早 2 条文件与元信息均删除。
4. 队列：勾 6 人并发 2，进度计数正确；取消后在跑的收尾、未跑的取消；重启队列清空。
5. 双主题通过。

## 6. P3 蓝图

### 6.1 文件变更

Rust：
- 在 `image_ai.rs` 内新增变量构建函数（PlayerRecord + CodeDictionary → HashMap），不另建文件。
- 新命令加在 commands.rs：`preview_prompt(uid, templateId)`——**不重复实现渲染**，内部就是「变量构建 + 复用 P1 已有的 `image_ai::render`」，与 P1 的 `render_prompt(body, vars)` 共用同一套占位符语义：P1 的 `render_prompt` 吃文本（用户编辑后的正文），P3 的 `preview_prompt` 多一步「按 UID 取数据源填 vars」，最后都落到 `render`。二者并存、不合并（前者服务于手工编辑流，后者服务于名单行跳转）。
- 新命令加在 commands.rs：`save_and_replace(imageId, uid, saveBackup)`：临时图原子复制到 `<当前头像包>/_generated/{uid}_{index}.png` → 复用 commands.rs 现有 `replace_single_face` 全部既有步骤（批量互斥/快照存在/快照恢复 live/`xml_parser::single_replacement`/mapping_cache 同步）。

前端（改既有文件，不新开组件）：
- PlayersTable 行操作加「送生图」；PlayerViewer 加「用此球员生成」。
- ImageAiPanel 左栏顶部加数据源条（复用 AvatarThumb 显示来源头像 + UID/姓名）；结果卡加「保存并替换」（确认框，saveBackup 默认读 appStore.saveBackup）。

### 6.2 变量契约

| 变量 | 来源 |
|---|---|
| uid / name | record 字段 |
| primaryNat / secNat | 三字码原文 |
| ethnicity | dictionary.translate(Ethnicity, ethnicityCode) |
| hairLength | dictionary 转译（头发长度码 0–5） |
| hairColor | record 原文，**不过字典**（FM 发色码无权威来源，见 §1.3） |
| gender / age / height / club / ca / pa | detailed 原文，无 detailed 则缺省 |
| skinCode | dictionary 转译（肤色码 0–19；原值 detailed.skinCode） |

缺失字段不入 map（走 `|默认值`）；未知码回退原值。

### 6.3 验收标准

1. 名单行「送生图」自动切 AI Tab、变量已填、提示词可改。
2. 基础名单无 detailed：gender/age 走默认值不报错。
3. 保存并替换：图入 `_generated/`、config.xml 该行更新、名单预览头像即时变化、快照同步；批量进行中被拒绝。
4. 完整性检查将 `_generated/` 列为额外目录并在文档说明。

## 7. 风险与对策核对表

| 风险 | 对策 | 落点 |
|---|---|---|
| API 协议差异 | trait 统一 submit→poll→fetch，同步=立即完成；每适配器一个文件 | P2 image_ai_providers/ |
| 高级参数差异 | capabilities 声明，UI 动态显隐，不支持只记录不传递 | Task 7 Params 区 |
| `response_format` 个别模型不支持（gpt-image-1 传了会 400） | 能力标志 `supportsResponseFormat` 驱动，出厂 JSON 置 false | Task 1/2/3 |
| 提示词里的 `{{变量\|默认值}}` 原文被直送 API | 选模板时与生成前各渲染一次；`missing` 非空即拦截报错 | Task 3/4/6/7 |
| 返回 URL 时效 | 响应阶段立即下载，统一落 .imgtmp | image_ai::generate |
| 费用估算偏差 | 单价用户可配 + 出厂参考值（标注时点）；有真实 token 优先 realUsage | Config/台账 |
| Key 泄露 | 不落盘、不回显、掩码、进程退出即失效 | commands/store |
| 临时目录残留 | 启动清理（抗崩溃），仅手动保存才落目标位置 | Config::prepare_session |

## 8. 自检记录

- **Spec 覆盖**：模板引擎（Task 3 `render` + Task 4 `render_prompt` + Task 6 接线）、提示词编辑（Task 7）、API 接入（Task 3/4，P2 三家）、参数含高级（Task 7 能力集，P2）、图片管理（Task 3 临时目录 + Task 7 结果区，P2 LRU）、手动保存（Task 4/7）、批量队列（P2）、台账全字段（UsageRecord + Task 3/7）、Key 策略（Task 4/6/8）。
- **复用优先**：无平行命令层、无平行默认数据硬编码、无平行分页器/图标/弹窗；仅单一新功能模块文件与 4 个确有新 UI 的组件；P2 拆目录有 templates/ 先例支撑。全局类只新增 `button.btn.sm` 一条（消掉两处对话框的重复定义），其余全部沿用既有类。
- **一致性**：Rust camelCase 与 TS 字段对齐；Config/GenRequest/GenResult/UsageRecord 命名跨任务统一；`Capabilities.supportsResponseFormat` 三处（Rust 结构 / 出厂 JSON / types.ts）齐全；码值表键统一 `skinCode`（与 `hairColor` 明确区分）；出厂默认模板用 `{{skinCode}}` 而非 `{{hairColor}}`（后者 P3 会写进数字码）。
- **命名**：外部协议字段名逐字照抄厂商文档（§5.2 与 §2「命名规范」双处声明），不被本项目 camelCase/snake_case 风格同化——`sequential_image_generation`/`negative_prompt`/`b64_json` 等一律原样；项目自己的标识符则遵循各自生态既有习惯。
- **约定遵守**：所有界面文案经 `t.gen.*`（无散落中文，协议值如 `'success'/'failed'` 留在组件内不入表）；色值只用设计令牌（`var(--danger)` 等）；图标只走 `ui/icons.ts`（含展开箭头用 chevron，不用 `▾/▸` 文本）；对话框 api 调用一律 `showError`；不回显 Key；服务商 ID 编辑态只读（防误改造成重复条目）。
- **P2/P3** 按 scope check 以蓝图给出，阶段启动时各自展开为同级详细计划。
