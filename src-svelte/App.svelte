<!-- 根组件：定义整个应用的布局骨架，把侧边栏、主区（配置卡/替换面板/结果区）、状态栏、日志抽屉组合在一起。 -->
<script lang="ts">
  import { onMount } from "svelte"; // 生命周期：组件挂载后执行一次
  import { message } from "@tauri-apps/plugin-dialog"; // 原生弹框
  import Sidebar from "./lib/components/layout/Sidebar.svelte";
  import TopNav from "./lib/components/layout/TopNav.svelte";
  import SetupCard from "./lib/components/setup/SetupCard.svelte";
  import ReplacePanel from "./lib/components/replace/ReplacePanel.svelte";
  import ResultsSection from "./lib/components/results/ResultsSection.svelte";
  import PlayerViewer from "./lib/components/replace/PlayerViewer.svelte";
  import VerifyViews from "./lib/components/results/VerifyViews.svelte";
  import ImageAiPanel from "./lib/components/imageai/ImageAiPanel.svelte";
  import StatusBar from "./lib/components/layout/StatusBar.svelte";
  import LogDrawer from "./lib/components/layout/LogDrawer.svelte";
  import { t } from "./lib/text"; // 全局文案常量（用户可见文字统一在此维护）
  import { appStore, initListeners } from "./lib/stores/app.svelte"; // 订阅后端事件的初始化函数
  import { initQueueListeners } from "./lib/stores/imageai.svelte"; // 生图队列事件订阅（进程级，与队列坞是否挂载无关）
  import { showError } from "./lib/ui/dialog"; // 后台队列刷新失败时弹原生错误框
  import { applyTheme, ui } from "./lib/stores/ui.svelte"; // 深/浅主题令牌切换 + 当前工作台

  // 组件初始化即在 <html> 上写好 data-theme（首帧就是上次记忆的主题，不闪烁）。
  applyTheme();

  // 挂载后订阅后端事件；回调在替换结束时弹窗提示（完成/失败）。
  onMount(() => {
    initListeners(async (result) => {
      if (result === "finished")
        await message(t.app.replaceFinished, { title: t.brand });
      else if (result === "failed")
        await message(t.app.replaceFailed, { title: t.dialog.error });
    });
    // 队列事件订阅不挂在队列坞上：切走 AI 台期间 running/finished 仍照常入账，
    // 侧栏 queueRunning 护栏与回台后的快照/收尾摘要才不会因组件卸载而失真。
    void initQueueListeners(showError);
  });

  // 以下两条提示原挂在步骤引导（Stepper）下方，随改造一并迁到替换台顶部：
  // 它们讲的都是替换的前置条件，只在替换台出现才说得通。
  const missing = $derived(
    [
      !appStore.snapshot || appStore.snapshot.currentProfile === "No Profile"
        ? t.replace.needProfile
        : "",
      appStore.snapshot && !appStore.snapshot.currentGroup
        ? t.replace.needGroup
        : "",
      appStore.snapshot && !appStore.snapshot.currentRtf
        ? t.replace.needRtf
        : "",
    ].filter(Boolean),
  );
</script>

<div class="app">
  <Sidebar />
  <main class="content">
    <TopNav />
    {#if ui.workbench === "archive"}
      <SetupCard />
      <!-- 验证视图：扫描 RTF / 读取快照后就地看结果，不必跳去别的工作台 -->
      <VerifyViews />
    {:else if ui.workbench === "replace"}
      {#if appStore.browseMode}
        <p class="hint muted small">
          {t.replace.browseNotice(appStore.selectedProfile)}
        </p>
      {/if}
      {#if !appStore.replaceReady && missing.length > 0}
        <p class="hint muted small">
          {t.replace.todoPrefix}{missing.join(" · ")}
        </p>
      {/if}
      <!-- 读写分栏：左栏 320 写（批量替换 + 单人替换同屏常驻，不需下钻/返回），右栏弹性读
           （名单/快照结果视图独占全高）。窗口 <1100 时退化为上下堆叠（app.css 的 wb-split 断点）。 -->
      <div class="wb-split">
        <div class="wb-write">
          <ReplacePanel />
          <!-- 单人替换卡自带外壳（.viewer-card 同时是窄栏容器查询锚点），与 ReplacePanel 同栏常驻。 -->
          <PlayerViewer />
        </div>
        <ResultsSection />
      </div>
    {:else}
      <ImageAiPanel />
    {/if}
  </main>
  <StatusBar />
</div>

<!-- 日志抽屉浮在最上层（由 ui.logOpen 控制显示/隐藏），故放在主布局之外。 -->
<LogDrawer />
