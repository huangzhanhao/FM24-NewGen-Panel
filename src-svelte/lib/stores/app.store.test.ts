/** runTask 的单元测试：状态栏「轻量任务」进度（appStore.task / taskRunning）的生命周期。
 *  这些口径决定「点击四个扫描按钮时进度条是否出现、跑完是否停在 100%、多久后自动清空」，
 *  改动 runTask 或 StatusBar 前先看这里。 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// app.svelte.ts 顶层就 import 了 tauri 的 listen / invoke（模块副作用），跑 runTask 用不到它们，mock 掉即可。
const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

import { appStore, applySnapshot, BAR_LINGER_MS, runTask } from './app.svelte';
import type { AppSnapshot, RtfScanResult } from '../types';

/** 两条进度通道的清空态（store 里这两个字段的形状一致）。 */
const blank = { status: '', value: 0, current: 0, total: 0 };
/** 状态栏「替换中」的判据来自快照的 replacing 字段。 */
const snap = (replacing: boolean) => ({
  profiles: [], currentProfile: 'No Profile', groups: [], currentGroup: null,
  currentRtf: '', replacing, configXml: null, logPath: '',
});
/** 构造一个任意快照（applySnapshot 的入参）。 */
const snapOf = (over: Partial<AppSnapshot> = {}): AppSnapshot => ({ ...snap(false), ...over });

describe('runTask', () => {
  beforeEach(() => {
    // 假定时器：收尾会排一个「BAR_LINGER_MS 后清空」，用假定时器才能断言它，也免得真挂 20 秒。
    vi.useFakeTimers();
    appStore.task = { ...blank };
    appStore.progress = { ...blank };
    appStore.taskRunning = false;
    appStore.snapshot = null;
  });
  afterEach(() => vi.useRealTimers());

  it('点击即切到新任务名下 0%：不残留上一个任务的名称与计数', async () => {
    // 模拟「上一次扫描头像已跑完」留下的收尾态
    appStore.task = { status: '扫描头像', value: 100, current: 14, total: 14 };
    let release = () => {};
    const pending = new Promise<void>((r) => (release = r));
    const running = runTask('扫描RTF', () => pending);

    // 命令还没返回（后端此时正在读文件、数分母，第一条 task:progress 尚未发出）
    expect(appStore.taskRunning).toBe(true);
    expect(appStore.task).toEqual({ status: '扫描RTF', value: 0, current: 0, total: 0 });

    release();
    await running;
    expect(appStore.taskRunning).toBe(false);
  });

  it('命令没来得及上报就结束（小包几毫秒）：收尾补 100%', async () => {
    await runTask('检查完整性', async () => {});
    expect(appStore.taskRunning).toBe(false);
    expect(appStore.task.value).toBe(100); // 不补的话进度条一闪而过，等于没有反馈
  });

  it('小包无子目录（total 恒为 0）：仍补 100%，计数留空', async () => {
    await runTask('扫描头像', async () => {});
    expect(appStore.task).toEqual({ status: '扫描头像', value: 100, current: 0, total: 0 });
  });

  it('中途上报过进度：收尾把 current 补齐到 total，与百分比一致', async () => {
    await runTask('读取快照', async () => {
      appStore.task = { status: '读取快照', value: 50, current: 5, total: 10 };
    });
    expect(appStore.task).toEqual({ status: '读取快照', value: 100, current: 10, total: 10 });
  });

  it('命令抛错：只复位 taskRunning，不谎报 100%', async () => {
    await expect(
      runTask('扫描RTF', async () => {
        throw new Error('RTF 格式不合法');
      }),
    ).rejects.toThrow('RTF 格式不合法');
    expect(appStore.taskRunning).toBe(false);
    expect(appStore.task.value).toBe(0);
  });

  it('跑完保留 100%，到达 BAR_LINGER_MS 后自动清空两条通道', async () => {
    appStore.progress = { status: '完成！', value: 100, current: 0, total: 0 }; // 假设之前跑过替换
    await runTask('检查完整性', async () => {});
    expect(appStore.task.value).toBe(100);

    vi.advanceTimersByTime(BAR_LINGER_MS - 1);
    expect(appStore.task.value).toBe(100); // 还没到点：进度条留着让用户看到

    vi.advanceTimersByTime(1);
    expect(appStore.task).toEqual(blank);
    expect(appStore.progress).toEqual(blank);
  });

  it('20 秒内又点了新任务：旧定时器作废，不会把新任务的进度清掉', async () => {
    await runTask('检查完整性', async () => {});
    let release = () => {};
    const pending = new Promise<void>((r) => (release = r));
    const running = runTask('扫描RTF', () => pending);

    vi.advanceTimersByTime(BAR_LINGER_MS); // 走到上一个任务的清空时刻
    expect(appStore.task.status).toBe('扫描RTF'); // 仍是新任务，没被清空
    expect(appStore.taskRunning).toBe(true);

    release();
    await running;
  });

  it('20 秒内开始了替换：定时器到点也不清空（替换正占着状态栏）', async () => {
    await runTask('检查完整性', async () => {});
    appStore.snapshot = snap(true); // 替换开始
    appStore.progress = { status: '映射球员头像…', value: 66, current: 2, total: 3 };

    vi.advanceTimersByTime(BAR_LINGER_MS);

    expect(appStore.progress.value).toBe(66); // 替换进度没被抹掉
  });
});

