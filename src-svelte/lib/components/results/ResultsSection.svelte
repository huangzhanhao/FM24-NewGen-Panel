<!-- 替换台的「结果视图」（读写分栏的读侧）：名单预览 / 快照预览两个读视图之间切换。
     单人替换已移出为替换台左栏常驻卡；AI 生图已独立为第三个工作台（顶栏切换）。 -->
<script lang="ts">
  import PlayersTable from './PlayersTable.svelte';
  import SnapshotPreview from './SnapshotPreview.svelte';
  import { t } from '../../text'; // 全局文案常量
  import { appStore } from '../../stores/app.svelte';

  // 当前视图存全局态（appStore.resultsView）：「读取快照」成功后要从当前任务卡跨组件切过来。
  const view = $derived(appStore.resultsView);
</script>

<div class="card grow">
  <!-- 两个分段按钮，选中的加 active 类。 -->
  <div class="seg-tabs">
    <button class:active={view === 'table'} onclick={() => (appStore.resultsView = 'table')}>{t.results.tabTable}</button>
    <button class:active={view === 'snapshot'} onclick={() => (appStore.resultsView = 'snapshot')}>{t.results.tabSnapshot}</button>
  </div>
  <div class="card-body">
    <!-- 条件渲染：切走即卸载，本地搜索/页码/排序随之重置（与存档资料台验证视图同口径）。
         dense：替换台右栏 670 窄栏用精简列集（次要列可勾选展开）；存档资料台的全宽验证视图不传、显示完整 18 列。 -->
    {#if view === 'table'}
      <PlayersTable dense />
    {:else if view === 'snapshot'}
      <SnapshotPreview />
    {/if}
    <!-- resultsView 联合里残留的 'viewer' / 'generate' 已无写入点也无渲染分支，S12 收尾时从类型上删除。 -->
  </div>
</div>
