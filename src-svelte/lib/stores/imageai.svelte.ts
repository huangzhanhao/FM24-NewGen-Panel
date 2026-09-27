/** AI 生图工作台状态（runes）。配置来自后端双来源文件，Key 不经前端持久化。 */
import { api } from '../services/api';
import { appStore } from './app.svelte';
import { loadImageBlobUrl } from '../services/images';
import { t } from '../text';
import type { GeneratedImage, ImageConfig, QueueEntry, RenderOutcome } from '../types';

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
  // 默认 350x350（验收反馈）：不在任何模型预设里时 UI 自动落「自定义…」，宽高输入显示 350/350。
  size: '350x350',
  n: 1,
  quality: 'auto',
  seed: '',
  generating: false,
  results: [] as GeneratedImage[],

  /** 会话内「已设置 Key」指示（id → 是否已设置）。Key 本体只进后端会话内存、后端不回传，
   *  这里仅记录设置过与否，供顶栏输入框占位与「已设置」徽标展示；不持久化，刷新即清。 */
  keysSet: {} as Record<string, boolean>,

  /** 生图数据源球员（P3）：名单/单人预览入口「送生图/用此球员生成」写入。
   *  null = 无数据源，变量走模板默认值（工作台手填 prompt 生效）；
   *  有数据源时单张/批量生成均改走 preview_prompt——按该球员的名单记录构建变量渲染模板正文。 */
  sourcePlayer: null as { uid: string; name: string } | null,

  // ---- 批量生成队列（P2）：queue 是后端快照的镜像，唯一写入方是 setQueue ----
  queue: [] as QueueEntry[],
  /** 是否有批量任务在跑：由 setQueue 按快照派生（存在 running 条目即在跑），
   *  queueStart 例外地乐观置位（首个 running 进度到达前先禁住启动按钮）。 */
  queueRunning: false,
  /** 名单表格的勾选（key=uid）。切组/换名单/翻页均不清空——勾选跟随球员，
   *  跨页勾选依然有效；是否清理由 UI 语义决定（目前没有自动清空的场景）。 */
  selectedInTable: {} as Record<string, boolean>,

  get provider() {
    return this.config?.providers.find((p) => p.id === this.providerId) ?? null;
  },
  get modelInfo() {
    return this.provider?.models.find((m) => m.model === this.model) ?? null;
  },
  /** 全局勾选数：直接统计 map 中为 true 的数量，不校验 uid 是否仍在当前名单/当前页
   *  （勾选跟随球员，见 selectedInTable 注释）。名单表格的「加入队列」按钮用的是
   *  「当前页勾选数」（组件内自算），这个 getter 供全局口径消费。 */
  get selectedCount() {
    return Object.values(this.selectedInTable).filter(Boolean).length;
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
  imageaiStore.size = imageaiStore.modelInfo?.capabilities.sizes[0] ?? '350x350';
  imageaiStore.n = 1;
  imageaiStore.seed = '';
  imageaiStore.quality = 'auto';
}

/** 写当前服务商的 API Key（顶栏直达与服务商设置弹窗共用的入口）：
 *  Key 仅进后端会话内存（set_provider_key），成功后标记本会话「已设置」。 */
export async function setProviderKey(id: string, key: string) {
  await api.setProviderKey(id, key);
  imageaiStore.keysSet[id] = true;
}

