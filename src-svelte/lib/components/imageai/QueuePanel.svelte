<!-- 生成队列：批量生成任务的排队/进度/结果一览。
     事件 → 快照：后端只推进度事件（gen:queue:progress / gen:queue:finished），条目状态
     以 query_queue 快照为唯一事实——进度事件仅更新进度条并节流触发一次快照刷新；
     收尾事件刷新快照 + 复位 running。挂载时先拉一次快照，同步「分段切走期间错过事件」的现状。 -->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { confirm } from '@tauri-apps/plugin-dialog';
  import { t } from '../../text';
  import { icons } from '../../ui/icons';
  import { showError } from '../../ui/dialog';
  import {
    imageaiStore, queueCancel, queueClear, queueQuery, queueRemove, queueStart,
  } from '../../stores/imageai.svelte';
  import type { QueueEntry, QueueFinished, QueueProgress, QueueStatus } from '../../types';

  let progress = $state<QueueProgress | null>(null); // 最近一次进度事件载荷（驱动进度条即时更新）
  let lastFinished = $state<QueueFinished | null>(null); // 最近一次批量收尾摘要（非 running 时展示）
  let picked = $state<Record<string, boolean>>({}); // 队列条目勾选（key=条目 id，组件内会话态）

  const entries = $derived(imageaiStore.queue);
  const queuedCount = $derived(entries.filter((e) => e.status === 'queued').length);
  const pickedIds = $derived(entries.filter((e) => picked[e.id]).map((e) => e.id));
  const progressPct = $derived(
    progress && progress.total > 0 ? Math.round((progress.current / progress.total) * 100) : 0,
  );

  // 进度事件节流：每条任务开始/收尾都 emit（一批 N 条就是 2N 个事件），逐个拉快照虽可接受，
  // 但 120ms 的 trailing 合并能把同一瞬间的连续事件并成一次查询；进度条本身吃事件载荷，不经过这里。
  const REFRESH_THROTTLE_MS = 120;
  let refreshTimer: ReturnType<typeof setTimeout> | undefined;
  function scheduleRefresh() {
    if (refreshTimer) return; // 已有待刷新：合并（后到的进度事件只更新 progress，不重排定时器）
    refreshTimer = setTimeout(() => {
      refreshTimer = undefined;
      void queueQuery().catch(showError);
    }, REFRESH_THROTTLE_MS);
  }

  // unlisten 是 Promise：保存下来，卸载时等 resolve 后逐个取消订阅。
  const unlistens: Array<Promise<() => void>> = [];
  onMount(() => {
    unlistens.push(
      listen<QueueProgress>('gen:queue:progress', (e) => {
        progress = e.payload;
        if (e.payload.status === 'running') imageaiStore.queueRunning = true;
        scheduleRefresh();
      }),
      listen<QueueFinished>('gen:queue:finished', (e) => {
        lastFinished = e.payload;
        imageaiStore.queueRunning = false;
        void queueQuery().catch(showError);
      }),
    );
    void queueQuery().catch(showError);
  });
  onDestroy(() => {
    for (const p of unlistens) void p.then((fn) => fn());
    if (refreshTimer) clearTimeout(refreshTimer);
  });

  function statusText(s: QueueStatus) {
    return s === 'queued' ? t.gen.queue.statusQueued
      : s === 'running' ? t.gen.queue.statusRunning
      : s === 'success' ? t.gen.queue.statusSuccess
      : t.gen.queue.statusFailed;
  }
  /** 详情列：失败显示原因（Rust 侧 error 为空串时字段不出现，?? 兜底），成功显示张数，其余留 —。 */
  function detailText(e: QueueEntry) {
    if (e.status === 'failed') return e.error || '—';
    if (e.status === 'success') return t.gen.queue.imagesDone(e.images?.length ?? 0);
    return '—';
  }

  function toggleAll(v: boolean) {
    const next = { ...picked };
    for (const e of entries) next[e.id] = v;
    picked = next;
  }

  // 启动是「将产生费用」的确认（信息级）；取消/清空/移除是破坏性确认（警告级）——均走系统弹窗。
  async function onStart() {
    if (queuedCount === 0) return;
    if (!(await confirm(t.gen.queue.startConfirm(queuedCount), { title: t.dialog.info }))) return;
    try {
      lastFinished = null;
      await queueStart();
    } catch (e) {
      await showError(e);
    }
  }
  async function onCancel() {
    if (!(await confirm(t.gen.queue.cancelConfirm, { title: t.dialog.warning }))) return;
    try {
      await queueCancel();
    } catch (e) {
      await showError(e);
    }
  }
  async function onClear() {
    if (entries.length === 0) return;
    if (!(await confirm(t.gen.queue.clearConfirm(entries.length), { title: t.dialog.warning }))) return;
    picked = {};
    try {
      await queueClear();
    } catch (e) {
      await showError(e);
    }
  }
  async function onRemove() {
    if (pickedIds.length === 0) return;
    if (!(await confirm(t.gen.queue.removeConfirm(pickedIds.length), { title: t.dialog.warning }))) return;
    picked = {};
    try {
      await queueRemove(pickedIds);
    } catch (e) {
      await showError(e);
    }
  }
