<!-- 结果区的 Tab 切换器：在「名单预览」「单人微调」「快照预览」「AI 生图」四个子组件之间切换。 -->
<script lang="ts">
  import PlayersTable from './PlayersTable.svelte';
  import PlayerViewer from './PlayerViewer.svelte';
  import SnapshotPreview from './SnapshotPreview.svelte';
  import ImageAiPanel from '../imageai/ImageAiPanel.svelte';
  import { t } from '../../text'; // 全局文案常量
  import { appStore } from '../../stores/app.svelte';

  // 当前 Tab 存全局态（appStore.resultsView）：「读取快照」成功后要从当前任务卡跨组件切过来。
  const view = $derived(appStore.resultsView);
</script>

<div class="card grow">
  <!-- 四个分段按钮，选中的加 active 类。 -->
  <div class="seg-tabs">
    <button class:active={view === 'table'} onclick={() => (appStore.resultsView = 'table')}>{t.results.tabTable}</button>
    <button class:active={view === 'viewer'} onclick={() => (appStore.resultsView = 'viewer')}>{t.results.tabViewer}</button>
    <button class:active={view === 'snapshot'} onclick={() => (appStore.resultsView = 'snapshot')}>{t.results.tabSnapshot}</button>
    <button class:active={view === 'generate'} onclick={() => (appStore.resultsView = 'generate')}>{t.gen.tabGenerate}</button>
  </div>
  <div class="card-body">
    <!-- 名单预览 / 快照预览保持条件渲染：切走即卸载，本地搜索/页码/排序随之重置。 -->
    {#if view === 'table'}
      <PlayersTable />
    {:else if view === 'snapshot'}
      <SnapshotPreview />
    {/if}
    <!-- 单人替换常驻挂载（保活）：切 tab 只显隐不销毁，UID/预览资料/图片/替换路径
         全部保留，直至用户重新搜索；隐藏层 display:none 不占布局、不进 Tab 焦点序。 -->
    <div class="tab-pane" class:active={view === 'viewer'}>
      <PlayerViewer />
    </div>
    <div class="tab-pane" class:active={view === 'generate'}>
      <ImageAiPanel />
    </div>
  </div>
</div>
