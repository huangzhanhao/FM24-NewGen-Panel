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
    saveAndReplace: vi.fn(async () => ({
      image: { id: 'g1', path: '/tmp/x.png', bytes: 1 },
      replaced: true,
    })),
    addToQueue: vi.fn(async (entries: QueueEntry[]) =>
      entries.map((e, i) => ({ ...e, id: `q${i + 1}` })),
    ),
    queryPlayersByUids: vi.fn(async () => []),
    queryQueue: vi.fn(async () => []),
    removeFromQueue: vi.fn(async () => []),
    startQueue: vi.fn(async () => {}),
    cancelQueue: vi.fn(async () => [] as QueueEntry[]),
    // 手动确认写回参数：后端只改排队条目并回全量快照
    updateQueueEntries: vi.fn(async () => [] as QueueEntry[]),
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
  queueApplyEdits,
  queueBadgeKey,
  queueBorrowForm,
  queueCancel,
  queueClearSelection,
  queueConfirmEntry,
  queueEntryConfig,
  queueFormRequest,
  queuePendingConfirmIds,
  queueRetry,
  queueSelectEntry,
  runningQueueEntry,
  queueStart,
  queueStartIds,
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

    // 预设为空的自由尺寸服务商：起步值取该模型声明的像素下限，不能落 350×350——那是「自定义…」
    // 输入框的起步值，一进这类服务商就开局报红。
    imageaiStore.config = {
      ...cfg,
      providers: [
        ...cfg.providers,
        {
          id: 'p2', kind: 'openai', name: 'P2', baseUrl: 'http://x/v1', defaultModel: 'mfree',
          models: [
            {
              model: 'mfree',
              capabilities: {
                sizes: [], maxN: 1, supportsQuality: false, supportsSeed: true,
                supportsSteps: false, supportsNegativePrompt: false, supportsResponseFormat: false,
                qualities: [], sizeRules: { totalMin: 768 * 768 },
              },
            },
          ],
        },
      ],
    };
    selectProvider('p2');
    expect(imageaiStore.size).toBe('768x768');
    // 起步值必须落在该模型的尺寸规则内：否则一进这类服务商卡片就红字。
    expect(imageaiStore.sizeError).toBeNull();
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
    // cancelQueue 用了一次性替身（mockImplementationOnce / mockRejectedValueOnce）：`mockClear()`
    // **不清**once 队列，若某次断言在消费它之前就抛错，残留会泄漏到随后用例。故 reset 后重挂默认实现。
    vi.mocked(api.cancelQueue).mockReset();
    vi.mocked(api.cancelQueue).mockImplementation(async () => []);
    vi.mocked(api.updateQueueEntries).mockReset();
    vi.mocked(api.updateQueueEntries).mockImplementation(async () => []);
    clearSelection();
    imageaiStore.queue = [];
    imageaiStore.queueRunning = false;
    imageaiStore.queueCancelling = false; // 过渡态不得跨用例携带
    imageaiStore.queueProgress = null; // 进度载荷会参与「在跑哪条」的判定，不得跨用例
    // 逐条确认的会话态同样不得跨用例：残留的选中/借用会让下一条用例误判「还需不需要确认」。
    queueClearSelection();
    imageaiStore.queueConfirmedIds = {};
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

  it('runningQueueEntry / queueEntryConfig：只读视图取到在跑那条的配置', () => {
    const mk = (id: string, status: QueueEntry['status'], req: Partial<QueueEntry['req']> = {}): QueueEntry => ({
      id, uid: id, name: id,
      req: {
        providerId: 'p1', model: 'm2', prompt: '脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null, ...req,
      },
      status, images: [], error: '',
    });
    // 无 running → null；有 → 返回那一条（串行下至多一条）
    setQueue([mk('q1', 'queued'), mk('q2', 'success')]);
    expect(runningQueueEntry()).toBeNull();
    const running = mk('q3', 'running');
    setQueue([mk('q1', 'queued'), running]);
    expect(runningQueueEntry()?.id).toBe('q3');

    // 快照滞后（120ms 节流）时，用进度事件里的 entryId 兜底：换条瞬间也要指到新那条
    setQueue([mk('q1', 'queued')]); // 快照里还没有 running（正是要覆盖的滞后形态）
    imageaiStore.queueProgress = { status: 'running', current: 0, total: 2, uid: 'q1', entryId: 'q1', stage: 'submitting' };
    expect(runningQueueEntry()?.id).toBe('q1');
    // 终态事件（非 running）不参与兜底
    imageaiStore.queueProgress = { status: 'success', current: 1, total: 2, uid: 'q1', entryId: 'q1' };
    expect(runningQueueEntry()).toBeNull();

    // 配置摘要只列非空参数：默认项不该冒出 seed/steps 之类的空值
    expect(queueEntryConfig(running)).toBe('p1/m2 · 768x768 · 1 张');
    expect(
      queueEntryConfig(mk('q4', 'running', { seed: 42, steps: 30, quality: 'hd', callMode: 'async', n: 2 })),
    ).toBe('p1/m2 · 768x768 · 2 张 · quality hd · seed 42 · steps 30 · async');
  });

  it('queueStartIds：勾选优先，未勾选才跑全部排队条目', () => {
    const mk = (id: string, status: QueueEntry['status']): QueueEntry => ({
      id, uid: id, name: id,
      req: {
        providerId: 'p1', model: 'm2', prompt: '脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null,
      },
      status, images: [], error: '',
    });
    const entries = [mk('q1', 'queued'), mk('q2', 'queued'), mk('q3', 'queued'), mk('q4', 'running')];
    // 有勾选：只取勾选的排队条目（勾选里的非排队条目被排除）
    expect(queueStartIds(entries, { q1: true, q2: true, q4: true })).toEqual(['q1', 'q2']);
    // 一条都没勾：跑全部排队条目（不含运行中/终态）
    expect(queueStartIds(entries, {})).toEqual(['q1', 'q2', 'q3']);
    // 只勾了非排队条目：同样回落到全部排队条目，避免出现「点了启动却什么都没跑」
    expect(queueStartIds(entries, { q4: true })).toEqual(['q1', 'q2', 'q3']);
  });

  it('queueBadgeKey：已取消（failed+cancelled）与失败分开成两档', () => {
    const base: QueueEntry = {
      id: 'q1', uid: 'u1', name: '甲',
      req: {
        providerId: 'p1', model: 'm2', prompt: '脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null,
      },
      status: 'queued', images: [], error: '',
    };
    expect(queueBadgeKey(base)).toBe('queued');
    expect(queueBadgeKey({ ...base, status: 'running' })).toBe('running');
    expect(queueBadgeKey({ ...base, status: 'success' })).toBe('success');
    // 用户取消与真失败在数据上同为 failed，徽标必须分开（否则取消会被显示成「失败」）。
    expect(queueBadgeKey({ ...base, status: 'failed' })).toBe('failed');
    expect(queueBadgeKey({ ...base, status: 'failed', errorKind: 'cancelled' })).toBe('cancelled');
  });

  it('queueApplyEdits 用后端返回的全量快照整份替换本地队列（queueAutoConfirm 默认全自动）', async () => {
    expect(imageaiStore.queueAutoConfirm).toBe(true);
    const before: QueueEntry = {
      id: 'q1', uid: 'u1', name: '甲',
      req: {
        providerId: 'p1', model: 'm2', prompt: '脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null,
      },
      status: 'queued', images: [], error: '',
    };
    setQueue([before]);
    const edited: QueueEntry = { ...before, req: { ...before.req, prompt: '改过的提示词' } };
    vi.mocked(api.updateQueueEntries).mockResolvedValueOnce([edited]);
    await queueApplyEdits([{ id: before.id, req: edited.req }]);
    // 入参形状是 IPC 契约（id + 完整 req），改动一条不顺手改别的字段
    expect(vi.mocked(api.updateQueueEntries)).toHaveBeenCalledWith([{ id: 'q1', req: edited.req }]);
    expect(imageaiStore.queue).toEqual([edited]);
  });

  it('queueBorrowForm：把该行参数直接灌入原表单；换行直接覆盖（不留备份）', () => {
    imageaiStore.config = cfg;
    selectProvider('p1');
    // 用户自己的工作台编辑：借用时被该行参数直接覆盖（约定：切行=直接覆盖，不做备份）
    imageaiStore.prompt = '用户自己的提示词';
    imageaiStore.negative = '用户负向';
    imageaiStore.n = 3;
    imageaiStore.seed = '7';
    imageaiStore.promptDirty = false;
    const mk = (id: string, req: Partial<QueueEntry['req']>): QueueEntry => ({
      id, uid: `u-${id}`, name: `球员${id}`,
      req: {
        providerId: 'p1', model: 'm2', prompt: '甲的脸', negativePrompt: '模糊', size: '768x768',
        n: 2, quality: 'high', seed: 42, callMode: 'sync', templateId: 't1', ...req,
      },
      status: 'queued', images: [], error: '',
    });
    setQueue([mk('q1', {}), mk('q2', { quality: null, prompt: '乙的脸' })]);

    expect(queueBorrowForm('q1')).toBe(true);
    expect(imageaiStore.queueBorrowing).toBe(true);
    expect(imageaiStore.queueSelectedId).toBe('q1');
    // 逐条映射（数值参数一律字符串化：表单里是字符串输入框）
    expect(imageaiStore).toMatchObject({
      providerId: 'p1', model: 'm2', templateId: 't1', prompt: '甲的脸', negative: '模糊',
      size: '768x768', n: 2, quality: 'high', seed: '42', steps: '', guidance: '',
      callMode: 'sync', promptExtend: true, watermark: false, enableThinking: true,
      promptExtendMode: 'direct', outputFormat: 'png',
    });
    // 该行文本已定稿 → 标 dirty，重掷不得再用后端组装结果覆盖它
    expect(imageaiStore.promptDirty).toBe(true);

    // 换行：直接覆盖表单（上一行没点确认，改动随之丢弃）；没有任何快照可回流
    queueBorrowForm('q2');
    expect(imageaiStore.queueSelectedId).toBe('q2');
    expect(imageaiStore.prompt).toBe('乙的脸');
    // quality=null 回落 'auto'（提交时再映射回 null）
    expect(imageaiStore.quality).toBe('auto');
    expect(imageaiStore.seed).toBe('42');
    // 用户原本那份编辑已被覆盖，不会自己回来
    expect(imageaiStore.prompt).not.toBe('用户自己的提示词');

    // 不存在的 id / 非排队条目 → 清选中、退出借用态，返回 false
    expect(queueBorrowForm('gone')).toBe(false);
    expect(imageaiStore.queueBorrowing).toBe(false);
    expect(imageaiStore.queueSelectedId).toBeNull();
  });

  it('queueConfirmEntry：从当前表单组装 req 写回 → 快照整份替换 → 打上已确认标记', async () => {
    imageaiStore.config = cfg;
    selectProvider('p1');
    const before: QueueEntry = {
      id: 'q1', uid: 'u1', name: '甲',
      req: {
        providerId: 'p1', model: 'm2', prompt: '甲的脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null, uid: 'u1', name: '甲', templateId: 't1',
      },
      status: 'queued', images: [], error: '',
    };
    setQueue([before]);
    queueBorrowForm('q1');
    // 用户在借用来的原表单里改：提示词与 seed
    imageaiStore.prompt = '确认后的脸';
    imageaiStore.seed = '42';
    // 借用期间换了模板：提示词已被新模板重渲染 → 写回的 templateId 必须跟着表单走，
    // 否则画廊记录会把这段文本记到旧模板名下。
    imageaiStore.templateId = 't2';
    const edited: QueueEntry = { ...before, req: { ...before.req, prompt: '确认后的脸', seed: 42 } };
    vi.mocked(api.updateQueueEntries).mockResolvedValueOnce([edited]);
    await queueConfirmEntry('q1');
    // 写回的是表单值（quality 'auto' → null、seed '42' → 42、templateId 取表单），身份字段从条目续上
    expect(vi.mocked(api.updateQueueEntries)).toHaveBeenCalledWith([
      {
        id: 'q1',
        req: expect.objectContaining({
          prompt: '确认后的脸', quality: null, seed: 42, size: '768x768', n: 1,
          uid: 'u1', name: '甲', templateId: 't2',
        }),
      },
    ]);
    expect(imageaiStore.queue).toEqual([edited]); // 后端返回的全量快照整份替换本地镜像
    expect(imageaiStore.queueConfirmedIds['q1']).toBe(true);

    // 未借用该条时用条目自带的 req：避免「改了 A 却确认 B」把 B 的参数当表单值写回
    queueClearSelection();
    vi.mocked(api.updateQueueEntries).mockClear();
    vi.mocked(api.updateQueueEntries).mockResolvedValueOnce([edited]);
    await queueConfirmEntry('q1');
    expect(vi.mocked(api.updateQueueEntries)).toHaveBeenCalledWith([{ id: 'q1', req: edited.req }]);

    // caps 支持 steps/guidance 时必须照发（能力位若读错会退化成 null，把参数静默丢掉）
    imageaiStore.config = {
      ...cfg,
      providers: [
        {
          ...cfg.providers[0],
          models: [
            ...cfg.providers[0].models,
            {
              model: 'm3',
              capabilities: {
                ...cfg.providers[0].models[0].capabilities,
                supportsSteps: true,
                supportsGuidance: true,
              },
            },
          ],
        },
      ],
    };
    const sampling: QueueEntry = {
      id: 'q2', uid: 'u2', name: '乙',
      req: {
        providerId: 'p1', model: 'm3', prompt: '乙的脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null, steps: 30, guidance: 3.5,
      },
      status: 'queued', images: [], error: '',
    };
    setQueue([sampling]);
    queueBorrowForm('q2');
    // 表单里是字符串，组装回去仍是数字（不是 null）——这是「参数照发」的唯一判据
    expect(imageaiStore).toMatchObject({ steps: '30', guidance: '3.5' });
    expect(queueFormRequest()).toMatchObject({ steps: 30, guidance: 3.5 });
    vi.mocked(api.updateQueueEntries).mockClear();
    vi.mocked(api.updateQueueEntries).mockResolvedValueOnce([sampling]);
    await queueConfirmEntry('q2');
    expect(vi.mocked(api.updateQueueEntries)).toHaveBeenCalledWith([
      { id: 'q2', req: expect.objectContaining({ steps: 30, guidance: 3.5 }) },
    ]);
  });

  it('queueClearSelection：只清选中，不回滚表单（刻意取舍，不是漏做）', () => {
    imageaiStore.config = cfg;
    selectProvider('p1');
    imageaiStore.prompt = '用户自己的提示词';
    imageaiStore.negative = '用户负向';
    imageaiStore.size = '512x512';
    imageaiStore.n = 4;
    imageaiStore.seed = '9';
    imageaiStore.promptDirty = false;
    const entry: QueueEntry = {
      id: 'q1', uid: 'u1', name: '甲',
      req: {
        providerId: 'p1', model: 'm2', prompt: '甲的脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null,
      },
      status: 'queued', images: [], error: '',
    };
    setQueue([entry]);
    queueSelectEntry('q1');
    expect(imageaiStore.queueBorrowing).toBe(true);
    // 借用期间表单已切成该行的值
    expect(imageaiStore).toMatchObject({ size: '768x768', n: 1, seed: '', steps: '', guidance: '' });
    queueClearSelection();
    expect(imageaiStore.queueSelectedId).toBeNull();
    expect(imageaiStore.queueBorrowing).toBe(false);
    // 表单**不回滚**：停在该行的参数上；用户原来的编辑不会自己回来（要恢复得自己改回去）。
    expect(imageaiStore).toMatchObject({
      providerId: 'p1', model: 'm2', prompt: '甲的脸', negative: '', size: '768x768', n: 1, seed: '',
    });
    // 传 null / 失效 id（快照里已没有这条）同样收口
    queueSelectEntry('gone');
    expect(imageaiStore.queueSelectedId).toBeNull();
    expect(imageaiStore.queueBorrowing).toBe(false);

    // 该行入队后服务商/模型已从配置里删改：拒绝借用（能力位未知时按「不支持」处理会把参数清空），
    // 表单保持原样、不进借用态，由调用方提示用户
    imageaiStore.prompt = '用户自己的提示词';
    setQueue([{ ...entry, id: 'q9', req: { ...entry.req, providerId: 'gone-provider' } }]);
    expect(queueBorrowForm('q9')).toBe(false);
    expect(imageaiStore.queueBorrowing).toBe(false);
    expect(imageaiStore.queueSelectedId).toBeNull();
    expect(imageaiStore.prompt).toBe('用户自己的提示词'); // 没有被那条的提示词污染
    setQueue([{ ...entry, id: 'q8', req: { ...entry.req, model: 'gone-model' } }]);
    expect(queueBorrowForm('q8')).toBe(false);
    expect(imageaiStore.queueBorrowing).toBe(false);
  });

  it('queuePendingConfirmIds：只挑未确认的，顺序按 entries，忽略不在这批/已不在队列的 id', () => {
    const mk = (id: string): QueueEntry => ({
      id, uid: `u-${id}`, name: `球员${id}`,
      req: {
        providerId: 'p1', model: 'm2', prompt: '脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null,
      },
      status: 'queued', images: [], error: '',
    });
    const entries = [mk('q1'), mk('q2'), mk('q3'), mk('q4')];
    // 勾选 3 条、确认 1 条 → 另 2 条待确认（顺序按 entries，与勾选顺序无关）
    imageaiStore.queueConfirmedIds = { q2: true };
    expect(queuePendingConfirmIds(entries, ['q3', 'q1', 'q2'])).toEqual(['q1', 'q3']);
    // 不存在于 entries 的 id（快照已刷新掉）忽略
    expect(queuePendingConfirmIds(entries, ['qGone', 'q1'])).toEqual(['q1']);
    // 全确认 → 空数组（启动放行）
    imageaiStore.queueConfirmedIds = { q1: true, q2: true, q3: true };
    expect(queuePendingConfirmIds(entries, ['q1', 'q2', 'q3'])).toEqual([]);
  });

  it('queueCancel 套用后端快照（排队中→已取消、在跑保留生成中）并管理「取消中」', async () => {
    const base: QueueEntry = {
      id: 'q1', uid: 'u1', name: '甲',
      req: {
        providerId: 'p1', model: 'm2', prompt: '脸', negativePrompt: '', size: '768x768',
        n: 1, quality: null, seed: null,
      },
      status: 'queued', images: [], error: '',
    };
    const running = { ...base, status: 'running' as const };
    const queued = { ...base, id: 'q2', uid: 'u2', name: '乙', status: 'queued' as const };

    // ① 取消：后端把「未提交」的条目当场落成终态，在跑那条原样返回；前端直接用返回值刷新，
    //    不必等执行中那条收尾时才有事件（否则用户会一直看到「排队中」）。
    setQueue([running, queued]);
    expect(imageaiStore.queueRunning).toBe(true);
    vi.mocked(api.cancelQueue).mockResolvedValueOnce([
      running,
      { ...queued, status: 'failed', error: '已取消', errorKind: 'cancelled' },
    ]);
    await queueCancel();
    expect(vi.mocked(api.cancelQueue)).toHaveBeenCalled();
    expect(imageaiStore.queueCancelling).toBe(true);
    // 已取消 ≠ 失败：数据上是 failed + errorKind=cancelled，标签另有映射。
    expect(imageaiStore.queue[1].status).toBe('failed');
    expect(imageaiStore.queue[1].errorKind).toBe('cancelled');
    // 已提交的那条继续「生成中」，不被取消口径改写。
    expect(imageaiStore.queue[0].status).toBe('running');

    // ② 快照里已无 running（在跑那条也收尾）→ 过渡态结束
    setQueue([]);
    expect(imageaiStore.queueRunning).toBe(false);
    expect(imageaiStore.queueCancelling).toBe(false);

    // ③ 竞态：`gen:queue:finished` 恰好落在 cancelQueue 的 await 窗口内（handler 写 false）。
    //    旧写法「先 await 再置位」会把它覆盖回 true，此后无事件 → 按钮永久 disabled。
    setQueue([running]);
    vi.mocked(api.cancelQueue).mockImplementationOnce(async () => {
      imageaiStore.queueCancelling = false;
      return [running];
    });
    await queueCancel();
    expect(imageaiStore.queueCancelling).toBe(false);

    // ④ 失败回滚（catch 分支）：请求失败必须收回过渡态并抛错，否则用户看到「取消中」却什么都没发生。
    vi.mocked(api.cancelQueue).mockRejectedValueOnce(new Error('后端拒绝'));
    await expect(queueCancel()).rejects.toThrow('后端拒绝');
    expect(imageaiStore.queueCancelling).toBe(false);

    // ⑤ 新一批开始同样复位，上一轮的取消态不得带进来
    await queueStart();
    expect(imageaiStore.queueCancelling).toBe(false);
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

  it('fitSizeToModel（selectModel 端到端）：跨形态归位、同形态与混排沿用、自由尺寸与脏值兜底', () => {
    // 预设照抄真实服务商的能力位口径（别造假形态，否则测的是假想模型）：
    // 纯档位＝火山 Seedream（1K/1.5K/2K）、纯像素＝百炼 z-image、
    // 混排＝万相 2.7（dashscope 的 wan27_caps 是 tiers.chain(pixels)，档位串与官方推荐像素值共存）。
    const base = {
      maxN: 1, supportsQuality: false, supportsSeed: true, supportsSteps: false,
      supportsNegativePrompt: true, supportsResponseFormat: false, qualities: [] as string[],
    };
    imageaiStore.config = {
      ...cfg,
      providers: [{
        id: 'px', kind: 'openai', name: 'PX', baseUrl: 'http://x/v1', defaultModel: 'm-pixel',
        models: [
          { model: 'm-pixel', capabilities: { ...base, sizes: ['1024x1024', '1664x928'] } },
          { model: 'm-tier', capabilities: { ...base, sizes: ['1K', '1.5K', '2K'] } },
          { model: 'm-mixed', capabilities: { ...base, sizes: ['1K', '2K', '4K', '4096*4096', '4096*2304'] } },
          // 自由尺寸模型：有像素下限（万相 2.7 的真实规则）与完全没有规则（自建 OpenAI 兼容）两种
          {
            model: 'm-free-rules',
            capabilities: { ...base, sizes: [], sizeRules: { totalMin: 768 * 768, totalMax: 4096 * 4096, ratioMin: 1 / 8, ratioMax: 8 } },
          },
          { model: 'm-free-norules', capabilities: { ...base, sizes: [], sizeRules: null } },
        ],
      }],
    };
    imageaiStore.providerId = 'px';
    /** 走真实调用路径：设好「切换前」的尺寸 → selectModel → 读回归位结果。 */
    const fit = (size: string, model: string) => {
      imageaiStore.size = size;
      selectModel(model);
      return imageaiStore.size;
    };

    // 1) 档位串 → 纯像素模型：必须换成该模型能收的像素预设（不换就是发 1K 让厂商 400）
    expect(fit('1K', 'm-pixel')).toBe('1024x1024');
    // 2) 像素 → 纯档位模型：反向同理
    expect(fit('1024x1024', 'm-tier')).toBe('1K');
    // 3) 混排模型（万相 2.7）两个方向都不归位：档位串与像素值它都认
    expect(fit('1K', 'm-mixed')).toBe('1K');
    // 不在预设里、但形态被接受（万相允许自定义宽*高）→ 原样保留。这条锁住「按整份预设判形态」：
    // 若改成只看 presets[0]（='1K'），这里会被误归位成 '1K'，用户自选的合法尺寸凭空丢失。
    expect(fit('2048*2048', 'm-mixed')).toBe('2048*2048');
    // 4) 同形态不在预设也保留：像素模型之间互切，用户自定义的 1328x1328 不动（既有设计）
    expect(fit('1328x1328', 'm-pixel')).toBe('1328x1328');
    // 5) sizes 空 + 声明了像素下限：档位串按该下限推出正方形（万相 2.7 下限 768²）
    expect(fit('1K', 'm-free-rules')).toBe('768x768');
    // 6) sizes 空 + 无任何规则：兜底 1024×1024（不是「自定义…」起步值 350×350）
    expect(fit('1K', 'm-free-norules')).toBe('1024x1024');
    // 7) 脏值 → 有预设：归位首个预设
    expect(fit('abc', 'm-tier')).toBe('1K');
    // 8) 脏值 → sizes 空：保持原值、不抛错
    const dirtyOnFree = () => fit('abc', 'm-free-norules');
    expect(dirtyOnFree).not.toThrow();
    expect(dirtyOnFree()).toBe('abc');
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
