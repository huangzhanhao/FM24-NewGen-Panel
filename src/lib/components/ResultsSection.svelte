<script lang="ts">
  import PlayersTable from './PlayersTable.svelte';
  import PlayerViewer from './PlayerViewer.svelte';

  let view = $state<'table' | 'viewer'>('table');
</script>

<div class="card grow">
  <div class="seg-tabs">
    <button class:active={view === 'table'} onclick={() => (view = 'table')}>名单预览</button>
    <button class:active={view === 'viewer'} onclick={() => (view = 'viewer')}>单人微调</button>
  </div>
  <div class="card-body">
    {#if view === 'table'}
      <PlayersTable />
    {:else}
      <PlayerViewer />
    {/if}
  </div>
</div>
