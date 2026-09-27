# AI 文生图模块 P2 Implementation Plan

> 子代理逐任务执行。P1 已建立模式（image_ai.rs 单文件模块、Config 双来源、commands.rs 统一命令层、前端 imageai 域）。P2 任务级契约如下；各任务按 TDD 实现，遵循 P1 计划 §2 的 Svelte 5 语法硬规则与命名规范。

**Goal：** 火山/百炼适配器 + 能力集驱动高级参数 + LRU 会话历史 + 名单勾选批量队列（并发 + 进度事件）。

**架构决策：**
- 适配器在 P2 成「族」：新建目录 `src-tauri/src/image_ai_providers/`（仿 templates/ 目录先例）。P1 的 OpenAI 协议纯函数与单测**整组迁入**，image_ai.rs 只保留契约/配置/渲染/台账/历史，生成时按 registry 分发。
- 队列按 replace_flow.rs 先例新建单文件 `src-tauri/src/image_ai_queue.rs`；队列命令仍在 commands.rs。
- 不引 tokio Semaphore；并发用 tauri::async_runtime + 计数。
- 前端仅新增 QueuePanel 组件，PlayersTable 就地加复选列（不重写表格）。

---

## 文件地图

### Rust

| 文件 | 动作 | 职责 |
|---|---|---|
| `src-tauri/assets/image_ai.json` | 修改 | 追加 volcano / dashscope 两家服务商及模型行（出厂资产） |
| `src-tauri/src/image_ai_providers/mod.rs` | 新建 | `ImageProvider` trait + `ProviderOutcome` + registry 分发（按 ProviderKind） |
| `src-tauri/src/image_ai_providers/openai.rs` | 新建 | OpenAI/custom 适配（P1 协议函数迁入 + 单测随迁） |
| `src-tauri/src/image_ai_providers/volcano.rs` | 新建 | 火山方舟 Seedream 同步适配 + 单测 |
| `src-tauri/src/image_ai_providers/dashscope.rs` | 新建 | 百炼通义万相异步（submit/poll/fetch）+ 单测 |
| `src-tauri/src/image_ai.rs` | 修改 | mod 声明；generate 改走 registry；LRU 历史函数 |
| `src-tauri/src/image_ai_queue.rs` | 新建 | QueueState/QueueEntry、并发 runner、进度事件 |
| `src-tauri/src/commands.rs` | 修改 | AppState 加 queue 字段；6 个队列命令 |
| `src-tauri/src/lib.rs` | 修改 | mod 声明；AppState 装配；invoke 登记 |

### 前端

| 文件 | 动作 | 职责 |
|---|---|---|
| `src-svelte/lib/types.ts` | 修改 | QueueEntry/QueueSnapshot 契约 |
| `src-svelte/lib/stores/imageai.svelte.ts` | 修改 | 队列状态、selectedPlayers 与队列动作 |
| `src-svelte/lib/components/imageai/QueuePanel.svelte` | 新建 | 队列列表/启动/取消/清空 + 进度展示 |
| `src-svelte/lib/components/imageai/ImageAiPanel.svelte` | 修改 | 加第三个子分段「队列」 |
| `src-svelte/lib/components/results/PlayersTable.svelte` | 修改 | 复选列（表头全选）+「加入队列」工具条 |
| `src-svelte/lib/services/api.ts` | 修改 | 6 个队列命令封装 |
| `src-svelte/lib/text.ts` | 修改 | t.gen.queue.* 文案 |

---

## 协议契约（2026-09 官方文档口径；字段逐字照抄厂商文档）

### 火山方舟 Seedream（同步 Bearer）
- Endpoint：`POST {base}/images/generations`，base 默认 `https://ark.cn-beijing.volces.com/api/v3`
- 请求体：`{model, prompt, size, response_format? , sequential_image_generation:"disabled", watermark?, seed?}`
  - `response_format` 按能力标志（gpt-image 式模型若不支持则不带；火山可带 `"b64_json"`）
  - size 例：`1024x1024`、`2048x2048`、`2304x1728`、`2560x1440`、`2048x3072`
