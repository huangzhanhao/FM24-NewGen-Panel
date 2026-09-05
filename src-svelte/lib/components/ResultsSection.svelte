<!-- 【新手导读】结果区的“Tab 切换器”：在「名单预览」和「单人微调」两个子组件之间切换。
     这里演示 Svelte 模板的两个核心语法：{#if}...{:else}...{/if} 条件渲染，以及 onclick 事件绑定。 -->
<script lang="ts">
  import PlayersTable from './PlayersTable.svelte';
  import PlayerViewer from './PlayerViewer.svelte';

  // $state 创建响应式变量：view 变化时，下面的 {#if} 会重新求值，自动切换显示的子组件。
  // 'table' | 'viewer' 是字面量联合类型，限定 view 只能是这两个值之一。
  let view = $state<'table' | 'viewer'>('table');
</script>

<!-- class="card grow" 的容器；grow 让它占满剩余高度。 -->
<div class="card grow">
  <!-- 顶部两个分段按钮；class:active 是 Svelte 的“条件类名”：表达式为真时才加上 active 类。 -->
  <div class="seg-tabs">
    <button class:active={view === 'table'} onclick={() => (view = 'table')}>名单预览</button>
    <button class:active={view === 'viewer'} onclick={() => (view = 'viewer')}>单人微调</button>
  </div>
  <div class="card-body">
    <!-- {#if}/{:else}：根据 view 只渲染其中一个子组件（另一个从 DOM 中移除）。 -->
    {#if view === 'table'}
      <PlayersTable />
    {:else}
      <PlayerViewer />
    {/if}
  </div>
</div>
