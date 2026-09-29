/** createPagedResource 骨架断言：令牌防乱序/作废/门控/换代复位/错误态/总页数/取页参数。
 *  manual 构造跳过 effect 接线（node 测试环境无 effect 根），直接驱动 load()/reset()
 *  测纯逻辑；换代与防抖的 effect 接线本身很薄（各 5 行），随应用运行验证。改骨架前先看这里。 */
import { describe, expect, it, vi } from 'vitest';
import { PagedResource, type PagedResourceOptions } from './pagedResource.svelte';
import type { PageResult } from '../types';

interface Row {
  uid: string;
}

/** 受控 fetch：手动 resolve，模拟「先发的请求后到」。 */
function deferred() {
  let resolve!: (v: PageResult<Row> | null) => void;
  const promise = new Promise<PageResult<Row> | null>((r) => (resolve = r));
  return { promise, resolve };
}

const pageOf = (total: number, uid: string): PageResult<Row> => ({ total, items: [{ uid }] });

/** manual 构造（无 effect 接线）；fetch 默认空页。 */
function setup(
  over: Partial<PagedResourceOptions<Row>> = {},
  fetch: PagedResourceOptions<Row>['fetch'] = vi.fn(async () => pageOf(0, 'x')),
): { res: PagedResource<Row>; fetch: PagedResourceOptions<Row>['fetch'] } {
  const res = new PagedResource<Row>(
    { fetch, deps: () => {}, resetKey: () => 0, ...over },
    { manual: true },
  );
  return { res, fetch };
}

describe('createPagedResource（分页骨架）', () => {
  it('load：offset/limit/search 按当前页组装，result/totalPages 写回', async () => {
    const { res, fetch } = setup({}, vi.fn(async () => pageOf(100, 'a')));
    await res.load();
    expect(fetch).toHaveBeenCalledWith({ offset: 0, limit: 8, search: '' });

    res.page = 1;
    res.search = 'abc';
    await res.load();
    expect(fetch).toHaveBeenLastCalledWith({ offset: 8, limit: 8, search: 'abc' });
    expect(res.result).toEqual(pageOf(100, 'a'));
    expect(res.totalPages).toBe(13); // 100/8 向上取整
  });

  it('乱序防护：迟到响应不得覆盖新结果', async () => {
    const d1 = deferred();
    const fetch = vi.fn().mockReturnValueOnce(d1.promise).mockResolvedValueOnce(pageOf(2, 'B'));
    const { res } = setup({}, fetch);

    const p1 = res.load(); // load#1 在途（d1）
    await res.load(); // load#2 完成 → 采纳 B
    expect(res.result).toEqual(pageOf(2, 'B'));

    d1.resolve(pageOf(1, 'A')); // load#1 迟到
    await p1;
    expect(res.result).toEqual(pageOf(2, 'B')); // 仍是被采纳的 B，不被迟到的 A 盖掉
  });

  it('fetcher 返回 null（组件自判过期）：作废不写', async () => {
    const fetch = vi.fn(async () => null);
    const { res } = setup({}, fetch);
    await res.load();
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(res.result).toBeNull();
  });

  it('shouldLoad=false：不发起查询（快照侧引导空态的门控）', async () => {
    const fetch = vi.fn();
    const { res } = setup({ shouldLoad: () => false }, fetch);
    await res.load();
    expect(fetch).not.toHaveBeenCalled();
    expect(res.result).toBeNull();
  });

  it('换代复位：回第 1 页、清搜索、作废在途并调用 onReset', async () => {
    const onReset = vi.fn();
    const { res, fetch } = setup({ onReset }, vi.fn(async () => pageOf(8, 'x')));
    await res.load();
    res.page = 2;
    res.search = 'abc';
    res.result = pageOf(8, 'old');

    res.reset();
    expect(res.page).toBe(0);
    expect(res.search).toBe('');
    expect(res.result).toBeNull();
    expect(onReset).toHaveBeenCalledTimes(1);

    await res.load(); // 复位后的重新拉取
    expect(fetch).toHaveBeenLastCalledWith({ offset: 0, limit: 8, search: '' });
  });

  it('onError：查询失败写入错误态 result', async () => {
    const onError = vi.fn(() => ({ total: 0, items: [] }));
    const { res } = setup({ onError }, vi.fn(async () => {
      throw new Error('boom');
    }));
    await res.load();
    expect(onError).toHaveBeenCalledTimes(1);
    expect(res.result).toEqual({ total: 0, items: [] });
  });

  it('无 onError：查询失败 result 置 null', async () => {
    const { res } = setup({}, vi.fn(async () => {
      throw new Error('boom');
    }));
    await res.load();
    expect(res.result).toBeNull();
  });
});
