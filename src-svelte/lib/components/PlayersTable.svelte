<!-- 【新手导读】名单预览表格：分页、搜索、点表头排序。数据留在后端，每次只拉当前页。
     重点演示：$effect 做“参数变化→防抖请求”，以及 {#each} 渲染表格行。 -->
<script lang="ts">
  import { api } from '../api';
  import { appStore } from '../stores/app.svelte';
  import { t } from '../text'; // 全局文案常量
  import { icons } from '../icons';
  import type { PageResult, PlayerRecord } from '../types';

  const PAGE_SIZE = 200; // 每页条数
  let page = $state(0);       // 当前页码（从 0 开始）
  let search = $state('');    // 搜索词
  let sortKey = $state('uid'); // 排序列
  let sortDesc = $state(false);// 是否降序
  let result = $state<PageResult<PlayerRecord> | null>(null); // 当前页结果
  let searchTimer: ReturnType<typeof setTimeout> | undefined;  // 防抖定时器句柄

  // 随机人筛选（仅拓展名单显示）：'any'=任意（默认）/ 'yes'=仅随机人 / 'no'=仅真实球员。
  let newganFilter = $state<'any' | 'yes' | 'no'>('any');

  // 是否为拓展名单：跟随最近一次「扫描RTF」的识别结果（detailed=true 即拓展球员/职员名单）。
  const isExtRoster = $derived(appStore.lastRtfScan?.detailed ?? false);
  // 发往后端的生效筛选：非拓展名单（下拉被隐藏）时强制按「任意」，避免旧筛选残留在请求里。
  const newganQuery = $derived<'any' | 'yes' | 'no'>(isExtRoster ? newganFilter : 'any');

  // 总页数：由总数除以每页数向上取整（至少 1 页）。
  const totalPages = $derived(result ? Math.max(1, Math.ceil(result.total / PAGE_SIZE)) : 1);

  /** 拉当前页数据（把页码/搜索/随机人筛选/排序传给后端）。 */
  async function load() {
    result = await api.queryPlayers({
      offset: page * PAGE_SIZE, limit: PAGE_SIZE,
      search, sortKey, sortDesc,
      newgan: newganQuery === 'any' ? null : newganQuery === 'yes',
    });
  }
  // 参数变化自动刷新：effect 内同步读取依赖（runes 追踪），请求 300ms 防抖。
  // void page 等是“显式读取”，让 $effect 知道要追踪这些变量；任一变化就重新执行。
  // rosterVersion：「扫描RTF」成功后 +1，无需执行 Replace 即可在此预览名单。
  $effect(() => {
    void page; void search; void sortKey; void sortDesc; void newganQuery; void appStore.rosterVersion;
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
  // 性别列：仅当当前页有记录带性别（拓展职员/新版拓展球员）才渲染，旧版 17 列名单不占位。
  const hasGender = $derived(!!result?.items.some((r) => r.detailed?.gender));

  /** 随机人列的标签配色：中文名单值为「是/否」，英文名单兼容 Yes/No；其余值原样显示不着色。 */
  function newganCls(v: string) {
    if (/否|no/i.test(v)) return 'tag-no';
    if (/是|yes/i.test(v)) return 'tag-yes';
    return '';
  }
</script>

<!-- 工具条：搜索框（放大镜图标）+ 总条数 + 右侧分页。组件根是纵向 flex，撑满卡片正文高度。 -->
<div class="table-pane">
  <div class="table-toolbar">
    <div class="search">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
           stroke-linecap="round" aria-hidden="true">{@html icons.search.body}</svg>
      <input type="search" placeholder={t.table.searchPlaceholder} bind:value={search} aria-label={t.table.searchPlaceholder} />
    </div>
    <!-- 随机人筛选：仅拓展名单显示（默认「任意」）；切换后回到第 1 页再查询。 -->
    {#if isExtRoster}
      <label class="newgan-filter">
        <span class="muted small">{t.table.newganFilterLabel}</span>
        <select bind:value={newganFilter} onchange={() => (page = 0)} aria-label={t.table.newganFilterLabel}>
          <option value="any">{t.table.newganAny}</option>
          <option value="yes">{t.table.newganYes}</option>
          <option value="no">{t.table.newganNo}</option>
        </select>
      </label>
    {/if}
    {#if result}<span class="muted small">{t.table.recordCount(result.total)}</span>{/if}
    <span class="spacer"></span>
    <div class="pager">
      <button class="btn" onclick={() => (page = 0)} disabled={page === 0} title={t.table.prevPage}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.chevronLeft.body}</svg>
      </button>
      <span>{t.table.pageOf(page + 1, totalPages)}</span>
      <button class="btn" onclick={() => (page += 1)} disabled={page + 1 >= totalPages} title={t.table.nextPage}>
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.chevronRight.body}</svg>
      </button>
    </div>
  </div>

  <!-- 有数据时渲染表格；否则显示虚线空状态引导。 -->
  {#if result && result.items.length > 0}
    <div class="table-scroll">
      <table class="grid">
        <thead>
          <tr>
            <!-- 可排序的表头绑 onclick=sortBy；arrow() 拼接升降序箭头。
                 排序键（'uid'/'primaryNat'/'name'/'ethnicityCode'）是发往后端的协议值，勿改。 -->
            <th onclick={() => sortBy('uid')}>{t.table.colUid}{arrow('uid')}</th>
            <th onclick={() => sortBy('primaryNat')}>{t.table.colNat}{arrow('primaryNat')}</th>
            <th>{t.table.col2ndNat}</th>
            <th onclick={() => sortBy('name')}>{t.table.colName}{arrow('name')}</th>
            <th>{t.table.colHair}</th>
            <th>{t.table.colColor}</th>
            <th onclick={() => sortBy('ethnicityCode')}>{t.table.colEth}{arrow('ethnicityCode')}</th>
            <!-- 仅当名单含详细信息时才多渲染这几列；性别列仅新版拓展名单（职员/新版球员）有值。 -->
            {#if hasDetailed}
              <th>{t.table.colSkin}</th><th>{t.table.colClub}</th><th>{t.table.colAge}</th><th>{t.table.colHeight}</th><th>{t.table.colNewgan}</th>
              {#if hasGender}<th>{t.table.colGender}</th>{/if}
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
              <!-- 详细列：?. 安全访问 detailed，为空时显示 —；随机人列用是/否胶囊着色。 -->
              {#if hasDetailed}
                <td>{r.detailed?.skinCode ?? '—'}</td>
                <td>{r.detailed?.club ?? '—'}</td>
                <td>{r.detailed?.age ?? '—'}</td>
                <td>{r.detailed?.height ?? '—'}</td>
                <td>
                  {#if r.detailed?.isNewgan}
                    <span class={newganCls(r.detailed.isNewgan)}>{r.detailed.isNewgan}</span>
                  {:else}—{/if}
                </td>
                {#if hasGender}<td>{r.detailed?.gender || '—'}</td>{/if}
              {/if}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <div class="empty">{t.table.emptyHint}</div>
  {/if}
</div>

<style>
  /* 撑满父级 .card-body：工具条定高 + 表格滚动区吃剩余高度（粘顶表头在其中生效） */
  .table-pane { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 2px; }
  /* 随机人筛选：横向排列的“标签+下拉”，下拉用紧凑尺寸（全局 select 默认 width:100%） */
  .newgan-filter { display: inline-flex; align-items: center; gap: 6px; white-space: nowrap; }
  .newgan-filter select { width: auto; padding: 6px 26px 6px 8px; }
</style>
