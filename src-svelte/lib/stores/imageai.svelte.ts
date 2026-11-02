/** AI 生图工作台状态（runes）。配置来自后端双来源文件，Key 不经前端持久化；
 *  queue 是后端快照的镜像（唯一写入方 setQueue），事件订阅为进程级（App 挂载时调用 initQueueListeners 一次）。 */
import { listen } from '@tauri-apps/api/event';
import { api } from '../services/api';
import { appStore } from './app.svelte';
import { loadImageBlobUrl } from '../services/images';
import { t } from '../text';
import type {
  Capabilities,
  GenErrorKind,
  GenRequest,
  GeneratedImage,
  ImageConfig,
  PromptTemplate,
  ProviderConfig,
  QueueEntry,
  QueueStatus,
  QueueFinished,
  QueueProgress,
  SizeRules,
} from '../types';

/** 模板变量说明（与后端 build_vars 的实际产出键一一对应）。
 *  发长码不产出变量：发型词库按发长码分档，选出的风格词自带长度语义。 */
export const VARIABLE_HINTS: ReadonlyArray<readonly [string, string]> = [
  ['name', '姓名'], ['ethnicity', '种族'], ['hairColor', '发色'],
  ['skinCode', '肤色'], ['hairStyle', '发型样式'], ['beard', '胡须'],
  ['identity', '身份'], ['shot', '景别'],
  ['gender', '性别'], ['age', '年龄'], ['height', '身高'],
  ['primaryNat', '国籍码'], ['secNat', '第二国籍码'], ['nationality', '国籍'],
  ['club', '俱乐部'], ['jersey', '球衣'], ['kitDetail', '球衣细节'], ['based', '联赛地区'], ['career', '生涯归属'],
  ['expression', '表情'], ['pose', '动作'], ['lighting', '打光方向'], ['bgTone', '背景'],
  ['ca', 'CA'], ['pa', 'PA'], ['uid', 'UID'],
];

/** 解析 宽×高（小写 `x`）/「宽*高」尺寸。 */
export function parseSize(s: string): [number, number] | null {
  const m = /^(\d+)[x*](\d+)$/.exec(s.trim());
  return m ? [Number(m[1]), Number(m[2])] : null;
}

/** Seed 官方取值上限（百炼四份文生图 API 参考口径一致：[0, 2147483647]）。 */
export const SEED_MAX = 2147483647;

/** 解析 Seed 输入：空串/非数字/负数/超官方上限一律视为随机（null）。
 *  输入框是 number 型且声明了 min/max，但浏览器不拦手输的越界值 ⇒ 生效判定只能在这里兜；
 *  越界时的可见提示见 imageaiStore.seedOutOfRange，别让用户以为超限值生效了。 */
export function parseSeed(s: string): number | null {
  if (s.trim() === '') return null;
  const n = Number(s);
  return Number.isFinite(n) && n >= 0 && n <= SEED_MAX ? Math.trunc(n) : null;
}

/** 采样步数：留空 = 不发送（用服务端默认）；非数字按不发送处理。 */
function parseSteps(s: string): number | null {
  if (s.trim() === '') return null;
  const n = Number(s);
  return Number.isFinite(n) && n >= 1 ? Math.trunc(n) : null;
}

/** 引导系数：留空 = 不发送（用服务端默认）；非数字按不发送处理。 */
function parseGuidance(s: string): number | null {
  if (s.trim() === '') return null;
  const n = Number(s);
  return Number.isFinite(n) && n >= 0 ? n : null;
}

/**
 * 自由尺寸的前端预校验（与 Rust provider 层共享的 validate_size 同口径）；返回 null = 通过。
 * 这里只是「早提示」UX：权威判定在 Provider 层（image_ai_providers），前端放行不代表后端一定放行，
 * 因此该函数只做提示，不据此跳过任何后端调用。
 */
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

/**
 * 「档位串模型」（wan 的 1K/2K/4K、火山 Seedream 的 1K/1.5K/2K…）选「自定义…」时的起步宽高：
 * 这些模型的预设解析不出宽高，直接取该模型声明的**像素下限**当最小合法正方形——
 * 总像素下限（`totalMin`）开方即边长；只有单边下限（`minSide`）时用它；
 * 两者都没声明（自建 OpenAI 兼容服务）则保持历史字面量 350×350。
 */
export function customSizeStart(rules: SizeRules | null | undefined): [number, number] {
  const floor = rules?.totalMin ? Math.ceil(Math.sqrt(rules.totalMin)) : (rules?.minSide ?? 0);
  const side = floor > 0 ? floor : 350;
  return [side, side];
}

/**
 * 尺寸字面量的**形态**：档位串（wan / 火山 Seedream 的 `1K`、`1.5K`、`2K`、`4K`）还是像素对
 * （`1024x1024`、百炼的 `1024*1024`）。`null` ＝ 两者都不是（空串、脏值）。
 * 像素形态直接复用 [`parseSize`] 的同一判据（同一个正则），不另立第二份宽×高的定义。
 */
function sizeForm(size: string): 'tier' | 'pixel' | null {
  const v = size.trim();
  if (/^\d+(\.\d+)?[Kk]$/.test(v)) return 'tier';
  return parseSize(v) ? 'pixel' : null;
}

/**
 * 某组预设**能接受**哪些形态。必须看整份列表、不能只看第一条：dashscope 万相 2.7 的预设就是
 * 「`1K/2K/4K` 档位串 + 官方推荐像素值」混排（见 `src-tauri/src/image_ai_providers/dashscope.rs`
 * 的 `wan27_caps`：`tiers.chain(pixels)`），按首元素一刀切会把用户自选的像素值误判成跨形态、
 * 每次切模型都白归位一次。
 */
function presetForms(presets: readonly string[]): { tier: boolean; pixel: boolean } {
  let tier = false;
  let pixel = false;
  for (const p of presets) {
    const f = sizeForm(p);
    if (f === 'tier') tier = true;
    else if (f === 'pixel') pixel = true;
  }
  return { tier, pixel };
}

