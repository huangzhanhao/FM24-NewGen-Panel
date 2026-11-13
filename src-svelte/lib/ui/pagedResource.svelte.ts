/**
 * 本地分页骨架（名单预览 / 快照预览共用）：后端给出全量（换代时取一次，全量或流式分批皆可），
 * 分页/筛选/排序全在前端内存完成——翻页/搜索/排序/行数变化都不产生任何后端往返。
 *
 * 【分工】组件持有搜索词（paged.search，输入框直接绑定）与筛选/排序状态，
 * 经 opts.select 作用到全量上；本骨架负责：换代取全量、页码、切片（view）、错误兜底。
 *
 * 【每页行数 = 布局自适应，无行数常量】传入 opts.pane（恒渲染的 .table-pane）即开启：
 * 面板装得下几行，每页就是几行。行数 = floor(.table-scroll.clientHeight / --table-row-h) − 1
 * （减一行粘顶表头）：clientHeight 是浏览器布局后的实测值，工具条一排/两排（窄栏 flex 换行）、
 * 横滚条占不占位都已自动扣好，JS 不做固定减法；ResizeObserver 只随高度变化重算——本地切片零成本。
 *
 * 【换代】resetKey 变化 → 复位 + 重取全量；令牌防乱序：晚到的旧响应不得覆盖新结果。
 */
export interface PagedResourceOptions<T> {
  /** 取全量（换代且门控放行时调用一次）。与 fetchStream 至少给一个；快照预览走这条全量通道。 */
  fetch?: () => Promise<T[]>;
  /** 流式取数：提供时优先于 fetch。每批到达即经 onBatch 交付，调用方不必等整份读完；
   *  null = 已交付的批次作废（重新取数前的重置信号）。promise resolve 即流结束。 */
  fetchStream?: (onBatch: (batch: T[] | null) => void) => Promise<void>;
  /** 换代键：变化即换代（复位 + 重新取全量）。 */
  resetKey: () => string | number;
  /** 载入门控：false 时不取数（快照侧 readable=false 的引导空态）。 */
  shouldLoad?: () => boolean;
  /** 换代复位后的组件侧清理（清排序/头像账等）。 */
  onReset?: () => void;
  /** 取数失败：返回空全量并自行提示。 */
  onError?: (e: unknown) => T[];
  /** 组件的筛选/排序（读组件本地状态的纯函数）；分页切片在其后执行。 */
  select?: (items: T[]) => T[];
  /** 表格面板（恒渲染）。提供即开启「每页行数随面板高度自适应」。 */
  pane?: () => HTMLElement | undefined;
}

export class PagedResource<T> {
  /** 当前页码（0 起）——模板可 bind:page={paged.page}。 */
  page = $state(0);
  /** 搜索词——模板可 bind:value={paged.search}；筛选语义由 opts.select 解释。 */
  search = $state('');
  /** 全量结果；null = 尚无数据或取数失败。 */
  result = $state<T[] | null>(null);
  /**
   * 每页行数：由面板高度实测写入（#measure），**没有设计常量**。
   * 初值 0 = 未测量；实测发生在挂载 effect（同步），早于异步取数返回，用户看不到中间态。
   */
  pageSize = $state(0);

  readonly #opts: PagedResourceOptions<T>;
  #token = 0; // 令牌：换代后，在途旧响应不得写回

  constructor(opts: PagedResourceOptions<T>, wiring?: { manual?: boolean }) {
    this.#opts = opts;
    // 数据源二选一：两个都缺时骨架只会永远空态，构造期就报出来（不是运行期静默无数据）
    if (!opts.fetch && !opts.fetchStream) {
      throw new Error('createPagedResource 需要 fetch 或 fetchStream 之一作为数据源');
    }
    if (wiring?.manual) return;
    // 行数自适应：滚动盒 clientHeight 是浏览器布局后的实测值（工具条换行/横滚条占位都已扣除），
    // 同时观测 pane 与实际 .table-scroll 的高度；空态 ↔ 表格是条件渲染，childList 补挂/补测。
    // 只认高度变化：纵滚条进出只改宽度，若也重测会形成「减行→条消失→加行→条又出现」的自激环。
    $effect(() => {
      const pane = opts.pane?.();
      if (!pane) return;
      let lastH = -1;
      const remeasure = () => {
        const scroller = pane.querySelector<HTMLElement>('.table-scroll');
        if (!scroller || scroller.clientHeight === lastH) return; // 空态等挂载；纯宽度变化忽略
        lastH = scroller.clientHeight;
        this.#measure(pane);
      };
      const ro = new ResizeObserver(remeasure);
      ro.observe(pane);
      const observed = new WeakSet<Element>();
      const track = (el: Element | null) => {
        if (el && !observed.has(el)) {
          observed.add(el);
          ro.observe(el);
        }
      };
      track(pane.querySelector('.table-scroll'));
      remeasure();
      const mo = new MutationObserver(() => {
        track(pane.querySelector('.table-scroll'));
        remeasure();
      });
      mo.observe(pane, { childList: true });
      return () => {
        ro.disconnect();
        mo.disconnect();
      };
    });
    // 换代/门控变化：复位并重取全量（门控分支在 #load 里；此处读取 shouldLoad 是为了追踪）
    $effect(() => {
      opts.resetKey();
      opts.shouldLoad?.();
      this.#applyReset();
      void this.#load();
    });
  }

