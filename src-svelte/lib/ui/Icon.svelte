<!-- 线性图标统一出口：包掉每处 5-8 行的内联 svg 属性样板（viewBox/fill/stroke/linecap/linejoin/aria-hidden）。
     filled 图标（play）按 icons 表的 filled 标记自动改用 fill=currentColor、免 stroke 属性；
     尺寸两种给法：传 size 渲染 width/height 属性，或不传、由调用方 CSS 类定尺寸（与旧手写一一对应）。 -->
<script lang="ts">
  import { icons, type IconDef } from "./icons";

  type IconName = keyof typeof icons;

  let {
    name,
    class: cls = "",
    size,
    stroke = 2,
    filled,
  }: {
    name: IconName;
    class?: string;
    size?: number;
    stroke?: number;
    /** 不传 = 按 icons 表的 filled 标记；显式 false 可把实心图标画成描边（play 有两处以描边使用）。 */
    filled?: boolean;
  } = $props();

  const def: IconDef = $derived(icons[name]);
  const solid = $derived(filled ?? !!def.filled);
</script>

<svg
  viewBox="0 0 24 24"
  width={size}
  height={size}
  fill={solid ? "currentColor" : "none"}
  stroke={solid ? undefined : "currentColor"}
  stroke-width={solid ? undefined : stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  class={cls}>{@html def.body}</svg>
