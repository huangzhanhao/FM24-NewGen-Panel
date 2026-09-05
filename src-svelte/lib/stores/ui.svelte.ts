/** 跨组件 UI 状态：日志抽屉开关等（与业务数据无关的界面状态） */
// 【新手导读：Svelte 5 runes】$state(...) 创建一个“响应式对象”：它的属性被修改时，
// 所有用到它的组件会自动重新渲染。这就是 Svelte 5 的状态管理核心（取代旧版的 writable store）。
// 【文件名为何是 .svelte.ts】这是 Svelte 5 约定：在组件外的 .ts 文件里使用 $state 等 rune，
// 文件名必须带 .svelte，编译器才会把它当作 runes 处理。
export const ui = $state({
  logOpen: false, // 日志抽屉是否打开
});
