/** AI 生图工作台状态（runes）。配置来自后端双来源文件，Key 不经前端持久化。 */
import { listen } from '@tauri-apps/api/event';
import { api } from '../services/api';
import { appStore } from './app.svelte';
import { loadImageBlobUrl } from '../services/images';
import { t } from '../text';
import type {
  GeneratedImage,
  ImageConfig,
  PromptTemplate,
  ProviderConfig,
  QueueEntry,
  QueueFinished,
  QueueProgress,
  RenderOutcome,
  SizeRules,
} from '../types';

/** 模板变量说明（P3 起由上游自动填充；P1 仅展示与追加占位符）。
    `skinCode` 对应 RTF 的 FM 内置肤色码（0–19，见 FM NewGAN种族代码示例.html）；
    `hairColor` 对应名单中的发色字段——两者是独立字段，勿混用。 */
export const VARIABLE_HINTS: ReadonlyArray<readonly [string, string]> = [
  ['name', '姓名'], ['ethnicity', '种族'], ['hairLength', '发长'], ['hairColor', '发色'],
  ['skinCode', '肤色'],
  ['gender', '性别'], ['age', '年龄'], ['primaryNat', '国籍'], ['secNat', '第二国籍'],
  ['height', '身高'], ['club', '俱乐部'], ['ca', 'CA'], ['pa', 'PA'], ['uid', 'UID'],
];

/** 解析 `宽*高`/`宽x高` 尺寸。 */
export function parseSize(s: string): [number, number] | null {
  const m = /^(\d+)[x*](\d+)$/.exec(s.trim());
  return m ? [Number(m[1]), Number(m[2])] : null;
}

/** 自由尺寸前端校验（与 Rust validate_size 同口径）；返回 null = 通过，否则返回中文错误。
 *  仅在模型声明了 sizeRules 且当前尺寸不在预设枚举时调用。 */
export function validateSizeRules(size: string, rules: SizeRules | null | undefined): string | null {
  if (!rules) return null;
  const p = parseSize(size);
  if (!p) return t.gen.sizeError.format;
  const [w, h] = p;
  if (rules.minSide && (w < rules.minSide || h < rules.minSide))
    return t.gen.sizeError.minSide(rules.minSide);
  if (rules.maxSide && (w > rules.maxSide || h > rules.maxSide))
    return t.gen.sizeError.maxSide(rules.maxSide);
  const total = w * h;
  if (rules.totalMin && total < rules.totalMin)
    return t.gen.sizeError.totalMin(Math.round(Math.sqrt(rules.totalMin)));
  if (rules.totalMax && total > rules.totalMax)
    return t.gen.sizeError.totalMax(Math.round(Math.sqrt(rules.totalMax)));
  if (rules.ratioMin || rules.ratioMax) {
    const ratio = w / h;
    if ((rules.ratioMin && ratio + 1e-4 < rules.ratioMin)
      || (rules.ratioMax && ratio - 1e-4 > rules.ratioMax)) {
      return t.gen.sizeError.ratio(
        rules.ratioMin ? Math.round(rules.ratioMin * 100) / 100 : 0,
        rules.ratioMax ? Math.round(rules.ratioMax * 100) / 100 : 0,
      );
    }
  }
  return null;
}

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
  /** 百炼智能改写（默认随模型能力走：千问/万相 true、z-image false；不支持时发送 null）。 */
  promptExtend: true,
  /** 「AI生成」水印（官方默认 false；仅当前模型 supportsWatermark 时发送）。 */
  watermark: false,
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
  /** 最近一次队列进度事件载荷（驱动队列坞进度条即时更新）。
   *  订阅是进程级的（initQueueListeners 由 App 挂载时调用一次），切走 AI 台事件不丢。 */
  queueProgress: null as QueueProgress | null,
  /** 最近一次批量收尾摘要（队列坞收起/展开两态都展示）；下次 queueStart 时复位。 */
  queueFinished: null as QueueFinished | null,
  /** 最近一次入队时的作用域「档 · 组」（S9 队列坞标题的护栏展示）。
   *  后端 QueueEntry 不带作用域（会话内存态、无跨会话归属问题），这里只在前端记账；
   *  空闲切作用域不清队列（S10），混批入队时显示最后一次入队的作用域。 */
  enqueueScope: '',
  /** 名单表格的勾选（key=uid）。切组/换名单/翻页均不清空——勾选跟随球员，
   *  跨页勾选依然有效；是否清理由 UI 语义决定（目前没有自动清空的场景）。 */
  selectedInTable: {} as Record<string, boolean>,

  get provider() {
    return this.config?.providers.find((p) => p.id === this.providerId) ?? null;
  },
  /** 当前模型明细；无明细行但等于服务商默认模型时合成通用能力（与后端 Config::model
   *  的兜底同口径）：不限张数、高级参数一律不发。 */
  get modelInfo() {
    const found = this.provider?.models.find((m) => m.model === this.model);
    if (found) return found;
    if (this.model && this.model === this.provider?.defaultModel) {
      return {
        model: this.model,
        label: this.model,
        capabilities: {
          sizes: [],
          maxN: 0,
          supportsQuality: false,
          supportsSeed: false,
          supportsSteps: false,
          supportsNegativePrompt: false,
          supportsResponseFormat: false,
          qualities: [],
        },
      };
    }
    return null;
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
  // 百炼开关回到该模型的官方默认（千问/万相改写开、z-image 关；水印一律关），
  // 避免把上一家服务商/上一个模型的选择带过来。
  imageaiStore.promptExtend = imageaiStore.modelInfo?.capabilities.promptExtendDefault ?? true;
  imageaiStore.watermark = false;
}

