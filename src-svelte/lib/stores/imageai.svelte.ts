/** AI 生图工作台状态（runes）。配置来自后端双来源文件，Key 不经前端持久化；
 *  queue 是后端快照的镜像（唯一写入方 setQueue），事件订阅为进程级（App 挂载时调用 initQueueListeners 一次）。 */
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

/** 模板变量说明 */
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

/** 自由尺寸前端校验（与 Rust validate_size 同口径）；返回 null = 通过。 */
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
  size: '350x350', // 不在任何模型预设里时 UI 自动落「自定义…」
  n: 1,
  quality: 'auto',
  seed: '',
  promptExtend: true, // 百炼智能改写；默认随模型能力走，切服务商/模型时归位
  watermark: false, // 「AI生成」水印，官方默认关；仅模型支持时发送
  generating: false,
  results: [] as GeneratedImage[],

  /** 会话内「已设置 Key」指示。Key 本体只进后端会话内存、不回传不持久化，刷新即清。 */
  keysSet: {} as Record<string, boolean>,

  /** 生图数据源球员（名单/单人预览入口写入）。null = 变量走模板默认值、手填 prompt 生效；
   *  有数据源时单张/批量均改走 preview_prompt，按该球员的名单记录渲染模板正文。 */
  sourcePlayer: null as { uid: string; name: string } | null,

  // ---- 批量生成队列：queue 是后端快照的镜像，唯一写入方是 setQueue ----
  queue: [] as QueueEntry[],
  /** 由 setQueue 按快照派生（存在 running 条目即在跑）；queueStart 例外地乐观置位。 */
  queueRunning: false,
  /** 最近一次队列进度事件载荷（驱动队列坞进度条）；订阅是进程级的，切走 AI 台事件不丢。 */
  queueProgress: null as QueueProgress | null,
  /** 最近一次批量收尾摘要；下次 queueStart 时复位。 */
  queueFinished: null as QueueFinished | null,
  /** 最近一次入队的作用域「档 · 组」（队列坞标题护栏）。后端 QueueEntry 不带作用域，仅前端记账。 */
  enqueueScope: '',
  /** 名单表格勾选（key=uid）。切组/换名单/翻页不清空——勾选跟随球员。 */
  selectedInTable: {} as Record<string, boolean>,

  get provider() {
    return this.config?.providers.find((p) => p.id === this.providerId) ?? null;
  },
  /** 当前模型明细；无明细行但等于服务商默认模型时合成通用能力（与后端 Config::model 兜底同口径）：
   *  不限张数、高级参数一律不发。 */
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
  /** 提示词超出当前模型上限（Panel CTA 禁用 + PromptCard 计数标红共用）；上限未声明时恒不超限。 */
  get promptOver(): boolean {
    const limit = this.modelInfo?.capabilities?.maxPromptChars ?? 0;
    return limit > 0 && this.prompt.length > limit;
  },

  /** 全局勾选数（不校验 uid 是否仍在当前页）。名单表格按钮用的是组件内自算的「当前页勾选数」。 */
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
  // 改写开关回该模型的官方默认（千问/万相开、z-image 关），不把上一家的选择带过来。
  imageaiStore.promptExtend = imageaiStore.modelInfo?.capabilities.promptExtendDefault ?? true;
  imageaiStore.watermark = false;
}

/** Key 仅进后端会话内存；成功后标记本会话「已设置」。 */
export async function setProviderKey(id: string, key: string) {
  await api.setProviderKey(id, key);
  imageaiStore.keysSet[id] = true;
}

// ---------------- 服务商 / 模板的写入出口（config 整包替换单点化） ----------------
/** 保存/删除均以后端返回的全量 ImageConfig 单点写回；写回后兜一致性：当前 providerId/templateId
 *  若已不在新配置中（删的正是当前项）就地回退，否则下拉悬空、生图带着指向已删条目的脏参数发出去。 */
