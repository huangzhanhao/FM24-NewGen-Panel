/** Svelte actions（组件通用的 DOM 行为） */

/**
 * 只读路径输入框：值或框宽变化后把滚动位置钉到最右侧，
 * 保证长路径的尾部（种族目录 / 文件名）始终可见，左、中部分内容被截断。
 * 用法：`use:pinScrollEnd={snap.currentGroup}`（参数仅用于触发 update）
 */
export function pinScrollEnd(node: HTMLInputElement, _value = '') {
  let raf = 0;
  const toEnd = () => { node.scrollLeft = node.scrollWidth; };
  const schedule = () => {
    toEnd(); // 布局已就绪时（如只改路径值）当场钉住
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(toEnd); // 首次挂载 / 列宽未定时，等一帧再钉
  };
  const ro = new ResizeObserver(schedule);
  ro.observe(node);
  schedule();
  return {
    update(_next: string) { schedule(); },
    destroy() {
      cancelAnimationFrame(raf);
      ro.disconnect();
    },
  };
}
