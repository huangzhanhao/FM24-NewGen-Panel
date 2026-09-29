<!-- 名单预览 / 快照预览的二选一视图卡：存档资料台与替换台共用同一组件，职责按使用处分化——
     存档资料台是「验证视图」（这份名单/快照扫得对不对），替换台是「结果视图」（替换完变成什么样了，
     dense 精简列集）。两处用途不同，视图状态也各存各的：存档台用 ui.verifyView，替换台用
     appStore.resultsView（「读取快照」成功后要从当前任务卡跨组件切换的历史口径），
     经 bind:view 由使用方注入——组件本身不关心状态住哪，两边状态也不互相串。 -->
<script lang="ts">
  import PlayersTable from "./PlayersTable.svelte";
  import SnapshotPreview from "./SnapshotPreview.svelte";
  import { t } from "../../text"; // 全局文案常量

  let {
    view = $bindable(),
    dense = false,
  }: { view: "table" | "snapshot"; dense?: boolean } = $props();
</script>

<div class="card grow">
  <div class="seg-tabs">
    <button class:active={view === "table"} onclick={() => (view = "table")}
      >{t.results.tabTable}</button
    >
    <button class:active={view === "snapshot"} onclick={() => (view = "snapshot")}
      >{t.results.tabSnapshot}</button
    >
  </div>
  <div class="card-body">
    <!-- 条件渲染：切走即卸载，本地搜索/页码/排序随之重置（两处使用同口径）。 -->
    {#if view === "table"}
      <PlayersTable {dense} />
    {:else}
      <SnapshotPreview />
    {/if}
  </div>
</div>
