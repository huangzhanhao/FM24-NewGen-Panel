/**
 * 帮助窗口的开窗/复用逻辑。
 *
 * 文档面板被设计成真实 OS 窗口而不是应用内浮层：浮层是 WebView 里的 DOM，
 * 无论怎么定位都会被窗口客户区裁掉，只有独立窗口才能拖到主窗口外、跨显示器摆放。
 *
 * 命名沿用 help（而非 docs）：仓库里已有 docs/ 设计文档目录，避免混淆。
 */
import { emitTo } from "@tauri-apps/api/event";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { HelpTab } from "./help";
import { t } from "./text";

/** 窗口标签：复用/聚焦/事件定向都靠它，全局唯一。 */
const LABEL = "help";

/** 是否运行在 Tauri 容器里；浏览器直接调试 `pnpm dev` 时没有该内部对象。 */
function inTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

/**
 * 打开帮助窗口。
 * - 已存在：提到前台并切到指定页签（不重建，保留用户当前滚动位置）；
 * - 不存在：新建一个普通窗口，位置交给系统居中，用户可自行拖出主窗口。
 */
export async function openHelpWindow(tab: HelpTab): Promise<void> {
  if (!inTauri()) {
    // 浏览器调试回退：用新标签页承载同一份 ?panel=help 页面
    window.open(`?panel=help&tab=${tab}`, LABEL);
    return;
  }

  const existing = await WebviewWindow.getByLabel(LABEL);
  if (existing) {
    await existing.setFocus();
    await emitTo(LABEL, "help:tab", tab);
    return;
  }

  const win = new WebviewWindow(LABEL, {
    url: `index.html?panel=help&tab=${tab}`,
    title: t.help.title,
    width: 1000,
    height: 720,
    minWidth: 560,
    minHeight: 420,
    resizable: true,
    center: true,
  });
  // 等窗口真正建好再返回：紧接着再点侧栏时 getByLabel 才能命中，不会连开两个
  await new Promise<void>((resolve) => {
    win.once("tauri://created", () => resolve());
    win.once("tauri://error", () => resolve());
  });
}
