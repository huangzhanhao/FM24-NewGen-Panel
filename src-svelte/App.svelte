<!-- 【新手导读】根组件，定义整个应用的布局骨架。
     Svelte 组件 = 一个 .svelte 文件，由三部分构成：
       <script> —— 逻辑（变量、函数、导入）；模板 —— HTML 结构；<style> —— 仅作用于本组件的样式。
     本组件把侧边栏、主区（配置卡/替换面板/结果区）、状态栏、日志抽屉组合在一起。 -->
<script lang="ts">
  // lang="ts" 表示这个 script 块用 TypeScript 写。
  import { onMount } from 'svelte'; // Svelte 生命周期：组件挂载到页面后执行一次
  import { message } from '@tauri-apps/plugin-dialog'; // 原生弹框
  // 下面导入各个子组件（Svelte 里组件名首字母大写，在模板中当标签用）。
  import Sidebar from './lib/components/Sidebar.svelte';
  import SetupCard from './lib/components/SetupCard.svelte';
  import ReplacePanel from './lib/components/ReplacePanel.svelte';
  import ResultsSection from './lib/components/ResultsSection.svelte';
  import StatusBar from './lib/components/StatusBar.svelte';
  import LogDrawer from './lib/components/LogDrawer.svelte';
  import { initListeners } from './lib/stores/app.svelte'; // 订阅后端事件的初始化函数

  // onMount 里的代码在组件首次渲染到页面后运行（适合做“启动时订阅事件”这类副作用）。
  onMount(() => {
    // initListeners 订阅后端的进度/完成/日志事件；传入一个回调，在替换结束时弹窗提示。
    initListeners(async (result) => {
      if (result === 'finished') await message('Finished! :)', { title: 'NewGAN Manager' });
      else if (result === 'failed') await message('替换失败，请查看日志', { title: 'Error' });
    });
  });
</script>

<!-- 下面是模板（HTML）：class="app" 的容器采用网格/弹性布局，具体样式在 app.css 里。 -->
<div class="app">
  <Sidebar />
  <main class="content">
    <SetupCard />
    <ReplacePanel />
    <ResultsSection />
  </main>
  <StatusBar />
</div>

<!-- 日志抽屉浮在最上层（由 ui.logOpen 控制显示/隐藏），故放在主布局之外。 -->
<LogDrawer />