/**
 * 自由尺寸模型（`caps.sizes` 为空）需要的像素值——切模型时从档位串归位用、切服务商时起步用，
 * 两处同一个口径。优先取该模型声明的**像素下限**
 * （`totalMin` 开方＝最小合法正方形；只有 `minSide` 时用它），两者都没声明就用 1024×1024——
 * 这是各家生图模型最常见的正方形出图档（魔搭/硅基流动千问系列的 1K 档就是它）。
 * 这里刻意不用 [`customSizeStart`] 的 350×350 兜底：那是「自定义…」输入框的**起步值**，
 * 而这里要的是一个「真要发出去」的取值。
 */
function tierToPixel(rules: SizeRules | null | undefined): string {
  if (rules?.totalMin || rules?.minSide) {
    const [side] = customSizeStart(rules);
    return `${side}x${side}`;
  }
  return '1024x1024';
}

/**
 * 切模型时把尺寸收口成新模型**真能接受**的字面量——只在「跨形态」时归位，同形态一律原样返回。
 *
 * 为什么必须归位：档位串（`1K`）发给只认像素的模型（z-image、千问 3.0 一类）会被厂商判非法，
 * `start_queue` 按整批拦下；反向（像素值发给只认档位的火山 Seedream）同理。
 * 为什么同形态不动：值可能不在新预设里，但那是用户有意挑的自定义尺寸，UI 会落在「自定义…」
 * 并走 `sizeRules` 校验（既有设计，别顺手改成「不在预设就归位」）。
 *
 * 边界：
 * - 模型明细缺失（该模型已从配置里删掉）：无从判断形态，保留原值，不猜。
 * - `caps.sizes` 为空（自由尺寸模型）：没有预设可归位；像素值/脏值原样保留，唯独档位串它不认
 *   → 推一个合法像素值（依据见 [`tierToPixel`]）。
 * - 当前值两种形态都不匹配（空串/脏值）：按「不属于任何形态」处理，归位首个预设；无预设则不动。
 */
function fitSizeToModel(current: string, caps: Capabilities | undefined): string {
  if (!caps) return current;
  const presets = caps.sizes ?? [];
  const form = sizeForm(current);
  if (presets.length === 0) return form === 'tier' ? tierToPixel(caps.sizeRules) : current;
  const accepts = presetForms(presets);
  if (form === 'tier') return accepts.tier ? current : presets[0];
  if (form === 'pixel') return accepts.pixel ? current : presets[0];
  return presets[0];
}

export const imageaiStore = $state({
  config: null as ImageConfig | null,
  providerId: 'openai',
  model: '',
  templateId: 'default',
  prompt: '',
  /** 输入框是否被手工改过。未改 → 每次生成都重掷随机细节（组装新提示词再发出）；
   *  改过 → 原样发送框内文本，不用后端组装结果覆盖用户的修改。 */
  promptDirty: false,
  negative: '',
  size: '350x350', // 不在任何模型预设里时 UI 自动落「自定义…」
  n: 1,
  quality: 'auto',
  seed: '',
  steps: '', // 采样步数（硅基流动 num_inference_steps / 魔搭 steps）；留空或模型不支持时不发送
  guidance: '', // 引导系数（硅基流动 guidance_scale / 魔搭 guidance）；留空或模型不支持时不发送
  promptExtend: true, // 百炼智能改写；默认随模型能力走，切服务商/模型时归位
  watermark: false, // 「AI生成」水印，官方默认关；仅模型支持时发送
  outputFormat: 'png' as 'png' | 'jpeg', // 输出图片格式；仅模型声明 output_format 能力时发送
  promptExtendMode: 'direct', // 改写模式 direct/agent；仅 3.0 支持，切模型归位官方默认档
  enableThinking: true, // 思考模式；官方默认开，且仅改写开启时生效
  callMode: 'sync' as 'sync' | 'async', // 调用方式；仅支持异步的模型可选，切服务商/模型时归位同步
  generating: false,
  results: [] as GeneratedImage[],
  /** 载入的持久画廊记录（跨会话留存；图片仅存会话临时目录，可能已被清理 → url 为空，渲染端显示占位）。
   *  与 results 一起构成画廊数据源，去重见 gallery 派生。 */
  records: [] as GeneratedImage[],

  /** 哪些服务商「已设置 Key」。密文随服务商行落盘、明文只进后端会话内存，且永不回传前端；
   *  刷新后由 getProviderKeyState 认回，判断依据只有布尔。 */
  keysSet: {} as Record<string, boolean>,

  /** 生图数据源球员（名单/单人预览入口写入）。null = 变量走模板默认值、手填 prompt 生效；
   *  有数据源时单张/批量均改走 preview_prompt，按该球员的名单记录渲染模板正文。 */
  sourcePlayer: null as { uid: string; name: string } | null,

  // ---- 批量生成队列：queue 是后端快照的镜像，唯一写入方是 setQueue ----
  queue: [] as QueueEntry[],
  /** 由 setQueue 按快照派生（存在 running 条目即在跑）；queueStart 例外地乐观置位。 */
  queueRunning: false,
  /** 已请求取消、但队列尚未完全收尾。取消只拦「还没提交」的请求：未派发条目立刻变「已取消」，
   *  已提交的那条**不再中断**，只能等本次响应返回（结果照常落盘并记画廊）——执行中没有任何
   *  检查点，故这段真空期可能长达一整个请求；必须有反馈，否则用户会以为按钮没生效而反复点。 */
  queueCancelling: false,
  /** 最近一次队列进度事件载荷（驱动队列坞进度条）；订阅是进程级的，切走 AI 台事件不丢。 */
  queueProgress: null as QueueProgress | null,
  /** 最近一次批量收尾摘要；下次 queueStart 时复位。 */
  queueFinished: null as QueueFinished | null,
  /** 启动方式：true = 全自动（点启动即提交，原行为）；false = 手动确认（逐条核对参数后才启动）。
   *  会话内状态，不落盘——它是一次操作时的选择，不是用户偏好。 */
  queueAutoConfirm: true,
  /** 手动模式下被核对的那一条（右侧工作台此刻装的是该行的参数，见 queueBorrowForm）。会话内、不落盘：
   *  它只是「现在在看哪一条」的临时光标，刷新后重新点一行即可。 */
  queueSelectedId: null as string | null,
  /** 已逐条确认过的条目 id。确认后再改参数会清掉对应键（绿退橙）——由工作台卡的冒泡
   *  input/change 触发（见 GenSettingsCard 的 onCardEdited），不做字段比较：交互即视为「可能改过」。
   *  指向已移除条目的残留键无副作用（读时按 id 判断）。 */
  queueConfirmedIds: {} as Record<string, boolean>,
  /** 借用中：表单此刻属于某条队列行。归位 effect 与 provider/model 选择器据此让路。 */
  queueBorrowing: false,
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
        capabilities: {
          sizes: [],
          maxN: 0,
          supportsQuality: false,
          supportsSeed: false,
          supportsSteps: false,
          supportsGuidance: false,
          supportsNegativePrompt: false,
          supportsResponseFormat: false,
          supportsOutputFormat: false,
          qualities: [],
          supportsPromptExtend: false,
          supportsWatermark: false,
          maxPromptChars: 0,
          sizeRules: null,
          supportsN: true,
          promptExtendDefault: false,
        },
      };
    }
    return null;
  },
  /** 提示词超出当前模型上限（CTA 禁用 + 生成设置卡计数标红共用）；上限未声明时恒不超限。 */
  get promptOver(): boolean {
    const limit = this.modelInfo?.capabilities?.maxPromptChars ?? 0;
    return limit > 0 && this.prompt.length > limit;
  },
  /** 尺寸违规信息（CTA 禁用/title + 生成设置卡自定义行共用）；null = 通过。
   *  预设尺寸天然合法（含 wan 的 "1K/2K" 档位串，本就不是 WxH 格式），只有非预设值
   *  （自定义宽高、或无预设模型的兜底值）才走 sizeRules——避免档位串误报格式错误。 */
  get sizeError(): string | null {
    const caps = this.modelInfo?.capabilities;
    if (!caps?.sizeRules || caps.sizes.includes(this.size)) return null;
    return validateSizeRules(this.size, caps.sizeRules);
  },

  /** Seed 超出官方取值范围 [0, SEED_MAX]：输入框标红 + 悬停说明。
   *  非数字/负数不在此列（那是既有的「非法即随机」契约，用户一眼就知道自己填错了）；
   *  只有「看着像合法 seed 但超上限」才需要提示，因为生成时会被 parseSeed 静默按随机处理。 */
  get seedOutOfRange(): boolean {
    const s = this.seed.trim();
    if (s === '') return false;
    const n = Number(s);
    return Number.isFinite(n) && n > SEED_MAX;
  },

  /** 全局勾选数（不校验 uid 是否仍在当前页）。名单表格按钮用的是组件内自算的「当前页勾选数」。 */
  get selectedCount() {
    return Object.values(this.selectedInTable).filter(Boolean).length;
  },

  /** 画廊数据源 = 本会话新增 results ∪ 载入的持久 records，按图 id 去重。
   *  去重时本会话结果优先：它带可用的 blob URL，而同一张图的持久记录 url 可能为空（或未加载）。 */
  get gallery(): GeneratedImage[] {
    const seen = new Set<string>();
    const out: GeneratedImage[] = [];
    for (const r of this.results) {
      if (seen.has(r.id)) continue;
      seen.add(r.id);
      out.push(r);
    }
    for (const r of this.records) {
      if (seen.has(r.id)) continue;
      seen.add(r.id);
      out.push(r);
    }
    return out;
  },
});

