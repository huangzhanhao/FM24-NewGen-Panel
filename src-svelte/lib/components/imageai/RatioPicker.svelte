<!-- 画面方向选择器：按当前模型尺寸预设解析比例并归组（1:1/3:4/4:3/9:16/16:9），
     用真实比例的矩形轮廓单选；点选即把该比例对应的官方推荐尺寸写入。
     预设里一个可识别比例都没有时（如纯自由尺寸未给预设）调用方不渲染本组件。 -->
<script lang="ts">
  import { t } from "../../text"; // 全局文案常量

  interface Props {
    sizes: string[];
    value: string;
    onselect: (size: string) => void;
  }
  let { sizes, value, onselect }: Props = $props();

  /** 归一化解析 `1024*1024` / `1024x1024` → [w, h]。 */
  function parse(s: string): [number, number] | null {
    const m = /^(\d+)[x*](\d+)$/.exec(s.trim());
    return m ? [Number(m[1]), Number(m[2])] : null;
  }

  // 目标比例（短边/长边 → 标签），顺序即展示顺序。
  const RATIOS = [
    { label: '1:1', w: 1, h: 1 },
    { label: '3:4', w: 3, h: 4 },
    { label: '4:3', w: 4, h: 3 },
    { label: '9:16', w: 9, h: 16 },
    { label: '16:9', w: 16, h: 9 },
  ] as const;

  /** 从预设归组：每组取第一个命中的预设作为「推荐档」（配置已按官方顺序给好）。 */
  const groups = $derived(
    RATIOS.map((r) => {
      const target = r.w / r.h;
      const hit = sizes.find((s) => {
        const p = parse(s);
        if (!p) return false;
        const q = p[0] / p[1];
        return Math.abs(q - target) / target < 0.03;
      });
      return hit ? { label: r.label, w: r.w, h: r.h, size: hit } : null;
    }).filter((g): g is NonNullable<typeof g> => g !== null),
  );

  const activeLabel = $derived.by(() => {
    const cur = parse(value);
    if (!cur) return null;
    const q = cur[0] / cur[1];
    return (
      groups.find((g) => Math.abs(g.w / g.h - q) / (g.w / g.h) < 0.03)?.label ?? null
    );
  });

  // 矩形轮廓尺寸：最长边 26px、最短边不低于 10px，保证 16:9 仍可辨。
  function box(w: number, h: number): { width: number; height: number } {
    const long = 26;
    const short = Math.max(10, Math.round((long * Math.min(w, h)) / Math.max(w, h)));
    return w >= h ? { width: long, height: short } : { width: short, height: long };
  }
</script>

{#if groups.length > 0}
  <div class="ratio-row" role="radiogroup" aria-label={t.gen.ratioAria}>
    {#each groups as g (g.label)}
      <button
        type="button"
        role="radio"
        aria-checked={activeLabel === g.label}
        class="ratio"
        class:active={activeLabel === g.label}
        title={`${g.label}（${g.size.replace('*', '×')}）`}
        onclick={() => onselect(g.size)}
      >
        <span class="shape" style={`width:${box(g.w, g.h).width}px;height:${box(g.w, g.h).height}px`}></span>
        <span class="rl">{g.label}</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .ratio-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 2px 0 8px;
  }
  .ratio {
    flex: 1 1 0;
    min-width: 52px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 5px;
    padding: 7px 4px 6px;
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
    color: var(--text-2);
    cursor: pointer;
    transition: border-color var(--dur-fast) var(--ease-out), background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }
  .ratio:hover {
    border-color: var(--hairline-3);
  }
  .ratio.active {
    border-color: var(--accent-line);
    background: var(--accent-soft);
    color: var(--accent);
  }
  /* 比例轮廓：未选中描边色，选中填充主题色 */
  .shape {
    border: 1.5px solid currentColor;
    border-radius: 3px;
    opacity: 0.9;
  }
  .rl {
    font-size: 11.5px;
    font-family: var(--mono);
    line-height: 1;
  }
</style>
