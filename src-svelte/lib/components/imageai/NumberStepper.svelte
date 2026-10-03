<!-- NumberStepper · 生图张数步进器：30px 高的 [−] 数值 [+]，min/max 到界硬禁用；
     max<=1 的单图模型（火山 seedream、百炼 z-image 等）调用方不渲染本控件、改显示说明文字 -->
<script lang="ts">
  import { t } from "../../text";

  interface Props {
    value: number;
    min?: number;
    max: number;
    onchange: (v: number) => void;
    ariaLabel?: string;
  }
  let { value, min = 1, max, onchange, ariaLabel }: Props = $props();

  function step(delta: number) {
    const next = Math.min(max, Math.max(min, value + delta));
    if (next !== value) onchange(next);
  }
</script>

<div class="stepper">
  <button
    type="button"
    class="step-btn"
    aria-label={t.gen.nLess}
    disabled={value <= min}
    onclick={() => step(-1)}>−</button
  >
  <input
    class="num"
    type="number"
    aria-label={ariaLabel ?? t.gen.nCount}
    {min}
    {max}
    step="1"
    {value}
    oninput={(e) => {
      const v = e.currentTarget.valueAsNumber;
      if (Number.isFinite(v)) onchange(Math.min(max, Math.max(min, v)));
    }}
  />
  <button
    type="button"
    class="step-btn"
    aria-label={t.gen.nMore}
    disabled={value >= max}
    onclick={() => step(1)}>+</button
  >
</div>

<style>
  .stepper {
    flex: 1;
    display: flex;
    align-items: stretch;
    gap: var(--sp-2);
  }
  .step-btn {
    flex: none;
    width: var(--control-h);
    min-height: var(--control-h);
    height: var(--control-h);
    padding: 0;
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    background: var(--bg-raised);
    color: var(--text-1);
    font-size: var(--fs-lg);
    line-height: 1;
    cursor: pointer;
    transition:
      border-color var(--dur-fast) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out);
  }
  .step-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }
  /* 禁用外观（透明度/not-allowed）沿用全局 button:disabled 契约，此处不覆盖 */
  .num {
    flex: 1;
    min-width: 0;
    /* 与两侧按钮同高：显式锁标准控件高，避免继承输入框 7px 纵向 padding 后高出 30px */
    height: var(--control-h);
    text-align: center;
    font-family: var(--mono);
    padding: 0 var(--sp-1);
  }
</style>