/** 在途加载承诺：名单预览入队与 AI 面板挂载可能同时触发加载（或用户连点），共享同一次 IPC；
 *  失败后承诺清空，允许下次进入时重试。 */
let loadPromise: Promise<void> | null = null;

/** 拉取后端配置并选中默认服务商/模板；config 已水合时立即兑现，保留面板 onMount 的懒加载语义。 */
export function loadImageState(): Promise<void> {
  if (imageaiStore.config) return Promise.resolve();
  if (!loadPromise) {
    loadPromise = (async () => {
      imageaiStore.config = await api.getImageAiState();
      // Key 已加密落盘：后端只回「哪些服务商配了 Key」（id 列表），据此点亮 keysSet。
      // 失败不阻断——最坏情况是界面显示未设置、生成时后端仍能取到 Key。
      try {
        // 整表重建而不是逐条置 true：后端列表是唯一事实源，删服务商/清 Key 后旧标记必须消失。
        const ids = await api.getProviderKeyState();
        imageaiStore.keysSet = Object.fromEntries(ids.map((id) => [id, true]));
      } catch {
        // 忽略：留待用户重新输入，或下次进入时再拉
      }
      selectProvider(imageaiStore.config.settings.defaultProviderId || 'openai');
      const tpl =
        imageaiStore.config.templates.find((x) => x.id === imageaiStore.templateId) ??
        imageaiStore.config.templates[0];
      if (tpl) await applyTemplate(tpl.id);
      // 画廊记录与 config 同批拉取；记录是次要信息，失败不阻断工作台可用性。
      try {
        await loadGalleryRecords();
      } catch {
        // 忽略：config 已水合，记录留待下次进入工作台时重试
      }
    })().finally(() => {
      loadPromise = null;
    });
  }
  return loadPromise;
}

/** 载入持久画廊记录：图片仍在会话临时目录的，预读成 blob URL；已被清理的 url 留空（渲染端显示占位）。
 *  单张图片读取失败只退化为该条占位，不阻塞整批。 */
async function loadGalleryRecords() {
  const items = await api.getGallery();
  const recs: GeneratedImage[] = [];
  for (const it of items) {
    let url = '';
    if (it.imageAvailable && it.path) {
      try {
        url = await loadImageBlobUrl(it.path);
      } catch {
        url = '';
      }
    }
    // savedPath/savedName 来自 GalleryItem（Rust 的 #[serde(flatten)] 把记录字段平铺到顶层）；
    // 后端对从未保存过的记录省略该键，这里统一收敛成 null，渲染端只判空。
    recs.push({ id: it.id, path: it.path ?? '', url, imageAvailable: it.imageAvailable, removable: false, savedPath: it.savedPath ?? null, savedName: it.savedName ?? null });
  }
  imageaiStore.records = recs;
}

