/**
 * 分页取数骨架（名单预览 / 快照预览两表格共用）：
 * 页码、搜索词、令牌防乱序、300ms 防抖、换代复位、总页数派生。
 *
 * 【为什么抽它】两表格的分页状态/防抖/令牌/复位 effect 逐行同构（历史复制粘贴），
 * 是「改一处漏两处」的面；列定义、排序键、额外筛选、头像装载等视图关切留在组件。
 *
 * 【边界】本文件只管「取数骨架」，不碰 Tauri 弹窗：
 *  - fetch 由组件闭包提供（排序/筛选/路径等参数组件自取）；返回 null 表示组件自判过期
 *    （如快照路径已换），本轮作废不写；
 *  - deps 是触发刷新的响应式依赖（在 effect 内读取建立追踪）；resetKey 变化则换代复位；
 *  - 失败经 onError 收口（返回错误态的 result；showError 由组件闭包调，本文件不依赖
 *    Tauri 弹窗，vitest 可直测）。
 *
 * 【两种构造方式】组件用 createPagedResource（自动接线换代/防抖两个 effect）；
 * vitest 用 manual: true（跳过接线，直接驱动 load/reset 测纯逻辑——effect 根在
 * node 测试环境不可用，见 pagedResource.test.ts）。
 * page/search 是 $state 字段：模板可 bind:page={paged.page} / bind:value={paged.search}。
 */
import type { PageResult } from '../types';

/** 一次取页的入参（组件闭包补齐自己的排序/筛选/路径）。 */
export interface PagedFetchArgs {
  offset: number;
  limit: number;
  search: string;
}

export interface PagedResourceOptions<T> {
  /** 拉一页；组件闭包自取额外参数。返回 null = 组件自判过期，作废不写。 */
  fetch: (args: PagedFetchArgs) => Promise<PageResult<T> | null>;
  /** 触发刷新的响应式依赖（在 effect 内读取建立追踪；本体 page/search 已含）。 */
  deps: () => unknown;
  /** 换代键：值变化即换代复位（回第 1 页、清搜索、作废在途，并调用 onReset）。 */
  resetKey: () => string | number;
  /** 每页条数（两表格现状同为 8，默认 8）。 */
  pageSize?: number;
  /** 载入门控：返回 false 时本轮不查询（快照侧 readable=false 的引导空态）。 */
  shouldLoad?: () => boolean;
  /** 换代复位后的组件侧清理（清排序/筛选/图账等）。 */
  onReset?: () => void;
  /** 拉取成功后的组件侧动作（装载本页头像图等）。 */
  onLoaded?: (result: PageResult<T>) => void;
  /** 查询失败：返回错误态 result 并自行提示（不传则 result 置 null、不弹窗）。 */
  onError?: (e: unknown) => PageResult<T> | null;
}

export class PagedResource<T> {
  /** 当前页码（0 起）——模板可 bind:page={paged.page}。 */
  page = $state(0);
  /** 搜索词——模板可 bind:value={paged.search}。 */
  search = $state('');
  /** 当前页结果；null = 尚无数据，或查询失败且 onError 未提供错误态。 */
  result = $state<PageResult<T> | null>(null);

  readonly #opts: PagedResourceOptions<T>;
  readonly #pageSize: number;
  #token = 0; // 令牌：翻页/筛选/换代后，在途旧响应不得写回
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(
    opts: PagedResourceOptions<T>,
    /** manual: true = 不接 effect（vitest 直测纯逻辑用）；默认自动接线。 */
    wiring?: { manual?: boolean },
  ) {
    this.#opts = opts;
    this.#pageSize = opts.pageSize ?? 8; // 两表格现状同为 8（单页 8 行）
    if (wiring?.manual) return;

    // 换代（resetKey 变化）：换代语义 = 「这份数据从头看」，触发面收敛到 resetKey 一个读取。
    $effect(() => {
      opts.resetKey();
      this.reset();
    });

    // 页码/搜索/组件依赖变化：300ms 防抖拉取（两表格现状一致，避免边输入边频繁请求）。
    $effect(() => {
      void this.page;
      void this.search;
      opts.deps();
      this.#schedule();
      return () => clearTimeout(this.#timer); // 依赖变化/组件销毁时取消定时器
    });
  }

  /** 总页数：总数除以每页向上取整（至少 1 页）。 */
  get totalPages(): number {
    return this.result ? Math.max(1, Math.ceil(this.result.total / this.#pageSize)) : 1;
  }

  /** 换代复位：回第 1 页、清搜索、作废在途，组件侧清理交 onReset。 */
  reset(): void {
    this.result = null;
    this.page = 0;
    this.search = '';
    this.#token++;
    this.#opts.onReset?.();
  }

  /** 拉当前页。fetcher 返回 null = 组件自判过期，作废不写。 */
  async load(): Promise<void> {
    const opts = this.#opts;
    if (opts.shouldLoad && !opts.shouldLoad()) return;
    const mine = ++this.#token;
    let got: PageResult<T> | null = null;
    try {
      got = await opts.fetch({
        offset: this.page * this.#pageSize,
        limit: this.#pageSize,
        search: this.search,
      });
    } catch (e) {
      if (mine !== this.#token) return; // 过期失败不落状态
      this.result = opts.onError ? opts.onError(e) : null;
      return;
    }
    // 期间又翻页/换代/过期：整份作废——不拦的话表体停在第 N 页、分页器却显示第 M 页。
    if (mine !== this.#token || got === null) return;
    this.result = got;
    opts.onLoaded?.(got);
  }

  #schedule(): void {
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => this.load(), 300);
  }
}

/** 工厂：组件用 `const paged = createPagedResource<T>({ ... })` 创建（构造期自动接线 effect）。 */
export function createPagedResource<T>(opts: PagedResourceOptions<T>): PagedResource<T> {
  return new PagedResource<T>(opts);
}