/** 写当前服务商的 API Key（顶栏直达与服务商设置弹窗共用的入口）：
 *  Key 仅进后端会话内存（set_provider_key），成功后标记本会话「已设置」。 */
export async function setProviderKey(id: string, key: string) {
  await api.setProviderKey(id, key);
  imageaiStore.keysSet[id] = true;
}

// ---------------- 服务商 / 模板的写入出口（config 整包替换单点化） ----------------
/** 保存/删除服务商与模板：后端返回全量 ImageConfig，config 由 store 单点写回
 *  （原先 ProviderDialog / TemplateDialog 四处直写）。写回后各兜一层一致性：
 *  当前 providerId / templateId 若已不在新配置中（删的正是当前项、后端清理），就地回退——
 *  否则下拉悬空，生图会带着指向已删条目的脏参数发出去。 */
export async function saveProvider(provider: ProviderConfig) {
  imageaiStore.config = await api.saveProvider(provider);
  ensureProviderValid();
}

export async function deleteProvider(id: string) {
  imageaiStore.config = await api.deleteProvider(id);
  ensureProviderValid();
}

/** providerId 悬空的回退走 selectProvider：模型/尺寸/改写开关等派生值一并归位到默认服务商口径。 */
function ensureProviderValid() {
  const cfg = imageaiStore.config;
  if (!cfg || cfg.providers.some((p) => p.id === imageaiStore.providerId)) return;
  selectProvider(cfg.settings.defaultProviderId || 'openai');
}

export async function saveTemplate(template: PromptTemplate) {
  imageaiStore.config = await api.savePromptTemplate(template);
  await ensureTemplateValid();
}

export async function deleteTemplate(id: string) {
  imageaiStore.config = await api.deletePromptTemplate(id);
  await ensureTemplateValid();
}

/** templateId 悬空的回退走 applyTemplate：negative/prompt 一并归位到第一个模板
 *  （渲染失败向上抛给调用方弹窗；此时 templateId/negative 已切换、prompt 已退回模板原文，状态一致）。 */
async function ensureTemplateValid() {
  const cfg = imageaiStore.config;
  if (!cfg || cfg.templates.some((t) => t.id === imageaiStore.templateId)) return;
  const fallback = cfg.templates[0];
  if (fallback) await applyTemplate(fallback.id);
}

