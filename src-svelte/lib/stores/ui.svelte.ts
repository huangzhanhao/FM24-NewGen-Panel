/**
 * 跨组件 UI 状态：日志抽屉开关、深/浅主题等（与业务数据无关的界面状态）。
 *
 * 文件名必须带 .svelte：Svelte 5 约定在组件外的 .ts 里使用 $state 等 rune 时，
 * 只有 .svelte.ts 会被编译器按响应式源码处理，普通 .ts 不行。
 */

/** 主题取值：dark=深色「教练席海军蓝」（默认），light=浅色「日间球场」。 */
export type Theme = 'dark' | 'light';
const THEME_KEY = 'ngm-theme'; // localStorage 记忆键名

/** 工作台：'archive'=存档资料 / 'replace'=批量·单人头像替换 / 'ai'=AI生成头像。
 *  顺序即主流程顺序（先备料 → 再替换 → 可选 AI 生图），顶栏序号圆按此顺序排列。 */
export type Workbench = 'archive' | 'replace' | 'ai';

/** 启动时读取上次选择的主题（没有记录时默认深色）。 */
function readTheme(): Theme {
  try {
    return localStorage.getItem(THEME_KEY) === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark'; // 隐私模式等无法访问 localStorage 时静默回退
  }
}

export const ui = $state({
  logOpen: false, // 日志抽屉是否打开
  theme: readTheme() as Theme, // 当前主题（写入 <html data-theme> 切换整套 CSS 令牌）
  /** 当前工作台。与 logOpen 同类——它是「界面在哪一屏」，不是业务数据，故放这里而非 appStore；
   *  也不持久化：每次启动都从「存档资料」开始，用户先看清手里有什么料。 */
  workbench: 'archive' as Workbench,
  /** 存档资料台的验证视图当前 Tab。与替换台的 appStore.resultsView **各用各的**：
   *  两处共用同一批子组件，但用途不同（这里验证备料、那里看执行结果），状态不该互相串。 */
  verifyView: 'table' as 'table' | 'snapshot',
});

/** 把当前主题写到 <html data-theme>——这是深/浅两套 CSS 变量的唯一切换开关。 */
export function applyTheme() {
  document.documentElement.dataset.theme = ui.theme;
}

/** 深/浅切换，并把选择持久化到 localStorage（下次启动沿用）。 */
export function toggleTheme() {
  ui.theme = ui.theme === 'dark' ? 'light' : 'dark';
  try {
    localStorage.setItem(THEME_KEY, ui.theme);
  } catch {
    // 存储不可用时仅对本次会话生效
  }
  applyTheme();
}
