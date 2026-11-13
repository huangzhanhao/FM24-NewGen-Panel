/** createPagedResource 骨架断言：全量落地/本地切片/筛选排序/令牌防乱序/门控/换代复位/错误态/流式分批。
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
  fetch: NonNullable<PagedResourceOptions<Row>['fetch']> = vi.fn(async () => rowsOf()),
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

  it('流式模式的 isStale 在换代后返回 true（数据源据此停止拉取）', async () => {
    let first!: () => boolean;
    const fetchStream = vi.fn(async (onBatch: (b: Row[] | null) => void, isStale: () => boolean) => {
      first ??= isStale;
      onBatch(rowsOf('u1'));
    });
    const res = new PagedResource<Row>({ resetKey: () => 0, fetchStream }, { manual: true });
    await res.reset();
    expect(first()).toBe(false); // 当前代：可继续拉
    await res.reset();
    expect(first()).toBe(true); // 上一代令牌已过期：旧流应中断
  });

  it('流式模式：逐批追加、null 批次清空、换代后旧流批次被令牌丢弃', async () => {
    // ① 每批到达即追加：result 随批次递增，末态等于全量
    const seen: number[] = [];
    let res!: PagedResource<Row>;
    const fetchStream = vi.fn(async (onBatch: (b: Row[] | null) => void) => {
      onBatch(rowsOf('u1'));
      seen.push(res.result!.length);
      onBatch(rowsOf('u2'));
      seen.push(res.result!.length);
      onBatch(rowsOf('u3', 'u4'));
      seen.push(res.result!.length);
    });
    res = new PagedResource<Row>({ resetKey: () => 0, fetchStream }, { manual: true });
    res.pageSize = 4;
    await res.reset();
    expect(seen).toEqual([1, 2, 4]); // 每批落进 result 后才继续拉下一批
    expect(res.result).toEqual(rowsOf('u1', 'u2', 'u3', 'u4'));
    expect(res.view.total).toBe(4);

    // ② null 批次 = 作废已交付内容（重新取数前），之后继续追加
    const resetStream = vi.fn(async (onBatch: (b: Row[] | null) => void) => {
      onBatch(rowsOf('old1', 'old2'));
      onBatch(null);
      expect(res2.result).toEqual([]);
      onBatch(rowsOf('new1'));
    });
    const res2 = new PagedResource<Row>({ resetKey: () => 0, fetchStream: resetStream }, { manual: true });
    await res2.reset();
    expect(res2.result).toEqual(rowsOf('new1'));

    // ③ 换代：旧流在途批次的后续交付被令牌丢弃，新流结果不被污染
    let emitOld!: (b: Row[] | null) => void;
    let releaseOld!: () => void;
    const oldPending = new Promise<void>((r) => (releaseOld = r));
    const twoStreams = vi
      .fn()
      .mockImplementationOnce(async (onBatch: (b: Row[] | null) => void) => {
        emitOld = onBatch;
        onBatch(rowsOf('S1'));
        await oldPending;
      })
      .mockImplementationOnce(async (onBatch: (b: Row[] | null) => void) => {
        onBatch(rowsOf('N1'));
        onBatch(rowsOf('N2'));
      });
    const res3 = new PagedResource<Row>({ resetKey: () => 0, fetchStream: twoStreams }, { manual: true });
    void res3.reset(); // 换代#1 在途
    await res3.reset(); // 换代#2 完成 → 采纳 N1、N2
    expect(res3.result).toEqual(rowsOf('N1', 'N2'));

    emitOld(rowsOf('S2')); // 换代#1 的迟到批次
    expect(res3.result).toEqual(rowsOf('N1', 'N2'));
    releaseOld();
    await Promise.resolve();
    expect(res3.result).toEqual(rowsOf('N1', 'N2'));
  });
});
