<!-- ImageAiPanel · AI 生图工作台父组件，生图表单与提交：服务配置条、生成设置卡、
     CTA、内联错误条、结果卡，并挂载左栏常驻生成队列卡。
     子组件各自成簇：SourcePlayerCard（数据源卡）、GenSettingsCard（生成设置卡）、
     ResultGallery（结果画廊/灯箱），仅经 ongenerate/onopentemplates/onregenerate 单向回调联动。 -->
<script lang="ts">
  import { onMount } from "svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import QueueDock from "./QueueDock.svelte";
  import ProviderDialog from "./ProviderDialog.svelte";
  import TemplateDialog from "./TemplateDialog.svelte";
  import SourcePlayerCard from "./SourcePlayerCard.svelte"; // 数据源卡：ongenerate 转发主 CTA 链路
  import GenSettingsCard from "./GenSettingsCard.svelte"; // 生成设置卡：onopentemplates 回调
  import { t } from "../../text";
  import { formatError, showError } from "../../ui/dialog";
  import { tip } from "../../stores/app.svelte";
  import Icon from "../../ui/Icon.svelte";
  import ResultGallery from "./ResultGallery.svelte"; // 结果画廊/灯箱：onregenerate 回调
  import {
    clearResults,
    generate,
    imageaiStore,
    loadImageState,
    selectModel,
    selectProvider,
    runningQueueEntry,
  } from "../../stores/imageai.svelte";

  let providerOpen = $state(false);
  let templateOpen = $state(false);

  // ---- 生成反馈：内联错误条（含重试）+ 耗时计时 ----
  let genError = $state("");
  let elapsed = $state(0);
  let timer: ReturnType<typeof setInterval> | undefined;

  $effect(() => {
    if (imageaiStore.generating) {
      elapsed = 0;
      timer ??= setInterval(() => (elapsed += 1), 1000);
    } else if (timer) {
      clearInterval(timer);
      timer = undefined;
    }
    return () => {
      if (timer) {
        clearInterval(timer);
        timer = undefined;
      }
    };
  });

  // ---- 连接就绪度：Key 已设置 + 模型已选（顶栏 ③chip 的绿档还额外要求名单可读，口径更严）----
  // 无真正的连通性测试命令，这里以「Key 已设置」为代理判据，措辞不冒充「已连通」。
  // 本地 ComfyUI 无鉴权：免 Key（与后端 needs_key 口径一致），就绪度只看模型。
  const kindNeedsKey = $derived(imageaiStore.provider?.kind !== "localcomfyui");
  const keySet = $derived(!kindNeedsKey || !!imageaiStore.keysSet[imageaiStore.providerId]);
  const readyForKey = $derived(keySet && !!imageaiStore.model);
  const connStatus = $derived(readyForKey ? t.gen.statusReady : keySet ? t.gen.statusNoModel : t.gen.statusNoKey);

  /** 状态灯的点击进入服务商设置里编辑Key。 */
  function openProviderSettings() {
    providerOpen = true;
  }

  const caps = $derived(imageaiStore.modelInfo?.capabilities);

  /** 提示词字数上限（CTA 悬停提示用）；超限判定单点在 store.promptOver，计数显示随 GenSettingsCard。 */
  const promptLimit = $derived(caps?.maxPromptChars ?? 0);

  /** 能力摘要行：把当前模型能力翻成一句话（参数控件按同一能力集显隐，两者口径必须一致）。 */
  const capSummary = $derived.by(() => {
    const parts: string[] = [];
    const c = caps;
    if (!c) return t.gen.capNoModel;
    parts.push(c.sizes.length > 0 ? t.gen.capPresetSizes(c.sizes.length) : t.gen.capFreeSize);
    parts.push(c.maxN > 0 ? t.gen.capMaxN(c.maxN) : t.gen.capNAny);
    if (c.supportsQuality) parts.push(t.gen.capQuality);
    if (c.supportsSeed) parts.push(t.gen.capSeed);
    if (c.supportsNegativePrompt) parts.push(t.gen.capNegative);
    if (c.supportsPromptExtend) parts.push(t.gen.capPromptExtend);
    if (c.supportsWatermark) parts.push(t.gen.capWatermark);
    return parts.join(" · ");
  });

  /** 队列正在跑的那条：此时生成设置卡是它的只读视图，单张 CTA 必须一起禁用——
   *  否则屏上显示队列任务、点下去发的却是被隐藏表单里你先前的内容，等于生成错的东西还多花一次钱。 */
  const queueTask = $derived(runningQueueEntry());
  /** 手动模式下选中了队列行：右侧此刻显示的是那一条的参数，单张生成同样会发错内容 → 一起禁用。 */
  const queueEditLock = $derived(!imageaiStore.queueAutoConfirm && imageaiStore.queueSelectedId !== null);
  /** CTA 可用性：生成中 / 队列占用 / 队列行核对中 / 尺寸违规（store 单点，GenSettingsCard 同源）/ 提示词超长时禁用。 */
  const genDisabled = $derived(
    imageaiStore.generating ||
      queueTask !== null ||
      queueEditLock ||
      !!imageaiStore.sizeError ||
      imageaiStore.promptOver,
  );

  onMount(() => {
    if (!imageaiStore.config) void loadImageState().catch(showError);
  });

  /** meta（同参重生）：结果卡「再生成」传入该图记录的 {size, seed} 按原参重出；
   *  CTA 与内联重试不传 meta，用工作台当前值。 */
  async function onGenerate(meta?: { size: string; seed: string }) {
    // 禁用态统一在入口拦截：快捷键（Ctrl+Enter）与数据源卡回调会绕过按钮 disabled，
    // 必须与主 CTA 同条件。带 meta 的「再生成」用原图尺寸，只在生成中/队列占用/核对中拦截，
    // 不受当前 sizeError 影响。
    if (queueTask || imageaiStore.generating || queueEditLock || (!meta && genDisabled)) return;
    genError = "";
    try {
      await generate(meta);
    } catch (e) {
      genError = formatError(e); // 生成失败写进结果区的内联错误条，供就地重试；后端错误已是结构化对象，须走统一格式化。
    }
  }

  // 清空结果是破坏性操作（临时图仅存会话目录），必须确认。
  async function onClearResults() {
    const n = imageaiStore.results.length;
    if (n === 0) return;
    if (
      !(await confirm(t.gen.clearResultsConfirm(n), {
        title: t.dialog.warning,
      }))
    )
      return;
    clearResults();
  }
