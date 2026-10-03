<!-- RosterSnapshotTabs · 名单预览 / 快照预览的二选一视图卡：存档资料台与替换台共用
     存档台是「验证视图」（这份名单/快照扫得对不对），替换台是「结果视图」（替换后变成什么样，dense 精简列集）
     Tab 选择两处共用同一份 appStore.rtfPreview（经 bind:view 由使用方注入）；
     表内视图态（页码/搜索/排序/筛选/列展开）是组件本地态，切工作台或切 Tab 卸载后回到初始
     卡片几何（内边距/Tab 行留白）走全局 .card / .seg-tabs，无工作台特例 -->
<script lang="ts">
  import PlayersTable from "./PlayersTable.svelte";
  import SnapshotPreview from "./SnapshotPreview.svelte";
  import { t } from "../../text"; // 全局文案常量

  let { view = $bindable(), dense = false }: { view: "table" | "snapshot"; dense?: boolean } = $props();
</script>

<div class="card grow">
  <div class="seg-tabs">
    <button class:active={view === "table"} onclick={() => (view = "table")}>{t.results.tabTable}</button>
    <button class:active={view === "snapshot"} onclick={() => (view = "snapshot")}>{t.results.tabSnapshot}</button>
  </div>
  <div class="card-body">
    <!-- 条件渲染：切走即卸载，本地搜索/页码/排序随之重置（Tab 选择本身存在 appStore，不受影响） -->
    {#if view === "table"}
      <PlayersTable {dense} />
    {:else}
      <SnapshotPreview />
    {/if}
  </div>
</div>
