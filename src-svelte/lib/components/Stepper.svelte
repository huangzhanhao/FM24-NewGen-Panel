<!-- 顶部五步旅程引导：纯展示组件，完成态/当前态全部由后端快照派生，业务状态推进时自动点亮，绝不反向驱动业务。 -->
<script lang="ts">
  import { appStore } from "../stores/app.svelte";
  import { t } from "../text";
  import { icons } from "../icons";

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
    if (appStore.lastResult === "finished" && !snap.replacing) {
      return { current: 4, doneUpTo: 3 };
    }
    // 替换进行中：能发起说明前三步已完成，第四部为当前步
    if (snap.replacing) return { current: 3, doneUpTo: 2 };
    if (!hasGroup) return { current: 0, doneUpTo: -1 }; // 第 1 步：选档与头像包
    if (!hasRtf) return { current: 1, doneUpTo: 0 }; // 第 2 步：校验与名单
    return { current: 2, doneUpTo: 1 }; // 第 3 步：选模式（等待发起替换）
  });

  /** 返回某一步的视觉状态类名：done=已完成（实心对勾）/ current=当前步 / ''=未到达。 */
  function stepClass(i: number) {
    if (i <= progress.doneUpTo) return "done";
    if (i === progress.current) return "current";
    return "";
  }

  // 以下三个展示态原属「替换执行」卡片；提示内容上移到步骤引导下方后随之迁移，统一取自 store getter。
  const snap = $derived(appStore.snapshot);
  const browseMode = $derived(appStore.browseMode);
  const ready = $derived(appStore.replaceReady);
  // 未就绪原因（引导用户补全）：列出三个可能缺失的项。
  const missing = $derived(
    [
      !snap || snap.currentProfile === "No Profile"
        ? t.replace.needProfile
        : "",
      snap && !snap.currentGroup ? t.replace.needGroup : "",
      snap && !snap.currentRtf ? t.replace.needRtf : "",
    ].filter(Boolean),
  );
</script>

<div class="stepper-card">
  <ol class="stepper" aria-label="操作进度">
    {#each t.stepper.steps as label, i (label)}
      <!-- 步骤之间的连接线（第一步前面没有）；它前面的步骤完成时整条点亮 -->
      {#if i > 0}
        <li
          class="step-line"
          class:done={i - 1 <= progress.doneUpTo}
          aria-hidden="true"
        ></li>
      {/if}
      <li class="step {stepClass(i)}">
        <span class="num">
          {#if i <= progress.doneUpTo}
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.6"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
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
  <!-- 状态提示（原位于「替换执行」卡片底部，交互/判定逻辑不变）：浏览未激活存档时置顶提醒。 -->
  {#if browseMode}
    <p class="hint muted small">
      {t.replace.browseNotice(appStore.selectedProfile)}
    </p>
  {/if}
  <!-- 未就绪时把待完成项列出来引导用户。 -->
  {#if !ready && missing.length > 0}
    <p class="hint muted small">{t.replace.todoPrefix}{missing.join(" · ")}</p>
  {/if}
</div>

<style>
  /* 对勾图标尺寸（scoped，只作用于本组件的序号圆） */
  .num svg {
    width: 13px;
    height: 13px;
  }
  /* 步骤引导下方的状态提示：与步骤行、两条提示之间保持小间距（覆盖 <p> 默认外边距） */
  .hint {
    margin: 8px 0 0;
  }
  .hint + .hint {
    margin-top: 4px;
  }
</style>
