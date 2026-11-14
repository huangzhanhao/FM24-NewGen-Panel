/**
 * 前端入口：把根组件挂载到 index.html 的 #app 元素上。
 *
 * 同一个页面承载两种窗口：主应用（App）与独立帮助窗口（HelpWindow）。
 * 帮助窗口由 helpWindow.ts 以 `index.html?panel=help&tab=…` 打开，
 * 这里只渲染文档本体，避免开第二个完整应用实例（重复订阅后端事件、重复拉数据）。
 */
import { mount } from 'svelte';
import './app.css'; // 全局样式（副作用导入）
import App from './App.svelte';
import HelpWindow from './lib/components/help/HelpWindow.svelte';

const target = document.getElementById('app')!;
const isHelpWindow = new URLSearchParams(location.search).get('panel') === 'help';

// 两个入口各自 mount 一次：mount 的返回类型与组件类型绑定，三元合并会丢失类型
const app = isHelpWindow ? mount(HelpWindow, { target }) : mount(App, { target });
export default app; // 导出实例，便于按需调用 app.unmount 等
