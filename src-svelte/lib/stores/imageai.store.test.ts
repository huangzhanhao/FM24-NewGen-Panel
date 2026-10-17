import { beforeEach, describe, expect, it, vi } from 'vitest';

// store 经 api 调后端做模板渲染/队列提交；单测里整模块替身化，避免触碰 Tauri invoke。
// 替身实现刻意保留「展开默认值」语义，好让断言覆盖 render_prompt 的接线效果；
// addToQueue 模拟后端 add：给条目补发 id、原样返回快照。
vi.mock('../services/api', () => ({
  api: {
    renderPrompt: vi.fn(async (body: string) => ({
      text: body.replace(/\{\{(\w+)\|([^}]*)\}\}/g, '$2'),
      missing: [],
    })),
    // P3 数据源：按 uid 拼一句可区分的渲染结果（个别用例会 mockResolvedValueOnce 覆盖）
    previewPrompt: vi.fn(async (uid: string) => ({ text: `${uid} 的渲染结果`, missing: [] })),
    // images 留空：generate 的 blob 加载循环不执行，无需替身 readImage
    generateImage: vi.fn(async () => ({ images: [] })),
    saveAndReplace: vi.fn(async () => ({ id: 'g1', path: '/tmp/x.png', bytes: 1 })),
    addToQueue: vi.fn(async (entries: QueueEntry[]) =>
      entries.map((e, i) => ({ ...e, id: `q${i + 1}` })),
    ),
    queryQueue: vi.fn(async () => []),
    removeFromQueue: vi.fn(async () => []),
    // 画廊记录：默认空表；个别用例 mockResolvedValueOnce 覆盖
    getGallery: vi.fn(async () => []),
    // loadImageState 拉取配置（默认给一份最小合法配置；用例可覆盖）
    getImageAiState: vi.fn(async () => ({
      providers: [],
      settings: { defaultProviderId: 'openai', historyLimit: 20, galleryRecordLimit: 100, concurrency: 2, namingPattern: '{uid}_{index}' },
      templates: [],
    })),
  },
}));

// saveAndReplace 读「替换执行」面板共享的备份开关：替身成普通对象即可（store 只读这一个字段）。
vi.mock('./app.svelte', () => ({
  appStore: { saveBackup: true },
}));

// formatError 只在弹窗侧用到 message；替身避免触碰 Tauri dialog 插件。
vi.mock('@tauri-apps/plugin-dialog', () => ({
  message: vi.fn(async () => {}),
  confirm: vi.fn(async () => true),
}));

import { api } from '../services/api';
import { appStore } from './app.svelte';
import { formatError } from '../ui/dialog';
import {
  applyTemplate,
  buildQueueEntries,
  clearSelection,
  clearSourcePlayer,
  customSizeStart,
  enqueuePlayers,
  generate,
  imageaiStore,
  isRetryableErrorKind,
  loadImageState,
  rerollPrompt,
  saveAndReplace,
  selectProvider,
  parseSeed,
  selectModel,
  setQueue,
  queueRetry,
  setSourcePlayer,
  shouldShowRetriedBadge,
  stageOwnerId,
  toggleSelect,
  toggleSelectPage,
  validateSizeRules,
  SEED_MAX,
} from './imageai.svelte';
import type { ImageConfig, QueueEntry } from '../types';

const cfg: ImageConfig = {
  providers: [
    {
      id: 'p1', kind: 'openai', name: 'P1', baseUrl: 'http://x/v1', defaultModel: 'm2',
      models: [
        {
          model: 'm2',
          capabilities: {
            sizes: ['768x768'], maxN: 4, supportsQuality: false, supportsSeed: true,
            supportsSteps: false, supportsNegativePrompt: true, supportsResponseFormat: true,
            qualities: [],
          },
        },
      ],
    },
  ],
  settings: { defaultProviderId: 'p1', historyLimit: 20, galleryRecordLimit: 100, concurrency: 2, namingPattern: '{uid}_{index}' },
  templates: [],
};

