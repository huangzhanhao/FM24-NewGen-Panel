<!-- 根组件：只做布局骨架与启动装配，不含业务规则。
     骨架 = 侧边栏 + 主区（按 ui.workbench 切换三个工作台）+ 状态栏 + 日志抽屉（浮层，置于主布局外）。
     装配 = 首帧前写 data-theme（防主题闪烁）、挂载时订阅后端事件（替换流水线经 notifyReplaceResult
     弹收尾提示，生图队列订阅与队列坞是否挂载无关）。前置条件提示等业务规则单点在 stores 里。 -->
<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
  import Sidebar from "./lib/components/layout/Sidebar.svelte";
  import TopNav from "./lib/components/layout/TopNav.svelte";
  import SetupCard from "./lib/components/setup/SetupCard.svelte";
  import ReplacePanel from "./lib/components/replace/ReplacePanel.svelte";
  import PlayerViewer from "./lib/components/replace/PlayerViewer.svelte";
  import RosterSnapshotTabs from "./lib/components/results/RosterSnapshotTabs.svelte";
  import ImageAiPanel from "./lib/components/imageai/ImageAiPanel.svelte";
  import StatusBar from "./lib/components/layout/StatusBar.svelte";
  import LogDrawer from "./lib/components/layout/LogDrawer.svelte";
  import { t } from "./lib/text";
  import { appStore, initListeners } from "./lib/stores/app.svelte";
  import { initQueueListeners } from "./lib/stores/imageai.svelte";
  import { notifyReplaceResult, showError } from "./lib/ui/dialog";
  import { applyTheme, ui } from "./lib/stores/ui.svelte";

  // 首帧前就得在 <html> 上写好 data-theme：晚一帧就会按默认主题闪一下。
  applyTheme();

  onMount(() => {
    initListeners(notifyReplaceResult);
    // 队列事件订阅不挂在队列坞上：切走 AI 台期间 running/finished 仍照常入账，
    // 侧栏 queueRunning 护栏与回台后的快照/收尾摘要才不会因组件卸载而失真。
    void initQueueListeners(showError);
    // 主窗口关闭时顺手关掉独立帮助窗口：帮助窗口还开着的话，进程会继续驻留、应用像没退出
    if (!("__TAURI_INTERNALS__" in window)) return;
    let unlistenClose: (() => void) | undefined;
    void getCurrentWindow()
      .onCloseRequested(async () => {
        const help = await WebviewWindow.getByLabel("help");
        await help?.close();
      })
      .then((un) => (unlistenClose = un));
    return () => unlistenClose?.();
  });
</script>

<div class="app">
  <Sidebar />
  <main class="content">
    <TopNav />
    {#if ui.workbench === "archive"}
      <SetupCard />
      <!-- 验证视图：扫描 RTF / 读取快照后就地看结果；Tab 存 appStore.rtfPreview 两工作台共享，表内浏览态随组件卸载复位 -->
      <RosterSnapshotTabs bind:view={appStore.rtfPreview} />
    {:else if ui.workbench === "replace"}
      {#if appStore.browseMode}
        <p class="hint muted small">
          {t.replace.browseNotice(appStore.selectedProfile)}
        </p>
      {/if}
      {#if appStore.missingPrerequisites.length > 0}
        <p class="hint muted small">
          {t.replace.todoPrefix}{appStore.missingPrerequisites.join(" · ")}
        </p>
      {/if}
      <!-- 读写分栏：左栏 320 写（批量+单人常驻同栏），右栏弹性读；<1100 退化为上下堆叠（app.css 的 wb-split 断点）。 -->
      <div class="wb-split">
        <div class="wb-write">
          <ReplacePanel />
          <!-- .viewer-card 同时是窄栏容器查询锚点，单人替换卡据此感知本栏宽度。 -->
          <PlayerViewer />
        </div>
        <RosterSnapshotTabs bind:view={appStore.rtfPreview} dense />
      </div>
    {:else}
      <ImageAiPanel />
    {/if}
  </main>
  <StatusBar />
</div>

<!-- 浮在最上层，由 ui.logOpen 控制显隐，故置于主布局之外。 -->
<LogDrawer />