/** 采样步数 / 引导系数随模型归位：支持则回模型声明默认值，不支持则留空（不发送）。 */
function resetStepsGuidance() {
  const caps = imageaiStore.modelInfo?.capabilities;
  imageaiStore.steps = caps?.supportsSteps && caps.stepsDefault ? String(caps.stepsDefault) : '';
  imageaiStore.guidance =
    caps?.supportsGuidance && caps.guidanceDefault ? String(caps.guidanceDefault) : '';
}

export function selectProvider(id: string) {
  imageaiStore.providerId = id;
  imageaiStore.model = imageaiStore.provider?.defaultModel ?? '';
  // 尺寸从新模型的首个预设起步；预设为空（自由尺寸模型，只有 sizeRules）时按该模型声明的像素
  // 下限起步。这里不能用 350×350 兜底：那是「自定义…」输入框的起步值，低于绝大多数模型的下限，
  // 一切到自由尺寸的服务商就开局报红。
  const caps = imageaiStore.modelInfo?.capabilities;
  imageaiStore.size = caps?.sizes.length ? caps.sizes[0] : tierToPixel(caps?.sizeRules);
  imageaiStore.n = 1;
  imageaiStore.seed = '';
  resetStepsGuidance();
  imageaiStore.quality = 'auto';
  // 改写开关回该模型的官方默认（千问/万相开、z-image 关），不把上一家的选择带过来。
  imageaiStore.promptExtend = imageaiStore.modelInfo?.capabilities.promptExtendDefault ?? true;
  // 水印回该模型声明的官方默认（火山 Seedream 全系列 false），同样不把上一家的选择带过来。
  imageaiStore.watermark = imageaiStore.modelInfo?.capabilities.watermarkDefault ?? false;
  // 输出格式回适配器默认 png（能力位只声明「可设」，没有逐模型默认值可读）。
  imageaiStore.outputFormat = 'png';
  // 调用方式随模型复位：不同模型异步支持不同，不把上一家的选择带过来（与 promptExtend 同口径）。
  imageaiStore.callMode = 'sync';
}

/** Key 明文只进后端会话内存（密文落盘）；空串＝清除。成功后同步「已设置」标记。 */
export async function setProviderKey(id: string, key: string) {
  await api.setProviderKey(id, key);
  imageaiStore.keysSet[id] = !!key.trim();
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
  // 该 id 的 Key 已随后端配置行消失；前端标记一并清掉，避免同 id 重建后误显示「已设置」。
  imageaiStore.keysSet[id] = false;
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
  // 尺寸只在**跨形态**时归位（档位串 1K/2K ↔ 像素 1024x1024，判据见 fitSizeToModel）：
  // 同形态内继续沿用旧值——原值不在新预设里也不动，UI 落「自定义…」，用户可能是有意沿用；
  // 跨形态必须换，否则发出去的是新模型根本不认的字面量（start_queue 整批拦下）。
  imageaiStore.size = fitSizeToModel(imageaiStore.size, caps);
  if (caps && imageaiStore.n > caps.maxN) imageaiStore.n = caps.maxN;
  // 改写默认值逐模型不同（z-image 关，开启即思考模式、单价翻倍）：切模型归位。
  imageaiStore.promptExtend = caps?.promptExtendDefault ?? true;
  // 水印默认值同样按模型声明存放（watermarkDefault）：切模型归位，不把上一个模型的开关带过来。
  imageaiStore.watermark = caps?.watermarkDefault ?? false;
  // 输出格式同理回默认 png：不同模型的可用格式集不同（火山 4.5/4.0 根本不吃该参数）。
  imageaiStore.outputFormat = 'png';
  // 采样步数 / 引导系数逐模型不同（硅基流动 Qwen-Image 不支持 guidance 等）：切模型归位。
  resetStepsGuidance();
  imageaiStore.promptExtendMode = caps?.promptExtendModeDefault || 'direct';
  imageaiStore.enableThinking = caps?.enableThinkingDefault ?? true;
  // 调用方式随模型复位：新模型未必支持异步（与 selectProvider 同口径）。
  imageaiStore.callMode = 'sync';
}

/** 选模板：回填负向文案，并把正文经后端组装一次写进输入框（与生成前同一实现）。
 *  有数据源时按该球员记录组装（含本轮随机细节），否则只把占位符展开成默认值。
 *  失败向上抛给调用方弹窗。 */
export async function applyTemplate(id: string) {
  const tpl = imageaiStore.config?.templates.find((x) => x.id === id);
  if (!tpl) return;
  imageaiStore.templateId = id;
  imageaiStore.negative = tpl.negative;
  const sp = imageaiStore.sourcePlayer;
  try {
    imageaiStore.prompt = sp
      ? (await api.previewPrompt(sp.uid, id)).text
      : (await api.renderPrompt(tpl.body)).text;
    imageaiStore.promptDirty = false;
  } catch (e) {
    // 失败时正文退回模板原文，不留上一个模板的残文；错误仍向上抛。
    imageaiStore.prompt = tpl.body;
    throw e;
  }
}

/** 设定生图数据源球员（名单行「送生图」与单人预览「送生图」共用入口）：
 *  随即用后端按该球员记录组装一版提示词回填输入框（含本轮随机细节），供用户查看与修改。 */
export function setSourcePlayer(uid: string, name: string) {
  imageaiStore.sourcePlayer = { uid, name };
  void assembleForSource(uid);
}

/** 按球员记录组装提示词并回填输入框。失败静默：名单未扫到该球员时框内保留原文，
 *  真正缺什么会在生成时抛可读错误。两处 uid 校验用于丢弃换人期间返回的过期结果。 */
async function assembleForSource(uid: string) {
  try {
    if (!imageaiStore.config) await loadImageState();
    if (imageaiStore.sourcePlayer?.uid !== uid) return;
    const rendered = await api.previewPrompt(uid, imageaiStore.templateId);
    if (imageaiStore.sourcePlayer?.uid !== uid) return;
    imageaiStore.prompt = rendered.text;
    imageaiStore.promptDirty = false;
  } catch {
    // 组装失败不打断选人流程
  }
}

