<!-- 【新手导读】底部状态栏：显示运行状态、进度条、错误数徒章，并提供“日志”按钮开关抽屉。
     重点演示 $derived（派生状态）：它基于其他响应式值自动计算，依赖变化时自动重算。 -->
<script lang="ts">
  import { appStore } from '../stores/app.svelte';
  import { filterLines, logStore } from '../stores/log.svelte';
  import { ui } from '../stores/ui.svelte';
  import { t } from '../text'; // 全局文案常量

  // $derived(表达式)：把表达式“记住”为一个响应式值，每当它依赖的状态变化就重新计算。
  const snap = $derived(appStore.snapshot); // 当前全局快照（可能为 null）
  // 错误条数：把日志按 ERROR 阈值过滤后取长度（snap/logStore.lines 变化时自动重算）。
  const errorCount = $derived(filterLines(logStore.lines, 'ERROR', false).length);
  // 状态文字：正在替换时显示进度（后端文案，中文）；否则有选中组显示就绪，没有则提示未选头像包。
  // ?. 是可选链：snap 为 null 时不会报错，而是返回 undefined。
  const status = $derived(
    snap?.replacing ? appStore.progress.status : snap?.currentGroup ? t.status.ready : t.status.noGroup,
  );
</script>

<footer class="statusbar">
  <!-- 小圆点：class:busy 在替换中时加上（变成忙碌颜色）。 -->
  <span class="dot" class:busy={snap?.replacing}>●</span>
  <span>{status}</span>
  <!-- 只有替换中或有过进度时才显示迷你进度条；style="width:xx%" 直接把百分比写成行内样式。 -->
  {#if snap?.replacing || appStore.progress.value > 0}
    <div class="progress-track mini"><div class="progress-fill" style="width:{appStore.progress.value}%"></div></div>
    <span class="muted">{appStore.progress.value}%</span>
  {/if}
  <!-- flex-1 的空隔层把后面的按钮推到最右侧。 -->
  <span class="flex-1"></span>
  <!-- 日志按钮：点击切换 ui.logOpen；有错误时额外显示一个红色数量徒章。 -->
  <button class="link" onclick={() => (ui.logOpen = !ui.logOpen)}>
    日志{#if errorCount > 0}<span class="badge">{errorCount}</span>{/if}
  </button>
</footer>
