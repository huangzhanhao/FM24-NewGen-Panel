/// <reference types="svelte" />
/// <reference types="vite/client" />
/**
 * 全局类型声明。
 *
 * - `svelte`：引入 svelte 包自带的 `declare module '*.svelte'`，让 `.ts` 里 `import App from './App.svelte'` 能被识别。
 * - `vite/client`：让 TS 认识 `import logo from './x.png'`、`import './app.css'` 等静态资源导入（结果是打包后的 URL）。
 */
declare module '*.svelte' {
  import type { Component } from 'svelte';
  const component: Component;
  export default component;
}