export function clearSourcePlayer() {
  imageaiStore.sourcePlayer = null;
}

/** 手动重掷：按当前数据源重新组装一版提示词回填输入框（含新一轮随机细节），覆盖框内手改。
 *  无数据源时空操作——纯模板渲染是确定性的默认值填充，没有可重掷的随机量。 */
export function rerollPrompt() {
  const sp = imageaiStore.sourcePlayer;
  if (sp) void assembleForSource(sp.uid);
}

/** meta（同参重生）：结果卡传入该图记录的 {size, seed} 按原参重出；不传（CTA/重试）用工作台当前值。 */
export async function generate(meta?: { size?: string; seed?: string }) {
  if (!imageaiStore.model) throw new Error(t.gen.needModel);
  imageaiStore.generating = true;
  try {
    // 组装口径：有数据源且框内未被手工改过 → 后端重新组装（重掷随机细节）后回填；
    // 其余情况一律以框内文本为准——用户的修改不被模板原文覆盖，这就是「所见即所发」。
    const sp = imageaiStore.sourcePlayer;
    const reroll = !!sp && !imageaiStore.promptDirty;
    const rendered = reroll
      ? await api.previewPrompt(sp.uid, imageaiStore.templateId)
      : await api.renderPrompt(imageaiStore.prompt);
    if (rendered.missing.length > 0) throw new Error(t.gen.missingPlaceholders(rendered.missing));
    imageaiStore.prompt = rendered.text; // 回填：框内文本与实发一致
    imageaiStore.promptDirty = false;
    const caps = imageaiStore.modelInfo?.capabilities;
    // 同参重生：meta 提供原图尺寸/Seed（'' = 随机）；不传则用工作台当前值。
    const size = meta?.size || imageaiStore.size;
    const seedRaw = meta ? meta.seed ?? '' : imageaiStore.seed;
    const seedNum = parseSeed(seedRaw); // 无效输入按随机处理，结果卡展示的也是实际生效值
    const usedSeed = seedNum === null ? '' : String(seedNum);
    // 人员信息只进日志与记录（适配器不发送）：名单里查不到就留空，不影响生成。
    const rec = sp
      ? (await api.queryPlayersByUids([sp.uid]).catch(() => []))[0] ?? null
      : null;
    const res = await api.generateImage({
      providerId: imageaiStore.providerId,
      model: imageaiStore.model,
      prompt: rendered.text,
      negativePrompt: imageaiStore.negative,
      // 数据源球员身份与模板 id 随请求带入：后端据此写画廊记录（无数据源时为空）。
      uid: imageaiStore.sourcePlayer?.uid ?? null,
      name: imageaiStore.sourcePlayer?.name ?? null,
      nationality: rec?.primaryNat?.trim() || null,
      club: rec?.detailed?.club?.trim() || null,
      templateId: imageaiStore.templateId,
      size,
      n: imageaiStore.n,
      quality: imageaiStore.quality === 'auto' ? null : imageaiStore.quality,
      seed: seedNum,
      // 采样步数 / 引导系数仅在该模型声明支持时携带（后端按同一能力集再门控一遍）。
      steps: caps?.supportsSteps ? parseSteps(imageaiStore.steps) : null,
      guidance: caps?.supportsGuidance ? parseGuidance(imageaiStore.guidance) : null,
      // 百炼开关仅当前模型声明支持时才带值，否则传 null（后端按同一能力集再门控一遍）。
      promptExtend: caps?.supportsPromptExtend ? imageaiStore.promptExtend : null,
      watermark: caps?.supportsWatermark ? imageaiStore.watermark : null,
      promptExtendMode: caps?.supportsPromptExtendMode ? imageaiStore.promptExtendMode : null,
      enableThinking: caps?.supportsEnableThinking ? imageaiStore.enableThinking : null,
      // 输出格式仅在该模型声明可设时带值（火山 4.5/4.0 不吃该参数，后端按同一能力位再门控一遍）。
      outputFormat: caps?.supportsOutputFormat ? imageaiStore.outputFormat : null,
      // 仅当模型支持异步且用户选了异步才传 'async'；否则传 null（后端默认同步）。
      callMode: caps?.supportsAsync && imageaiStore.callMode === 'async' ? 'async' : null,
    });
    // 实发参数快照随结果记录（结果卡展示 + 同参重生的入参来源）。
    const used = { size, seed: usedSeed };
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
 *  提示词**逐球员**按模板组装（各带一份自己的随机细节）——批量不读输入框，
 *  框内编辑只对单张生效，否则一行文本无法体现每名球员各自的变量。 */
export async function buildQueueEntries(rows: ReadonlyArray<PlayerRow>): Promise<QueueEntry[]> {
  if (!imageaiStore.model) throw new Error(t.gen.needModel);
  const entries: QueueEntry[] = [];
  const caps = imageaiStore.modelInfo?.capabilities;
  for (const r of rows) {
    // 渲染失败/缺占位符即整批失败：与单张 generate 口径一致（missing 不得原文送进 API）。
    const rendered = await api.previewPrompt(r.uid, imageaiStore.templateId);
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
        seed: parseSeed(imageaiStore.seed),
        steps: caps?.supportsSteps ? parseSteps(imageaiStore.steps) : null,
        guidance: caps?.supportsGuidance ? parseGuidance(imageaiStore.guidance) : null,
        uid: r.uid,
        name: r.name,
        templateId: imageaiStore.templateId,
        promptExtend: caps?.supportsPromptExtend ? imageaiStore.promptExtend : null,
        watermark: caps?.supportsWatermark ? imageaiStore.watermark : null,
        promptExtendMode: caps?.supportsPromptExtendMode ? imageaiStore.promptExtendMode : null,
        enableThinking: caps?.supportsEnableThinking ? imageaiStore.enableThinking : null,
        // 输出格式仅在该模型声明可设时带值；否则传 null（后端按同一能力位再门控一遍）。
        outputFormat: caps?.supportsOutputFormat ? imageaiStore.outputFormat : null,
        // 仅当模型支持异步且用户选了异步才传 'async'；否则传 null（后端默认同步）。
        callMode: caps?.supportsAsync && imageaiStore.callMode === 'async' ? 'async' : null,
      },
      status: 'queued',
      images: [],
      error: '',
      errorKind: null,
    });
  }
  // 人员信息（国籍/俱乐部）随请求带入：**只进日志与记录**，适配器不发送。
  // 批量查一次即可（名单未就绪时失败即忽略，日志里只是少两个字段）。
  const recs = await api.queryPlayersByUids([...new Set(rows.map((r) => r.uid))]).catch(() => []);
  const byUid = new Map(recs.map((r) => [r.uid, r]));
  for (const e of entries) {
    const rec = byUid.get(e.uid);
    e.req.nationality = rec?.primaryNat?.trim() || null;
    e.req.club = rec?.detailed?.club?.trim() || null;
  }
  return entries;
}

