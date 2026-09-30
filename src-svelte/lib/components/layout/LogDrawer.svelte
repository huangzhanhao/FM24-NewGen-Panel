<!-- LogDrawer · 左侧滑出的日志抽屉：按级别过滤展示日志，可打开日志文件与清空显示；虚拟滚动只渲染可见行 -->
<script lang="ts">
  import { VList } from "virtua/svelte";
  import type { VListHandle } from "virtua/svelte";
  import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
  import { appStore, reloadLogs } from "../../stores/app.svelte";
  import {
    clearDisplay,
    filterLines,
    LEVELS,
    logStore,
  } from "../../stores/log.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { t } from "../../text";
  import Icon from "../../ui/Icon.svelte";
  import type { LogLine } from "../../types";

  // 抽屉打开期间，过滤条件变化才从 Rust 环形缓冲重查（前端缓冲只承接 log:batch 增量）
  // 【为何不把「打开抽屉」也当触发点】reloadLogs 是清空 + 全量回填：重开时重查会把用户刚点
  // 「清空显示」清掉的日志从后端原样灌回。故打开时只记条件基线；启动首次回填由 initListeners
  // 负责，重开期间的日志靠 log:batch 补齐
  let syncedKey: string | null = null; // 已重查过的过滤条件（非响应式）；null = 尚未建立基线

  $effect(() => {
    if (!ui.logOpen) {
      syncedKey = null; // 关抽屉即忘掉基线：下次打开只记条件、不重查
      return;
    }
    stick = true; // 打开与切换过滤都恢复贴底，默认看到最新日志
    // 同步读取 level/only 建立依赖：任一项变化本 effect 即重跑
    const key = `${logStore.level}|${logStore.only}`;
    if (syncedKey === null) {
      syncedKey = key; // 刚打开：只记基线，不回填
      return;
    }
    if (key === syncedKey) return;
    syncedKey = key;
    void reloadLogs();
  });

  let vlist = $state<VListHandle>();

  /** 是否贴底跟随：用户向上滚离底部时暂停自动跟随，滚回底部后恢复 */
  let stick = true;

  /** 滚动时判断是否贴底：距底部 24px 内视为贴底 */
  function onScroll() {
    const vl = vlist;
    if (!vl) return;
    stick =
      vl.getScrollSize() - vl.getScrollOffset() - vl.getViewportSize() < 24;
  }

  /** 跳到最后一行（align:end 让行尾贴住视口底部）；视口未测得尺寸时等下一帧重试 */
  function jumpToBottom(count: number) {
    const vl = vlist;
    if (!vl) return; // 抽屉已关闭（组件销毁）则终止重试
    if (vl.getViewportSize() <= 0) {
      requestAnimationFrame(() => jumpToBottom(count));
      return;
    }
    vl.scrollToIndex(count - 1, { align: "end" });
  }

  // 显示用日志：按级别与“仅该级别”过滤后的结果
  const shown = $derived(
    filterLines(logStore.lines, logStore.level, logStore.only),
  );

  // 贴底跟随：数据变化（回填 / 新日志到达 / 过滤结果更新）时滚到最后一行。
  // 推迟到微任务，确保 VList 内部已按新数据完成布局
  $effect(() => {
    if (!ui.logOpen) return;
    const list = shown; // 取数组引用：回填或过滤变化都会生成新数组
    const n = list.length;
    if (!n || !stick) return;
    void vlist; // 依赖 VList 挂载完成
    queueMicrotask(() => jumpToBottom(n));
  });

  /** 用系统默认程序打开快照的 logPath；失败（如 .log 无关联程序）时降级为在资源管理器中定位 */
  async function openLogFile() {
    const p = appStore.snapshot?.logPath;
    if (!p) return;
    try {
      await openPath(p);
    } catch {
      await revealItemInDir(p);
    }
  }
</script>

<!-- 窗口级 Esc 监听（svelte:window 不能放在 {#if} 内）；抽屉未开时短路 -->
<svelte:window
  onkeydown={(e) => e.key === "Escape" && ui.logOpen && (ui.logOpen = false)}
/>

{#if ui.logOpen}
  <!-- 半透明遮罩：点击关闭抽屉；键盘关闭由上面的窗口级 Esc 承担 -->
  <div
    class="drawer-backdrop"
    role="presentation"
    onclick={() => (ui.logOpen = false)}
  ></div>
  <aside class="drawer">
    <div class="drawer-tools">
      <strong>{t.log.title}</strong>
      <label class="switch">
        <input type="checkbox" bind:checked={logStore.only} />
        <span class="track"></span>
        {t.log.onlyLevel}
      </label>
      <!-- 级别值是过滤协议值（与后端 query_logs 同一口径），勿改 -->
      <select bind:value={logStore.level}>
        {#each LEVELS as l}<option value={l}>{l}</option>{/each}
      </select>
      <span class="flex-1"></span>
      <button class="btn" onclick={openLogFile}>{t.log.openFileBtn}</button>
      <button class="btn" onclick={clearDisplay}>{t.log.clearBtn}</button>
      <button
        class="icon-btn"
        onclick={() => (ui.logOpen = false)}
        aria-label={t.log.closeAria}
      >
        <Icon name="x" />
      </button>
    </div>
    <div class="drawer-body">
      <!-- VList：data 传全量、只渲染可见行；onscroll 跟踪是否贴底，决定新日志到达时是否自动跟随 -->
      <VList
        bind:this={vlist}
        data={shown}
        onscroll={onScroll}
        style="height:100%"
      >
        <!-- 行模板：VList 按可见行逐行回调此 snippet -->
        {#snippet children(line: LogLine)}
          <!-- 行色按级别小写类名（.error 等）；字号/等宽/内边距见 app.css .drawer-body .log-line -->
          <div class="log-line {line.level.toLowerCase()}">
            | {line.ts} | {line.level} | {line.module} | {line.message}
          </div>
        {/snippet}
      </VList>
    </div>
  </aside>
{/if}