- 200：`data[].b64_json` 或 `data[].url`；错误：`{error:{code,message}}`
- 模型行：seedream-5-0-flash 0.12、seedream-5-0-lite 0.22、seedream-4-0 0.20（元/张）
- 能力：maxN=1、supportsSeed=true、supportsQuality=false

### 百炼通义万相（异步任务）
- 提交：`POST https://dashscope.aliyuncs.com/api/v1/services/aigc/text2image/image-synthesis`
  - Header：`Authorization: Bearer <key>`、`Content-Type: application/json`、**`X-DashScope-Async: enable`**
  - Body：`{"model":"wanx2.1-t2i-turbo","input":{"prompt":"..."},"parameters":{"size":"1024*1024","n":1,"seed":1,"negative_prompt":"...","prompt_extend":true}}`
  - 提交响应取 `output.task_id`
- 轮询：`GET https://dashscope.aliyuncs.com/api/v1/tasks/{task_id}`（同 Authorization）
  - `output.task_status ∈ PENDING/RUNNING/SUCCEEDED/FAILED`
  - 成功：`output.results[].url`（**24h 有效，立即下载**）
  - 失败：`output.code` / `output.message`
  - 退避：3s 起步 ×1.5，总超时 180s
- 模型行：wanx2.1-t2i-turbo 0.14、wanx2.1-t2i-plus 0.20（元/张）
- 能力：maxN=4、supportsSeed、supportsNegativePrompt=true；适配器把 size 显示用的 `x` 转为 `*`

---

## Rust 接口契约

### image_ai_providers/mod.rs

```rust
pub(crate) mod dashscope;
pub(crate) mod openai;
pub(crate) mod volcano;

use crate::image_ai::GenRequest;
use std::path::Path;

/// 适配器产出的单张图片：字节 + 建议文件后缀（png）。
pub struct FetchedImage { pub bytes: Vec<u8>, pub extension: String }

/// 一次生成的适配器结果：图片 + 厂商真实用量（无则 None）。
pub struct ProviderOutcome {
    pub images: Vec<FetchedImage>,
    pub real_usage: Option<crate::image_ai::RealUsage>,
}

/// 统一适配器：一次 async 调用内含 submit→(poll)→fetch；同步厂商 poll 为立即完成。
#[allow(async_fn_in_trait)] // 单 crate 内使用，无需 dyn
pub trait ImageProvider {
    async fn generate(
        &self,
        client: &reqwest::Client,
        data_dir: &Path,
        key: &str,
        req: &GenRequest,
    ) -> Result<ProviderOutcome, String>;
}

/// 唯一分发：按 ProviderKind 取适配器。
pub fn for_kind(kind: crate::image_ai::ProviderKind) -> Box<dyn ImageProvider> {
    use crate::image_ai::ProviderKind::*;
    match kind {
        OpenAI | Custom => Box::new(openai::OpenAiProvider),
        Volcano => Box::new(volcano::VolcanoProvider),
        Dashscope => Box::new(dashscope::DashscopeProvider),
    }
}
```

注：`Box<dyn ImageProvider>` 要求对象安全：async fn 不适合 dyn。实现上 trait 用
`fn generate<'a>(&'a self, ...) -> BoxFuture<'a, Result<ProviderOutcome, String>>`
形式（返回 `Box::pin(async move { ... })`），保持对象安全；registry 才能返回 Box<dyn>。

