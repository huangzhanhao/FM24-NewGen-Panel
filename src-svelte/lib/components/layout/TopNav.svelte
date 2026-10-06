<!-- TopNav · 顶栏三工作台导航：存档资料 → 头像替换 → AI 生图
     三样视觉语义：序号圆（顶栏是 1→2→3 的有序流程，不是平级标签页）、
     chip 间连接线（有序语义）、状态点（该台就绪度）

     【关键分离】当前台 = 用户选择（chip 高亮，可自由跳转）；该做哪一步 = 派生
     （连接线点亮 + 状态点变色）。两者各表达一件事，不打架：即使停在 ③AI 生成头像，
     若 ① 还没备齐，①② 之间的连接线不会点亮、① 的状态点仍是琥珀 -->
<script lang="ts">
  import { appStore } from '../../stores/app.svelte';
  import { imageaiStore } from '../../stores/imageai.svelte';
  import { ui } from '../../stores/ui.svelte';
  import type { Workbench } from '../../stores/ui.svelte';
  import { t } from '../../text';

  /** chip 就绪度三档：ok=绿（可开工）/ warn=琥珀（有最小前提但缺项）/ idle=灰（前提都没有）。
   *  区分依据是「有没有可开工的最小前提」，不是缺了几项 */
  type ChipState = 'ok' | 'warn' | 'idle';

  /** 三台定义：顺序即主流程顺序（备料 → 替换 → 可选 AI 生图） */
  const items: ReadonlyArray<{ id: Workbench; label: string }> = [
    { id: 'archive', label: t.workbench.archive },
    { id: 'replace', label: t.workbench.replace },
    { id: 'ai', label: t.workbench.ai },
  ];

  /** 快照是否已读：与快照预览同一口径——snapshotReadPath 非空即读过当前档的 config.xml */
  const snapshotRead = $derived(!!appStore.snapshotReadPath);

  /** 单台就绪度判据：全部只读 store getter，不反向驱动业务 */
  function stateOf(id: Workbench): ChipState {
    const s = appStore.snapshot;
    if (id === 'archive') {
      if (!s || s.currentProfile === 'No Profile' || !s.currentGroup) return 'idle';
      return s.currentRtf && snapshotRead ? 'ok' : 'warn';
    }
    if (id === 'replace') {
      if (!s || !s.currentGroup) return 'idle';
      return appStore.replaceReady ? 'ok' : 'warn';
    }
    // AI：模型已选 + 本会话已设 Key + 名单可读 → 绿；模型或 Key 至少设了一个但不齐 → 琥珀；都没设 → 灰
    // 本地 ComfyUI 无鉴权：免 Key（与 ImageAiPanel / 后端 needs_key 同口径）。
    const modelSet = !!imageaiStore.model;
    const keySet =
      imageaiStore.provider?.kind === "localcomfyui" ||
      !!imageaiStore.keysSet[imageaiStore.providerId];
    if (modelSet && keySet && appStore.rosterReady) return 'ok';
    return modelSet || keySet ? 'warn' : 'idle';
  }

  const states = $derived(items.map((it) => stateOf(it.id)));

  /** ③chip 的队列角标：待跑 + 在跑条数。队列坞只在 AI 台渲染，切走后仍靠它感知队列在跑/攒了多少；
   *  不放状态栏——那里横向没有余量，且已被两条进度通道占着 */
  const queueBadge = $derived(
    imageaiStore.queue.filter(
      (e) => e.status === "queued" || e.status === "running",
    ).length,
  );

  /** 连接线点亮条件：它**前面所有的台**都就绪 */
  function lineDone(i: number): boolean {
    return states.slice(0, i).every((s) => s === 'ok');
  }

  const stateTitle: Record<ChipState, string> = {
    ok: t.workbench.stateOk,
    warn: t.workbench.stateWarn,
    idle: t.workbench.stateIdle,
  };

  /** 顶栏右端作用域回显：始终显示**激活档**（真实作用域），不随侧栏下拉的浏览态变化——
   *  队列绑定的是作用域，「现在到底在哪个档组」必须一眼可见 */
  const basename = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
  const scopeText = $derived.by(() => {
    const s = appStore.snapshot;
    if (!s) return '';
    const g = s.currentGroup ? basename(s.currentGroup) : t.workbench.scopeNoGroup;
    return `${s.currentProfile}${t.workbench.scopeSep}${g}`;
  });
</script>

<div class="topnav-card">
  <ol class="topnav" aria-label="工作台切换">
    {#each items as it, i (it.id)}
      <!-- chip 之间的连接线（第一枚前面没有）：它之前的台全部就绪时整条点亮 -->
      {#if i > 0}
        <li class="wb-line" class:done={lineDone(i)} aria-hidden="true"></li>
      {/if}
      <li>
        <!-- 状态点只表达就绪度，**绝不因此禁用 chip**：点进去才知道差哪一项，禁用等于把用户锁在外面 -->
        <button
          class="wb"
          class:current={ui.workbench === it.id}
          aria-current={ui.workbench === it.id ? 'page' : undefined}
          onclick={() => (ui.workbench = it.id)}
        >
          <span class="num">{i + 1}</span>
          <span>{it.label}</span>
          <span class="wb-dot {states[i]}" title={stateTitle[states[i]]}></span>
          {#if i === 2 && queueBadge > 0}
            <span class="wb-badge" title={t.gen.queue.tabQueue}
              >{queueBadge}</span
            >
          {/if}
        </button>
      </li>
    {/each}
  </ol>
  {#if scopeText}
    <span class="wb-scope">{scopeText}</span>
  {/if}
</div>
