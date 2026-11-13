/** api 封装的单元测试：校验命令名与参数对象（驼峰）与后端约定一致。 */
import { describe, expect, it, vi } from 'vitest';

// vi.mock 会被提升到文件顶部，必须用 vi.hoisted 共享 mock 实例
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { PLAYERS_CHUNK_LIMIT, api } from './api';
import type { PlayerRecord } from '../types';

describe('api 封装', () => {
  it('startReplace 传参与命令名正确', async () => {
    invoke.mockResolvedValue(undefined);
    await api.startReplace({ mode: 'Preserve', filterNewgan: true, allowDuplicates: false, saveBackup: true, allowBlank: false, rosterSource: 'fm' });
    expect(invoke).toHaveBeenCalledWith('start_replace', {
      args: { mode: 'Preserve', filterNewgan: true, allowDuplicates: false, saveBackup: true, allowBlank: false, rosterSource: 'fm' },
    });
  });

  it('preflightReplace 只传 mode（命令名蛇形）', async () => {
    invoke.mockResolvedValue({ needsConfirm: true, existing: 7 });
    await expect(api.preflightReplace('Generate')).resolves.toEqual({ needsConfirm: true, existing: 7 });
    expect(invoke).toHaveBeenCalledWith('preflight_replace', { mode: 'Generate' });
  });

  it('queryPlayersStream 分块推流并校验 total', async () => {
    invoke.mockReset();
    invoke
      .mockResolvedValueOnce({ items: [{ uid: 'r-1' }, { uid: 'r-2' }], total: 3 })
      .mockResolvedValueOnce({ items: [{ uid: 'r-3' }], total: 3 });
    const batches: (PlayerRecord[] | null)[] = [];
    await api.queryPlayersStream((b) => batches.push(b));
    expect(batches).toEqual([[{ uid: 'r-1' }, { uid: 'r-2' }], [{ uid: 'r-3' }]]);
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenNthCalledWith(1, 'query_players_chunk', { offset: 0, limit: PLAYERS_CHUNK_LIMIT });
    expect(invoke).toHaveBeenNthCalledWith(2, 'query_players_chunk', { offset: 2, limit: PLAYERS_CHUNK_LIMIT });
  });

  it('queryPlayersStream 块间 total 变化先作废已交付批次再从头重取一次', async () => {
    invoke.mockReset();
    invoke
      .mockResolvedValueOnce({ items: [{ uid: 'r-1' }], total: 2 })
      .mockResolvedValueOnce({ items: [{ uid: 'r-2' }], total: 3 }) // total 变化 → 旧缓存作废，从头重取
      .mockResolvedValueOnce({ items: [{ uid: 'r-1' }], total: 3 })
      .mockResolvedValueOnce({ items: [{ uid: 'r-2' }, { uid: 'r-3' }], total: 3 });
    const batches: (PlayerRecord[] | null)[] = [];
    await api.queryPlayersStream((b) => batches.push(b));
    expect(batches).toEqual([
      [{ uid: 'r-1' }],
      null, // 作废信号先于重取：上层据此清掉旧批次
      [{ uid: 'r-1' }],
      [{ uid: 'r-2' }, { uid: 'r-3' }],
    ]);
    expect(invoke).toHaveBeenCalledTimes(4);
    expect(invoke).toHaveBeenNthCalledWith(3, 'query_players_chunk', { offset: 0, limit: PLAYERS_CHUNK_LIMIT });
  });

  it('queryPlayersStream 二次 total 不一致抛错（已交付批次不含异代缓存）', async () => {
    invoke.mockReset();
    invoke
      .mockResolvedValueOnce({ items: [{ uid: 'r-1' }], total: 2 })
      .mockResolvedValueOnce({ items: [{ uid: 'r-2' }], total: 3 })
      .mockResolvedValueOnce({ items: [{ uid: 'r-1' }], total: 3 })
      .mockResolvedValueOnce({ items: [{ uid: 'r-2' }], total: 4 });
    const batches: (PlayerRecord[] | null)[] = [];
    await expect(api.queryPlayersStream((b) => batches.push(b))).rejects.toThrow('名单在分块读取期间发生变化');
    expect(batches).toEqual([[{ uid: 'r-1' }], null, [{ uid: 'r-1' }]]);
  });

  it('queryPlayersStream 换代（isStale=true）后不再拉后续分块', async () => {
    invoke.mockReset();
    invoke.mockResolvedValue({ items: [{ uid: 'r-1' }], total: 5 });
    const batches: (PlayerRecord[] | null)[] = [];
    let stale = false;
    await api.queryPlayersStream(
      (b) => {
        batches.push(b);
        stale = true; // 模拟首批交付后即换代
      },
      () => stale,
    );
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(batches).toEqual([[{ uid: 'r-1' }]]);
  });

  it('setCurrentFm 传参与命令名正确', async () => {
    invoke.mockResolvedValue({});
    await api.setCurrentFm('X.fm');
    expect(invoke).toHaveBeenCalledWith('set_current_fm', { path: 'X.fm' });
  });

  it('scanFm 无参命令名正确', async () => {
    invoke.mockResolvedValue({ total: 0 });
    await api.scanFm();
    expect(invoke).toHaveBeenCalledWith('scan_fm');
  });
});
