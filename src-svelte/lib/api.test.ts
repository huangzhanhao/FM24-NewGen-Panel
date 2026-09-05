import { describe, expect, it, vi } from 'vitest';

// vi.mock 会被提升到文件顶部，必须用 vi.hoisted 共享 mock 实例
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { api } from './api';

describe('api 封装', () => {
  it('startReplace 传参与命令名正确', async () => {
    invoke.mockResolvedValue(undefined);
    await api.startReplace({ mode: 'Preserve', filterNewgan: true, allowDuplicates: false, saveBackup: true });
    expect(invoke).toHaveBeenCalledWith('start_replace', {
      args: { mode: 'Preserve', filterNewgan: true, allowDuplicates: false, saveBackup: true },
    });
  });

  it('queryPlayers 序列化 query 参数', async () => {
    invoke.mockResolvedValue({ total: 0, items: [] });
    await api.queryPlayers({ offset: 0, limit: 200, search: 'x', sortKey: 'uid', sortDesc: false });
    expect(invoke).toHaveBeenCalledWith('query_players', {
      query: { offset: 0, limit: 200, search: 'x', sortKey: 'uid', sortDesc: false },
    });
  });
});