/** applySnapshot：作用域（当前档 / 当前组 / 名单）变化时复位「名单可读」标记。
 *  后端在切档 / 切组 / 换名单时会清空名单缓存（commands.rs 的 clear_caches_if_scope_changed），
 *  前端只能从快照的这三个量看出缓存已作废——不复位就会继续显示**上一个档 / 组的名单**。
 *  改动这段、或改后端那个函数前先看这里。 */
describe('applySnapshot', () => {
  const roster = (): RtfScanResult => ({
    total: 3, newgan: 0, detailed: false, format: 'rtf', formatCode: 'basic', language: 'en',
  });

  beforeEach(() => {
    appStore.snapshot = null;
    appStore.rosterReady = false;
    appStore.lastRtfScan = null;
    appStore.rosterVersion = 0;
  });

  it('切档：标记复位 + rosterVersion 递增（名单预览回到引导空态）', async () => {
    appStore.snapshot = snapOf({ currentProfile: '档A', currentGroup: 'C:/pack', currentRtf: 'C:/a.rtf' });
    appStore.rosterReady = true;
    appStore.lastRtfScan = roster();

    await applySnapshot(snapOf({ currentProfile: '档B' }));

    expect(appStore.rosterReady).toBe(false);
    expect(appStore.lastRtfScan).toBeNull();
    expect(appStore.rosterVersion).toBe(1);
  });

  it('切组 / 换名单：同样复位（名单是「当前档当前组的这份名单」）', async () => {
    appStore.snapshot = snapOf({ currentProfile: '档A', currentGroup: 'C:/pack', currentRtf: 'C:/a.rtf' });
    appStore.rosterReady = true;
    await applySnapshot(snapOf({ currentProfile: '档A', currentGroup: 'C:/other', currentRtf: '' }));
    expect(appStore.rosterReady).toBe(false);

    appStore.snapshot = snapOf({ currentProfile: '档A', currentGroup: 'C:/pack', currentRtf: 'C:/a.rtf' });
    appStore.rosterReady = true;
    await applySnapshot(snapOf({ currentProfile: '档A', currentGroup: 'C:/pack', currentRtf: 'C:/b.rtf' }));
    expect(appStore.rosterReady).toBe(false);
  });

  it('同档同组同名单（替换结束后的刷新）：不动标记，刚置上的可读态不被冲掉', async () => {
    appStore.snapshot = snapOf({ currentProfile: '档A', currentGroup: 'C:/pack', currentRtf: 'C:/a.rtf' });
    appStore.rosterReady = true;
    appStore.lastRtfScan = roster();

    await applySnapshot(snapOf({ currentProfile: '档A', currentGroup: 'C:/pack', currentRtf: 'C:/a.rtf', replacing: true }));

    expect(appStore.rosterReady).toBe(true);
    expect(appStore.lastRtfScan).not.toBeNull();
    expect(appStore.rosterVersion).toBe(0);
  });

  it('启动首次赋值（此前没有快照）：不比较、不复位', async () => {
    appStore.rosterReady = true;

    await applySnapshot(snapOf({ currentProfile: '档A' }));

    expect(appStore.rosterReady).toBe(true);
    expect(appStore.rosterVersion).toBe(0);
  });
});
