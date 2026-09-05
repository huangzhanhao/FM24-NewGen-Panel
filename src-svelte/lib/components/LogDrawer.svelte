<script lang="ts">
  import { VList } from 'virtua/svelte';
  import { openPath } from '@tauri-apps/plugin-opener';
  import { appStore, reloadLogs } from '../stores/app.svelte';
  import { clearDisplay, filterLines, LEVELS, logStore } from '../stores/log.svelte';
  import { ui } from '../stores/ui.svelte';
  import type { LogLine } from '../types';

  // 抽屉打开期间，过滤条件变化 → 从 Rust 环形缓冲重查（前端缓冲仅承接事件增量）
  $effect(() => {
    if (!ui.logOpen) return;
    void logStore.level;
    void logStore.only;
    void reloadLogs();
  });

  const shown = $derived(filterLines(logStore.lines, logStore.level, logStore.only));

  async function openLogFile() {
    if (appStore.snapshot?.logPath) await openPath(appStore.snapshot.logPath);
  }
</script>

{#if ui.logOpen}
  <div class="drawer-backdrop" role="button" tabindex={0} aria-label="关闭日志面板" onclick={() => (ui.logOpen = false)} onkeydown={(e) => e.key === 'Escape' && (ui.logOpen = false)}></div>
  <aside class="drawer">
    <div class="row">
      <strong>运行日志</strong>
      <label class="small"><input type="checkbox" bind:checked={logStore.only} /> 仅该级别</label>
      <select bind:value={logStore.level}>
        {#each LEVELS as l}<option value={l}>{l}</option>{/each}
      </select>
      <span class="flex-1"></span>
      <button class="btn" onclick={openLogFile}>打开日志文件</button>
      <button class="btn" onclick={clearDisplay}>清空显示</button>
      <button class="icon-btn" onclick={() => (ui.logOpen = false)}>×</button>
    </div>
    <div class="drawer-body">
      <VList data={shown} style="height:100%" class="mono small">
        {#snippet children(line: LogLine)}
          <div class="log-line {line.level.toLowerCase()}" style="padding:1px 8px;white-space:nowrap">
            | {line.ts} | {line.level.padEnd(8)} | {line.module} | {line.message}
          </div>
        {/snippet}
      </VList>
    </div>
  </aside>
{/if}