export function selectModel(model: string) {
  imageaiStore.model = model;
  const caps = imageaiStore.modelInfo?.capabilities;
  // 尺寸不随模型切换重置：原尺寸不在新模型预设里时保持现值（UI 自动落「自定义…」），
  // 用户可能是有意沿用自定义尺寸，不要悄悄改掉。
  if (caps && imageaiStore.n > caps.maxN) imageaiStore.n = caps.maxN;
  // 智能改写默认值逐模型不同（z-image 默认关，开启即思考模式、单价翻倍）：切模型归位。
  imageaiStore.promptExtend = caps?.promptExtendDefault ?? true;
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

/** meta（同参重生）：结果卡传入该图记录的 {size, seed} 按原参重出；不传（CTA/重试）则用工作台当前值。
 *  meta.seed 为空串 = 原图就是随机种子，重生时同样随机。 */
export async function generate(meta?: { size?: string; seed?: string }) {
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
    const caps = imageaiStore.modelInfo?.capabilities;
    // 同参重生：meta 提供该图记录的尺寸/Seed（'' = 随机，与原图一致）；不传则用工作台当前值。
    const size = meta?.size || imageaiStore.size;
    const seedRaw = meta ? meta.seed ?? '' : imageaiStore.seed;
    const res = await api.generateImage({
      providerId: imageaiStore.providerId,
      model: imageaiStore.model,
      prompt: rendered.text,
      negativePrompt: imageaiStore.negative,
      size,
      n: imageaiStore.n,
      quality: imageaiStore.quality === 'auto' ? null : imageaiStore.quality,
      seed: seedRaw === '' ? null : Number(seedRaw),
      // 百炼开关：仅当前模型声明支持时才带值，否则传 null（如 wan2.7 无改写、z-image 无水印），
      // 后端按同一能力集再门控一遍。
      promptExtend: caps?.supportsPromptExtend ? imageaiStore.promptExtend : null,
      watermark: caps?.supportsWatermark ? imageaiStore.watermark : null,
    });
    // 本次实发参数快照随结果记录（结果卡展示 + 同参重生的入参来源；失败不产生结果）。
    const used = { size, seed: seedRaw };
    const imgs: GeneratedImage[] = [];
    for (const im of res.images) {
      imgs.push({
        id: im.id,
        path: im.path,
        url: await loadImageBlobUrl(im.path),
        ...used,
      });
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

/** 清空本次会话全部生成结果（blob URL 逐个回收，避免临时引用泄漏）。 */
export function clearResults() {
  for (const r of imageaiStore.results) URL.revokeObjectURL(r.url);
  imageaiStore.results = [];
}

// ---------------- 结果卡「保存并替换」（P3） ----------------

/** 把生成图写为数据源球员的新头像：后端复制到 <头像包>/_generated/ 并复用单人换脸全链
 *  （批量替换进行中由后端拒绝并返回中文错误）。备份开关读「替换执行」面板共享的
 *  appStore.saveBackup（与单人替换同一口径）。sourcePlayer 为 null 时 UI 已禁用按钮，
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
  const caps = imageaiStore.modelInfo?.capabilities;
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
        promptExtend: caps?.supportsPromptExtend ? imageaiStore.promptExtend : null,
        watermark: caps?.supportsWatermark ? imageaiStore.watermark : null,
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
 *  后端返回的全量快照写回 store（条目 id 以后端补发为准）；
 *  同时记下入队时作用域（队列坞标题的护栏展示，见 enqueueScope 注释）。 */
export async function enqueuePlayers(rows: ReadonlyArray<PlayerRow>): Promise<number> {
  const entries = await buildQueueEntries(rows);
  imageaiStore.enqueueScope = currentScopeLabel();
  setQueue(await api.addToQueue(entries));
  return entries.length;
}

/** 当前作用域的「档 · 组」展示串（组名取末段）；两段皆空时返回空串（坞标题隐藏）。 */
function currentScopeLabel(): string {
  const s = appStore.snapshot;
  if (!s) return '';
  const base = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? '';
  const g = s.currentGroup ? base(s.currentGroup) : '';
  return [s.currentProfile, g].filter(Boolean).join(' · ');
}

/** 启动批量生成。后端校验失败（缺 Key/重入）会抛错；成功则乐观置 running。 */
export async function queueStart() {
  imageaiStore.queueFinished = null; // 新一批开始：收起上一次收尾摘要（原由队列坞 onStart 复位）
  await api.startQueue();
  imageaiStore.queueRunning = true;
}

export async function queueCancel() {
  await api.cancelQueue();
}

export function queueClear() {
  imageaiStore.enqueueScope = ''; // 队列清空 → 入队作用域标注一并失效
  return api.clearQueue().then(setQueue);
}

export function queueRemove(ids: string[]) {
  return api.removeFromQueue(ids).then(setQueue);
}

/** 单条重试：先移除失败/成功旧条目，再用同一请求参数入一条新任务（id 清空由后端补发）。
 *  仅在队列未运行时允许（UI 已按 queueRunning 禁用启动/移除；这里不重复加锁）。 */
export async function queueRetry(entry: QueueEntry) {
  await api.removeFromQueue([entry.id]);
  const fresh: QueueEntry = {
    ...entry,
    id: '',
    status: 'queued',
    images: [],
    error: '',
  };
  setQueue(await api.addToQueue([fresh]));
}

export function queueQuery() {
  return api.queryQueue().then(setQueue);
}

// ---------------- 队列事件订阅（进程级） ----------------
// 订阅曾经挂在 QueueDock 的 onMount 上：队列坞随 AI 台卸载，切到别的工作台期间
// running/finished 事件全部错过——快照停止刷新、收尾摘要丢失，queueRunning 也会
// 卡在 true（侧栏切作用域护栏因此误报「队列在跑」）。现与 app.svelte.ts 的
// initListeners 同口径：App 挂载调用一次，内部 inited 防重复，存活到进程结束。

let queueListenersInited = false;
/** 后台刷新/订阅出错的出口：store 不碰 UI，弹窗由 App 注入（与 initListeners 的回调同思路）。 */
let queueErrorHandler: (e: unknown) => void = () => {};

/** 进度事件节流：每条任务开始/收尾都 emit，120ms trailing 合并同一瞬间的连续事件。 */
const QUEUE_REFRESH_THROTTLE_MS = 120;
let queueRefreshTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleQueueRefresh() {
  if (queueRefreshTimer) return;
  queueRefreshTimer = setTimeout(() => {
    queueRefreshTimer = null;
    void queueQuery().catch(queueErrorHandler);
  }, QUEUE_REFRESH_THROTTLE_MS);
}

/** 订阅队列进度/收尾事件（App 挂载时调用一次）。
 *  - progress：载荷存入 store 驱动进度条，running 时乐观置位，并节流刷新队列快照；
 *  - finished：记收尾摘要、置 running=false，立即拉全量快照（撤掉挂起的节流查询，
 *    免得 120ms 后再打一次等价 IPC）。
 *  另外启动即先 query 一次：QueueDock 尚未挂载也能同步队列现状（侧栏 queueRunning 护栏靠它）。 */
export async function initQueueListeners(onError: (e: unknown) => void): Promise<void> {
  if (queueListenersInited) return; // 防止重复订阅
  queueListenersInited = true;
  queueErrorHandler = onError;
  void queueQuery().catch(onError);
  await listen<QueueProgress>('gen:queue:progress', (e) => {
    imageaiStore.queueProgress = e.payload;
    if (e.payload.status === 'running') imageaiStore.queueRunning = true;
    scheduleQueueRefresh();
  });
  await listen<QueueFinished>('gen:queue:finished', (e) => {
    imageaiStore.queueFinished = e.payload;
    imageaiStore.queueRunning = false;
    if (queueRefreshTimer) {
      clearTimeout(queueRefreshTimer);
      queueRefreshTimer = null;
    }
    void queueQuery().catch(queueErrorHandler);
  });
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
