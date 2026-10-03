/** createPagedResource 骨架断言：全量落地/本地切片/筛选排序/令牌防乱序/门控/换代复位/错误态。
 *  manual 构造跳过 effect 接线（node 测试环境无 effect 根），直接驱动 reset() 测纯逻辑；
 *  换代的 effect 接线本身很薄（几行），随应用运行验证。改骨架前先看这里。 */
import { describe, expect, it, vi } from 'vitest';
import { PagedResource, type PagedResourceOptions } from './pagedResource.svelte';

interface Row {
  uid: string;
}

/** 受控 fetch：手动 resolve，模拟「先发的请求后到」。 */
function deferred() {
  let resolve!: (v: Row[]) => void;
  const promise = new Promise<Row[]>((r) => (resolve = r));
  return { promise, resolve };
}

const rowsOf = (...uids: string[]): Row[] => uids.map((uid) => ({ uid }));

/** manual 构造（无 effect 接线）；fetch 默认空全量。 */
function setup(
  over: Partial<PagedResourceOptions<Row>> = {},
  fetch: PagedResourceOptions<Row>['fetch'] = vi.fn(async () => rowsOf()),
): { res: PagedResource<Row>; fetch: PagedResourceOptions<Row>['fetch'] } {
  const res = new PagedResource<Row>(
    { fetch, resetKey: () => 0, ...over },
    { manual: true },
  );
  return { res, fetch };
}

describe('createPagedResource（本地分页骨架）', () => {
  it('全量落地：view 按 pageSize 切片，total 为全量条数，翻页是纯内存切片', async () => {
    const fetch = vi.fn(async () => rowsOf(...Array.from({ length: 20 }, (_, i) => `u${i + 1}`)));
    const { res } = setup({}, fetch);
    res.pageSize = 8; // 行数由面板实测写入（组件内）；测试里手动定档
    await res.reset();
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(res.view.total).toBe(20);
    expect(res.view.items.map((r) => r.uid)).toEqual(['u1', 'u2', 'u3', 'u4', 'u5', 'u6', 'u7', 'u8']);
    expect(res.totalPages).toBe(3); // 20/8 向上取整

    res.page = 2; // 第 3 页只剩 4 条
    expect(res.view.items.map((r) => r.uid)).toEqual(['u17', 'u18', 'u19', 'u20']);
  });

  it('select 筛选/排序只影响 view，不重新取数', async () => {
    const fetch = vi.fn(async () => rowsOf('b1', 'a2', 'b2', 'a1'));
    const { res } = setup(
      {
        select: (rows) =>
          rows
            .filter((r) => r.uid.startsWith(res.search))
            .sort((a, b) => a.uid.localeCompare(b.uid)),
      },
      fetch,
    );
    res.pageSize = 4;
    await res.reset();
    expect(res.view.total).toBe(4);
    expect(res.view.items.map((r) => r.uid)).toEqual(['a1', 'a2', 'b1', 'b2']);

    res.search = 'a'; // 筛选：fetch 不再被调用，view 即时重算
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(res.view.total).toBe(2);
    expect(res.view.items.map((r) => r.uid)).toEqual(['a1', 'a2']);
  });

  it('乱序防护：换代后迟到的前一份全量不得覆盖新结果', async () => {
    const d1 = deferred();
    const fetch = vi.fn().mockReturnValueOnce(d1.promise).mockResolvedValueOnce(rowsOf('B'));
    const { res } = setup({}, fetch);

    void res.reset(); // 换代#1 在途（d1）
    await res.reset(); // 换代#2 完成 → 采纳 B
    expect(res.result).toEqual(rowsOf('B'));

    d1.resolve(rowsOf('A')); // 换代#1 迟到
    await Promise.resolve();
    await Promise.resolve();
    expect(res.result).toEqual(rowsOf('B')); // 仍是被采纳的 B，不被迟到的 A 盖掉
  });

  it('shouldLoad=false：不发起取数（快照侧引导空态的门控）', async () => {
    const fetch = vi.fn();
    const { res } = setup({ shouldLoad: () => false }, fetch);
    await res.reset();
    expect(fetch).not.toHaveBeenCalled();
    expect(res.result).toBeNull();
  });

  it('换代复位：回第 1 页、清搜索、清全量并调用 onReset', async () => {
    const onReset = vi.fn();
    const { res, fetch } = setup({ onReset }, vi.fn(async () => rowsOf('x')));
    await res.reset();
    res.page = 2;
    res.search = 'abc';

    await res.reset();
    expect(res.page).toBe(0);
    expect(res.search).toBe('');
    expect(onReset).toHaveBeenCalledTimes(2); // 首次装载 + 本次换代各一次
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(res.view.total).toBe(1);
  });

  it('取数失败：onError 返回空全量（组件自行提示）', async () => {
    const { res } = setup(
      { onError: () => rowsOf() },
      vi.fn(async () => {
        throw new Error('boom');
      }),
    );
    await res.reset();
    expect(res.result).toEqual([]);
    expect(res.view.total).toBe(0);
  });
});
