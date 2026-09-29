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
  },
}));

// saveAndReplace 读「替换执行」面板共享的备份开关：替身成普通对象即可（store 只读这一个字段）。
vi.mock('./app.svelte', () => ({
  appStore: { saveBackup: true },
}));

import { api } from '../services/api';
import { appStore } from './app.svelte';
import {
  applyTemplate,
  buildQueueEntries,
  clearSelection,
  clearSourcePlayer,
  enqueuePlayers,
  generate,
  imageaiStore,
  saveAndReplace,
  selectProvider,
  setQueue,
  queueRetry,
  setSourcePlayer,
  toggleSelect,
  toggleSelectPage,
  validateSizeRules,
} from './imageai.svelte';
import type { ImageConfig, QueueEntry } from '../types';

const cfg: ImageConfig = {
  providers: [
    {
      id: 'p1', kind: 'custom', name: 'P1', baseUrl: 'http://x/v1', defaultModel: 'm2',
      models: [
        {
          model: 'm2', label: 'M2',
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
    vi.mocked(api.addToQueue).mockClear();
    clearSelection();
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

describe('queue store', () => {
  beforeEach(() => {
    vi.mocked(api.renderPrompt).mockClear();
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

  it('buildQueueEntries renders prompt defaults and maps workbench params', async () => {
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
    // prompt 走 render_prompt 展开「默认值版本」，占位符原文不得进请求
    expect(api.renderPrompt).toHaveBeenCalledWith('一张{{gender|男性}}脸');
    expect(entries[0].req.prompt).toBe('一张男性脸');
    expect(entries[1].req.prompt).toBe('一张男性脸');
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

  it('setSourcePlayer / clearSourcePlayer', () => {
    setSourcePlayer('u1', '甲');
    expect(imageaiStore.sourcePlayer).toEqual({ uid: 'u1', name: '甲' });
    clearSourcePlayer();
    expect(imageaiStore.sourcePlayer).toBeNull();
  });

  it('generate with a source player renders via previewPrompt and keeps the manual prompt', async () => {
    setSourcePlayer('u1', '甲');
    vi.mocked(api.previewPrompt).mockResolvedValueOnce({ text: '甲的专属提示词', missing: [] });
    await generate();
    expect(api.previewPrompt).toHaveBeenCalledWith('u1', 'tpl1');
    expect(api.renderPrompt).not.toHaveBeenCalled(); // 数据源模式不走手填渲染链
    // 请求携带渲染结果；工作台手填 prompt 不被回写（渲染基座是模板，回写会污染下一次渲染）
    expect(vi.mocked(api.generateImage).mock.calls[0][0].prompt).toBe('甲的专属提示词');
    expect(imageaiStore.prompt).toBe('一张{{gender|男性}}脸');
  });

  it('generate with a source player rejects when placeholders are missing', async () => {
    setSourcePlayer('u1', '甲');
    vi.mocked(api.previewPrompt).mockResolvedValueOnce({ text: '', missing: ['skinCode'] });
    await expect(generate()).rejects.toThrow('占位符未填充：skinCode');
    expect(api.generateImage).not.toHaveBeenCalled(); // missing 不得原文送进 API
  });

  it('generate without a source player keeps the manual renderPrompt flow', async () => {
    await generate();
    expect(api.renderPrompt).toHaveBeenCalledWith('一张{{gender|男性}}脸');
    expect(api.previewPrompt).not.toHaveBeenCalled();
  });

  it('buildQueueEntries renders per player via previewPrompt with a source player', async () => {
    setSourcePlayer('u0', '源');
    const entries = await buildQueueEntries([
      { uid: 'u1', name: '甲' },
      { uid: 'u2', name: '乙' },
    ]);
    // 队列逐球员渲染：每行各问一次 preview_prompt，prompt 各自携带该球员的变量
    expect(api.previewPrompt).toHaveBeenCalledTimes(2);
    expect(api.renderPrompt).not.toHaveBeenCalled();
    expect(entries[0].req.prompt).toBe('u1 的渲染结果');
    expect(entries[1].req.prompt).toBe('u2 的渲染结果');
  });

  it('buildQueueEntries rejects when a per-row preview reports missing placeholders', async () => {
    setSourcePlayer('u0', '源');
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

  it('queueRetry 移除旧条目并用同参数重新排队', async () => {
    const failed: QueueEntry = {
      id: 'q9', uid: 'u1', name: '甲',
      req: {
        providerId: 'p1', model: 'm2', prompt: '脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null,
      },
      status: 'failed', images: [], error: 'boom',
    };
    await queueRetry(failed);
    expect(api.removeFromQueue).toHaveBeenCalledWith(['q9']);
    const sent = vi.mocked(api.addToQueue).mock.calls.at(-1)?.[0];
    expect(sent).toHaveLength(1);
    expect(sent?.[0].id).toBe('');
    expect(sent?.[0].status).toBe('queued');
    expect(sent?.[0].error).toBe('');
    expect(sent?.[0].req).toEqual(failed.req);
  });
});
