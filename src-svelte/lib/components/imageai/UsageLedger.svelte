<!-- 用量台账：筛选 + 表格 + TablePager（与名单/快照同款分页器）+ 本页合计。
     切走即卸载，再次进入自动重载。 -->
<script lang="ts">
  import { api } from '../../services/api';
  import TablePager from '../results/TablePager.svelte';
  import { imageaiStore } from '../../stores/imageai.svelte';
  import { showError } from '../../ui/dialog';
  import { t } from '../../text';
  import type { UsageRecord } from '../../types';

  const PAGE_SIZE = 20;
  let page = $state(0);
  let total = $state(0);
  let items = $state<UsageRecord[]>([]);
  let providerId = $state('');
  let model = $state('');
  let status = $state('');
  let date = $state('');

  const totalPages = $derived(Math.max(1, Math.ceil(total / PAGE_SIZE)));
  const sumCost = $derived(items.reduce((a, r) => a + r.estimatedCost, 0));
  const currency = $derived(items.find((r) => r.currency)?.currency ?? '');

  async function reload() {
    try {
      const p = await api.queryUsage({
        offset: page * PAGE_SIZE, limit: PAGE_SIZE,
        providerId, model, status, date,
      });
      total = p.total;
      items = p.items;
    } catch (e) {
      await showError(e); // 统一错误弹窗，避免 $effect 里静默失败
    }
  }

  // 首次进入与翻页/筛选变化均重载。
  $effect(() => { void page; void providerId; void model; void status; void date; reload(); });
</script>

<div class="ledger">
  <div class="filters">
    <select bind:value={providerId} onchange={() => (page = 0)}>
      <option value="">{t.gen.filterAllProviders}</option>
      {#each imageaiStore.config?.providers ?? [] as p}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
    <input placeholder={t.gen.filterModelPlaceholder} bind:value={model} onchange={() => (page = 0)} />
    <select bind:value={status} onchange={() => (page = 0)}>
      <option value="">{t.gen.filterAllStatus}</option>
      <option value="success">{t.gen.statusSuccess}</option>
      <option value="failed">{t.gen.statusFailed}</option>
    </select>
    <input placeholder={t.gen.filterDatePlaceholder} bind:value={date} onchange={() => (page = 0)} />
  </div>

  <div class="table-scroll">
    <table class="grid">
      <thead>
        <tr>
          <th>{t.gen.colTs}</th><th>{t.gen.provider}</th><th>{t.gen.colModel}</th>
          <th>{t.gen.colSize}</th><th>{t.gen.colN}</th>
          <th>{t.gen.colSeed}</th><th>{t.gen.colPrompt}</th>
          <th>{t.gen.colCost}</th><th>{t.gen.colStatus}</th>
        </tr>
      </thead>
      <tbody>
        {#each items as r (r.id)}
          <tr>
            <td>{r.ts}</td><td>{r.providerName}</td><td>{r.model}</td>
            <td class="c">{r.size}</td><td class="c">{r.n}</td>
            <td class="c">{r.seed ?? ''}</td>
            <td class="prompt-cell" title={r.prompt}>{r.status === 'failed' ? r.error : r.prompt}</td>
            <td class="c">{r.estimatedCost} {r.currency}</td>
            <td class="c {r.status}">{r.status === 'success' ? t.gen.statusSuccess : t.gen.statusFailed}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>

  <div class="foot">
    <span class="muted small">{t.gen.ledgerTotal}：{sumCost.toFixed(3)} {currency} · {t.gen.count(total)}</span>
    <TablePager bind:page {totalPages} />
  </div>
</div>

<style>
  /* 父级 .panel 是 flex 列且高度确定，故用 flex:1（不是 height:100%——那会连 .bar 一起算，溢出面板）。 */
  .ledger { display: flex; flex-direction: column; flex: 1; min-height: 0; gap: 8px; }
  .filters { display: flex; gap: 8px; flex-wrap: wrap; }
  .filters input { width: 150px; }
  .table-scroll { flex: 1; min-height: 0; overflow: auto; }
  .c { text-align: center; }
  .prompt-cell { max-width: 280px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .success { color: var(--accent); }
  .failed { color: var(--danger); }
  .foot { display: flex; justify-content: space-between; align-items: center; gap: 10px; }
</style>