</script>

<div class="panel">
  <!-- ① 服务配置条（全宽单行）：进台第一眼即见服务商 / 模型 / Key 就绪状态。
       完整表单与能力摘要分别留在设置弹窗与模型下拉的悬停提示。
       借用队列行期间这两枚下拉照常可用：换模型＝换这条任务要发的请求，改完在右侧参数卡底部点「确认」
       即按卡片当前值写回（改动会把已确认标记打回橙，需再确认一次）。 -->
  <section class="conn-bar">
    <label class="cb-field">
      <span class="cb-label">{t.gen.provider}</span>
      <!-- 悬停提示 = 全名：窄窗下这枚下拉会被压窄截断，悬停仍要能读到完整服务商名 -->
      <select
        id="ig-provider"
        use:tip={imageaiStore.provider?.name ?? ""}
        value={imageaiStore.providerId}
        onchange={(e) => selectProvider(e.currentTarget.value)}
      >
        {#each imageaiStore.config?.providers ?? [] as p (p.id)}
          <option value={p.id}>{p.name}</option>
        {/each}
      </select>
    </label>
    <label class="cb-field">
      <span class="cb-label">{t.gen.model}</span>
      <!-- 悬停提示 = 模型 ID + 能力摘要：模型 ID 是最容易被截断的一行文字，能力摘要本就挂在这里 -->
      <select
        id="ig-model"
        value={imageaiStore.model}
        use:tip={imageaiStore.model ? `${imageaiStore.model} · ${capSummary}` : capSummary}
        onchange={(e) => selectModel(e.currentTarget.value)}
      >
        <!-- 默认模型不在明细行里时也要能选中（自建 OpenAI 兼容服务常无明细行） -->
        {#if imageaiStore.provider?.defaultModel && !(imageaiStore.provider?.models ?? []).some((m) => m.model === imageaiStore.provider?.defaultModel)}
          <option value={imageaiStore.provider.defaultModel}>{imageaiStore.provider.defaultModel}</option>
        {/if}
        {#each imageaiStore.provider?.models ?? [] as m}
          <option value={m.model}>{m.model}</option>
        {/each}
      </select>
    </label>
    <!-- Key 的唯一状态表达（判据 = keySet && model）。文字色与边框色随档位走：
         .warn 标「有未决项」（琥珀），缺 Key 时额外挂 ⚠ 图标；绿点绿边绿字 = 已就绪。
         点它打开服务商设置（Key 的唯一编辑入口就在那里） -->
    <button
      class="cb-status"
      class:ok={readyForKey}
      class:warn={!readyForKey}
      use:tip={readyForKey ? t.gen.statusReadyTitle : keySet ? connStatus : t.gen.keyMissingHint}
      onclick={openProviderSettings}
    >
      {#if keySet}
        <span class="cb-dot" aria-hidden="true"></span>
      {:else}
        <Icon name="alert" class="cb-ic" stroke={2} />
      {/if}
      {connStatus}
    </button>
    <button class="btn sm" onclick={() => (providerOpen = true)}>
      <Icon name="gear" class="ic" />
      {t.gen.providerSettings}
    </button>
  </section>

  <!-- ② 主工作区：左栏 = 球员源单人档案卡 + 常驻生成队列卡（占其下方空白），右栏 = 生成设置 / 结果 -->
  <div class="wb-grid">
    <div class="wb-left">
      <!-- 数据源卡的「生成」走同一条 onGenerate，错误进右栏内联条 -->
      <SourcePlayerCard ongenerate={() => onGenerate()} />
      <QueueDock />
    </div>

    <div class="wb-right">
      <!-- 生成设置（内容 + 画幅两区）；模板管理弹窗由本父级持有开关状态；Ctrl+Enter 走同一 onGenerate -->
      <GenSettingsCard onopentemplates={() => (templateOpen = true)} />

      <button
        class="btn primary cta"
        disabled={genDisabled}
        use:tip={queueTask
          ? t.gen.queue.queueRunningTag
          : queueEditLock
            ? t.gen.queue.queueEditHint
            : (imageaiStore.sizeError ?? (imageaiStore.promptOver ? t.gen.promptTooLong(promptLimit) : ""))}
        onclick={() => onGenerate()}
      >
        {#if imageaiStore.generating}
          <span class="cta-spin" aria-hidden="true"></span>
          {t.gen.genSec(elapsed)}
        {:else}
          {t.gen.generate}
        {/if}
      </button>
      <!-- 内联错误条：生成失败就地展示 + 重试（放在结果卡之上：它讲的是这次尝试，不是已有结果） -->
      {#if genError}
        <div class="inline-alert" role="alert">
          <Icon name="alert" class="ia-ic" stroke={1.8} />
          <div class="ia-body">
            <span class="ia-title">{t.gen.genErrorTitle}</span>
            <span class="ia-msg" use:tip={genError}>{genError}</span>
          </div>
          <button class="btn sm ia-retry" onclick={() => onGenerate()}>
            <Icon name="refresh" class="ic" />
            {t.gen.retry}
          </button>
          <button class="icon-btn" use:tip={t.gen.closeAlert} onclick={() => (genError = "")}>
            <Icon name="x" size={13} />
          </button>
        </div>
      {/if}

      <!-- 本次结果：独立卡片——标题行（含张数与清空）+ 画廊；卡片按内容撑高，滚动归页面层 -->
      <section class="pcard grow">
        <div class="pcard-h">
          <span
            >{t.gen.resultsTitle}{#if imageaiStore.results.length > 0}<span class="res-count"
                >{t.gen.resultsCount(imageaiStore.results.length)}</span
              >{/if}</span
          >
          <button
            class="btn sm"
            disabled={imageaiStore.results.length === 0}
            use:tip={t.gen.clearResultsTitle}
            onclick={onClearResults}
          >
            {t.gen.clearResults}
          </button>
        </div>
        <ResultGallery onregenerate={onGenerate} />
      </section>
    </div>
  </div>
</div>

<ProviderDialog bind:open={providerOpen} />
<TemplateDialog bind:open={templateOpen} />

<style>
  /* 直挂 .content（flex 列）：flex:1 吃顶栏之外的剩余高度（height:100% 会把顶栏高度也算进去），
     内部各区（配置条/左右分栏/队列坞）分段滚动，不撑出工作台高度预算 */
  .panel {
    display: flex;
    flex-direction: column;
    /* 只长不缩：内容超高时自身撑高、由外层 .content（页面层）滚动。
       若在这里收缩，滚动条就会贴着卡片右缘出现、并把卡片压窄 10px（用户明确要它在外层） */
    flex: 1 0 auto;
    min-height: 0;
    min-width: 0;
    gap: var(--sp-4);
  }

  /* ---------- 服务配置条：全宽单行、控件内联标签 ---------- */
  .conn-bar {
    flex: none;
    display: flex;
    align-items: center;
    flex-wrap: nowrap;
    gap: var(--sp-3) var(--sp-5);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    padding: var(--sp-3) var(--sp-5);
    box-shadow: var(--shadow-1);
  }
  .cb-field {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
    flex: 0 1 auto;
  }
  .cb-label {
    font-size: var(--fs-sm);
    color: var(--text-2);
    flex-shrink: 0;
  }
  /* 全局 select 是 width:100%（铺满网格单元的口径），配置条内按内容 96–220px 自适应 */
  .cb-field select {
    width: auto;
    flex: 0 1 auto;
    max-width: 220px;
    min-width: 96px;
    text-overflow: ellipsis;
  }
  /* 设置钮恒按内容定宽：被压窄的话钮内文字会折成两行，把「单行配置条」破坏得更彻底 */
  .conn-bar > .btn {
    flex: none;
  }
  /* 就绪状态灯：三档文字与边框同色，色语与顶栏 chip 状态点一致
       .warn = 有未决项（缺 Key 或未选模型）→ 琥珀
       .ok   = 已就绪 → 绿
     底色固定 --bg-inset：软底会让这枚胶囊在整条配置条里跳出来，而它的语气只是
     「顺手可点的一句状态」，不是要抢视线的告警 */
  .cb-status {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    border: 1px solid var(--hairline);
    background: var(--bg-inset);
    border-radius: var(--radius-pill);
    padding: var(--pill-padding-y) var(--pill-padding-x);
    font-size: var(--fs-sm);
    color: var(--text-2);
    cursor: pointer;
    flex-shrink: 0;
    transition:
      color var(--dur-fast) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out);
  }
  /* 未就绪档：点/字/边同色，一眼看出是「待补」而非「正常但未连」 */
  .cb-status.warn {
    color: var(--warn);
    border-color: var(--warn);
  }
  /* 就绪档：绿点 + 绿字 + 绿边。描边用 --ok 而非 --accent-line：
     顶栏 chip 的选中态描边是主色（那是「当前台」的强调），这里表达的是「条件已满足」，
     用语义绿才不会与顶栏的选中绿混淆 */
  .cb-status.ok {
    color: var(--ok);
    border-color: var(--ok);
  }
  /* 悬停只加深度不改语义色：把边框推到 45% 不透明，文字不动 */
  .cb-status:hover {
    border-color: color-mix(in srgb, currentColor 45%, transparent);
  }
  .cb-status .cb-dot {
    width: 7px;
    height: 7px;
    border-radius: var(--radius-circle);
    background: currentColor; /* 随文字档位走，不再硬编码 warn */
    flex-shrink: 0;
  }
  /* 缺 Key 档的 ⚠ 符号：颜色继承 currentColor，与本档的点/字/边同族。
     :global 是必需的：class 由 Icon 组件透传到 svg，scoped 编译器看不见这层 */
  :global(.cb-status .cb-ic) {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
  }

  /* ---------- 响应式：配置条折行回退（≤1024px，与下方栅格堆叠同断点） ----------
     宽窗单行常驻；窄窗（本组件已切单列）宁可变两行，也不做横向溢出与控件互相重叠。 */
  @media (max-width: 1024px) {
    .conn-bar {
      flex-wrap: wrap;
    }
  }

  /* ---------- 主工作区 ---------- */
  .wb-grid {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(300px, 340px) minmax(0, 1fr);
    gap: var(--sp-5);
  }
  /* 两栏都不再自带滚动：滚动条统一交给页面层（.content），
     它落在窗口边缘、与卡片之间有 --sp-6 的留白，不再贴着卡片右缘、也不占卡片宽度 */
  .wb-left {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-height: 0;
  }
  .wb-right {
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  /* 主操作钮：sticky 贴在生成设置卡与结果卡之间随时可点（滚动在页面层，滚动时它吸在视口底部）；
     几何沿用 button.btn 标准档（30px） */
  .cta {
    position: sticky;
    bottom: 0;
    flex: none;
  }

  /* ---------- 本次结果（独立卡片） ---------- */
  /* 滚动上移到页面层后，右栏没有「剩余高度」可填：结果卡按内容自然撑高，画廊随之展开，
     卡内不再出现第二条滚动条（220px 下限仍由 app.css 的 .pcard.grow 提供） */
  .wb-right .pcard.grow {
    flex: none;
  }
  .res-count {
    margin-left: var(--sp-3);
    font-size: var(--fs-sm);
    font-weight: 400;
    color: var(--text-3);
    font-family: var(--mono);
  }

  /* 内联错误条 */
  .inline-alert {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding: var(--sp-3) var(--sp-4);
    border: 1px solid var(--danger);
    border-radius: var(--radius-md);
    background: var(--danger-soft);
  }
  :global(.ia-ic) {
    width: 17px;
    height: 17px;
    color: var(--danger);
    flex: none;
  }
  .ia-body {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
  }
  .ia-title {
    font-size: var(--fs-md);
    font-weight: var(--fw-semibold);
    color: var(--danger);
  }
  .ia-msg {
    font-size: var(--fs-sm);
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ia-retry {
    flex: none;
  }

  /* CTA spinner */
  .cta-spin {
    width: 14px;
    height: 14px;
    border-radius: var(--radius-circle);
    border: 2px solid var(--on-accent-track);
    border-top-color: var(--accent-contrast);
    animation: cta-rot 0.7s linear infinite;
  }
  @keyframes cta-rot {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cta-spin {
      animation-duration: 1.6s;
    }
  }

  /* ---------- 响应式：≤1024 单栏堆叠 ----------
     不能沿用 grid 单列：grid 自动行高在定高滚动容器里会把内部 flex:1 1 0 的子列
     （QueueDock/结果卡）折叠成 0 高，配合自然高直接与下一行视觉重叠。
     改 flex 列 + 两列自然高；滚动仍统一归页面层（与宽屏同一条路径）。 */
  @media (max-width: 1024px) {
    .wb-grid {
      display: flex;
      flex-direction: column;
    }
    .wb-left,
    .wb-right {
      flex: none;
      min-height: 0;
    }
  }
</style>