/** 快照唯一写入方：整份替换本地镜像并派生 queueRunning。挂载时 query 一次即能同步错过的现状。 */
export function setQueue(entries: QueueEntry[]) {
  imageaiStore.queue = entries;
  imageaiStore.queueRunning = entries.some((e) => e.status === 'running');
  // 快照里已无 running：说明取消（或自然收尾）已经落地，过渡态该收了。
  if (!imageaiStore.queueRunning) imageaiStore.queueCancelling = false;
}

/** 组装并入队，返回本次提交条数；后端全量快照写回 store。 */
export async function enqueuePlayers(rows: ReadonlyArray<PlayerRow>): Promise<number> {
  // 名单预览位于存档/替换工作台，用户可能从未打开过 AI 台：此时 config 尚未水合、model 还是初值 ''，
  // 入队前按需拉一次后端持久化配置，让默认服务商/模型生效。
  if (!imageaiStore.config) await loadImageState();
  const entries = await buildQueueEntries(rows);
  setQueue(await api.addToQueue(entries));
  return entries.length;
}

/** 启动批量生成。`ids` 只跑这些条目（按勾选启动），省略即跑全部排队条目。
 *  后端校验失败（缺 Key/重入）会抛错；成功则乐观置 running。 */
export async function queueStart(ids?: string[]) {
  imageaiStore.queueFinished = null; // 新一批开始：收起上一次收尾摘要
  imageaiStore.queueCancelling = false; // 上一轮的取消过渡态不得带进新一批
  await api.startQueue(ids);
  imageaiStore.queueRunning = true;
}

/** 逐条确认后的参数写回后端队列：后端只改排队中的条目，返回的全量快照整份替换本地镜像。
 *  参数合法性不在这里校验——启动时后端 `validate_entries` 会整批拦下不合规的条目。 */
export async function queueApplyEdits(edits: { id: string; req: GenRequest }[]) {
  setQueue(await api.updateQueueEntries(edits));
}

// ---------------- 手动确认：把工作台原表单「借」给一条队列行 ----------------
// 复核一条队列任务的参数必须用**工作台那套表单**（同一批控件、同一套交互），不为它另画一份输入框。
// 故选中行时把该行参数灌进本 store 的表单字段，用户就在原表单里改；点「确认」按既有映射写回队列。
//
// 【语义（刻意取舍，不是漏做——别当 bug 修）】
// - 切行 = 直接覆盖：上一行没点过「确认」的修改随覆盖丢弃，不做快照备份、不做逐字段还原。
// - 退出选中（切全自动 / 队列开跑 / 该行消失）= 不回滚：表单停在最后一条的参数上，
//   用户原来的工作台编辑不会自己回来——要恢复就自己改回去。
// - 确认后再改：工作台卡的冒泡 input/change 清掉已确认标记（绿退橙，实现见 GenSettingsCard
//   的 onCardEdited）；再点一次「确认」才会把新值写回后端。

/**
 * 把一条队列行借给工作台表单：把该行参数直接灌进表单字段（覆盖现值，不留备份）。
 * id 为空 / 条目已不在队列 / 条目非排队中 → 清选中、退出借用态，返回 false。
 * @returns 是否真的进入了借用态；false = 调用方要提示用户（见下「能力位未知」）。
 */
export function queueBorrowForm(id: string | null): boolean {
  const entry = id ? imageaiStore.queue.find((e) => e.id === id) ?? null : null;
  if (!entry || entry.status !== 'queued') {
    imageaiStore.queueSelectedId = null;
    imageaiStore.queueBorrowing = false;
    return false;
  }
  // 前置检查：该行的服务商/模型还认得出来吗？条目入队后配置可能被删改（换 id、删模型），
  // 此时 modelInfo 为 null，queueFormRequest 里所有 `caps?.supportsXxx ? … : null` 会退化成 null——
  // **能力位未知不能按「不支持」处理**：那样一进借用就判脏，确认回去等于把 steps/guidance/
  // 改写/水印/输出格式这些参数静默清空。用户虽可在借用期把模型换掉自救，但那样这一行的
  // 参数已被整体改写（等于借着一份坏值重新组装），不如直接拒绝借用、让他在队列里先修配置。
  // 故宁可不借：与 modelInfo 的兜底同口径（有明细行、或等于服务商默认模型的通用档都算认得）。
  const provider = imageaiStore.config?.providers.find((p) => p.id === entry.req.providerId);
  const modelOk =
    (provider?.models.some((m) => m.model === entry.req.model) ?? false) ||
    provider?.defaultModel === entry.req.model;
  if (!provider || !modelOk) {
    imageaiStore.queueSelectedId = null;
    imageaiStore.queueBorrowing = false;
    return false;
  }
  imageaiStore.queueBorrowing = true;
  const req = entry.req;
  imageaiStore.providerId = req.providerId;
  imageaiStore.model = req.model;
  imageaiStore.templateId = req.templateId ?? 'default';
  imageaiStore.prompt = req.prompt;
  // 该行文本已定稿：标 dirty 让「重掷」不再用后端组装结果覆盖它。
  imageaiStore.promptDirty = true;
  imageaiStore.negative = req.negativePrompt;
  imageaiStore.size = req.size;
  imageaiStore.n = req.n;
  imageaiStore.quality = req.quality ?? 'auto';
  imageaiStore.seed = req.seed != null ? String(req.seed) : '';
  imageaiStore.steps = req.steps != null ? String(req.steps) : '';
  imageaiStore.guidance = req.guidance != null ? String(req.guidance) : '';
  imageaiStore.callMode = req.callMode === 'async' ? 'async' : 'sync';
  imageaiStore.promptExtend = req.promptExtend ?? true;
  imageaiStore.watermark = req.watermark ?? false;
  imageaiStore.enableThinking = req.enableThinking ?? true;
  imageaiStore.promptExtendMode = req.promptExtendMode ?? 'direct';
  imageaiStore.outputFormat = req.outputFormat === 'jpeg' ? 'jpeg' : 'png';
  imageaiStore.queueSelectedId = entry.id;
  return true;
}

