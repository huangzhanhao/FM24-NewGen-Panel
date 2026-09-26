<!-- 根组件：定义整个应用的布局骨架，把侧边栏、主区（配置卡/替换面板/结果区）、状态栏、日志抽屉组合在一起。 -->
<script lang="ts">
  import { onMount } from "svelte"; // 生命周期：组件挂载后执行一次
  import { message } from "@tauri-apps/plugin-dialog"; // 原生弹框
  import Sidebar from "./lib/components/layout/Sidebar.svelte";
  import Stepper from "./lib/components/layout/Stepper.svelte";
  import SetupCard from "./lib/components/setup/SetupCard.svelte";
  import ReplacePanel from "./lib/components/replace/ReplacePanel.svelte";
  import ResultsSection from "./lib/components/results/ResultsSection.svelte";
  import StatusBar from "./lib/components/layout/StatusBar.svelte";
  import LogDrawer from "./lib/components/layout/LogDrawer.svelte";
  import { t } from "./lib/text"; // 全局文案常量（用户可见文字统一在此维护）
  import { initListeners } from "./lib/stores/app.svelte"; // 订阅后端事件的初始化函数
  import { applyTheme } from "./lib/stores/ui.svelte"; // 深/浅主题令牌切换

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
  });
</script>

<div class="app">
  <Sidebar />
  <main class="content">
    <Stepper />
    <SetupCard />
    <ReplacePanel />
    <ResultsSection />
  </main>
  <StatusBar />
</div>

<!-- 日志抽屉浮在最上层（由 ui.logOpen 控制显示/隐藏），故放在主布局之外。 -->
<LogDrawer />
