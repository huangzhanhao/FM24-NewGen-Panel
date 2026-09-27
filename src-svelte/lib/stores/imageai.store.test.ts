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
    addToQueue: vi.fn(async (entries: QueueEntry[]) =>
      entries.map((e, i) => ({ ...e, id: `q${i + 1}` })),
    ),
    queryQueue: vi.fn(async () => []),
  },
}));

import { api } from '../services/api';
import {
  applyTemplate,
  buildQueueEntries,
  clearSelection,
  enqueuePlayers,
  imageaiStore,
  selectProvider,
  setQueue,
  toggleSelect,
  toggleSelectPage,
} from './imageai.svelte';
import type { ImageConfig, QueueEntry } from '../types';

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