/**
 * 按**当前工作台表单**组装一条 GenRequest（借用中才有效，否则 null）。
 * 映射规则与 buildQueueEntries 逐项一致：两处口径不同会让「单条确认」与「入队」发出不同参数。
 * 不含 uid/name/nationality/club（身份字段，表单没有，由调用方从条目续上）。
 * templateId 也不在这里返回：它是「这段文本出自哪个模板」的来源标记，由 queueConfirmEntry 取表单值补上。
 * caps 取当前 modelInfo——借用时 provider/model 已切到该行，故它就是该行的能力位。
 */
export function queueFormRequest(): GenRequest | null {
  if (!imageaiStore.queueBorrowing) return null;
  const caps = imageaiStore.modelInfo?.capabilities;
  return {
    providerId: imageaiStore.providerId,
    model: imageaiStore.model,
    prompt: imageaiStore.prompt,
    negativePrompt: imageaiStore.negative,
    size: imageaiStore.size,
    n: imageaiStore.n,
    quality: imageaiStore.quality === 'auto' ? null : imageaiStore.quality,
    seed: parseSeed(imageaiStore.seed),
    steps: caps?.supportsSteps ? parseSteps(imageaiStore.steps) : null,
    guidance: caps?.supportsGuidance ? parseGuidance(imageaiStore.guidance) : null,
    promptExtend: caps?.supportsPromptExtend ? imageaiStore.promptExtend : null,
    watermark: caps?.supportsWatermark ? imageaiStore.watermark : null,
    promptExtendMode: caps?.supportsPromptExtendMode ? imageaiStore.promptExtendMode : null,
    enableThinking: caps?.supportsEnableThinking ? imageaiStore.enableThinking : null,
    outputFormat: caps?.supportsOutputFormat ? imageaiStore.outputFormat : null,
    // 仅当模型支持异步且用户选了异步才传 'async'；否则传 null（后端默认同步）。
    callMode: caps?.supportsAsync && imageaiStore.callMode === 'async' ? 'async' : null,
  };
}

/** 选中一条队列行来核对参数：把该行借给工作台表单（见 queueBorrowForm）。
 *  传 null 或该 id 已不在队列里（快照刷新把它刷掉了）视为清空选中。
 *  @returns 借用是否成功；false 时调用方要提示「该行的服务商/模型已不在配置里」。 */
export function queueSelectEntry(id: string | null): boolean {
  return queueBorrowForm(id);
}

/** 清空选中行（**只清光标，不回滚表单**：见本节开头的语义说明）。切回自动模式、队列开跑、
 *  被选中的条目已消失时都要收口，否则右侧工作台会一直标着一条不再需要核对的参数。 */
export function queueClearSelection() {
  imageaiStore.queueSelectedId = null;
  imageaiStore.queueBorrowing = false;
}

/** 确认一条：把工作台表单组装的参数写回后端队列（快照整份替换）后打上已确认标记。
 *  表单只在「这一条正是被借用的那条」时生效——否则用户可能改了 A 却确认 B，写回的是 B 的旧参数。
 *  身份字段（uid/name/国籍/俱乐部）从条目续上：表单不含这些，也不该被改动。
 *  模板 id 却取**当前表单值**：借用期间模板选择器仍可用，换模板会把提示词重渲染成新模板的产出，
 *  此时若仍写回条目的旧 templateId，画廊记录会记错「这段文本来自哪个模板」。
 *  失败（IPC/锁等异常）原样抛给调用方展示；参数合法性不在这里判——启动时 `validate_entries` 兜底。 */
export async function queueConfirmEntry(id: string): Promise<void> {
  const entry = imageaiStore.queue.find((e) => e.id === id);
  if (!entry) return;
  const form = imageaiStore.queueSelectedId === id ? queueFormRequest() : null;
  const req: GenRequest = form
    ? {
        ...form,
        uid: entry.req.uid ?? null,
        name: entry.req.name ?? null,
        // 国籍/俱乐部沿用入队那一刻查名单记下的值，不在这里重查：确认的对象是「这条已入队任务」，
        // 它当初是按那份名单资料组装的；重查会把名单后来的变更混进一条早已定稿的任务里。
        // （buildQueueEntries 入队时查名单是另一回事——那是首次组装，与「复核既有任务」刻意不同口径。）
        nationality: entry.req.nationality ?? null,
        club: entry.req.club ?? null,
        templateId: imageaiStore.templateId,
      }
    : entry.req;
  await queueApplyEdits([{ id, req }]);
  imageaiStore.queueConfirmedIds[id] = true;
}

/** 本次要启动的 id 里还没确认的那些（顺序按 entries，便于「还有 N 条未确认」之外再定位到具体行）。
 *  不在这批 ids 里、或已不在队列里的 id 都不参与——启动判据只看真的要跑的那几条。 */
export function queuePendingConfirmIds(entries: QueueEntry[], ids: string[]): string[] {
  const wanted = new Set(ids);
  return entries
    .filter((e) => wanted.has(e.id) && !imageaiStore.queueConfirmedIds[e.id])
    .map((e) => e.id);
}

