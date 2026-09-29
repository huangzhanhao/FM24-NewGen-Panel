<!-- 日志抽屉（从左侧滑出）：按级别过滤展示日志，支持打开日志文件、清空显示；列表用虚拟滚动只渲染可见行。 -->
<script lang="ts">
  import { VList } from "virtua/svelte"; // 虚拟滚动列表组件（只渲染视口内的行，提升大数据量性能）
  import type { VListHandle } from "virtua/svelte";
  import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener"; // 用系统默认程序打开本地文件；失败时降级为资源管理器定位
  import { appStore, reloadLogs } from "../../stores/app.svelte";
  import {
    clearDisplay,
    filterLines,
    LEVELS,
    logStore,
  } from "../../stores/log.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { t } from "../../text"; // 全局文案常量
  import Icon from "../../ui/Icon.svelte";
  import type { LogLine } from "../../types";

  // 抽屉打开期间，过滤条件变化 → 从 Rust 环形缓冲重查（前端缓冲仅承接事件增量）。
  // 【为何不把「打开抽屉」也当触发点】reloadLogs 是「清空 + 全量回填」：若重开抽屉时重查，
  // 会把用户刚点「清空显示」清掉的日志从后端缓冲原样灌回来（清空扛不过一次开合）。故打开时
  // 只记下条件基线、不重查；启动时的首次回填由 initListeners 负责，重开期间的日志靠 log:batch
  // 增量补齐，不漏数据。打开与切换过滤都恢复贴底，让用户默认看到最新日志。
  let syncedKey: string | null = null; // 已重查过的过滤条件（非响应式）；null = 抽屉刚打开、尚未建立基线

  $effect(() => {
    if (!ui.logOpen) {
      syncedKey = null; // 关抽屉即忘掉基线：下次打开只记条件、不重查
      return;
    }
    stick = true;
    // 同步读取 level/only（连同上面的 ui.logOpen）建立依赖：任一变化本 effect 即重跑。
    const key = `${logStore.level}|${logStore.only}`;
    if (syncedKey === null) {
      syncedKey = key; // 刚打开：只记基线，不回填
      return;
    }
    if (key === syncedKey) return;
    syncedKey = key; // 条件确实变了才重查（对等旧版切换过滤时按新条件重渲染 log_store）
    void reloadLogs();
  });

  let vlist = $state<VListHandle>(); // VList 句柄：用于滚动定位与读取滚动状态

  /** 是否贴底跟随：用户向上滚动离开底部时暂停自动跟随，滚回底部后恢复。 */
  let stick = true;

  /** 滚动时判断是否贴底（距底部 24px 内视为贴底）。 */
  function onScroll() {
    const vl = vlist;
    if (!vl) return;
    stick =
      vl.getScrollSize() - vl.getScrollOffset() - vl.getViewportSize() < 24;
  }

  /** 跳到最后一行（align:end = 行尾对齐视口底部）；视口未完成测量时等下一帧重试。 */
  function jumpToBottom(count: number) {
    const vl = vlist;
    if (!vl) return; // 抽屉已关闭（组件销毁）则终止重试
    if (vl.getViewportSize() <= 0) {
      requestAnimationFrame(() => jumpToBottom(count));
      return;
    }
    vl.scrollToIndex(count - 1, { align: "end" });
  }

  // 当前要显示的日志：按级别与“仅该级别”过滤后的结果（依赖变化时自动重算）。
  const shown = $derived(
    filterLines(logStore.lines, logStore.level, logStore.only),
  );

  // 贴底跟随：数据变化（打开抽屉回填 / 新日志到达 / 过滤结果更新）时滚到最后一行，
  // 用户无需手动滚动到底。推迟到微任务执行，确保 VList 内部已按新数据完成布局。
  $effect(() => {
    if (!ui.logOpen) return;
    const list = shown; // 依赖数组引用：回填或过滤变化都会生成新数组
    const n = list.length;
    if (!n || !stick) return;
    void vlist; // 依赖 VList 挂载完成
    queueMicrotask(() => jumpToBottom(n));
  });

  /** 用系统默认程序打开日志文件（路径来自快照的 logPath）。
   *  打开失败（如 .log 无默认关联程序）时降级：在资源管理器里定位该文件，保证按钮总有可见反应。 */
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

<!-- {#if ui.logOpen}：只有抽屉打开时才渲染以下所有内容。 -->
{#if ui.logOpen}
  <!-- 半透明遮罩：点击关闭；键盘关闭由上面的窗口级 Esc 承担，故遮罩本身仅作展示层。 -->
  <div
    class="drawer-backdrop"
    role="presentation"
    onclick={() => (ui.logOpen = false)}
  ></div>
  <aside class="drawer">
    <!-- 顶部工具条：标题 + “仅该级别”开关 + 级别下拉 + 打开文件/清空/关闭按钮。 -->
    <div class="drawer-tools">
      <strong>{t.log.title}</strong>
      <label class="switch">
        <input type="checkbox" bind:checked={logStore.only} />
        <span class="track"></span>
        {t.log.onlyLevel}
      </label>
      <!-- 级别值（DEBUG/INFO…）是过滤协议值，勿改。 -->
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
      <!-- VList 虚拟列表：data 传入全部数据，但只渲染可见区域的行。
           onscroll 用于跟踪用户是否贴底（决定新日志到达时是否自动跟随）。 -->
      <VList
        bind:this={vlist}
        data={shown}
        onscroll={onScroll}
        style="height:100%"
      >
        <!-- {#snippet children(line)} 定义每一行的渲染模板（VList 会逐行回调它）。 -->
        {#snippet children(line: LogLine)}
          <!-- 行内类名用级别小写（如 .error），对应不同颜色。
               字号/等宽/内边距由全局 .drawer-body .log-line 提供。 -->
          <div class="log-line {line.level.toLowerCase()}">
            | {line.ts} | {line.level} | {line.module} | {line.message}
          </div>
        {/snippet}
      </VList>
    </div>
  </aside>
{/if}
