# AI 文生图 P3 Implementation Plan

> 承接 P1/P2（image_ai.rs 单文件核心 + image_ai_providers/ 适配器族 + image_ai_queue.rs 队列）。
> 触发点：用户验收反馈④「提示词转译后，肤色带入色卡号」+ 决策「直接接球员数据源」。

**Goal：** 球员数据源 → 提示词变量自动填充（肤色经码值字典转译为色卡号）→ 结果卡「保存并替换」闭环。

**架构决策：**
- 变量构建是**纯函数**，住 image_ai.rs（与 render 同层）；不新建模块。
- 「保存并替换」复用 `replace_single_face` 全链（互斥/快照/恢复 live/single_replacement/mapping_cache），不重写替换逻辑。
- 生成图落盘到 `<头像包>/_generated/`（用 json_store::copy_atomic），命名用 settings.naming_pattern（`{uid}_{index}`）。
- 前端 prompt 渲染链路升级：数据源模式下由前端调 `preview_prompt(uid)` 取渲染结果再组装 GenRequest/QueueEntry（P2 的 buildQueueEntries 已预留逐行渲染结构）。

---

## 协议契约

### 变量构建（image_ai.rs）

```rust
/// 从球员记录构建模板变量：缺失字段不入 map（走模板默认值）。
pub fn build_vars(rec: &crate::rtf_scanner::PlayerRecord, dict: &CodeDictionary) -> HashMap<String, String>;
```
| 变量 | 来源 | 说明 |
|---|---|---|
| uid / name | rec.uid / rec.name | 原文 |
| primaryNat / secNat | rec.primary_nat / rec.sec_nat | 三字码原文 |
| ethnicity | rec.ethnicity_code → dict.translate(Ethnicity, ..) | 如 "0" → "北欧/白人" |
| hairLength | rec.hair_length → dict.translate(HairLength, ..) | 基础名单可能为空 → 不入 map |
| hairColor | rec.hair_color | **原文不过字典**（色卡即原文） |
| skinCode | rec.detailed.skin_code → dict.translate(SkinCode, ..) | 如 "12" → "深红棕褐色（#b47454）"；**基础名单无 detailed → 不入 map** |
| gender/age/height/club/ca/pa | rec.detailed.* | 同上，detailed 缺失一律不入 map |

### 命令（commands.rs，2 个）

```rust
#[tauri::command]
pub fn preview_prompt(state, uid: String, template_id: String) -> Result<image_ai::RenderOutcome, String>;
// 从 rtf_cache 按 uid 找 PlayerRecord（找不到报「名单中没有该球员」）→ build_vars → render 模板 body。

#[tauri::command]
pub async fn save_and_replace(state, image_id: String, uid: String, save_backup: bool) -> Result<GenImage, String>;
// .imgtmp 按 image_id 找文件（找不到报错）→ 目标 <当前组目录>/_generated/{naming_pattern 展开}.png
//   （index = _generated 下同 uid 已有数量 + 1；目录不存在自动创建；copy_atomic 落盘）
// → 复用 replace_single_face(state, uid, 新路径, save_backup)（批量互斥/快照链路全部继承）。
```
- `check_integrity`：`_generated/` 已在组目录内，扫描器按既有「图片直铺/14/65」判定不受影响；若被列为额外目录仅提示不报错（验证后视结果决定是否需要改动）。

### 前端

| 文件 | 动作 | 职责 |
|---|---|---|
| `services/api.ts` | 修改 | previewPrompt / saveAndReplace 封装 |
| `stores/imageai.svelte.ts` | 修改 | `sourcePlayer: {uid, name} \| null`；generate/enqueue 前若有数据源则先 previewPrompt 取渲染 prompt |
| `components/imageai/ImageAiPanel.svelte` | 修改 | 顶栏加数据源条（AvatarThumb + uid/name + 清除按钮；无数据源时显示「未选择球员，变量走默认值」） |
| `components/results/PlayersTable.svelte` | 修改 | 行尾「送生图」按钮：设 sourcePlayer 并切到 AI 生图 Tab |
| `components/results/PlayerViewer.svelte` | 修改 | 「用此球员生成」按钮（同上） |
| `components/imageai/ResultsSection…`（结果卡所在） | 修改 | 结果卡加「保存并替换」按钮（confirm 确认框；saveBackup 默认读 appStore.saveBackup） |
| `text.ts` | 修改 | t.gen.source.* / t.gen.saveReplace.* 文案 |

---

## 任务划分

- **P3-1**：Rust `build_vars` + `preview_prompt` 命令 + 单测（拓展名单全变量、基础名单走默认值、肤色转译断言 "12" → 色卡号文本）。
- **P3-2**：`save_and_replace` 命令（naming/copy_atomic/复用 replace_single_face）+ 单测（TempDir：落盘命名、_generated 自建、uid 重复递增 index；互斥拒绝路径）。
- **P3-3**：前端全部（数据源条/两个入口/结果卡保存并替换/buildQueueEntries 逐行 preview_prompt）；vitest 覆盖 sourcePlayer 状态与组装。
- **P3-4**：全量验证（cargo test / pnpm test / pnpm check / tauri dev 冒烟 + 双主题 + 860px）+ 完整性检查回归（_generated 不引发误报）+ 文档回写。

## 验收标准

1. 拓展名单选中球员 → 工作台提示词 {{skinCode}} 自动替换为色卡号文本；基础名单不报错、走模板默认值。
2. 「保存并替换」后头像即时变化、快照同步；`_generated/` 命名 `{uid}_{index}.png` 递增不覆盖。
3. 队列逐球员渲染：不同球员的 prompt 各自携带其变量。
4. 批量替换进行中点「保存并替换」被拒绝并提示。
5. cargo test 全绿、pnpm check 0/0、pnpm test 全绿、双主题与 860px 通过。

## 风险

- replace_single_face 是命令函数（持 State）：save_and_replace 直接调用它需要 State 参数传递——同命令模块内 `replace_single_face(state, ...)` 可直接复用（同为 commands.rs 函数），注意它内部已有互斥与日志。
- 基础名单 detailed 为 None：build_vars 必须容忍，绝不可 panic。
- naming_pattern 用户可改：解析只支持 `{uid}`/`{index}` 两个占位符，其余字面量原样。
