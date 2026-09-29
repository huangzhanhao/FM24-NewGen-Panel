<!-- 存档资料台的「验证视图」：名单预览 / 快照预览。
     与替换台的「结果视图」共用同一批子组件（PlayersTable / SnapshotPreview），但职责不同：
     这里回答「这份名单 / 这份快照扫得对不对」，替换台那里回答「替换完变成什么样了」。
     视图状态因此也各用各的（ui.verifyView vs appStore.resultsView），不互相串。 -->
<script lang="ts">
  import PlayersTable from './PlayersTable.svelte';
  import SnapshotPreview from './SnapshotPreview.svelte';
  import { t } from '../../text'; // 全局文案常量
  import { ui } from '../../stores/ui.svelte';
</script>

<div class="card grow">
  <div class="seg-tabs">
    <button
      class:active={ui.verifyView === 'table'}
      onclick={() => (ui.verifyView = 'table')}>{t.results.tabTable}</button
    >
    <button
      class:active={ui.verifyView === 'snapshot'}
      onclick={() => (ui.verifyView = 'snapshot')}>{t.results.tabSnapshot}</button
    >
  </div>
  <div class="card-body">
    <!-- 条件渲染：切走即卸载，本地搜索 / 页码 / 排序随之重置（与替换台结果区同口径）。 -->
    {#if ui.verifyView === 'table'}
      <PlayersTable />
    {:else}
      <SnapshotPreview />
    {/if}
  </div>
</div>
