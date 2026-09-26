/** api 封装的单元测试：校验命令名与参数对象（驼峰）与后端约定一致。 */
import { describe, expect, it, vi } from 'vitest';

// vi.mock 会被提升到文件顶部，必须用 vi.hoisted 共享 mock 实例
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { api } from './api';

describe('api 封装', () => {
  it('startReplace 传参与命令名正确', async () => {
    invoke.mockResolvedValue(undefined);
    await api.startReplace({ mode: 'Preserve', filterNewgan: true, allowDuplicates: false, saveBackup: true, allowBlank: false });
    expect(invoke).toHaveBeenCalledWith('start_replace', {
      args: { mode: 'Preserve', filterNewgan: true, allowDuplicates: false, saveBackup: true, allowBlank: false },
    });
  });

  it('preflightReplace 只传 mode（命令名蛇形）', async () => {
    invoke.mockResolvedValue({ needsConfirm: true, existing: 7 });
    await expect(api.preflightReplace('Generate')).resolves.toEqual({ needsConfirm: true, existing: 7 });
    expect(invoke).toHaveBeenCalledWith('preflight_replace', { mode: 'Generate' });
  });

  it('queryPlayers 序列化 query 参数', async () => {
    invoke.mockResolvedValue({ total: 0, items: [] });
    await api.queryPlayers({ offset: 0, limit: 200, search: 'x', sortKey: 'uid', sortDesc: false, newgan: null });
    expect(invoke).toHaveBeenCalledWith('query_players', {
      query: { offset: 0, limit: 200, search: 'x', sortKey: 'uid', sortDesc: false, newgan: null },
    });
  });
});