export async function saveProvider(provider: ProviderConfig) {
  imageaiStore.config = await api.saveProvider(provider);
  ensureProviderValid();
}

export async function deleteProvider(id: string) {
  imageaiStore.config = await api.deleteProvider(id);
  ensureProviderValid();
}

/** providerId 悬空时回退默认服务商，模型/尺寸等派生值经 selectProvider 一并归位。 */
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

/** templateId 悬空时回退第一个模板（渲染失败向上抛，此时状态已切换一致）。 */
async function ensureTemplateValid() {
  const cfg = imageaiStore.config;
  if (!cfg || cfg.templates.some((t) => t.id === imageaiStore.templateId)) return;
  const fallback = cfg.templates[0];
  if (fallback) await applyTemplate(fallback.id);
}

export function selectModel(model: string) {
  imageaiStore.model = model;
  const caps = imageaiStore.modelInfo?.capabilities;
  // 尺寸不随模型切换重置：原值不在新预设时 UI 落「自定义…」，用户可能是有意沿用。
  if (caps && imageaiStore.n > caps.maxN) imageaiStore.n = caps.maxN;
  // 改写默认值逐模型不同（z-image 关，开启即思考模式、单价翻倍）：切模型归位。
  imageaiStore.promptExtend = caps?.promptExtendDefault ?? true;
}

/** 选模板：回填负向文案，并把正文经后端渲染一次（与生成前渲染同一实现）；失败向上抛给调用方弹窗。 */
export async function applyTemplate(id: string) {
  const tpl = imageaiStore.config?.templates.find((x) => x.id === id);
  if (!tpl) return;
  imageaiStore.templateId = id;
  imageaiStore.negative = tpl.negative;
  try {
    imageaiStore.prompt = (await api.renderPrompt(tpl.body)).text;
  } catch (e) {
    // 失败时正文退回模板原文，不留上一个模板的残文；错误仍向上抛。
    imageaiStore.prompt = tpl.body;
    throw e;
  }
}

export function appendVariable(key: string) {
  imageaiStore.prompt = `${imageaiStore.prompt}{{${key}}}`;
}

/** 设定/清除生图数据源球员（名单行「送生图」与单人预览「用此球员生成」共用入口）。 */
export function setSourcePlayer(uid: string, name: string) {
  imageaiStore.sourcePlayer = { uid, name };
}

export function clearSourcePlayer() {
  imageaiStore.sourcePlayer = null;
}

/** meta（同参重生）：结果卡传入该图记录的 {size, seed} 按原参重出；不传（CTA/重试）用工作台当前值。 */
export async function generate(meta?: { size?: string; seed?: string }) {
  if (!imageaiStore.model) throw new Error(t.gen.needModel);
  imageaiStore.generating = true;
  try {
    // 有数据源走 preview_prompt（按球员记录渲染模板正文，手填 prompt 不参与，换球员即复用同一模板）；
    // 无数据源用当前文本渲染——用户可能手工改写过，模板原文不得覆盖编辑结果。
    const rendered = imageaiStore.sourcePlayer
      ? await api.previewPrompt(imageaiStore.sourcePlayer.uid, imageaiStore.templateId)
      : await api.renderPrompt(imageaiStore.prompt);
    if (rendered.missing.length > 0) throw new Error(t.gen.missingPlaceholders(rendered.missing));
    if (!imageaiStore.sourcePlayer) imageaiStore.prompt = rendered.text; // 回写：所见即所发
    const caps = imageaiStore.modelInfo?.capabilities;
    // 同参重生：meta 提供原图尺寸/Seed（'' = 随机）；不传则用工作台当前值。
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
      // 百炼开关仅当前模型声明支持时才带值，否则传 null（后端按同一能力集再门控一遍）。
      promptExtend: caps?.supportsPromptExtend ? imageaiStore.promptExtend : null,
      watermark: caps?.supportsWatermark ? imageaiStore.watermark : null,
    });
    // 实发参数快照随结果记录（结果卡展示 + 同参重生的入参来源）。
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

