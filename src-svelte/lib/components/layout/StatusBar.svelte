<!-- StatusBar · 底部状态栏：显示运行状态、进度与错误数徽章，并提供日志抽屉开关 -->
<script lang="ts">
  import { appStore } from '../../stores/app.svelte';
  import { filterLines, logStore } from '../../stores/log.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { t } from '../../text';

  const snap = $derived(appStore.snapshot); // 后端激活快照（可能为 null）
  // 错误条数：日志按 ERROR 阈值过滤后取长度
  const errorCount = $derived(filterLines(logStore.lines, 'ERROR', false).length);
  // 状态栏要展示的最新一行日志正文（没有日志时为空，不占位）
  const lastLog = $derived(logStore.lines.length > 0 ? logStore.lines[logStore.lines.length - 1].message : '');
  // 轻量任务（检查完整性/扫描头像/扫描RTF/读取快照）进度：taskRunning 由 runTask 在命令执行期间置位，
  // 进度值由后端 task:progress 事件写入 appStore.task（与替换流水线的 progress 分两条通道）
  const task = $derived(appStore.task);
  // 替换中：替换进度优先（replace:progress 一开始就会把 task 清零，两条通道不会打架）
  const replacing = $derived(!!snap?.replacing);
  // 任务进度是否占据状态栏：在跑，或刚跑完还没到清空时刻（runTask 收尾补的 100%，
  // 20 秒后由 store 清空）——后者让几毫秒就结束的小包/空包也留下反馈，而不是进度条一闪即逝
  const showTask = $derived(!replacing && (appStore.taskRunning || task.value > 0));
  // 当前占据状态栏的那条进度：两条通道载荷形状一致（status + value + current/total），
  // 所以下面的百分比与计数只需算一遍
  const shown = $derived(showTask ? task : appStore.progress);
  // 状态文字：任务在跑 → 任务名；替换中 → 后端阶段文案；否则按有无选中组显示就绪/未选头像包。
  // 任务跑完后文字回到「就绪」、进度条停在 100%，即「● 就绪 ▬▬▬ 100%」
  const status = $derived(
    appStore.taskRunning
      ? task.status
      : replacing
        ? appStore.progress.status
        : snap?.currentGroup
          ? t.status.ready
          : t.status.noGroup,
  );
  const value = $derived(shown.value);
  // 是否显示进度条：任务在跑/刚跑完、替换中，或替换流水线留下过进度
  const showBar = $derived(showTask || replacing || appStore.progress.value > 0);
  // 「第 n 个 / 共 total 个」：两条通道都带（替换的逐球员抽图、四个扫描任务的逐项上报）；
  // 纯阶段锚点（如「生成 config.xml…」）与无可数单位的阶段 total 为 0，只显示百分比
  const counter = $derived(shown.total > 0 ? `${shown.current}/${shown.total}` : '');
</script>

<footer class="statusbar">
  <!-- 小圆点：class:busy 在替换中或有轻量任务在跑时加上（琥珀色呼吸动画，样式见 app.css） -->
  <span class="dot" class:busy={replacing || appStore.taskRunning}></span>
  <span class="status">{status}</span>
  {#if showBar}
    <div class="progress-track mini"><div class="progress-fill" style="width:{value}%"></div></div>
    <span class="muted counter">{#if counter}{counter} · {/if}{value}%</span>
  {/if}
  <!-- 最新一行日志预览：空间不足时省略号截断（样式见 app.css .statusbar .last-log） -->
  {#if lastLog}
    <span class="last-log" title={lastLog}>{lastLog}</span>
  {/if}
  <!-- flex-1 的空隔层把后面的按钮推到最右侧 -->
  <span class="flex-1"></span>
  <!-- 日志按钮：切换 ui.logOpen；有错误时显示红色数量徽章 -->
  <button class="link" onclick={() => (ui.logOpen = !ui.logOpen)}>
    {t.status.log}{#if errorCount > 0}<span class="badge">{errorCount}</span>{/if}
  </button>
</footer>