</script>

<div class="queue">
  <div class="toolbar">
    <button class="btn sm primary" disabled={imageaiStore.queueRunning || queuedCount === 0} onclick={onStart}>
      <svg viewBox="0 0 24 24" width="11" height="11" fill="currentColor" aria-hidden="true">{@html icons.play.body}</svg>
      {t.gen.queue.startBtn}
    </button>
    <button class="btn sm" disabled={!imageaiStore.queueRunning} onclick={onCancel}>{t.gen.queue.cancelBtn}</button>
    <span class="spacer"></span>
    <button class="btn sm danger" disabled={imageaiStore.queueRunning || pickedIds.length === 0} onclick={onRemove}>
      {t.gen.queue.removeBtn}
    </button>
    <button class="btn sm danger" disabled={imageaiStore.queueRunning || entries.length === 0} onclick={onClear}>
      {t.gen.queue.clearBtn}
    </button>
  </div>

  {#if imageaiStore.queueRunning && progress}
    <div class="progress-row">
      <span class="muted small">{t.gen.queue.progressLabel}</span>
      <div class="pbar" role="progressbar" aria-valuenow={progress.current} aria-valuemin={0} aria-valuemax={progress.total}>
        <div class="pfill" style={`width:${progressPct}%`}></div>
      </div>
      <span class="mono small">{progress.current} / {progress.total}</span>
    </div>
  {:else if lastFinished}
    <div class="muted small finished-row">
      {t.gen.queue.finishedSummary(lastFinished.finished, lastFinished.failed, lastFinished.total)}
    </div>
  {/if}

  {#if entries.length === 0}
    <div class="empty grow">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">{@html icons.image.body}</svg>
      <p>{t.gen.queue.empty}</p>
    </div>
  {:else}
    <div class="table-scroll">
      <table class="grid">
        <colgroup>
          <col class="w-sel" />
          <col class="w-player" />
          <col class="w-uid" />
          <col class="w-status" />
        </colgroup>
        <thead>
          <tr>
            <th class="static c">
              <input
                type="checkbox"
                aria-label={t.gen.queue.selectAllAria}
                checked={entries.length > 0 && pickedIds.length === entries.length}
                indeterminate={pickedIds.length > 0 && pickedIds.length < entries.length}
                onchange={(e) => toggleAll(e.currentTarget.checked)}
              />
            </th>
            <th class="static">{t.gen.queue.colPlayer}</th>
            <th class="static">{t.table.colUid}</th>
            <th class="static c">{t.gen.colStatus}</th>
            <th class="static">{t.gen.queue.colDetail}</th>
          </tr>
        </thead>
        <tbody>
          {#each entries ?? [] as e (e.id)}
            <tr>
              <td class="c">
                <input
                  type="checkbox"
                  aria-label={t.gen.queue.selectRowAria(e.name)}
                  checked={!!picked[e.id]}
                  onchange={(ev) => (picked[e.id] = ev.currentTarget.checked)}
                />
              </td>
              <td>{e.name}</td>
              <td class="mono">{e.uid}</td>
              <td class="c"><span class="badge {e.status}">{statusText(e.status)}</span></td>
              <td class="detail" title={detailText(e)}>{detailText(e)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    <div class="foot">
      <span class="muted small">{t.gen.queue.count(entries.length)}</span>
    </div>
  {/if}
</div>

<style>
  /* 与 UsageLedger 同款骨架：父级 .panel 是 flex 列且高度确定，flex:1 撑满。 */
  .queue { display: flex; flex-direction: column; flex: 1; min-height: 0; gap: 8px; }
  .toolbar { display: flex; align-items: center; gap: 8px; }
  .foot { display: flex; justify-content: flex-end; }
  /* 进行中进度条：宽度过渡 160ms（--dur）平滑推进；fill 用强调色令牌，双主题自适配。 */
  .progress-row { display: flex; align-items: center; gap: 10px; }
  .pbar { flex: 1; height: 6px; border-radius: 99px; background: var(--progress-bg); overflow: hidden; }
  .pfill { height: 100%; border-radius: 99px; background: var(--accent); transition: width var(--dur) var(--ease-out); }
  .finished-row { white-space: nowrap; }
  /* 状态徽标：全部取设计令牌（含 soft 底），深/浅主题各自成对定义，无字面色值。 */
  .badge { border-radius: 99px; padding: 1px 9px; font-size: 11.5px; white-space: nowrap; }
  .badge.queued { color: var(--text-2); background: var(--bg-raised); }
  .badge.running { color: var(--warn); background: var(--warn-soft); }
  .badge.success { color: var(--ok); background: var(--ok-soft); }
  .badge.failed { color: var(--danger); background: var(--danger-soft); }
  .detail { max-width: 320px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); }
  .w-sel { width: 40px; }
  .w-player { width: 140px; }
  .w-uid { width: 110px; }
  .w-status { width: 72px; }
  .table-scroll { flex: 1; min-height: 0; overflow: auto; }
  /* 空态（全局 .empty 是 flex 居中块）：撑满剩余高度。 */
  .grow { flex: 1; }
</style>
