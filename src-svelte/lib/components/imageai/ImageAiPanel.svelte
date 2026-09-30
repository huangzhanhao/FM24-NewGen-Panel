<!-- ImageAiPanel · AI 生图工作台父组件，生图表单与提交：服务配置条/Key 就绪卡、画面参数卡、Seed 卡、
     CTA、结果工具条、内联错误条，并挂载左栏常驻生成队列卡。
     子组件各自成簇：SourcePlayerCard（数据源卡）、PromptCard（提示词卡）、
     ResultGallery（结果画廊/灯箱），仅经 ongenerate/onopentemplates/onregenerate 单向回调联动。 -->
<script lang="ts">
  import { onMount } from "svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import QueueDock from "./QueueDock.svelte";
  import ProviderDialog from "./ProviderDialog.svelte";
  import TemplateDialog from "./TemplateDialog.svelte";
  import NumberStepper from "./NumberStepper.svelte";
  import RatioPicker from "./RatioPicker.svelte";
  import SourcePlayerCard from "./SourcePlayerCard.svelte"; // 数据源卡：ongenerate 转发主 CTA 链路
  import PromptCard from "./PromptCard.svelte"; // 提示词卡：onopentemplates 回调
  import { t } from "../../text";
  import { showError } from "../../ui/dialog";
  import Icon from "../../ui/Icon.svelte";
  import ResultGallery from "./ResultGallery.svelte"; // 结果画廊/灯箱：onregenerate 回调
  import {
    clearResults,
    generate,
    imageaiStore,
    loadImageState,
    parseSize,
    selectModel,
    selectProvider,
    setProviderKey,
    validateSizeRules,
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

  // ---- API Key 直达输入：掩码显示、不回显真值，成功即清空 ----
  let keyInput = $state("");
  let keyEl = $state<HTMLInputElement | null>(null);

  // ---- 连接就绪度：Key 已设置 + 模型已选（顶栏 ③chip 的绿档还额外要求名单可读，口径更严）----
  // 无真正的连通性测试命令，这里以「Key 已设置」为代理判据，措辞不冒充「已连通」。
  const keySet = $derived(!!imageaiStore.keysSet[imageaiStore.providerId]);
  const readyForKey = $derived(keySet && !!imageaiStore.model);
  const connStatus = $derived(
    readyForKey
      ? t.gen.statusReady
      : keySet
        ? t.gen.statusNoModel
        : t.gen.statusNoKey,
  );

  /** 就绪卡「去设置」与状态灯的共用动作：聚焦 Key 输入框（配置条就在台顶，聚焦即直达）。 */
  function focusKey() {
    keyEl?.focus();
  }

  // ---- 尺寸：预设 + 自定义混合。SIZE_CUSTOM 是下拉里「自定义…」的哨兵值，
  //      预设尺寸均为 WxH 形式（小写 x），不会与之相撞。 ----
  const SIZE_CUSTOM = "__custom__";
  // 「自定义…」是显式选择的模式：即便合成值恰好等于某预设也保持自定义态，
  // 直到改选预设、或切服务商重置了尺寸才退出（切模型不动尺寸）。
  let sizeCustomMode = $state(false);
  let sizeW = $state(parseSize(imageaiStore.size)?.[0] ?? 350);
  let sizeH = $state(parseSize(imageaiStore.size)?.[1] ?? 350);

  const caps = $derived(imageaiStore.modelInfo?.capabilities);
  const sizePresets = $derived(caps?.sizes ?? []);
  const sizeSelectValue = $derived(
    sizeCustomMode || !sizePresets.includes(imageaiStore.size)
      ? SIZE_CUSTOM
      : imageaiStore.size,
  );

  /** 尺寸字符串显示归一化：'*' → '×'，值本身不变（百炼要 '*'、OpenAI 要 'x'，适配器已各自归一）。 */
  function sizeLabel(s: string): string {
    return s.replace("*", "×");
  }

  /** 自定义宽高的实时校验信息（仅模型声明了 sizeRules 时硬校验；其余维持软提示）。 */
  const sizeError = $derived(
    sizeSelectValue === SIZE_CUSTOM && caps?.sizeRules
      ? validateSizeRules(imageaiStore.size, caps.sizeRules)
      : null,
  );

  /** 提示词字数上限（CTA 悬停提示用）；超限判定单点在 store.promptOver，计数显示随 PromptCard。 */
  const promptLimit = $derived(caps?.maxPromptChars ?? 0);

  /** 质量分段：值取模型声明的 qualities，中文映射展示（值仍发英文）。 */
  const QUALITY_LABELS: Record<string, string> = {
    auto: t.gen.qualityAuto,
    low: t.gen.qualityLow,
    medium: t.gen.qualityMedium,
    high: t.gen.qualityHigh,
  };
  const qualityOptions = $derived(
    caps?.qualities
      ? ["auto", ...caps.qualities.filter((q) => q !== "auto")]
      : [],
  );

  /** 能力摘要行：把当前模型能力翻成一句话（参数控件按同一能力集显隐，两者口径必须一致）。 */
  const capSummary = $derived.by(() => {
    const parts: string[] = [];
    const c = caps;
    if (!c) return t.gen.capNoModel;
    parts.push(
      c.sizes.length > 0
        ? t.gen.capPresetSizes(c.sizes.length)
        : t.gen.capFreeSize,
    );
    parts.push(c.maxN > 0 ? t.gen.capMaxN(c.maxN) : t.gen.capNAny);
    if (c.supportsQuality) parts.push(t.gen.capQuality);
    if (c.supportsSeed) parts.push(t.gen.capSeed);
    if (c.supportsNegativePrompt) parts.push(t.gen.capNegative);
    if (c.supportsPromptExtend) parts.push(t.gen.capPromptExtend);
    if (c.supportsWatermark) parts.push(t.gen.capWatermark);
    return parts.join(" · ");
  });

  /** CTA 可用性：生成中 / 尺寸违规 / 提示词超长时禁用。 */
  const genDisabled = $derived(
    imageaiStore.generating || !!sizeError || imageaiStore.promptOver,
  );

  // store.size 变化（初始挂载、切服务商重置、自定义输入写回）时把 WxH 回填进两个数字输入；
  // 解析失败保持现值。输入过程中的同值回填是幂等的，不会打断正在输入的数字。
  $effect(() => {
    const parsed = parseSize(imageaiStore.size);
    if (parsed) {
      sizeW = parsed[0];
      sizeH = parsed[1];
    }
  });

  // 切服务商会把 size 重置为新模型的首个预设 → 退出「自定义…」强制态；
  // 同时清空 Key 输入框（Key 跟随服务商，输入内容不跨服务商残留）。
  $effect(() => {
    void imageaiStore.providerId;
    sizeCustomMode = false;
    keyInput = "";
  });

  async function commitKey() {
    const key = keyInput.trim();
    if (!key) return; // 空值 = 未输入，不动已设置的 Key
    try {
      await setProviderKey(imageaiStore.providerId, key);
      keyInput = "";
    } catch (e) {
      await showError(e);
    }
  }

  function onSizeSelect(e: Event) {
    const v = (e.currentTarget as HTMLSelectElement).value;
    if (v === SIZE_CUSTOM) {
      sizeCustomMode = true;
      imageaiStore.size = `${sizeW}x${sizeH}`; // 以当前宽高起步，立即生成也是所见即所发
    } else {
      sizeCustomMode = false;
      imageaiStore.size = v;
    }
  }

  function onCustomSize() {
    // 宽高都合法（64–4096）才写回；清空/输入中间态不写，store 保留上一个合法值
    const ok = (v: number) => Number.isFinite(v) && v >= 64 && v <= 4096;
    if (ok(sizeW) && ok(sizeH)) imageaiStore.size = `${sizeW}x${sizeH}`;
  }

  onMount(() => {
    if (!imageaiStore.config) void loadImageState().catch(showError);
  });

  /** meta（同参重生）：结果卡「再生成」传入该图记录的 {size, seed} 按原参重出；
   *  CTA 与内联重试不传 meta，用工作台当前值。 */
  async function onGenerate(meta?: { size: string; seed: string }) {
    // Key 框有未提交内容时先串行提交，避免 change 异步与生成竞态（提交失败中止生成）。
    if (keyInput.trim()) await commitKey();
    genError = "";
    try {
      await generate(meta);
    } catch (e) {
      // 生成失败写进结果区的内联错误条，供就地重试。
      genError = e instanceof Error ? e.message : String(e);
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
       Key 只进后端会话内存（setProviderKey），不改持久化语义；
       baseUrl 等完整表单留在服务商设置弹窗，能力摘要收进模型下拉的悬停提示。 -->
  <section class="conn-bar">
    <label class="cb-field">
      <span class="cb-label">{t.gen.provider}</span>
      <select
        id="ig-provider"
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
      <select
        id="ig-model"
        value={imageaiStore.model}
        title={capSummary}
        onchange={(e) => selectModel(e.currentTarget.value)}
      >
        <!-- 默认模型不在明细行里时也要能选中（custom 服务商常无明细行） -->
        {#if imageaiStore.provider?.defaultModel && !(imageaiStore.provider?.models ?? []).some((m) => m.model === imageaiStore.provider?.defaultModel)}
          <option value={imageaiStore.provider.defaultModel}
            >{imageaiStore.provider.defaultModel}</option
          >
        {/if}
        {#each imageaiStore.provider?.models ?? [] as m}
          <option value={m.model}>{m.label}</option>
        {/each}
      </select>
    </label>
    <label class="cb-field cb-key">
      <span class="cb-label">{t.gen.keyBarLabel}</span>
      <div class="key-row">
        <input
          id="ig-key"
          type="password"
          autocomplete="off"
          placeholder={imageaiStore.keysSet[imageaiStore.providerId]
            ? t.gen.keyBarSetPlaceholder
            : t.gen.keyBarPlaceholder}
          bind:this={keyEl}
          bind:value={keyInput}
          onchange={commitKey}
          onkeydown={(e) => {
            if (e.key === "Enter") void commitKey();
          }}
        />
        <button
          class="btn sm key-btn"
          title={t.gen.keySubmit}
          onclick={commitKey}
        >
          <Icon name="check" class="ic" stroke={2.4} />
        </button>
        {#if imageaiStore.keysSet[imageaiStore.providerId]}
          <span class="key-ok" title={t.gen.keyBarSetPlaceholder}
            >{t.gen.keySetBadge}</span
          >
        {/if}
      </div>
    </label>
    <!-- 就绪状态灯：绿 = Key 已设置且模型已选（连通性以实际生成为准）；点击直达 Key 输入框 -->
    <button
      class="cb-status"
      class:ok={readyForKey}
      title={readyForKey ? t.gen.statusReadyTitle : connStatus}
      onclick={focusKey}
    >
      <span class="cb-dot" aria-hidden="true"></span>
      {connStatus}
    </button>
    <button class="btn sm" onclick={() => (providerOpen = true)}>
      <Icon name="gear" class="ic" />
      {t.gen.providerSettings}
    </button>
  </section>

  <!-- 就绪卡：Key 未设置时条件出现，一句话说差什么 + 直达修复（聚焦上方 Key 输入框），设好自动消失 -->
  {#if !keySet}
    <div class="conn-warn" role="status">
      <Icon name="alert" class="ic" stroke={1.8} />
      <span>{t.gen.keyMissingHint}</span>
      <button class="btn sm" onclick={focusKey}>{t.gen.keyGoSet}</button>
    </div>
  {/if}

  <!-- ② 主工作区：左栏 = 球员源单人档案卡 + 常驻生成队列卡（占其下方空白），右栏 = 提示词 / 参数 / 结果
       （队列常驻后，结果工具条不再有队列开关） -->
  <div class="wb-grid">
    <div class="wb-left">
      <!-- 数据源卡的「生成」走同一条 onGenerate：未提交 Key 先串行提交，错误进右栏内联条 -->
      <SourcePlayerCard ongenerate={() => onGenerate()} />
      <QueueDock />
    </div>

    <div class="wb-right">
      <PromptCard onopentemplates={() => (templateOpen = true)} />

      <!-- 画面参数 -->
      <section class="pcard">
        <div class="pcard-h"><span>{t.gen.cardCanvas}</span></div>
        <!-- 画面方向：由预设尺寸解析归组；自由尺寸模型未识别到比例时不渲染 -->
        <RatioPicker
          sizes={sizePresets}
          value={imageaiStore.size}
          onselect={(s) => {
            sizeCustomMode = false;
            imageaiStore.size = s;
          }}
        />
        <div class="frow">
          <label for="ig-size">{t.gen.size}</label>
          <select id="ig-size" value={sizeSelectValue} onchange={onSizeSelect}>
            {#each sizePresets as s (s)}
              <option value={s}>{sizeLabel(s)}</option>
            {/each}
            <option value={SIZE_CUSTOM}>{t.gen.sizeCustom}</option>
          </select>
        </div>
        {#if sizeSelectValue === SIZE_CUSTOM}
          <div class="frow size-custom">
            <label for="ig-size-w">{t.gen.sizeWLabel}</label>
            <input
              id="ig-size-w"
              type="number"
              min="64"
              max="4096"
              step="1"
              class:invalid={!!sizeError}
              bind:value={sizeW}
              oninput={onCustomSize}
            />
            <span aria-hidden="true">×</span>
            <label for="ig-size-h">{t.gen.sizeHLabel}</label>
            <input
              id="ig-size-h"
              type="number"
              min="64"
              max="4096"
              step="1"
              class:invalid={!!sizeError}
              bind:value={sizeH}
              oninput={onCustomSize}
            />
          </div>
          {#if sizeError}
            <p class="field-error">{sizeError}</p>
          {:else}
            <p class="hint">{t.gen.sizeCustomHint}</p>
          {/if}
        {/if}
        <div class="frow">
          <span class="frow-label">{t.gen.n}</span>
          {#if caps && caps.maxN > 1}
            <NumberStepper
              value={imageaiStore.n}
              min={1}
              max={caps.maxN}
              ariaLabel={t.gen.n}
              onchange={(v) => (imageaiStore.n = v)}
            />
          {:else if caps?.maxN === 1}
            <span class="static-note">{t.gen.nSingle}</span>
          {:else}
            <NumberStepper
              value={imageaiStore.n}
              min={1}
              max={99}
              ariaLabel={t.gen.n}
              onchange={(v) => (imageaiStore.n = v)}
            />
          {/if}
        </div>
        {#if caps?.supportsQuality}
          <div class="frow col-frow">
            <span class="frow-label">{t.gen.quality}</span>
            <div class="seg3 q-seg" role="group" aria-label={t.gen.quality}>
              {#each qualityOptions as q (q)}
                <button
                  type="button"
                  class:active={imageaiStore.quality === q}
                  aria-pressed={imageaiStore.quality === q}
                  onclick={() => (imageaiStore.quality = q)}
                >
                  {QUALITY_LABELS[q] ?? q}
                </button>
              {/each}
            </div>
          </div>
        {/if}
        {#if caps?.supportsWatermark}
          <label class="switch opt-row">
            <input type="checkbox" bind:checked={imageaiStore.watermark} />
            <span class="track"></span>
            <span class="opt-text">
              {t.gen.watermark}
              <small>{t.gen.watermarkHint}</small>
            </span>
          </label>
        {/if}
      </section>

      <!-- 复现控制：不支持 Seed 时整块不渲染 -->
      {#if caps?.supportsSeed}
        <section class="pcard">
          <div class="pcard-h"><span>{t.gen.cardRepro}</span></div>
          <div class="frow">
            <label for="ig-seed">{t.gen.seed}</label>
            <input
              id="ig-seed"
              type="number"
              min="0"
              max="2147483647"
              placeholder={t.gen.seedPlaceholder}
              bind:value={imageaiStore.seed}
            />
            <button
              type="button"
              class="icon-btn seed-dice"
              title={t.gen.seedRandomTitle}
              onclick={() => (imageaiStore.seed = "")}
            >
              <Icon name="dice" class="ic" stroke={1.8} />
            </button>
          </div>
        </section>
      {/if}

      <button
        class="btn primary cta"
        disabled={genDisabled}
        title={sizeError ??
          (imageaiStore.promptOver ? t.gen.promptTooLong(promptLimit) : "")}
        onclick={() => onGenerate()}
      >
        {#if imageaiStore.generating}
          <span class="cta-spin" aria-hidden="true"></span>
          {t.gen.genSec(elapsed)}
        {:else}
          {t.gen.generate}
        {/if}
      </button>
      <div class="res-bar">
        <span class="res-title">{t.gen.resultsTitle}</span>
        {#if imageaiStore.results.length > 0}
          <span class="res-count"
            >{t.gen.resultsCount(imageaiStore.results.length)}</span
          >
        {/if}
        <span class="spacer"></span>
        <button
          class="btn sm"
          disabled={imageaiStore.results.length === 0}
          title={t.gen.clearResultsTitle}
          onclick={onClearResults}
        >
          {t.gen.clearResults}
        </button>
      </div>

      <!-- 内联错误条：生成失败就地展示 + 重试 -->
      {#if genError}
        <div class="inline-alert" role="alert">
          <Icon name="alert" class="ia-ic" stroke={1.8} />
          <div class="ia-body">
            <span class="ia-title">{t.gen.genErrorTitle}</span>
            <span class="ia-msg" title={genError}>{genError}</span>
          </div>
          <button class="btn sm ia-retry" onclick={() => onGenerate()}>
            <Icon name="refresh" class="ic" />
            {t.gen.retry}
          </button>
          <button
            class="icon-btn"
            title={t.gen.closeAlert}
            onclick={() => (genError = "")}
          >
            <Icon name="x" size={13} />
          </button>
        </div>
      {/if}

      <ResultGallery onregenerate={onGenerate} />
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
    flex: 1;
    min-height: 0;
    min-width: 0;
    gap: 10px;
  }

  /* ---------- 服务配置条：全宽单行、控件内联标签 ---------- */
  .conn-bar {
    flex: none;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 14px;
    border: 1px solid var(--hairline);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    padding: 8px 12px;
    box-shadow: var(--shadow-1);
  }
  .cb-field {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
  }
  .cb-label {
    font-size: 12px;
    color: var(--text-2);
    flex-shrink: 0;
  }
  /* 全局 select 是 width:100%（铺满网格单元的口径），单行条里改为内容自适应 */
  .cb-field select {
    width: auto;
    max-width: 220px;
  }
  /* Key 字段吃余宽，输入框有地方打长 Key */
  .cb-key {
    flex: 1 1 220px;
  }
  .cb-key .key-row {
    flex: 1;
  }
  /* 就绪状态灯：绿 = Key 已设置且模型已选；琥珀 = 有未决项。色语与顶栏 chip 状态点一致 */
  .cb-status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid var(--hairline);
    background: var(--bg-inset);
    border-radius: 99px;
    padding: 4px 11px;
    font-size: 12px;
    color: var(--text-2);
    cursor: pointer;
    flex-shrink: 0;
  }
  .cb-status .cb-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--warn);
    flex-shrink: 0;
  }
  .cb-status.ok {
    color: var(--ok);
    border-color: var(--accent-line);
    background: var(--accent-soft);
  }
  .cb-status.ok .cb-dot {
    background: var(--ok);
  }
  /* 就绪卡：Key 未设置时条件出现于配置条下方，琥珀软底 + 直达按钮 */
  .conn-warn {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border: 1px solid var(--warn);
    border-left-width: 3px;
    border-radius: var(--radius-md);
    background: var(--warn-soft);
    font-size: 12.5px;
    color: var(--text-2);
  }
  .conn-warn :global(.ic) {
    color: var(--warn);
    flex-shrink: 0;
  }
  .conn-warn .btn {
    margin-left: auto;
  }
  .key-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .key-row input {
    flex: 1;
    min-width: 0;
    padding-right: 34px;
  }
  .key-btn {
    position: absolute;
    right: 3px;
    width: 24px;
    min-height: 24px;
    height: 24px;
    padding: 0;
  }
  .key-ok {
    position: absolute;
    right: 34px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--ok);
    font-size: 11.5px;
    pointer-events: none;
  }
  .key-ok::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }

  /* ---------- 主工作区 ---------- */
  .wb-grid {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(300px, 340px) minmax(0, 1fr);
    gap: 12px;
  }
  .wb-left {
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow-y: auto;
    padding-right: 2px;
    min-height: 0;
  }
  .wb-right {
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .frow {
    margin: 7px 0;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .frow > label,
  .frow-label {
    width: 48px;
    flex: none;
    color: var(--text-2);
    font-size: 12.5px;
    text-align: right;
  }
  .frow select,
  .frow input {
    flex: 1;
    min-width: 0;
  }
  /* 自定义尺寸行：宽高覆盖 .frow 里 input 的 flex:1，按内容宽 */
  .size-custom > input {
    flex: none;
    width: 72px;
  }
  .size-custom > label {
    width: auto;
  }
  .static-note {
    flex: 1;
    font-size: 12.5px;
    color: var(--text-3);
  }
  /* 竖排行（标签在上、控件在下）：质量分段用 */
  .col-frow {
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
  }
  .col-frow .frow-label {
    width: auto;
    text-align: left;
  }
  .q-seg {
    width: 100%;
  }
  .q-seg button {
    flex: 1;
    white-space: nowrap;
  }
  /* 开关行（水印）：文字主副两行 */
  .opt-row {
    align-items: flex-start;
    margin: 8px 0 2px;
    font-size: 12.5px;
    color: var(--text-1);
  }
  .opt-row .track {
    margin-top: 1px;
  }
  .opt-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    line-height: 1.35;
  }
  .opt-text small {
    color: var(--text-3);
    font-size: 11.5px;
  }
  .seed-dice {
    flex: none;
    width: 30px;
    height: 30px;
  }
  /* 主 CTA：40px 高满宽，贴提示词/参数区与画廊之间随时可点（右栏无整列滚动，sticky 仅兜底） */
  .cta {
    position: sticky;
    bottom: 0;
    flex: none;
    min-height: 40px;
    padding: 9px;
    font-size: 14px;
  }

  /* ---------- 结果区 ---------- */
  .res-bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .res-title {
    display: inline-flex;
    align-items: center;
    font-size: 13px;
    font-weight: 600;
  }
  .res-title::before {
    content: "";
    width: 3px;
    height: 14px;
    border-radius: 2px;
    background: var(--accent);
    margin-right: 8px;
  }
  .res-count {
    font-size: 12px;
    color: var(--text-3);
    font-family: var(--mono);
  }
  .spacer {
    flex: 1;
  }

  /* 内联错误条 */
  .inline-alert {
    flex: none;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 8px 10px;
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
    font-size: 12.5px;
    font-weight: 600;
    color: var(--danger);
  }
  .ia-msg {
    font-size: 12px;
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
    border-radius: 50%;
    border: 2px solid rgba(255, 255, 255, 0.35);
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

  /* ---------- 响应式：≤1024（约窗口 1180 以下）单栏堆叠，整栅格滚动 ---------- */
  @media (max-width: 1024px) {
    .wb-grid {
      grid-template-columns: minmax(0, 1fr);
      overflow-y: auto;
    }
    .wb-left {
      overflow: visible;
      padding-right: 0;
    }
  }
</style>
