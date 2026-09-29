/**
 * SVG 图标集中管理（不引入任何图标库依赖）。
 *
 * 所有图标都是 24×24 的线性 SVG，stroke 使用 currentColor（跟随文字颜色），便于统一线宽与圆角风格，
 * 一处改图标全站生效。组件层一律经共享的 <Icon> 消费（lib/ui/Icon.svelte）：
 *   import Icon from '../../ui/Icon.svelte';
 *   <Icon name="folder" />              线宽默认 2；filled 图标自动切实心
 *   <Icon name="x" size={13} />         传 size 渲染 width/height，否则由调用方 CSS 类定尺寸
 *   <Icon name="alert" stroke={1.8} />
 * satisfies 让本表键名是字面量集合：Icon 的 name 拼错在编译期报错。
 */
export interface IconDef {
  /** 放进 <svg> 内部的路径/形状标记 */
  body: string;
  /** true = 实心图标（fill=currentColor），默认线性（stroke=currentColor） */
  filled?: boolean;
}

export const icons = {
  plus: { body: '<path d="M12 5v14M5 12h14" />' },
  folder: { body: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2.5h8a2 2 0 0 1 2 2V18a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />' },
  file: {
    body: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" /><path d="M14 3v5h5" />',
  },
  gear: {
    body: '<circle cx="12" cy="12" r="3.2" />'
      + '<path d="M19.4 13.5a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.56V19.6a2 2 0 1 1-4 0v-.08a1.7 1.7 0 0 0-1.11-1.56 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.56-1.03H2.9a2 2 0 1 1 0-4h.08a1.7 1.7 0 0 0 1.56-1.11 1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34H9a1.7 1.7 0 0 0 1.03-1.56V2.9a2 2 0 1 1 4 0v.08a1.7 1.7 0 0 0 1.03 1.56 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87V9a1.7 1.7 0 0 0 1.56 1.03h.08a2 2 0 1 1 0 4h-.08a1.7 1.7 0 0 0-1.58 1.47z" />',
  },
  search: { body: '<circle cx="11" cy="11" r="7" /><path d="m20 20-3.8-3.8" />' },
  play: { body: '<path d="M7 5.5v13a1 1 0 0 0 1.54.84l10-6.5a1 1 0 0 0 0-1.68l-10-6.5A1 1 0 0 0 7 5.5z" />', filled: true },
  check: { body: '<path d="M20 6 9 17l-5-5" />' },
  x: { body: '<path d="M18 6 6 18M6 6l12 12" />' },
  chevronLeft: { body: '<path d="m15 6-6 6 6 6" />' },
  chevronRight: { body: '<path d="m9 6 6 6-6 6" />' },
  chevronUp: { body: '<path d="m6 15 6-6 6 6" />' },
  chevronDown: { body: '<path d="m6 9 6 6 6-6" />' },
  moon: { body: '<path d="M21 12.8A8.5 8.5 0 1 1 11.2 3 6.6 6.6 0 0 0 21 12.8z" />' },
  sun: {
    body: '<circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />',
  },
  image: {
    body: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-5-5L5 21" />',
  },
  download: {
    body: '<path d="M12 3v12m0 0 4-4m-4 4-4-4"/><path d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2" />',
  },
  refresh: { body: '<path d="M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6" />' },
  dice: {
    body: '<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="8.5" cy="8.5" r="1.3" fill="currentColor" stroke="none"/><circle cx="15.5" cy="8.5" r="1.3" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1.3" fill="currentColor" stroke="none"/><circle cx="8.5" cy="15.5" r="1.3" fill="currentColor" stroke="none"/><circle cx="15.5" cy="15.5" r="1.3" fill="currentColor" stroke="none"/>',
  },
  zoom: { body: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/><path d="M11 8v6M8 11h6" />' },
  alert: {
    body: '<path d="M12 3 2.5 20h19L12 3z"/><path d="M12 10v4.5"/><circle cx="12" cy="17.2" r="0.6" fill="currentColor"/>',
  },
} satisfies Record<string, IconDef>;
