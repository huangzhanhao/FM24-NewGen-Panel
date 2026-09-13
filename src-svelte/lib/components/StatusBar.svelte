<!-- 底部状态栏：显示运行状态、进度条、错误数徽章，并提供“日志”按钮开关抽屉。 -->
<script lang="ts">
  import { appStore } from '../stores/app.svelte';
  import { filterLines, logStore } from '../stores/log.svelte';
  import { ui } from '../stores/ui.svelte';
  import { t } from '../text'; // 全局文案常量

  const snap = $derived(appStore.snapshot); // 当前全局快照（可能为 null）
  // 错误条数：把日志按 ERROR 阈值过滤后取长度（snap/logStore.lines 变化时自动重算）。
  const errorCount = $derived(filterLines(logStore.lines, 'ERROR', false).length);
  // 状态文字：替换中显示后端进度文案，否则按是否有选中组显示就绪/未选头像包。
  const status = $derived(
    snap?.replacing ? appStore.progress.status : snap?.currentGroup ? t.status.ready : t.status.noGroup,
  );
</script>

<footer class="statusbar">
  <!-- 小圆点：class:busy 在替换中时加上（琥珀色呼吸动画，样式见 app.css）。 -->
  <span class="dot" class:busy={snap?.replacing}></span>
  <span>{status}</span>
  <!-- 替换中或有过进度时显示迷你进度条。 -->
  {#if snap?.replacing || appStore.progress.value > 0}
    <div class="progress-track mini"><div class="progress-fill" style="width:{appStore.progress.value}%"></div></div>
    <span class="muted">{appStore.progress.value}%</span>
  {/if}
  <!-- flex-1 的空隔层把后面的按钮推到最右侧。 -->
  <span class="flex-1"></span>
  <!-- 日志按钮：切换 ui.logOpen；有错误时显示红色数量徽章。 -->
  <button class="link" onclick={() => (ui.logOpen = !ui.logOpen)}>
    日志{#if errorCount > 0}<span class="badge">{errorCount}</span>{/if}
  </button>
</footer>
