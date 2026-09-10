<!-- 【新手导读】顶部五步旅程引导：纯展示组件（没有任何按钮/点击）。
     每一步的完成态/当前态都从后端真实快照「派生」（$derived），
     业务状态推进时它自动点亮，绝不反向驱动业务。 -->
<script lang="ts">
  import { appStore } from '../stores/app.svelte';
  import { t } from '../text';
  import { icons } from '../icons';

  /** 派生结果：current=当前步下标（0~4），doneUpTo=已完成步的最大下标（-1 表示尚无完成步）。 */
  const progress = $derived.by(() => {
    const snap = appStore.snapshot;
    if (!snap) return { current: 0, doneUpTo: -1 };
    // 当前组对象（快照里 currentGroup 是目录路径，可能为 null）
    const group = snap.currentGroup
      ? snap.groups.find((g) => g.imgDir === snap.currentGroup)
      : undefined;
    const hasGroup = !!group;
    const hasRtf = !!group?.rtf;
    // 最近一次替换完成 → 前四步全完成，第五步「查看与微调」为当前步
    if (appStore.lastResult === 'finished' && !snap.replacing) {
      return { current: 4, doneUpTo: 3 };
    }
    // 替换进行中：能发起说明前三步已完成，第四部为当前步
    if (snap.replacing) return { current: 3, doneUpTo: 2 };
    if (!hasGroup) return { current: 0, doneUpTo: -1 }; // 第 1 步：选档与头像包
    if (!hasRtf) return { current: 1, doneUpTo: 0 };    // 第 2 步：校验与名单
    return { current: 2, doneUpTo: 1 };                  // 第 3 步：选模式（等待发起替换）
  });

  /** 返回某一步的视觉状态类名：done=已完成（实心对勾）/ current=当前步 / ''=未到达。 */
  function stepClass(i: number) {
    if (i <= progress.doneUpTo) return 'done';
    if (i === progress.current) return 'current';
    return '';
  }
</script>

<ol class="stepper" aria-label="操作进度">
  {#each t.stepper.steps as label, i (label)}
    <!-- 步骤之间的连接线（第一步前面没有）；它前面的步骤完成时整条点亮 -->
    {#if i > 0}
      <li class="step-line" class:done={i - 1 <= progress.doneUpTo} aria-hidden="true"></li>
    {/if}
    <li class="step {stepClass(i)}">
      <span class="num">
        {#if i <= progress.doneUpTo}
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.6"
               stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            {@html icons.check.body}
          </svg>
        {:else}
          {i + 1}
        {/if}
      </span>
      <span>{label}</span>
    </li>
  {/each}
</ol>

<style>
  /* 对勾图标尺寸（scoped，只作用于本组件的序号圆） */
  .num svg { width: 13px; height: 13px; }
</style>
