<!-- 结果区的 Tab 切换器：在「名单预览」「单人微调」「快照预览」三个子组件之间切换。 -->
<script lang="ts">
  import PlayersTable from './PlayersTable.svelte';
  import PlayerViewer from './PlayerViewer.svelte';
  import SnapshotPreview from './SnapshotPreview.svelte';
  import { t } from '../text'; // 全局文案常量
  import { appStore } from '../stores/app.svelte';

  // 当前 Tab 存全局态（appStore.resultsView）：「读取快照」成功后要从当前任务卡跨组件切过来。
  const view = $derived(appStore.resultsView);
</script>

<div class="card grow">
  <!-- 三个分段按钮，选中的加 active 类。 -->
  <div class="seg-tabs">
    <button class:active={view === 'table'} onclick={() => (appStore.resultsView = 'table')}>{t.results.tabTable}</button>
    <button class:active={view === 'viewer'} onclick={() => (appStore.resultsView = 'viewer')}>{t.results.tabViewer}</button>
    <button class:active={view === 'snapshot'} onclick={() => (appStore.resultsView = 'snapshot')}>{t.results.tabSnapshot}</button>
  </div>
  <div class="card-body">
    <!-- 根据 view 只渲染其中一个子组件。 -->
    {#if view === 'table'}
      <PlayersTable />
    {:else if view === 'viewer'}
      <PlayerViewer />
    {:else}
      <SnapshotPreview />
    {/if}
  </div>
</div>
