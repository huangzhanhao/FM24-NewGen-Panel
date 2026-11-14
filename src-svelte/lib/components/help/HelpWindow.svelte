<!-- HelpWindow · 独立帮助窗口的根组件：同一 index.html 带 ?panel=help 打开，
     只渲染文档本体（不含主应用布局）。窗口是真实的 OS 窗口，可拖出主窗口、跨显示器。
     主窗口切换语言/主题时通过 Tauri 事件同步到这里。 -->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import HelpPanelBody from "./HelpPanelBody.svelte";
  import type { HelpTab } from "../../help";
  import { applyTheme, ui, type Lang, type Theme } from "../../stores/ui.svelte";
  import { t } from "../../text";

  // 开窗时的页签来自 URL；之后再点侧栏入口由 help:tab 事件切换
  const params = new URLSearchParams(location.search);
  let tab = $state<HelpTab>(params.get("tab") === "faq" ? "faq" : "tutorial");

  // 首帧前写 data-theme，避免窗口出现时闪一下默认配色
  applyTheme();

  // 窗口标题跟随界面语言（浏览器回退时改标签页标题）
  $effect(() => {
    const title = t.help.title;
    if (!("__TAURI_INTERNALS__" in window)) {
      document.title = title;
      return;
    }
    void getCurrentWindow().setTitle(title);
  });

  const unlisten: Array<() => void> = [];
  onMount(() => {
    // 浏览器里直接开页面（pnpm dev 调试）没有 Tauri 事件通道，跳过订阅即可
    if (!("__TAURI_INTERNALS__" in window)) return;
    void listen<HelpTab>("help:tab", (e) => (tab = e.payload)).then((un) => unlisten.push(un));
    void listen<Lang>("help:lang", (e) => (ui.lang = e.payload)).then((un) => unlisten.push(un));
    void listen<Theme>("help:theme", (e) => {
      ui.theme = e.payload;
      applyTheme();
    }).then((un) => unlisten.push(un));
  });

  onDestroy(() => unlisten.forEach((un) => un()));
</script>

<HelpPanelBody {tab} onTabChange={(t) => (tab = t)} />