### image_ai.rs 改造
- 顶部加 `pub(crate) mod image_ai_providers;`（若选择在 lib.rs 挂载，则在 lib.rs 加 `mod image_ai_providers;`——与项目模块挂载方式一致：统一在 lib.rs）。
- `generate(...)` 内把原 OpenAI 专有 HTTP 段替换为：
  ```rust
  let provider = image_ai_providers::for_kind(provider.kind);
  let client = reqwest::Client::builder()
      .connect_timeout(Duration::from_secs(15))
      .timeout(Duration::from_secs(180)).build()
      .map_err(|e| format!("HTTP 客户端初始化失败: {e}"))?;
  let outcome = provider.generate(&client, data_dir, key, &req).await?;
  ```
  随后把 outcome.images 统一写入 tmp_dir（P1 既有写文件循环：id/路径/GenImage），台账与估价逻辑不变。
- endpoint/build_body/parse_images/parse_real_usage/ImagePayload 删除（迁入 openai.rs），对应单测整组随迁。

### LRU 历史（image_ai.rs 内新增）

```rust
/// 生成落盘后裁剪历史：保留最新 settings.history_limit 张，超出部分删除文件。
/// 元数据按「写入时间倒序」维护；只裁剪 .imgtmp 内由本模块生成的文件。
pub fn enforce_history_limit(data_dir: &Path, limit: usize) -> Result<(), String>;
```
- 历史清单持久化 `data/.imgtmp/history.json`（Vec<HistoryEntry { id, ts }>，原子写）。
- generate 成功写盘后追加条目并调用裁剪；删除文件失败仅 warn 不阻断。
- P1「无上限」行为由消费 settings.history_limit 替代（默认 20）。

### image_ai_queue.rs

```rust
use crate::image_ai::{GenImage, GenRequest};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize)]
pub struct QueueEntry {
    pub id: String,                 // queue 条目 id
    pub uid: String,
    pub name: String,
    pub req: GenRequest,            // 含 prompt/model/size/n/seed
    pub status: QueueStatus,        // queued|running|success|failed
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<GenImage>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QueueStatus { Queued, Running, Success, Failed }

#[derive(Default)]
pub struct QueueState {
    pub entries: Vec<QueueEntry>,
    pub cancel: bool,
}

impl QueueState {
    pub fn add(&mut self, entries: Vec<QueueEntry>);
    pub fn remove(&mut self, ids: &[String]);
    pub fn clear(&mut self);
    pub fn snapshot(&self) -> Vec<QueueEntry>;

    /// 顺序消费队列，并发上限 concurrency：
    /// 用 Arc<Mutex<u8>> 占位计数 + async 循环获取（sleep 50ms 轮询空位），
    /// 每个任务调 image_ai::generate（内部已写盘+台账+LRU）。
    /// 事件：每任务状态变化与完成时 emit：
    ///   "gen:queue:progress"  {status:"running|success|failed", current, total, uid}
    ///   "gen:queue:finished"  {finished, failed, total}
    /// cancel=true 时未启动任务置 failed("cancelled")，在跑任务收尾。
    pub async fn run(
        &mut self,
        app: AppHandle,
        data_dir: std::path::PathBuf,
        cfg: crate::image_ai::Config,
        keys: HashMap<String, String>,
        concurrency: u32,
    );
}
```

### commands.rs 队列命令（6 个）

```rust
#[tauri::command]
pub async fn add_to_queue(state: State<'_, AppState>, entries: Vec<image_ai_queue::QueueEntry>)
    -> Result<Vec<image_ai_queue::QueueEntry>, String>;
#[tauri::command]
pub fn remove_from_queue(state: State<'_, AppState>, ids: Vec<String>)
    -> Result<Vec<image_ai_queue::QueueEntry>, String>;
#[tauri::command]
pub fn clear_queue(state: State<'_, AppState>)
    -> Result<Vec<image_ai_queue::QueueEntry>, String>;
#[tauri::command]
pub async fn start_queue(state: State<'_, AppState>) -> Result<(), String>;
#[tauri::command]
pub fn cancel_queue(state: State<'_, AppState>) -> Result<(), String>;
#[tauri::command]
pub fn query_queue(state: State<'_, AppState>)
    -> Result<Vec<image_ai_queue::QueueEntry>, String>;
```
- AppState 加：`pub queue: Arc<Mutex<image_ai_queue::QueueState>>`（lib.rs 装配 Default::default()）。
- start_queue：出锁前 clone Config（state.image）与 image_keys，await 不持锁；校验每条 req 的 Key 存在（缺失返回「请先输入该服务商 Key」）。

