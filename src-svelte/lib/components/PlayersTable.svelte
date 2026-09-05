<!-- 【新手导读】名单预览表格：分页、搜索、点表头排序。数据留在后端，每次只拉当前页。
     重点演示：$effect 做“参数变化→防抖请求”，以及 {#each} 渲染表格行。 -->
<script lang="ts">
  import { api } from '../api';
  import type { PageResult, PlayerRecord } from '../types';

  const PAGE_SIZE = 200; // 每页条数
  let page = $state(0);       // 当前页码（从 0 开始）
  let search = $state('');    // 搜索词
  let sortKey = $state('uid'); // 排序列
  let sortDesc = $state(false);// 是否降序
  let result = $state<PageResult<PlayerRecord> | null>(null); // 当前页结果
  let searchTimer: ReturnType<typeof setTimeout> | undefined;  // 防抖定时器句柄

  // 总页数：由总数除以每页数向上取整（至少 1 页）。
  const totalPages = $derived(result ? Math.max(1, Math.ceil(result.total / PAGE_SIZE)) : 1);

  /** 拉当前页数据（把页码/搜索/排序传给后端）。 */
  async function load() {
    result = await api.queryPlayers({
      offset: page * PAGE_SIZE, limit: PAGE_SIZE,
      search, sortKey, sortDesc,
    });
  }
  // 参数变化自动刷新：effect 内同步读取依赖（runes 追踪），请求 300ms 防抖。
  // void page 等是“显式读取”，让 $effect 知道要追踪这些变量；任一变化就重新执行。
  $effect(() => {
    void page; void search; void sortKey; void sortDesc;
    clearTimeout(searchTimer);            // 取消上一次未触发的请求
    searchTimer = setTimeout(load, 300);  // 300ms 后才真正请求（防抖：避免边输入边频繁请求）
    return () => clearTimeout(searchTimer); // 清理函数：依赖变化/组件销毁时取消定时器
  });

  /** 点表头排序：同一列则切换升/降序，不同列则切到该列并重置为升序。 */
  function sortBy(key: string) {
    if (sortKey === key) { sortDesc = !sortDesc; } else { sortKey = key; sortDesc = false; }
  }
  // 表头箭头指示：当前排序列显示 ↑/↓，其余列为空。
  const arrow = (key: string) => (sortKey === key ? (sortDesc ? ' ↓' : ' ↑') : '');

  // 是否含详细信息：当前页任一条有 detailed 就额外渲染那几列（适配基础/详细两种名单）。
  const hasDetailed = $derived(!!result?.items.some((r) => r.detailed));
</script>

<h3>Players Preview <span class="muted">{result ? `${result.total} records` : ''}</span></h3>

<!-- 搜索框 + 上一页/下一页。bind:value 把输入框与 search 双向绑定。 -->
<div class="row">
  <input type="text" placeholder="Search UID / name / nationality..." bind:value={search} class="flex-1" style="max-width:360px" />
  <button class="btn" onclick={() => (page = 0)} disabled={page === 0}>‹ Prev</button>
  <span class="muted">Page {page + 1} / {totalPages}</span>
  <button class="btn" onclick={() => (page += 1)} disabled={page + 1 >= totalPages}>Next ›</button>
</div>

<!-- 有数据时渲染表格；否则显示引导提示。 -->
{#if result && result.items.length > 0}
  <div style="max-height:340px;overflow:auto">
    <table class="grid">
      <thead>
        <tr>
          <!-- 可排序的表头绑 onclick=sortBy；arrow() 拼接升降序箭头。 -->
          <th onclick={() => sortBy('uid')}>UID{arrow('uid')}</th>
          <th onclick={() => sortBy('primaryNat')}>Nat{arrow('primaryNat')}</th>
          <th>2nd Nat</th>
          <th onclick={() => sortBy('name')}>Name{arrow('name')}</th>
          <th>Hair</th>
          <th>Color</th>
          <th onclick={() => sortBy('ethnicityCode')}>Eth{arrow('ethnicityCode')}</th>
          <!-- 仅当名单含详细信息时才多渲染这几列。 -->
          {#if hasDetailed}
            <th>Skin</th><th>Club</th><th>Age</th><th>Height</th><th>Weight</th><th>NewGAN</th>
          {/if}
        </tr>
      </thead>
      <tbody>
        <!-- {#each ... as r (r.uid)}：括号里的 (r.uid) 是“键”，帮 Svelte 高效复用 DOM。 -->
        {#each result.items as r (r.uid)}
          <tr>
            <td class="mono">{r.uid}</td>
            <td>{r.primaryNat}</td>
            <td>{r.secNat}</td>
            <td>{r.name}</td>
            <td>{r.hairLength}</td>
            <td>{r.hairColor}</td>
            <td>{r.ethnicityCode}</td>
            <!-- 详细列：?. 安全访问 detailed，为空时显示 —。 -->
            {#if hasDetailed}
              <td>{r.detailed?.skinCode ?? '—'}</td>
              <td>{r.detailed?.club ?? '—'}</td>
              <td>{r.detailed?.age ?? '—'}</td>
              <td>{r.detailed?.height ?? '—'}</td>
              <td>{r.detailed?.weight ?? '—'}</td>
              <td>{r.detailed?.isNewgan ?? '—'}</td>
            {/if}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{:else}
  <p class="muted">执行 Replace Faces 后可在此预览名单（最近一次解析结果）。</p>
{/if}