/** 请求取消：只置令牌，后端在「尚未提交」的检查点收手（见 QueueState.cancel 的取消口径）。 *  成功后进入「取消中」过渡态——按钮与状态文案据此切换，直到收尾事件或快照里无 running。 */
export async function queueCancel() {
  // 先置位再 await：`gen:queue:finished` 可能先到（handler 会把标志写 false），若反过来写就会
  // 把 false 覆盖回 true，此后不再有事件 → 按钮永久 disabled、提示常驻。
  imageaiStore.queueCancelling = true;
  try {
    // 后端把「未提交」的条目当场标成已取消并回传快照：直接套用，界面立刻变「已取消」，
    // 不必等执行中那条收尾时才有事件触发刷新。
    setQueue(await api.cancelQueue());
  } catch (e) {
    imageaiStore.queueCancelling = false; // 请求没成功：收回过渡态，让用户可以重试
    throw e;
  }
}

/** 本次要启动的条目 id：**有勾选只取勾选的排队条目，一条都没勾才取全部排队条目**
 *  （从名单「加入队列」后直接启动是主路径，不该强迫用户先勾选）。
 *  勾选里的非排队条目（成功/失败）不参与启动——那种选择只对「查看单人/移除」有意义。 */
export function queueStartIds(
  entries: QueueEntry[],
  picked: Record<string, boolean>,
): string[] {
  const queued = entries.filter((e) => e.status === "queued");
  const pickedQueued = queued.filter((e) => picked[e.id]).map((e) => e.id);
  return pickedQueued.length > 0 ? pickedQueued : queued.map((e) => e.id);
}

/** 队列里正在跑的那条（生产串行，至多一条）；没有则 null。
 *  单人卡与生图工作台据此切成「这条任务」的只读视图——用户不必再猜现在在跑谁、发的是什么。 */
export function runningQueueEntry(): QueueEntry | null {
  const byStatus = imageaiStore.queue.find((e) => e.status === "running");
  if (byStatus) return byStatus;
  // 快照刷新是 120ms 节流（trailing），换条瞬间还没落地：进度事件先到且带 entryId，
  // 用它立刻定位「现在跑的是哪条」，卡片与只读视图才不会滞后一整拍。
  const p = imageaiStore.queueProgress;
  if (p?.status === "running" && p.entryId) {
    return imageaiStore.queue.find((e) => e.id === p.entryId) ?? null;
  }
  return null;
}

/** 一条队列任务的生成配置摘要：服务商/模型 · 尺寸 · N 张 · 非空参数（未设置的项不占位）。
 *  只读展示用，判据与入队时的请求同源（直接读条目自带的 req）。 */
export function queueEntryConfig(e: QueueEntry): string {
  const r = e.req;
  return [
    `${r.providerId}/${r.model}`,
    r.size,
    r.n ? `${r.n} 张` : "",
    r.quality ? `quality ${r.quality}` : "",
    r.seed != null ? `seed ${r.seed}` : "",
    r.steps != null ? `steps ${r.steps}` : "",
    r.guidance != null ? `guidance ${r.guidance}` : "",
    r.callMode === "async" ? "async" : "",
    r.outputFormat === "jpeg" ? "jpeg" : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

/** 队列行的状态徽标类别。**已取消与失败必须分开**：两者数据上都是 status=failed，
 *  靠 errorKind=cancelled 区分——那是用户自己按的取消，不是跑挂了，文案与配色都不能共用。
 *  抽成纯函数是为了让这条判据只有一份并有单测兜底（组件只管取词与上色）。 */
export function queueBadgeKey(e: QueueEntry): QueueStatus | "cancelled" {
  return e.status === "failed" && e.errorKind === "cancelled" ? "cancelled" : e.status;
}

export function queueRemove(ids: string[]) {
  return api.removeFromQueue(ids).then(setQueue);
}

/** 是否给失败条目提供「重试本条」手动入口：网络/限流/服务商/内部错误、或旧快照未标注类别时为真；
 *  鉴权/参数/配置/内容审核/取消无意义（须先修配置或改提示词）。
 * 【与后端 GenError.retryable 的职责分工】二者判据不同、各有用途，**不要互相替换**：
 *  - 后端 `retryable` 是**自动重试**判据（队列执行期对同一请求自动再试）；
 *  - 本函数是**是否展示手动入口**的判据——对 provider/internal 类错误，即使后端不自动重试，
 *    让用户手动再试一次也始终合理。 */
export function isRetryableErrorKind(kind?: GenErrorKind | null): boolean {
  if (!kind) return true;
  return kind === 'network' || kind === 'rate_limited' || kind === 'provider' || kind === 'internal';
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
    errorKind: null,
    attempts: 0, // 全新任务：旧条目的尝试次数不得带入（后端 add 也会复位，这里显式对齐）
  };
  setQueue(await api.addToQueue([fresh]));
}

/** 是否展示「已重试 N 次」徽标：`attempts` 为总尝试次数（入队未跑为 0、首发为 1），
 *  仅 >1（即确实重试过）才显示。抽成纯函数以便单测（组件模板不便断言）。 */
export function shouldShowRetriedBadge(attempts?: number | null): boolean {
  return (attempts ?? 0) > 1;
}

/** 阶段事件应归属的条目 id：优先 entryId（同一球员重复入队时 uid 会重复、id 唯一），
 *  缺省回落到 uid（兼容不带该字段的旧事件）。抽成纯函数以便单测。 */
export function stageOwnerId(progress: Pick<QueueProgress, 'entryId' | 'uid'>): string {
  return progress.entryId ?? progress.uid;
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
    // 收尾即「取消中」结束：此时不可能还有在跑条目。
    imageaiStore.queueCancelling = false;
    // 立即拉全量快照并撤掉挂起的节流查询，免得 120ms 后再打一次等价 IPC。
    if (queueRefreshTimer) {
      clearTimeout(queueRefreshTimer);
      queueRefreshTimer = null;
    }
    void queueQuery().catch(queueErrorHandler);
    // 批量生成把记录写进 gallery.json，但画廊只在进工作台时拉过一次：此处补拉，否则新产出的记录
    // 要重进工作台才看得见（与队列快照同批刷新）。
    void loadGalleryRecords().catch(queueErrorHandler);
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
