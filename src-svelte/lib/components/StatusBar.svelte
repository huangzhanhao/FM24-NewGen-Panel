<script lang="ts">
  import { appStore } from '../stores/app.svelte';
  import { filterLines, logStore } from '../stores/log.svelte';
  import { ui } from '../stores/ui.svelte';

  const snap = $derived(appStore.snapshot);
  const errorCount = $derived(filterLines(logStore.lines, 'ERROR', false).length);
  const status = $derived(
    snap?.replacing ? appStore.progress.status : snap?.currentGroup ? 'Ready' : '未选择头像包',
  );
</script>

<footer class="statusbar">
  <span class="dot" class:busy={snap?.replacing}>●</span>
  <span>{status}</span>
  {#if snap?.replacing || appStore.progress.value > 0}
    <div class="progress-track mini"><div class="progress-fill" style="width:{appStore.progress.value}%"></div></div>
    <span class="muted">{appStore.progress.value}%</span>
  {/if}
  <span style="flex:1"></span>
  <button class="link" onclick={() => (ui.logOpen = !ui.logOpen)}>
    日志{#if errorCount > 0}<span class="badge">{errorCount}</span>{/if}
  </button>
</footer>
