// 【新手导读】前端入口文件。Tauri 窗口里加载的就是这个网页（见 index.html 引入本文件）。
// 职责：把根组件 App.svelte 挂载到页面的 #app 元素上，启动整个前端。
import { mount } from 'svelte'; // Svelte 5 的挂载函数（旧版是 new App(...)，5 改为 mount）
import './app.css';             // 引入全局样式（副作用导入，无需变量接收）
import App from './App.svelte'; // 根组件

// mount(组件, { target }) 把组件渲染进 target 指向的 DOM 节点。
// document.getElementById('app')! 末尾的 ! 是 TS 非空断言：我们确信该元素存在（index.html 里有）。
const app = mount(App, { target: document.getElementById('app')! });
export default app; // 导出实例，便于需要时调用 app.unmount() 等