describe('image store', () => {
  beforeEach(() => {
    vi.mocked(api.renderPrompt).mockClear();
    vi.mocked(api.addToQueue).mockClear();
    clearSelection();
  });

  it('selectProvider resets model-dependent fields', () => {
    imageaiStore.config = cfg;    imageaiStore.size = 'old';
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

describe('queue store', () => {
  beforeEach(() => {
    vi.mocked(api.renderPrompt).mockClear();
    vi.mocked(api.previewPrompt).mockClear();
    vi.mocked(api.addToQueue).mockClear();
    clearSelection();
    imageaiStore.queue = [];
    imageaiStore.queueRunning = false;
  });

  it('toggleSelect / toggleSelectPage / clearSelection / selectedCount', () => {
    toggleSelect('u1'); // 缺省取反：false → true
    expect(imageaiStore.selectedInTable['u1']).toBe(true);
    toggleSelect('u1'); // 再点取消
    expect(imageaiStore.selectedInTable['u1']).toBeFalsy();
    toggleSelect('u1', true); // 显式传值
    toggleSelectPage(['u2', 'u3'], true);
    expect(imageaiStore.selectedCount).toBe(3);
    // 批量取消只动指定 uid：u3 保留勾选
    toggleSelectPage(['u1', 'u2'], false);
    expect(imageaiStore.selectedCount).toBe(1);
    clearSelection();
    expect(imageaiStore.selectedCount).toBe(0);
  });

  it('buildQueueEntries renders each player from the template and maps workbench params', async () => {
    imageaiStore.config = cfg;
    selectProvider('p1'); // model=m2, size=768x768, n=1, quality=auto, seed=''
    imageaiStore.prompt = '一张{{gender|男性}}脸';
    imageaiStore.negative = '模糊';
    imageaiStore.n = 2;
    imageaiStore.seed = '42';
    const entries = await buildQueueEntries([
      { uid: 'u1', name: '甲' },
      { uid: 'u2', name: '乙' },
    ]);
    expect(entries).toHaveLength(2);
    // 提示词逐球员按模板组装：各行变量不同，框内编辑不参与（占位符原文不得进请求）
    expect(api.previewPrompt).toHaveBeenCalledTimes(2);
    expect(api.renderPrompt).not.toHaveBeenCalled();
    expect(entries[0].req.prompt).toBe('u1 的渲染结果');
    expect(entries[1].req.prompt).toBe('u2 的渲染结果');
    // 字段映射：id 留空由后端补发；quality=auto → null；seed 数字化；uid 透传
    expect(entries[0]).toMatchObject({
      id: '', uid: 'u1', name: '甲', status: 'queued', images: [], error: '',
    });
    expect(entries[0].req).toMatchObject({
      providerId: 'p1', model: 'm2', negativePrompt: '模糊',
      size: '768x768', n: 2, quality: null, seed: 42, uid: 'u1',
    });
  });

  it('buildQueueEntries rejects when model is missing', async () => {
    imageaiStore.config = cfg;
    imageaiStore.model = '';
    await expect(buildQueueEntries([{ uid: 'u1', name: '甲' }])).rejects.toThrow();
  });

  it('enqueuePlayers submits entries and stores backend snapshot', async () => {
    imageaiStore.config = cfg;
    selectProvider('p1');
    imageaiStore.prompt = '脸';
    const n = await enqueuePlayers([{ uid: 'u9', name: '丙' }]);
    expect(n).toBe(1);
    expect(api.addToQueue).toHaveBeenCalledTimes(1);
    // 后端补发的 id 随快照写回 store——前端以此 id 定位移除/取消
    expect(imageaiStore.queue).toHaveLength(1);
    expect(imageaiStore.queue[0].id).toBe('q1');
    expect(imageaiStore.queue[0].status).toBe('queued');
  });

  it('setQueue derives queueRunning from the snapshot', () => {
    const base: QueueEntry = {
      id: 'q1', uid: 'u1', name: '甲',
      req: {
        providerId: 'p1', model: 'm2', prompt: '脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null,
      },
      status: 'queued', images: [], error: '',
    };
    setQueue([{ ...base, status: 'running' }]);
    expect(imageaiStore.queueRunning).toBe(true);
    setQueue([{ ...base, status: 'queued' }]);
    expect(imageaiStore.queueRunning).toBe(false);
  });
});

describe('P3 数据源与保存并替换', () => {
  beforeEach(() => {
    vi.mocked(api.renderPrompt).mockClear();
    vi.mocked(api.previewPrompt).mockClear();
    vi.mocked(api.generateImage).mockClear();
    vi.mocked(api.saveAndReplace).mockClear();
    clearSourcePlayer();
    imageaiStore.config = cfg;
    selectProvider('p1'); // model=m2, size=768x768, n=1, quality=auto, seed=''
    imageaiStore.templateId = 'tpl1';
    imageaiStore.prompt = '一张{{gender|男性}}脸';
  });

  /** 选人会让 store 异步组装并回填一次输入框；用例统一经此选人，并清掉这第一跳的调用计数，
   *  后续断言只统计用例自己触发的调用。 */
  async function pickSource(uid: string, name: string) {
    setSourcePlayer(uid, name);
    await new Promise((r) => setTimeout(r, 0));
    vi.mocked(api.previewPrompt).mockClear();
  }

  it('setSourcePlayer assembles into the box / clearSourcePlayer resets it', async () => {
    await pickSource('u1', '甲');
    expect(imageaiStore.sourcePlayer).toEqual({ uid: 'u1', name: '甲' });
    expect(imageaiStore.prompt).toBe('u1 的渲染结果'); // 选人即组装回填
    expect(imageaiStore.promptDirty).toBe(false);
    clearSourcePlayer();
    expect(imageaiStore.sourcePlayer).toBeNull();
  });

  it('generate with a source player rerolls via previewPrompt and writes the result back', async () => {
    await pickSource('u1', '甲');
    vi.mocked(api.previewPrompt).mockResolvedValueOnce({ text: '甲的专属提示词', missing: [] });
    await generate();
    expect(api.previewPrompt).toHaveBeenCalledWith('u1', 'tpl1');
    expect(api.renderPrompt).not.toHaveBeenCalled(); // 数据源模式不走手填渲染链
    expect(vi.mocked(api.generateImage).mock.calls[0][0].prompt).toBe('甲的专属提示词');
    // 组装结果回填输入框：所见即所发
    expect(imageaiStore.prompt).toBe('甲的专属提示词');
  });

  it('generate sends the hand-edited box text verbatim instead of rerolling', async () => {
    await pickSource('u1', '甲');
    imageaiStore.prompt = '我手改过的提示词';
    imageaiStore.promptDirty = true;
    await generate();
    // 改过就不重掷：以框内文本为准，不再向后端要一套新随机细节
    expect(api.previewPrompt).not.toHaveBeenCalled();
    expect(vi.mocked(api.generateImage).mock.calls[0][0].prompt).toBe('我手改过的提示词');
    expect(imageaiStore.promptDirty).toBe(false);
  });

  it('rerollPrompt reassembles from the source and covers hand edits', async () => {
    await pickSource('u1', '甲');
    imageaiStore.prompt = '我手改过的提示词';
    imageaiStore.promptDirty = true;
    vi.mocked(api.previewPrompt).mockResolvedValueOnce({ text: '重掷后的新组合', missing: [] });
    rerollPrompt();
    await new Promise((r) => setTimeout(r, 0));
    expect(api.previewPrompt).toHaveBeenCalledWith('u1', 'tpl1');
    expect(imageaiStore.prompt).toBe('重掷后的新组合');
    expect(imageaiStore.promptDirty).toBe(false); // 覆盖手改后回到「未编辑」态
  });

  it('rerollPrompt is a no-op without a source player', async () => {
    clearSourcePlayer();
    rerollPrompt();
    await new Promise((r) => setTimeout(r, 0));
    expect(api.previewPrompt).not.toHaveBeenCalled();
  });

  it('generate with a source player rejects when placeholders are missing', async () => {
    await pickSource('u1', '甲');
    vi.mocked(api.previewPrompt).mockResolvedValueOnce({ text: '', missing: ['skinCode'] });
    await expect(generate()).rejects.toThrow('占位符未填充：skinCode');
    expect(api.generateImage).not.toHaveBeenCalled(); // missing 不得原文送进 API
  });

  it('generate without a source player keeps the manual renderPrompt flow', async () => {
    await generate();
    expect(api.renderPrompt).toHaveBeenCalledWith('一张{{gender|男性}}脸');
    expect(api.previewPrompt).not.toHaveBeenCalled();
  });

  it('buildQueueEntries rejects when a per-row preview reports missing placeholders', async () => {
    await pickSource('u0', '源');
    vi.mocked(api.previewPrompt).mockResolvedValueOnce({ text: 'x', missing: ['skinCode'] });
    await expect(buildQueueEntries([{ uid: 'u1', name: '甲' }])).rejects.toThrow(
      '占位符未填充：skinCode',
    );
  });

  it('saveAndReplace passes the source uid and the shared backup switch', async () => {
    setSourcePlayer('u1', '甲');
    await saveAndReplace('img1');
    expect(api.saveAndReplace).toHaveBeenCalledWith('img1', 'u1', true);
    (appStore as unknown as { saveBackup: boolean }).saveBackup = false; // 关掉共享备份开关
    await saveAndReplace('img2');
    expect(api.saveAndReplace).toHaveBeenLastCalledWith('img2', 'u1', false);
    (appStore as unknown as { saveBackup: boolean }).saveBackup = true; // 还原，不污染其它用例
  });

  it('saveAndReplace without a source player throws', async () => {
    await expect(saveAndReplace('img1')).rejects.toThrow();
    expect(api.saveAndReplace).not.toHaveBeenCalled();
  });
});

describe('万相 V2 参数：自由尺寸校验与队列重试', () => {
  const wan25Rules = {
    totalMin: 1280 * 1280,
    totalMax: 1440 * 1440,
    ratioMin: 0.25,
    ratioMax: 4,
  };

  it('validateSizeRules 接受官方约束内尺寸（x 与 * 两种分隔符）', () => {
    expect(validateSizeRules('1280*1280', wan25Rules)).toBeNull();
    expect(validateSizeRules('768x2700', wan25Rules)).toBeNull();
  });

  it('validateSizeRules 拒绝总像素/比例/格式违规', () => {
    expect(validateSizeRules('1024*1024', wan25Rules)).not.toBeNull();
    expect(validateSizeRules('768*3100', wan25Rules)).not.toBeNull();
    expect(validateSizeRules('abc', wan25Rules)).not.toBeNull();
  });

  it('validateSizeRules 单边规则（wan2.2：512–1440）', () => {
    expect(validateSizeRules('512*1440', { minSide: 512, maxSide: 1440 })).toBeNull();
    expect(validateSizeRules('500*1024', { minSide: 512, maxSide: 1440 })).not.toBeNull();
    expect(validateSizeRules('1024*1441', { minSide: 512, maxSide: 1440 })).not.toBeNull();
  });

  it('customSizeStart：起步值直接取模型声明的像素下限，不再写死 350×350', () => {
    // 总像素下限开方 = 最小合法正方形边长：火山 5.0 Pro/Flash 921600 = 960²
    const volcanoPro = { totalMin: 921_600, totalMax: 4_624_220, ratioMin: 1 / 16, ratioMax: 16 };
    expect(customSizeStart(volcanoPro)).toEqual([960, 960]);
    expect(customSizeStart({ totalMin: 3_686_400, totalMax: 16_777_216 })).toEqual([1920, 1920]); // 火山 Lite/4.5
    expect(customSizeStart({ totalMin: 768 * 768, totalMax: 4096 * 4096 })).toEqual([768, 768]); // 万相 2.7
    // 只声明单边下限（万相 2.2）时用单边下限
    expect(customSizeStart({ minSide: 512, maxSide: 1440 })).toEqual([512, 512]);
    // 无任何像素下限（自建 OpenAI 兼容服务）：保持历史字面量
    expect(customSizeStart(null)).toEqual([350, 350]);
    expect(customSizeStart({ totalMin: 0, minSide: 0 })).toEqual([350, 350]);
    // 起步值必须落在规则内——否则「一点自定义就吃红线」的老毛病会以新形式回来
    const [w, h] = customSizeStart(volcanoPro);
    expect(validateSizeRules(`${w}x${h}`, volcanoPro)).toBeNull();
  });

  it('queueRetry 移除旧条目并用同参数重新排队', async () => {
    const failed: QueueEntry = {
      id: 'q9', uid: 'u1', name: '甲',
      req: {
        providerId: 'p1', model: 'm2', prompt: '脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null,
      },
      status: 'failed', images: [], error: 'boom',
      attempts: 2, // 旧条目重试过：重新排队必须把尝试次数清零
    };
    await queueRetry(failed);
    expect(api.removeFromQueue).toHaveBeenCalledWith(['q9']);
    const sent = vi.mocked(api.addToQueue).mock.calls.at(-1)?.[0];
    expect(sent).toHaveLength(1);
    expect(sent?.[0].id).toBe('');
    expect(sent?.[0].status).toBe('queued');
    expect(sent?.[0].error).toBe('');
    expect(sent?.[0].errorKind).toBeNull();
    expect(sent?.[0].attempts).toBe(0); // 归零：新任务不继承旧条目的尝试次数
    expect(sent?.[0].req).toEqual(failed.req);
  });

  it('shouldShowRetriedBadge：仅 attempts>1 显示徽标', () => {
    expect(shouldShowRetriedBadge(0)).toBe(false); // 入队未跑
    expect(shouldShowRetriedBadge(1)).toBe(false); // 首发即定稿
    expect(shouldShowRetriedBadge(2)).toBe(true); // 重试过
    expect(shouldShowRetriedBadge(3)).toBe(true);
    expect(shouldShowRetriedBadge(undefined)).toBe(false); // 旧快照无该字段
    expect(shouldShowRetriedBadge(null)).toBe(false);
  });

  it('stageOwnerId：优先 entryId，缺失回落 uid（重复入队时 uid 相同、id 不同）', () => {
    // 同一球员重复入队：两个条目 uid 都是 u1、id 分别为 e1/e2。
    // 若用 uid 命中，两个条目会同时显示阶段；用 entryId 才能唯一定位。
    expect(stageOwnerId({ entryId: 'e1', uid: 'u1' })).toBe('e1');
    expect(stageOwnerId({ entryId: 'e2', uid: 'u1' })).toBe('e2');
    // 旧事件不带 entryId：回落到 uid。
    expect(stageOwnerId({ uid: 'u1' })).toBe('u1');
    expect(stageOwnerId({ entryId: null, uid: 'u1' })).toBe('u1');
  });
});

describe('seed 官方取值范围（百炼 [0, 2147483647]）', () => {
  it('parseSeed：范围内取整，超上限与非法输入一律按随机', () => {
    expect(parseSeed('0')).toBe(0);
    expect(parseSeed(String(SEED_MAX))).toBe(SEED_MAX);
    expect(parseSeed('42.9')).toBe(42);
    expect(parseSeed(String(SEED_MAX + 1))).toBeNull();
    expect(parseSeed('')).toBeNull();
    expect(parseSeed('abc')).toBeNull();
    expect(parseSeed('-1')).toBeNull();
  });

  it('seedOutOfRange 只标记「看着合法但超上限」，供输入框标红', () => {
    imageaiStore.seed = '';
    expect(imageaiStore.seedOutOfRange).toBe(false);
    imageaiStore.seed = String(SEED_MAX);
    expect(imageaiStore.seedOutOfRange).toBe(false);
    imageaiStore.seed = String(SEED_MAX + 1);
    expect(imageaiStore.seedOutOfRange).toBe(true);
    // 非数字/负数走既有的「非法即随机」契约，用户一眼知道自己填错了，不额外标红。
    imageaiStore.seed = 'abc';
    expect(imageaiStore.seedOutOfRange).toBe(false);
    imageaiStore.seed = '';
  });
});

describe('promptOver（提示词超限派生，D3 跨组件单点）', () => {
  beforeEach(() => {
    imageaiStore.config = cfg;
    imageaiStore.providerId = 'p1';
    imageaiStore.model = 'm2';
  });

  it('模型未声明 maxPromptChars：无上限，恒不超限', () => {
    imageaiStore.prompt = 'x'.repeat(50);
    expect(imageaiStore.promptOver).toBe(false);
  });

  it('超出 maxPromptChars：超限；恰好等于上限：不超限', () => {
    imageaiStore.config = {
      ...cfg,
      providers: [
        {
          ...cfg.providers[0],
          models: [
            {
              model: 'mlimit',
              capabilities: {
                ...cfg.providers[0].models[0].capabilities,
                maxPromptChars: 10,
              },
            },
          ],
        },
      ],
    };
    imageaiStore.model = 'mlimit';
    imageaiStore.prompt = 'x'.repeat(11);
    expect(imageaiStore.promptOver).toBe(true);
    imageaiStore.prompt = 'x'.repeat(10);
    expect(imageaiStore.promptOver).toBe(false);
  });
});

describe('画廊记录（A1：持久化 + 双闸门前端接线）', () => {
  beforeEach(() => {
    imageaiStore.config = cfg;
    selectProvider('p1'); // model=m2, size=768x768, n=1, quality=auto, seed=''
    imageaiStore.templateId = 'tpl1';
    imageaiStore.prompt = '一张{{gender|男性}}脸';
    imageaiStore.results = [];
    imageaiStore.records = [];
    clearSourcePlayer();
    vi.mocked(api.generateImage).mockClear();
    vi.mocked(api.renderPrompt).mockClear();
    vi.mocked(api.getGallery).mockClear();
  });

  it('generate 单张路径把数据源球员的 uid/name 带入请求', async () => {
    setSourcePlayer('u1', '甲');
    vi.mocked(api.previewPrompt).mockResolvedValueOnce({ text: '甲的提示词', missing: [] });
    vi.mocked(api.generateImage).mockResolvedValueOnce({ images: [] });
    await generate();
    const req = vi.mocked(api.generateImage).mock.calls[0][0];
    expect(req.uid).toBe('u1');
    expect(req.name).toBe('甲');
    expect(req.templateId).toBe('tpl1');
  });

  it('buildQueueEntries 逐行把球员名带入请求', async () => {
    const entries = await buildQueueEntries([
      { uid: 'u1', name: '甲' },
      { uid: 'u2', name: '乙' },
    ]);
    expect(entries[0].req.name).toBe('甲');
    expect(entries[1].req.name).toBe('乙');
    expect(entries[0].req.templateId).toBe('tpl1');
  });

  it('M1：请求携带当前模板 id（单张与队列、非硬编码）', async () => {
    imageaiStore.templateId = 'tpl-m1';
    setSourcePlayer('u1', '甲');
    vi.mocked(api.previewPrompt).mockResolvedValueOnce({ text: 'x', missing: [] });
    vi.mocked(api.generateImage).mockResolvedValueOnce({ images: [] });
    await generate();
    expect(vi.mocked(api.generateImage).mock.calls[0][0].templateId).toBe('tpl-m1');

    imageaiStore.templateId = 'tpl-m2'; // 换一个：证明是取当前值而非写死
    const entries = await buildQueueEntries([{ uid: 'u2', name: '乙' }]);
    expect(entries[0].req.templateId).toBe('tpl-m2');
  });

  it('gallery 合并本会话 results 与载入 records，并按图 id 去重（results 优先）', () => {
    imageaiStore.results = [{ id: 'x', path: '/p/x.png', url: 'blob:x' }];
    imageaiStore.records = [
      { id: 'x', path: '/p/x.png', url: 'blob:x-old', imageAvailable: true, removable: false },
      { id: 'y', path: '', url: '', imageAvailable: false, removable: false },
    ];
    const g = imageaiStore.gallery;
    expect(g.map((i) => i.id)).toEqual(['x', 'y']);
    expect(g[0].url).toBe('blob:x'); // 去重保留本会话结果
    expect(g[1].imageAvailable).toBe(false);
  });

  it('loadImageState 载入持久记录并入画廊（缺失图按占位）', async () => {
    imageaiStore.config = null; // 触发真实拉取
    imageaiStore.records = [];
    imageaiStore.results = [];
    vi.mocked(api.getImageAiState).mockResolvedValueOnce(cfg);
    vi.mocked(api.getGallery).mockResolvedValueOnce([
      {
        id: 'g1', ts: '2026-09-26 10:00:00', uid: 'u1', name: '甲',
        model: 'm', prompt: 'p', negativePrompt: '', templateId: '',
        imageAvailable: false, path: null,
      },
    ]);
    await loadImageState();
    expect(imageaiStore.records).toHaveLength(1);
    expect(imageaiStore.records[0].imageAvailable).toBe(false);
    expect(imageaiStore.records[0].removable).toBe(false);
    expect(imageaiStore.gallery.map((i) => i.id)).toContain('g1');
  });
});

describe('生成错误分类（A2：格式化与重试判定）', () => {
  it('formatError：Error 取 message，字符串原样，其余强转', () => {
    expect(formatError(new Error('boom'))).toBe('boom');
    expect(formatError('已取消')).toBe('已取消');
    expect(formatError(42)).toBe('42');
  });

  it('formatError：结构化错误取 message 并追加 userAction 建议', () => {
    const withAction = {
      kind: 'auth',
      message: '鉴权失败',
      retryable: false,
      userAction: '检查 API Key 是否正确或已过期',
    };
    expect(formatError(withAction)).toBe('鉴权失败\n建议：检查 API Key 是否正确或已过期');
    // 无建议类别（cancelled/internal 的 userAction 为 null）只给 message
    const noAction = {
      kind: 'internal',
      message: '队列正在执行中',
      retryable: false,
      userAction: null,
    };
    expect(formatError(noAction)).toBe('队列正在执行中');
  });

  it('isRetryableErrorKind：可重试类别与缺失类别为真，其余为假', () => {
    for (const k of ['network', 'rate_limited', 'provider', 'internal'] as const) {
      expect(isRetryableErrorKind(k)).toBe(true);
    }
    expect(isRetryableErrorKind(undefined)).toBe(true);
    expect(isRetryableErrorKind(null)).toBe(true);
    for (const k of [
      'auth',
      'invalid_param',
      'not_configured',
      'content_rejected',
      'cancelled',
      'io',
    ] as const) {
      expect(isRetryableErrorKind(k)).toBe(false);
    }
  });
});

describe('调用方式 callMode（A4：前端门控）', () => {
  // 同一服务商两个模型：m2 支持异步、m-sync 不支持，用于验证「能力位 + 用户选择」双门控。
  const asyncCfg: ImageConfig = {
    ...cfg,
    providers: [
      {
        ...cfg.providers[0],
        models: [
          {
            model: 'm2',
            capabilities: { ...cfg.providers[0].models[0].capabilities, supportsAsync: true },
          },
          {
            model: 'm-sync',
            capabilities: { ...cfg.providers[0].models[0].capabilities },
          },
        ],
      },
    ],
  };

  beforeEach(() => {
    imageaiStore.config = asyncCfg;
    imageaiStore.prompt = '脸';
    vi.mocked(api.generateImage).mockClear();
    vi.mocked(api.renderPrompt).mockClear();
    clearSourcePlayer();
  });

  it('selectModel 复位 callMode 为 sync', () => {
    selectProvider('p1');
    imageaiStore.callMode = 'async';
    selectModel('m-sync');
    expect(imageaiStore.callMode).toBe('sync');
  });

  it('selectProvider 复位 callMode 为 sync', () => {
    imageaiStore.callMode = 'async';
    selectProvider('p1');
    expect(imageaiStore.callMode).toBe('sync');
  });

  it('generate：支持异步的模型 + callMode=async 才把 async 送进请求', async () => {
    selectProvider('p1');
    selectModel('m2'); // supportsAsync=true
    imageaiStore.callMode = 'async';
    await generate();
    expect(vi.mocked(api.generateImage).mock.calls[0][0].callMode).toBe('async');
  });

  it('generate：模型不支持异步时即使 callMode=async 也送 null', async () => {
    selectProvider('p1');
    selectModel('m-sync'); // supportsAsync 缺省 false
    imageaiStore.callMode = 'async'; // 强行置位，模拟脏状态
    await generate();
    expect(vi.mocked(api.generateImage).mock.calls[0][0].callMode).toBeNull();
  });

  it('buildQueueEntries：同样按能力位门控 callMode', async () => {
    selectProvider('p1');
    selectModel('m2');
    imageaiStore.callMode = 'async';
    const entries = await buildQueueEntries([{ uid: 'u1', name: '甲' }]);
    expect(entries[0].req.callMode).toBe('async');

    selectModel('m-sync'); // 换不支持异步的模型
    imageaiStore.callMode = 'async'; // 再次强行置位
    const sync = await buildQueueEntries([{ uid: 'u2', name: '乙' }]);
    expect(sync[0].req.callMode).toBeNull();
  });
});

describe('输出格式 outputFormat（火山 Seedream / 硅基流动：能力位门控 + 请求带值）', () => {
  // 同一服务商两个模型：m2 可设 output_format、m-noformat 不可设，验证「勾了才能选、选了才发」。
  const fmtCfg: ImageConfig = {
    ...cfg,
    providers: [
      {
        ...cfg.providers[0],
        models: [
          {
            model: 'm2',
            capabilities: { ...cfg.providers[0].models[0].capabilities, supportsOutputFormat: true },
          },
          { model: 'm-noformat', capabilities: { ...cfg.providers[0].models[0].capabilities } },
        ],
      },
    ],
  };

  beforeEach(() => {
    imageaiStore.config = fmtCfg;
    imageaiStore.prompt = '脸';
    clearSourcePlayer();
    vi.mocked(api.generateImage).mockClear();
    vi.mocked(api.renderPrompt).mockClear();
  });

  it('selectProvider / selectModel 都把选择归位默认 png', () => {
    selectProvider('p1');
    selectModel('m2');
    imageaiStore.outputFormat = 'jpeg'; // 用户选择
    selectModel('m-noformat');
    expect(imageaiStore.outputFormat).toBe('png');
  });

  it('generate：声明可设的模型带当前选择；未声明的模型送 null', async () => {
    selectProvider('p1');
    selectModel('m2'); // supportsOutputFormat=true
    imageaiStore.outputFormat = 'jpeg';
    await generate();
    expect(vi.mocked(api.generateImage).mock.calls[0][0].outputFormat).toBe('jpeg');

    selectModel('m-noformat'); // 不支持该参数
    await generate();
    expect(vi.mocked(api.generateImage).mock.calls[1][0].outputFormat).toBeNull();
  });

  it('buildQueueEntries：同一条能力位门控', async () => {
    selectProvider('p1');
    selectModel('m2');
    imageaiStore.outputFormat = 'jpeg';
    const entries = await buildQueueEntries([{ uid: 'u1', name: '甲' }]);
    expect(entries[0].req.outputFormat).toBe('jpeg');

    selectModel('m-noformat');
    const none = await buildQueueEntries([{ uid: 'u2', name: '乙' }]);
    expect(none[0].req.outputFormat).toBeNull();
  });
});
