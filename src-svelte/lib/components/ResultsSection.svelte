<!-- 结果区的 Tab 切换器：在「名单预览」和「单人微调」两个子组件之间切换。 -->
<script lang="ts">
  import PlayersTable from './PlayersTable.svelte';
  import PlayerViewer from './PlayerViewer.svelte';
  import { t } from '../text'; // 全局文案常量

  // 当前 Tab：'table'=名单预览，'viewer'=单人微调。
  let view = $state<'table' | 'viewer'>('table');
</script>

<div class="card grow">
  <!-- 两个分段按钮，选中的加 active 类。 -->
  <div class="seg-tabs">
    <button class:active={view === 'table'} onclick={() => (view = 'table')}>{t.results.tabTable}</button>
    <button class:active={view === 'viewer'} onclick={() => (view = 'viewer')}>{t.results.tabViewer}</button>
  </div>
  <div class="card-body">
    <!-- 根据 view 只渲染其中一个子组件。 -->
    {#if view === 'table'}
      <PlayersTable />
    {:else}
      <PlayerViewer />
    {/if}
  </div>
</div>