/** 清空全部结果（blob URL 逐个回收）。 */
export function clearResults() {
  for (const r of imageaiStore.results) URL.revokeObjectURL(r.url);
  imageaiStore.results = [];
}

// ---------------- 结果卡「保存并替换」（P3） ----------------

/** 把生成图写为数据源球员的新头像：后端复制到 <头像包>/_generated/ 并复用单人换脸全链。
 *  备份开关读 appStore.saveBackup（与单人替换同口径）；sourcePlayer 为 null 时 UI 已禁用，这里再拦一道。 */
export async function saveAndReplace(imageId: string) {
  const sp = imageaiStore.sourcePlayer;
  if (!sp) throw new Error(t.gen.saveReplace.needSource);
  return api.saveAndReplace(imageId, sp.uid, appStore.saveBackup);
}

// ---------------- 批量生成队列（P2 Task5） ----------------

/** 提交加入队列的行数据（名单预览当前页勾选的球员）。 */
export interface PlayerRow { uid: string; name: string; }

/** 组装队列条目：其余参数取工作台当前值；id 留空由后端补发，status 复位为 queued。
 *  渲染：有数据源逐行 preview_prompt（每球员各自变量）；无数据源循环外 renderPrompt 一次复用
 *  （每行同参调用是 N 次等价 IPC）。 */
export async function buildQueueEntries(rows: ReadonlyArray<PlayerRow>): Promise<QueueEntry[]> {
  if (!imageaiStore.model) throw new Error(t.gen.needModel);
  const entries: QueueEntry[] = [];
  let shared: RenderOutcome | null = null;
  const caps = imageaiStore.modelInfo?.capabilities;
  for (const r of rows) {
    // 渲染失败/缺占位符即整批失败：与单张 generate 口径一致（missing 不得原文送进 API）。
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
        uid: r.uid,
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

/** 快照唯一写入方：整份替换本地镜像并派生 queueRunning。挂载时 query 一次即能同步错过的现状。 */
export function setQueue(entries: QueueEntry[]) {
  imageaiStore.queue = entries;
  imageaiStore.queueRunning = entries.some((e) => e.status === 'running');
}

/** 组装并入队，返回本次提交条数；后端全量快照写回 store，并记下入队作用域。 */
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
  imageaiStore.queueFinished = null; // 新一批开始：收起上一次收尾摘要
  await api.startQueue();
  imageaiStore.queueRunning = true;
}

export async function queueCancel() {
  await api.cancelQueue();
}

export function queueRemove(ids: string[]) {
  return api.removeFromQueue(ids).then(setQueue);
}

/** 单条重试：移除旧条目、用原参入一条新任务（id 由后端补发）；仅队列未运行时允许。 */
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
// 订阅曾挂在 QueueDock 的 onMount：切走 AI 台期间事件全错过，queueRunning 卡在 true。
// 现与 app.svelte.ts 的 initListeners 同口径：App 挂载调用一次，内部防重复，存活到进程结束。

let queueListenersInited = false;
/** 后台刷新/订阅出错的出口：store 不碰 UI，弹窗由 App 注入。 */
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

/** 订阅队列进度/收尾事件（App 挂载时调用一次）。启动即先 query 一次：QueueDock 未挂载也能同步队列现状。 */
export async function initQueueListeners(onError: (e: unknown) => void): Promise<void> {
  if (queueListenersInited) return;
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
    // 立即拉全量快照并撤掉挂起的节流查询，免得 120ms 后再打一次等价 IPC。
    if (queueRefreshTimer) {
      clearTimeout(queueRefreshTimer);
      queueRefreshTimer = null;
    }
    void queueQuery().catch(queueErrorHandler);
  });
}

// ---------------- 名单表格勾选（key=uid，切组/翻页不清空） ----------------

/** 勾选/取消单个球员；v 缺省为取反。 */
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