---

## 前端契约（types.ts 追加）

```ts
export type QueueStatus = 'queued' | 'running' | 'success' | 'failed';
export interface QueueEntry {
  id: string; uid: string; name: string;
  req: GenRequest;
  status: QueueStatus;
  images: GenImage[];
  error: string;
}
```
- store：`queue: QueueEntry[]`、queueRunning:boolean；`selectedInTable: Record<string, boolean>` 可放 store（跨 PlayersTable 与 QueuePanel）。
- 事件监听在 QueuePanel onMount：`listen('gen:queue:progress')` → query_queue 刷新；`listen('gen:queue:finished')` → query_queue + queueRunning=false。卸载取消监听。
- PlayersTable：第一列复选框（仅拓展名单有数据行可勾选；表头复选 = 全选当前页）；工具条按钮「加入队列（N）」→ add_to_queue（QueueEntry 由当前球员 + 当前工作台参数组装：prompt 在前端经 renderPrompt 渲染默认值版本；uid/name 取行数据）。
- seed 高级参数：P2 模型（volcano/dashscope）capabilities.supportsSeed=true，ParamsForm 自动出现 Seed 行（P1 框架已就绪，无需额外改动）。

---

## 任务划分

- **Task P2-1**：出厂资产 image_ai.json 追加两家（含模型行/能力/价格）；更新 P1 ensure 合并验证；单测：Config::builtin 含四家、关键能力断言。
- **Task P2-2**：image_ai_providers 族（mod/openai 迁移 + volcano + dashscope）；image_ai::generate 改 registry；协议单测随迁 + 火山/dashscope 请求体组装纯函数单测（用 mock server：wiremock 不引入——请求体纯函数与响应解析纯函数 TDD；HTTP 冒烟留给手工）。
- **Task P2-3**：LRU 历史（history.json + enforce_history_limit + 单测：limit=5 写 7 个，最早 2 文件与条目删除）。
- **Task P2-4**：image_ai_queue.rs + commands/AppState/lib 登记（队列纯逻辑单测：add/remove/clear、cancel 状态机；并发 run 用 mock 不触网——将 image_ai::generate 的调用点以同形 trait 注入测试替身）。
- **Task P2-5**：前端全部（api/store/types/QueuePanel/ImageAiPanel 子分段/PlayersTable 复选/文案）；vitest 覆盖 store 队列动作与勾选组装。
- **Task P2-6**：全量验证（cargo test / pnpm check / pnpm test）+ tauri dev 冒烟（启动无错；进程清理）。

## 验收标准

1. 四家各成功一张（火山/百炼手工用真实 Key；OpenAI 已在 P1）；能力不支持的参数 UI 不出现、不发送。
2. 异步任务：断网/审核失败有清晰错误、写失败台账、不计费；轮询退避正确。
3. LRU：limit=5 连生 7 次，最早 2 条文件与元信息均删除。
4. 队列：勾 6 人、并发 2，进度计数与事件正确；取消后在跑收尾、未跑置 cancelled；队列仅会话态（重启清空，但已生成图片与台账保留）。
5. cargo test 全绿、pnpm check 0/0、pnpm test 全绿、双主题与 860px 窄窗通过。

## 风险

- async trait 对象安全：用 BoxFuture 返回形式（已注明）。
- 测试不触网：协议纯函数 + 测试替身；不引入 wiremock/网络依赖。
- 队列持锁：await 前出锁（clone Config/keys），锁只保护 Vec。
- history.json 损坏：load 失败按空历史重建并 warn，不阻断生成。
