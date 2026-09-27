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