export function selectModel(model: string) {
  imageaiStore.model = model;
  const caps = imageaiStore.modelInfo?.capabilities;
  // 尺寸不随模型切换重置：原尺寸不在新模型预设里时保持现值（UI 自动落「自定义…」），
  // 用户可能是有意沿用自定义尺寸，不要悄悄改掉。
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

/** 设定/清除生图数据源球员（名单行「送生图」与单人预览「用此球员生成」共用入口；
 *  清除由工作台数据源条的按钮触发）。 */
export function setSourcePlayer(uid: string, name: string) {
  imageaiStore.sourcePlayer = { uid, name };
}

export function clearSourcePlayer() {
  imageaiStore.sourcePlayer = null;
}

export async function generate() {
  if (!imageaiStore.model) throw new Error(t.gen.needModel);
  imageaiStore.generating = true;
  try {
    // P3 数据源模式：改走 preview_prompt——按数据源球员的名单记录构建变量渲染「模板正文」，
    // 工作台手填 prompt 不参与（渲染基座是模板，换球员即可复用同一模板）。
    // 无数据源维持原链路：生成前再渲染一次——用户可能手工改写过文本，模板原文不得覆盖编辑结果；
    // 入参刻意用「当前文本」而非模板 ID，正是为了让手工编辑生效。
    const rendered = imageaiStore.sourcePlayer
      ? await api.previewPrompt(imageaiStore.sourcePlayer.uid, imageaiStore.templateId)
      : await api.renderPrompt(imageaiStore.prompt);
    if (rendered.missing.length > 0) throw new Error(t.gen.missingPlaceholders(rendered.missing));
    if (!imageaiStore.sourcePlayer) imageaiStore.prompt = rendered.text; // 回写：所见即所发（仅手填链路）
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

// ---------------- 结果卡「保存并替换」（P3） ----------------

/** 把生成图写为数据源球员的新头像：后端复制到 <头像包>/_generated/ 并复用单人换脸全链
 *  （批量替换进行中由后端拒绝并返回中文错误）。备份开关读「替换执行」面板共享的
 *  appStore.saveBackup（与单人微调同一口径）。sourcePlayer 为 null 时 UI 已禁用按钮，
 *  这里再拦一道；返回后端落盘的 GenImage（id/path/bytes）。 */
export async function saveAndReplace(imageId: string) {
  const sp = imageaiStore.sourcePlayer;
  if (!sp) throw new Error(t.gen.saveReplace.needSource);
  return api.saveAndReplace(imageId, sp.uid, appStore.saveBackup);
}

// ---------------- 批量生成队列（P2 Task5） ----------------

/** 提交加入队列的行数据（名单预览当前页勾选的球员）。 */
export interface PlayerRow { uid: string; name: string; }

/** 组装队列条目：其余参数取工作台当前值；id 留空由后端统一补发（见 image_ai_queue::QueueState::add），
 *  images/error 留空、status 复位为 queued（后端 add 也会再复位一次，双保险）。
 *  【渲染策略（P3）】有数据源：逐行 preview_prompt——每个球员按名单记录构建变量，
 *  prompt 各自携带其姓名/肤色等（队列逐球员渲染）；无数据源：各行入参完全相同，
 *  循环外 renderPrompt 一次复用（原实现每行调一次同参数 renderPrompt，是 N 次等价 IPC）。 */
export async function buildQueueEntries(rows: ReadonlyArray<PlayerRow>): Promise<QueueEntry[]> {
  if (!imageaiStore.model) throw new Error(t.gen.needModel);
  const entries: QueueEntry[] = [];
  let shared: RenderOutcome | null = null;
  for (const r of rows) {
    // 渲染失败/缺占位符即整批失败：与单张 generate 的口径一致（missing 不得原文送进 API）。
    const rendered = imageaiStore.sourcePlayer
      ? await api.previewPrompt(r.uid, imageaiStore.templateId)
      : (shared ??= await api.renderPrompt(imageaiStore.prompt));
    if (rendered.missing.length > 0) throw new Error(t.gen.missingPlaceholders(rendered.missing));
    entries.push({
      id: '',
      uid: r.uid,
      name: r.name,
      req: {
        providerId: imageaiStore.providerId,
        model: imageaiStore.model,
        prompt: rendered.text,
        negativePrompt: imageaiStore.negative,
        size: imageaiStore.size,
        n: imageaiStore.n,
        quality: imageaiStore.quality === 'auto' ? null : imageaiStore.quality,
        seed: imageaiStore.seed === '' ? null : Number(imageaiStore.seed),
        uid: r.uid, // P3 数据源生效；字段先落下（与单张请求同口径）
      },
      status: 'queued',
      images: [],
      error: '',
    });
  }
  return entries;
}

/** 快照唯一写入方：整份替换本地镜像，并派生 queueRunning（存在 running 条目即在跑）。
 *  挂载时 query 一次即能同步「组件卸载期间错过事件」的现状。 */
export function setQueue(entries: QueueEntry[]) {
  imageaiStore.queue = entries;
  imageaiStore.queueRunning = entries.some((e) => e.status === 'running');
}

/** 把当前页勾选的球员组装成队列条目并提交；返回本次提交条数（供成功提示）。
 *  后端返回的全量快照写回 store（条目 id 以后端补发为准）。 */
export async function enqueuePlayers(rows: ReadonlyArray<PlayerRow>): Promise<number> {
  const entries = await buildQueueEntries(rows);
  setQueue(await api.addToQueue(entries));
  return entries.length;
}

/** 启动批量生成。后端校验失败（缺 Key/重入）会抛错；成功则乐观置 running。 */
export async function queueStart() {
  await api.startQueue();
  imageaiStore.queueRunning = true;
}

export async function queueCancel() {
  await api.cancelQueue();
}

export function queueClear() {
  return api.clearQueue().then(setQueue);
}

export function queueRemove(ids: string[]) {
  return api.removeFromQueue(ids).then(setQueue);
}

export function queueQuery() {
  return api.queryQueue().then(setQueue);
}

// ---------------- 名单表格勾选（key=uid，切组/翻页不清空，见 selectedInTable 注释） ----------------

/** 勾选/取消单个球员；v 缺省为「取反」。 */
export function toggleSelect(uid: string, v?: boolean) {
  imageaiStore.selectedInTable[uid] = v ?? !imageaiStore.selectedInTable[uid];
}

/** 批量勾选/取消一组球员（表头全选作用于当前页数据行）。 */
export function toggleSelectPage(uids: string[], v: boolean) {
  for (const uid of uids) imageaiStore.selectedInTable[uid] = v;
}

/** 清空全部勾选。 */
export function clearSelection() {
  imageaiStore.selectedInTable = {};
}
