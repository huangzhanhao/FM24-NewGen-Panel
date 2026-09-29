<!-- 生图张数步进器：30px 高，[−] 数值 [+]；min/max 到界硬禁用。
     max<=1（火山单图模型）时调用方不应渲染本控件（改显示说明文字）。 -->
<script lang="ts">
  import { t } from "../../text"; // 全局文案常量

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
    onclick={() => step(-1)}
  >−</button>
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
    onclick={() => step(1)}
  >+</button>
</div>

<style>
  .stepper {
    flex: 1;
    display: flex;
    align-items: stretch;
    gap: 6px;
  }
  .step-btn {
    flex: none;
    width: 30px;
    min-height: 30px;
    height: 30px;
    padding: 0;
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    background: var(--bg-raised);
    color: var(--text-1);
    font-size: 15px;
    line-height: 1;
    cursor: pointer;
    transition: border-color var(--dur-fast) var(--ease-out), background-color var(--dur-fast) var(--ease-out);
  }
  .step-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-soft);
  }
  .step-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .num {
    flex: 1;
    min-width: 0;
    text-align: center;
    font-family: var(--mono);
    padding-left: 4px;
    padding-right: 4px;
  }
</style>
