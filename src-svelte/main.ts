/**
 * 前端入口：把根组件 App.svelte 挂载到 index.html 的 #app 元素上，启动整个前端。
 * Tauri 窗口加载的就是这个页面（index.html 引入本文件）。
 */
import { mount } from 'svelte';
import './app.css'; // 全局样式（副作用导入）
import App from './App.svelte';

// mount(组件, { target }) 把组件渲染进 target 指向的 DOM 节点。
// document.getElementById('app')! 末尾的 ! 是 TS 非空断言：我们确信该元素存在（index.html 里有）。
const app = mount(App, { target: document.getElementById('app')! });
export default app; // 导出实例，便于按需调用 app.unmount 等