  /**
   * 面板装得下几行，每页就是几行。直接读 .table-scroll 的实测 clientHeight——
   * 工具条一排/两排（窄栏 flex 换行）、横滚条占不占位，浏览器布局时都已扣除，
   * JS 不再维护固定区减法常数（常数曾只覆盖一排工具条，替换台两排时凭空多算一行逼出纵滚条）。
   */
  #measure(pane: HTMLElement): void {
    const scroller = pane.querySelector<HTMLElement>('.table-scroll');
    if (!scroller) return; // 空态：滚动盒条件渲染，挂载后由 childList 通告补测
    const n = Number.parseFloat(getComputedStyle(pane).getPropertyValue('--table-row-h'));
    const row = Number.isFinite(n) && n > 0 ? n : 40; // 行高 = 表头高（同一令牌）
    // 滚动盒内容高 / 行高，再减一行粘顶表头；夹 1 行下限（极小窗下纵滚条不可避免）
    this.pageSize = Math.max(1, Math.floor(scroller.clientHeight / row) - 1);
  }

  /** 筛选/排序后的当前页视图（模板与派生状态只读这里）。 */
  view = $derived.by(() => {
    const size = Math.max(1, this.pageSize); // 未测量（0）时退化为单行/页，测量后立即正确
    const all = this.#opts.select ? this.#opts.select(this.result ?? []) : (this.result ?? []);
    const total = all.length;
    const last = Math.max(0, Math.ceil(total / size) - 1);
    const page = Math.min(this.page, last); // 筛选后总页数变少时夹住，避免空页
    const start = page * size;
    return { total, items: all.slice(start, start + size) };
  });

  /** 总页数（按筛选后的 total，至少 1 页）。 */
  totalPages = $derived(Math.max(1, Math.ceil(this.view.total / Math.max(1, this.pageSize))));

  /** 换代复位：回第 1 页、清搜索、作废在途取数并清空全量，然后重取。 */
  reset(): void {
    this.#applyReset();
    void this.#load();
  }

  #applyReset(): void {
    this.page = 0;
    this.search = '';
    this.result = null;
    this.#token++;
    this.#opts.onReset?.();
  }

  async #load(): Promise<void> {
    if (this.#opts.shouldLoad?.() === false) return; // 门控：未就绪时不取数（快照侧引导空态）
    const mine = ++this.#token;
    const stream = this.#opts.fetchStream;
    if (stream) {
      // 流式：先置空全量，此后每批到达即追加——首屏随首批就绪，不必等整份读完
      this.result = [];
      try {
        await stream((batch) => {
          if (mine !== this.#token) return; // 期间换代：旧流后续批次不得写回
          if (batch === null) {
            this.result = [];
            return;
          }
          if (this.result === null) this.result = [];
          const cur = this.result;
          // 逐项追加：spread 大数组会撞引擎实参上限，逐项 push 也把写代理的长任务切得更碎
          for (const x of batch) cur.push(x);
        });
      } catch (e) {
        if (mine !== this.#token) return; // 过期失败不落状态
        this.result = this.#opts.onError ? this.#opts.onError(e) : null;
      }
      return;
    }
    const fetch = this.#opts.fetch;
    if (!fetch) return; // 构造期已保证二者其一，这里只为类型收窄
    let got: T[];
    try {
      got = await fetch();
    } catch (e) {
      if (mine !== this.#token) return; // 过期失败不落状态
      this.result = this.#opts.onError ? this.#opts.onError(e) : null;
      return;
    }
    if (mine !== this.#token) return; // 期间又换代：整份作废
    this.result = got;
  }
}

/** 工厂：组件用 `const paged = createPagedResource<T>({ ... })` 创建（构造期自动接线 effect）。 */
export function createPagedResource<T>(opts: PagedResourceOptions<T>): PagedResource<T> {
  return new PagedResource<T>(opts);
}
