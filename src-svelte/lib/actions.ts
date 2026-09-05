/** Svelte actions（组件通用的 DOM 行为） */
// 【新手导读：什么是 Svelte action】
// action 是一个普通函数，用在模板里写 use:函数名={参数}，用于对某个 DOM 元素做“底层操作”。
// 函数接收该 DOM 节点作为第一个参数，可选地返回 { update, destroy }：
//   update(newValue) —— 参数变化时自动调用；destroy() —— 元素被移除时调用（用于清理监听/定时器）。
// 适合封装那些“与具体组件无关、但需要直接操作 DOM”的行为（如下面的滚动定位）。

/**
 * 只读路径输入框：值或框宽变化后把滚动位置钉到最右侧，
 * 保证长路径的尾部（种族目录 / 文件名）始终可见，左、中部分内容被截断。
 * 用法：`use:pinScrollEnd={snap.currentGroup}`（参数仅用于触发 update）
 */
export function pinScrollEnd(node: HTMLInputElement, _value = '') {
  let raf = 0; // 保存 requestAnimationFrame 的 id，便于取消上一次未执行的帧
  // 把滚动条拉到最右（scrollLeft = 可滚动总宽）。
  const toEnd = () => { node.scrollLeft = node.scrollWidth; };
  const schedule = () => {
    toEnd(); // 布局已就绪时（如只改路径值）当场钉住
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(toEnd); // 首次挂载 / 列宽未定时，等一帧（浏览器重绘后）再钉
  };
  // ResizeObserver 监听元素尺寸变化（窗口缩放时输入框变宽/变窄），一变就重新钉住。
  const ro = new ResizeObserver(schedule);
  ro.observe(node);
  schedule(); // 初始化时先钉一次
  // 返回 action 的生命周期钩子：
  return {
    update(_next: string) { schedule(); }, // 绑定的参数变化时重新钉住
    destroy() {
      cancelAnimationFrame(raf); // 清理未执行的帧
      ro.disconnect();           // 停止尺寸监听，避免内存泄漏
    },
  };
}
