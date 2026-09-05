<script lang="ts">
  import { api } from '../api';
  import type { PageResult, PlayerRecord } from '../types';

  const PAGE_SIZE = 200;
  let page = $state(0);
  let search = $state('');
  let sortKey = $state('uid');
  let sortDesc = $state(false);
  let result = $state<PageResult<PlayerRecord> | null>(null);
  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  const totalPages = $derived(result ? Math.max(1, Math.ceil(result.total / PAGE_SIZE)) : 1);

  async function load() {
    result = await api.queryPlayers({
      offset: page * PAGE_SIZE, limit: PAGE_SIZE,
      search, sortKey, sortDesc,
    });
  }
  // 参数变化自动刷新：effect 内同步读取依赖（runes 追踪），请求 300ms 防抖
  $effect(() => {
    void page; void search; void sortKey; void sortDesc;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(load, 300);
    return () => clearTimeout(searchTimer);
  });

  function sortBy(key: string) {
    if (sortKey === key) { sortDesc = !sortDesc; } else { sortKey = key; sortDesc = false; }
  }
  const arrow = (key: string) => (sortKey === key ? (sortDesc ? ' ↓' : ' ↑') : '');

  const hasDetailed = $derived(!!result?.items.some((r) => r.detailed));
</script>

<h3>Players Preview <span class="muted">{result ? `${result.total} records` : ''}</span></h3>

<div class="row">
  <input type="text" placeholder="Search UID / name / nationality..." bind:value={search} class="flex-1" style="max-width:360px" />
  <button class="btn" onclick={() => (page = 0)} disabled={page === 0}>‹ Prev</button>
  <span class="muted">Page {page + 1} / {totalPages}</span>
  <button class="btn" onclick={() => (page += 1)} disabled={page + 1 >= totalPages}>Next ›</button>
</div>

{#if result && result.items.length > 0}
  <div style="max-height:340px;overflow:auto">
    <table class="grid">
      <thead>
        <tr>
          <th onclick={() => sortBy('uid')}>UID{arrow('uid')}</th>
          <th onclick={() => sortBy('primaryNat')}>Nat{arrow('primaryNat')}</th>
          <th>2nd Nat</th>
          <th onclick={() => sortBy('name')}>Name{arrow('name')}</th>
          <th>Hair</th>
          <th>Color</th>
          <th onclick={() => sortBy('ethnicityCode')}>Eth{arrow('ethnicityCode')}</th>
          {#if hasDetailed}
            <th>Skin</th><th>Club</th><th>Age</th><th>Height</th><th>Weight</th><th>NewGAN</th>
          {/if}
        </tr>
      </thead>
      <tbody>
        {#each result.items as r (r.uid)}
          <tr>
            <td class="mono">{r.uid}</td>
            <td>{r.primaryNat}</td>
            <td>{r.secNat}</td>
            <td>{r.name}</td>
            <td>{r.hairLength}</td>
            <td>{r.hairColor}</td>
            <td>{r.ethnicityCode}</td>
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
