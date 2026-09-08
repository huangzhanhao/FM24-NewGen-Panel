<!-- 【新手导读】日志抽屉（从右侧滑出的面板）：按级别过滤展示日志，支持打开日志文件、清空显示。
     重点演示：$effect 依赖追踪、虚拟列表 VList（只渲染可见行，万条日志也不卡）、snippet 插槽。 -->
<script lang="ts">
  import { VList } from 'virtua/svelte'; // 虚拟滚动列表组件（只渲染视口内的行，提升大数据量性能）
  import { openPath } from '@tauri-apps/plugin-opener'; // 用系统默认程序打开本地文件
  import { appStore, reloadLogs } from '../stores/app.svelte';
  import { clearDisplay, filterLines, LEVELS, logStore } from '../stores/log.svelte';
  import { ui } from '../stores/ui.svelte';
  import { t } from '../text'; // 全局文案常量
  import type { LogLine } from '../types';

  // 抽屉打开期间，过滤条件变化 → 从 Rust 环形缓冲重查（前端缓冲仅承接事件增量）。
  // $effect 里显式“读”了 level 和 only（void 表达式仅为触发依赖追踪），所以它们一变就重新拉取。
  $effect(() => {
    if (!ui.logOpen) return; // 抽屉关闭时不做任何事
    void logStore.level; // void 读取：仅为让 $effect 依赖这个值（不使用其返回值）
    void logStore.only;
    void reloadLogs();
  });

  // 当前要显示的日志：按级别与“仅该级别”过滤后的结果（依赖变化时自动重算）。
  const shown = $derived(filterLines(logStore.lines, logStore.level, logStore.only));

  /** 用系统默认程序打开日志文件（路径来自快照的 logPath）。 */
  async function openLogFile() {
    if (appStore.snapshot?.logPath) await openPath(appStore.snapshot.logPath);
  }
</script>

<!-- {#if ui.logOpen}：只有抽屉打开时才渲染以下所有内容。 -->
{#if ui.logOpen}
  <!-- 半透明遮罩：点它或按 Esc 关闭抽屉。role/tabindex/aria-label 是为了无障碍与键盘可聚焦。 -->
  <div class="drawer-backdrop" role="button" tabindex={0} aria-label={t.log.closeAria} onclick={() => (ui.logOpen = false)} onkeydown={(e) => e.key === 'Escape' && (ui.logOpen = false)}></div>
  <aside class="drawer">
    <!-- 顶部工具条：标题 + “仅该级别”复选框 + 级别下拉 + 打开文件/清空/关闭按钮。 -->
    <div class="row">
      <strong>{t.log.title}</strong>
      <label class="small"><input type="checkbox" bind:checked={logStore.only} /> {t.log.onlyLevel}</label>
      <!-- bind:value 把下拉选中值与 logStore.level 双向绑定。级别值（DEBUG/INFO…）是过滤协议值，勿改。 -->
      <select bind:value={logStore.level}>
        {#each LEVELS as l}<option value={l}>{l}</option>{/each}
      </select>
      <span class="flex-1"></span>
      <button class="btn" onclick={openLogFile}>{t.log.openFileBtn}</button>
      <button class="btn" onclick={clearDisplay}>{t.log.clearBtn}</button>
      <button class="icon-btn" onclick={() => (ui.logOpen = false)}>×</button>
    </div>
    <div class="drawer-body">
      <!-- VList 虚拟列表：data 传入全部数据，但只渲染可见区域的行。 -->
      <VList data={shown} style="height:100%" class="mono small">
        <!-- {#snippet children(line)} 定义每一行的渲染模板（VList 会逐行回调它）。 -->
        {#snippet children(line: LogLine)}
          <!-- 行内类名用级别小写（如 .error），对应不同颜色；padEnd(8) 把级别补齐到 8 字符对齐。 -->
          <div class="log-line {line.level.toLowerCase()}" style="padding:1px 8px;white-space:nowrap">
            | {line.ts} | {line.level.padEnd(8)} | {line.module} | {line.message}
          </div>
        {/snippet}
      </VList>
    </div>
  </aside>
{/if}
