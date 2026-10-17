<!-- GenSettingsCard · 生成设置卡：
     自上而下平铺——模板应用栏 / 正文 / 负向折叠（模型不支持则整项不出现）/ 调用方式 /
     参数区。参数区只有一层：左画幅帧（按宽高比实时缩放的输出预览，右侧一条竖分隔线），
     右参数流（比例 / 尺寸 / 张数 / Seed / 质量 / 输出格式 / 水印 / 改写模式 / 思考模式 / 智能改写，
     一个自动折行的流）；无嵌套面板、无分组标题。
     左缘：**盒型元素**（模板栏 / 文本域 / 画幅盒）外沿贴卡内容边 16px；**纯文字行**（正文标题 /
     负向条 / 调用方式 / 尺寸提示）缩进 --sp-4 落到 26px（= 文本域内文左缘）。画幅盒是盒不是文字，
     必须与文本域外沿齐平；画幅帧只在盒内居中缩放，不参与对齐。
     两个文本域的高度只由 rows 定（行高 = --fs-md 13px × 1.65 = 21.45px，上下 padding 与描边共 18px）：
     正文 4 行 ≈ 104px（`resize: vertical` 可拖高）、负向 2 行 = 61px。
     状态除 sizeCustomMode/sizeW/sizeH 等纯 UI 态外全部住在 imageaiStore；
     尺寸违规单点读 store.sizeError（CTA 禁用共用）；外部触点：管理模板 onopentemplates。 -->
<script lang="ts">
  import Icon from "../../ui/Icon.svelte";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text";
  import {
    applyTemplate,
    customSizeStart,
    imageaiStore,
    parseSize,
    rerollPrompt,
    SEED_MAX,
  } from "../../stores/imageai.svelte";
  import NumberStepper from "./NumberStepper.svelte";

  let {
    onopentemplates,
  }: {
    onopentemplates: () => void;
  } = $props();

  // 负向词的可见性完全由能力位决定：模型声明支持才渲染整块，且**默认展开输入框**；
  // 折叠条只是收起来的入口，折叠态在有内容时挂蓝点 + 摘要
  let showNeg = $state(true);

  const caps = $derived(imageaiStore.modelInfo?.capabilities);

  // 字数计数：超限判定 promptOver 由 store 单点派生（跨组件共用），这里只渲染计数
  const promptLen = $derived(imageaiStore.prompt.length);
  const promptLimit = $derived(caps?.maxPromptChars ?? 0);
  const promptNear = $derived(promptLimit > 0 && promptLen > promptLimit * 0.9 && !imageaiStore.promptOver);

  // ---- 尺寸：预设 + 自定义混合。SIZE_CUSTOM 是下拉里「自定义…」的哨兵值，
  //      预设尺寸均为 WxH 形式（小写 x），不会与之相撞。 ----
  const SIZE_CUSTOM = "__custom__";
  /** 「自定义…」的起步宽高：当前尺寸解析不出宽高（档位串模型）时，取该模型声明的像素下限
   *  （见 store.customSizeStart）——写死的 350×350 低于火山/wan 的总像素下限，开局就是一条红线。 */
  const startSize = () => customSizeStart(caps?.sizeRules);
  // 「自定义…」是显式选择的模式：即便合成值恰好等于某预设也保持自定义态，
  // 直到改选预设、或切服务商重置了尺寸才退出（切模型不动尺寸）。
  let sizeCustomMode = $state(false);
  let sizeW = $state(parseSize(imageaiStore.size)?.[0] ?? startSize()[0]);
  let sizeH = $state(parseSize(imageaiStore.size)?.[1] ?? startSize()[1]);

  const sizePresets = $derived(caps?.sizes ?? []);
  /** 尺寸输入框的上下限来自模型声明的约束：缺省 = 不设限（不硬编一套与模型无关的假范围），
   *  越界仍由 store.sizeError 按同一份约束兜底。 */
  const sideMin = $derived(caps?.sizeRules?.minSide || undefined);
  const sideMax = $derived(caps?.sizeRules?.maxSide || undefined);
  /** 输入框的悬停说明：摊开该模型真实的尺寸约束；无约束时不显示空提示。 */
  const sizeRangeTitle = $derived.by(() => {
    const r = caps?.sizeRules;
    if (!r) return undefined;
    const parts: string[] = [];
    if (r.minSide || r.maxSide) parts.push(`宽/高 ${r.minSide || "不限"}–${r.maxSide || "不限"} 像素`);
    if (r.totalMin || r.totalMax) parts.push(`总像素 ${r.totalMin || "不限"}–${r.totalMax || "不限"}`);
    return parts.length > 0 ? parts.join("；") : undefined;
  });
  /** 预设里只要出现解析不出宽×高的档位串（wan2.7 的 1K/2K/4K），就走「尺寸下拉 + 自定义宽高」分支；
   *  预设全是像素值（gpt/qwen/z-image）或没有预设时，才用宽×高双输入。
   *  判据必须是 every：wan2.7 的预设是档位串与官方推荐像素值混合的，用 some 会让档位下拉整个消失。 */
  const pixelSizeMode = $derived(sizePresets.every((s) => parseSize(s) !== null));
  const sizeSelectValue = $derived(
    sizeCustomMode || !sizePresets.includes(imageaiStore.size) ? SIZE_CUSTOM : imageaiStore.size,
  );

  /** 尺寸字符串显示归一化：'*' → '×'，值本身不变（百炼要 '*'、OpenAI 要 'x'，适配器已各自归一）。 */
  function sizeLabel(s: string): string {
    return s.replace("*", "×");
  }

  /** 质量分段：值取模型声明的 qualities，中文映射展示（值仍发英文）。 */
  const QUALITY_LABELS: Record<string, string> = {
    auto: t.gen.qualityAuto,
    low: t.gen.qualityLow,
    medium: t.gen.qualityMedium,
    high: t.gen.qualityHigh,
  };
  const qualityOptions = $derived(caps?.qualities ? ["auto", ...caps.qualities.filter((q) => q !== "auto")] : []);

  // store.size 变化（初始挂载、切服务商重置、自定义输入写回）时把 WxH 回填进两个数字输入；
  // 解析不出宽高（档位串模型的 1K/2K 等）时取该模型的规则起步值：所以切模型也要重算一次，
  // 否则上一个模型推出来的值会留在框里（火山 Pro 下限 921600、Lite 下限 1920²，两者不通用）。
  // 用户真填过的自定义值不会被动：它已写回 store.size，走上面的解析分支原样保留。
  // 输入过程中的同值回填是幂等的，不会打断正在输入的数字。
  $effect(() => {
    void imageaiStore.model;
    const parsed = parseSize(imageaiStore.size);
    if (parsed) {
      sizeW = parsed[0];
      sizeH = parsed[1];
    } else {
      const start = startSize();
      sizeW = start[0];
      sizeH = start[1];
    }
  });

  // 切服务商会把 size 重置为新模型的首个预设 → 退出「自定义…」强制态（Key 框清空留在父组件 effect）。
  $effect(() => {
    void imageaiStore.providerId;
    sizeCustomMode = false;
  });

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

  function syncCustomSize() {
    // 只拦「写不出合法尺寸」的输入（空/NaN/0）：真实边界由模型约束驱动（见 sideMin/sideMax
    // 与 store.sizeError），此处不抄第二套区间，免得两处口径漂移
    const ok = (v: number) => Number.isFinite(v) && v >= 1;
    if (ok(sizeW) && ok(sizeH)) imageaiStore.size = `${sizeW}x${sizeH}`;
  }

  /** 宽×高联动：比例选择器可见且当前命中标准比例时，改一边自动按比例算另一边（四舍五入），随后写回 store。
   *  自由尺寸模型（z-image，无比例字段）不做隐式联动，宽高完全独立。 */
  function onDimInput(side: "w" | "h") {
    const std = ratioGroups.length > 0 ? stdRatio : null;
    if (std) {
      const v = side === "w" ? sizeW : sizeH;
      if (Number.isFinite(v) && v > 0) {
        if (side === "w") sizeH = Math.round((v * std.h) / std.w);
        else sizeW = Math.round((v * std.w) / std.h);
      }
    }
    syncCustomSize();
  }

  // ---- 负向词折叠条：已填内容时给前 12 字摘要（折叠态也能感知到已有内容） ----
  const negSummary = $derived(imageaiStore.negative.trim().slice(0, 12));

  // ---- 比例：标准比例定义表 + 从模型预设归组（每组取首个命中尺寸作为「推荐档」，3% 容差；
  //      非整数比的预设（如 1728×2368 化简为 27:37≈0.73）也归 3:4，帧文案与下拉始终一致） ----
  const RATIO_DEFS = [
    { label: "1:1", w: 1, h: 1 },
    { label: "3:4", w: 3, h: 4 },
    { label: "4:3", w: 4, h: 3 },
    { label: "9:16", w: 9, h: 16 },
    { label: "16:9", w: 16, h: 9 },
  ] as const;

  const ratioGroups = $derived(
    RATIO_DEFS.map((def): { label: string; size: string } | null => {
      const target = def.w / def.h;
      const hit = sizePresets.find((s) => {
        const p = parseSize(s);
        return !!p && Math.abs(p[0] / p[1] - target) / target < 0.03;
      });
      return hit ? { label: def.label, size: hit } : null;
    }).filter((g): g is { label: string; size: string } => g !== null),
  );

  /** 当前尺寸命中的标准比例（3% 容差）；帧文案与比例 select 都以此为准，保证两者一致。 */
  const stdRatio = $derived.by(() => {
    const p = parseSize(imageaiStore.size);
    if (!p) return null;
    const q = p[0] / p[1];
    return RATIO_DEFS.find((d) => Math.abs(q - d.w / d.h) / (d.w / d.h) < 0.03) ?? null;
  });

  /** 比例 select 选中值：命中标准比例即该标签；当前宽高不属于任何标准比例时挂禁用的「自定义…」占位。 */
  const ratioSelectValue = $derived(stdRatio?.label ?? SIZE_CUSTOM);

  function onRatioSelect(e: Event) {
    const v = (e.currentTarget as HTMLSelectElement).value;
    if (v === SIZE_CUSTOM) return; // 已在自定义态，选择该项无动作
    const g = ratioGroups.find((x) => x.label === v);
    if (g) {
      sizeCustomMode = false;
      imageaiStore.size = g.size;
    }
  }

  function gcd(a: number, b: number): number {
    return b === 0 ? a : gcd(b, a % b);
  }

  /** 实时画幅帧：按当前尺寸的真实宽高比在固定规格盒（100×100，对应 120px 盒内留边）内等比缩放。
   *  比例文案优先显示标准标签（命中 3% 容差）；非标准的可解析尺寸先化简，项数 ≤20 显示整数比，
   *  否则显示相对小数（0.73:1 / 1.95:1）。档位串（如 wan 的 "1K"）无法解析：中性正方形。 */
  const frame = $derived.by(() => {
    const invalid = !!imageaiStore.sizeError;
    const box = { w: 100, h: 100 };
    const parsed = parseSize(imageaiStore.size);
    if (!parsed) {
      return {
        w: 72,
        h: 72,
        ratio: "",
        active: false,
        custom: false,
        invalid: false,
        dims: sizeLabel(imageaiStore.size),
      };
    }
    const [pw, ph] = parsed;
    const r = pw / ph;
    let w: number;
    let h: number;
    if (r >= 1) {
      h = Math.min(box.h, box.w / r);
      w = h * r;
    } else {
      w = Math.min(box.w, box.h * r);
      h = w / r;
    }
    let ratio: string;
    if (stdRatio) {
      ratio = stdRatio.label;
    } else {
      const g = gcd(pw, ph);
      const rw = pw / g;
      const rh = ph / g;
      ratio = Math.max(rw, rh) <= 20 ? `${rw}:${rh}` : r >= 1 ? `${r.toFixed(2)}:1` : `1:${(1 / r).toFixed(2)}`;
    }
    return {
      w: Math.round(w),
      h: Math.round(h),
      ratio,
      active: !!stdRatio,
      custom: !stdRatio,
      invalid,
      dims: `${pw}×${ph}`,
    };
  });
</script>

<section class="pcard">
  <div class="pcard-h">
    <span>{t.gen.cardSettings}</span>
  </div>

  <!-- ============ 上半区：画面内容（直接落在卡面） ============ -->
  <!-- 模板应用栏：无内框 select 嵌进统一容器，管理动作贴着模板选择器 -->
  <div class="tpl-bar">
    <Icon name="folder" size={15} class="bar-ic" />
    <select
      id="ig-template"
      value={imageaiStore.templateId}
      onchange={(e) => void applyTemplate(e.currentTarget.value).catch(showError)}
    >
      {#each imageaiStore.config?.templates ?? [] as tpl}
        <option value={tpl.id}>{tpl.name}</option>
      {/each}
    </select>
    <button class="btn sm bar-btn" onclick={onopentemplates}>
      <Icon name="gear" size={14} class="ic" />
      {t.gen.manageTemplate}
    </button>
  </div>

  <!-- 正文标题行：label 左；右侧 = 字数计数（模型有上限时） -->
  <div class="prompt-head">
    <label for="ig-prompt" class="field-label">{t.gen.prompt}</label>
    <span class="head-right">
      {#if promptLimit > 0}
        <span class="counter" class:near={promptNear} class:over={imageaiStore.promptOver}
          >{promptLen} / {promptLimit}</span
        >
      {/if}
    </span>
  </div>

  <!-- 单框可编辑：内容即实发文本。有数据源时由后端组装后回填此处，用户改动优先。
       重掷胶囊浮在框内右下角（仅数据源模式下出现）：textarea 内放不了元素，用 wrapper 定位 -->
  <div class="prompt-wrap">
    <textarea
      id="ig-prompt"
      rows="4"
      bind:value={imageaiStore.prompt}
      class:invalid={imageaiStore.promptOver}
      oninput={() => (imageaiStore.promptDirty = true)}
    ></textarea>
    {#if imageaiStore.sourcePlayer}
      <button type="button" class="reroll-btn" title={t.gen.rerollPromptTitle} onclick={rerollPrompt}>
        {t.gen.rerollPrompt}
      </button>
    {/if}
  </div>
  {#if imageaiStore.promptOver}
    <p class="field-error">{t.gen.promptTooLong(promptLimit)}</p>
  {/if}

  <!-- 负向提示词：模型不声明该能力位时整块不渲染（值仍留在 store 里但不发送）；
       支持时默认展开输入框，折叠条负责收起/展开，折叠态有内容则挂蓝点 + 摘要 -->
  {#if caps?.supportsNegativePrompt}
    <button type="button" class="neg-bar" aria-expanded={showNeg} onclick={() => (showNeg = !showNeg)}>
      <Icon name={showNeg ? "chevronUp" : "chevronDown"} size={14} class="ic" />
      <span>{t.gen.negativeCollapsed}</span>
      <span class="neg-right">
        {#if negSummary && !showNeg}
          <span class="neg-dot" aria-hidden="true"></span>
          <span class="neg-sum">{negSummary}…</span>
        {/if}
      </span>
    </button>
    {#if showNeg}
      <textarea
        class="neg-area"
        rows="2"
        placeholder={t.gen.negative}
        bind:value={imageaiStore.negative}
      ></textarea>
    {/if}
  {/if}

  {#if caps?.supportsAsync}
    <!-- 调用方式：仅支持异步的模型出现，可切换并随请求提交（后端按同一能力位再门控一遍） -->
    <div class="call-row">
      <span class="g-label">{t.gen.callMode}</span>
      <select bind:value={imageaiStore.callMode}>
        <option value="sync">{t.gen.callModeSync}</option>
        <option value="async">{t.gen.callModeAsync}</option>
      </select>
    </div>
  {/if}

  <!-- ============ 生成参数区：画幅帧（左，竖线分隔）+ 参数流（右）。
       不再有内层面板底色与分组标题，参数直接平铺在卡面 ============ -->
  <div class="canvas">
    <!-- 左栏：画幅帧（120px 方盒，见 .frame-box）。它是这组参数的「输出预览」而非可编辑控件，
         竖立后不与 30px 高的输入框争同一行的视觉层级；窄窗时降为横排（见 @media） -->
    <div class="canvas-body">
      <div class="frame-col">
        <div class="frame-box">
          <div
            class="frm"
            class:active={frame.active && !frame.invalid}
            class:custom={frame.custom && !frame.invalid}
            class:invalid={frame.invalid}
            style="width:{frame.w}px;height:{frame.h}px"
            aria-hidden="true"
          >
            {#if frame.ratio}
              <span class="frm-ratio">{frame.ratio}</span>
            {/if}
            <span class="frm-n" title={t.gen.nMaxHint(imageaiStore.n)}>×{imageaiStore.n}</span>
          </div>
        </div>
        <p class="frm-cap" class:err={frame.invalid}>
          {frame.invalid ? t.gen.sizeInvalidShort : frame.dims}
        </p>
      </div>

      <!-- 右栏：全部生成参数（比例/尺寸/张数/Seed/质量/水印）平等排布一个流，自动折行 -->
      <div class="param-col">
        <div class="gfields">
          {#if ratioGroups.length > 0}
            <!-- 比例简易下拉：选中即写入该比例的官方推荐档；非标准宽高时挂禁用的「自定义…」占位 -->
            <div class="gfield f-ratio">
              <label for="ig-ratio">{t.gen.ratio}</label>
              <select id="ig-ratio" value={ratioSelectValue} onchange={onRatioSelect}>
                {#each ratioGroups as g (g.label)}
                  <option value={g.label}>{g.label}</option>
                {/each}
                {#if !stdRatio}
                  <option value={SIZE_CUSTOM} disabled>{t.gen.sizeCustom}</option>
                {/if}
              </select>
            </div>
          {/if}
          {#if pixelSizeMode}
            <!-- 尺寸 = 宽×高双输入：命中标准比例时改任一边，另一边自动按比例换算 -->
            <div class="gfield f-size">
              <label for="ig-size-w">{t.gen.size}</label>
              <input
                id="ig-size-w"
                type="number"
                min={sideMin}
                max={sideMax}
                step="1"
                placeholder={t.gen.sizeWLabel}
                aria-label={t.gen.sizeWLabel}
                title={sizeRangeTitle}
                class:invalid={!!imageaiStore.sizeError}
                bind:value={sizeW}
                oninput={() => onDimInput("w")}
              />
              <span class="dim-x" aria-hidden="true">×</span>
              <input
                id="ig-size-h"
                type="number"
                min={sideMin}
                max={sideMax}
                step="1"
                placeholder={t.gen.sizeHLabel}
                aria-label={t.gen.sizeHLabel}
                title={sizeRangeTitle}
                class:invalid={!!imageaiStore.sizeError}
                bind:value={sizeH}
                oninput={() => onDimInput("h")}
              />
            </div>
          {:else}
            <!-- 档位串模型（wan 的 1K/2K/4K）：尺寸无法表达为宽高，走下拉；选「自定义…」时
                 在同一字段内追加宽×高双输入，使两种模型在流里的结构一致（都是「尺寸 + 宽×高」） -->
            <div class="gfield f-size">
              <label for="ig-size">{t.gen.size}</label>
              <select id="ig-size" value={sizeSelectValue} onchange={onSizeSelect}>
                {#each sizePresets as s (s)}
                  <option value={s}>{sizeLabel(s)}</option>
                {/each}
                <option value={SIZE_CUSTOM}>{t.gen.sizeCustom}</option>
              </select>
              {#if sizeSelectValue === SIZE_CUSTOM}
                <input
                  id="ig-tier-w"
                  type="number"
                  min={sideMin}
                  max={sideMax}
                  step="1"
                  placeholder={t.gen.sizeWLabel}
                  aria-label={t.gen.sizeWLabel}
                  title={sizeRangeTitle}
                  class:invalid={!!imageaiStore.sizeError}
                  bind:value={sizeW}
                  oninput={syncCustomSize}
                />
                <span class="dim-x" aria-hidden="true">×</span>
                <input
                  id="ig-tier-h"
                  type="number"
                  min={sideMin}
                  max={sideMax}
                  step="1"
                  placeholder={t.gen.sizeHLabel}
                  aria-label={t.gen.sizeHLabel}
                  title={sizeRangeTitle}
                  class:invalid={!!imageaiStore.sizeError}
                  bind:value={sizeH}
                  oninput={syncCustomSize}
                />
              {/if}
            </div>
          {/if}
          <div class="gfield f-n">
            <span class="g-label">{t.gen.n}</span>
            <!-- 张数双门控：supportsN 缺省按 true（后端 serde default = yes）；
                 仅显式 false（z-image 无 n 参数）才固定 1 张 -->
            {#if caps && caps.supportsN !== false && caps.maxN > 1}
              <NumberStepper
                value={imageaiStore.n}
                min={1}
                max={caps.maxN}
                ariaLabel={t.gen.n}
                onchange={(v) => (imageaiStore.n = v)}
              />
            {:else if caps?.supportsN === false || caps?.maxN === 1}
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
          {#if caps?.supportsSeed}
            <div class="gfield f-seed">
              <label for="ig-seed">{t.gen.seed}</label>
              <!-- number 型：min/max 把官方取值域 [0, SEED_MAX] 声明给平台（无障碍与校验 API 读得到），
                   但浏览器不拦手输的越界值 ⇒ 生效判定仍在 store.parseSeed（越界按随机），
                   可见反馈走 class:invalid + title。
                   不用 bind:value：number 输入的 bind 会把值转成 number/null，而 store.seed 是
                   字符串（'' = 随机、原样保留用户输入），parseSeed 与 seedOutOfRange 都按字符串读。 -->
              <input
                id="ig-seed"
                type="number"
                min="0"
                max={SEED_MAX}
                step="1"
                placeholder={t.gen.seedPlaceholder}
                title={imageaiStore.seedOutOfRange ? t.gen.seedOutOfRange(SEED_MAX) : ""}
                class:invalid={imageaiStore.seedOutOfRange}
                value={imageaiStore.seed}
                oninput={(e) => (imageaiStore.seed = e.currentTarget.value)}
              />
            </div>
          {/if}
          {#if caps?.supportsSteps}
            <!-- 采样步数（硅基流动 num_inference_steps / 魔搭 steps）：留空 = 用服务端默认 -->
            <div class="gfield f-steps">
              <label for="ig-steps">{t.gen.steps}</label>
              <input
                id="ig-steps"
                type="number"
                min="1"
                max={caps.stepsMax || undefined}
                step="1"
                placeholder={caps.stepsDefault ? String(caps.stepsDefault) : ""}
                title={t.gen.stepsHint}
                value={imageaiStore.steps}
                oninput={(e) => (imageaiStore.steps = e.currentTarget.value)}
              />
            </div>
          {/if}
          {#if caps?.supportsGuidance}
            <!-- 引导系数（硅基流动 guidance_scale / 魔搭 guidance）：留空 = 用服务端默认 -->
            <div class="gfield f-guidance">
              <label for="ig-guidance">{t.gen.guidance}</label>
              <input
                id="ig-guidance"
                type="number"
                min="0"
                max={caps.guidanceMax || undefined}
                step="0.5"
                placeholder={caps.guidanceDefault ? String(caps.guidanceDefault) : ""}
                title={t.gen.guidanceHint}
                value={imageaiStore.guidance}
                oninput={(e) => (imageaiStore.guidance = e.currentTarget.value)}
              />
            </div>
          {/if}
          {#if caps?.supportsQuality}
            <!-- 质量分段：与其余参数同流排布，折行时整字段一起走 -->
            <div class="gfield f-quality">
              <span class="g-label">{t.gen.quality}</span>
              <div class="seg3" role="group" aria-label={t.gen.quality}>
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
          {#if caps?.supportsOutputFormat}
            <!-- 输出格式（火山 Seedream 5.0 系列 / 硅基流动）：选项值与协议字段一字不差（png/jpeg，
                 不做大小写或中文转换），适配器按同一能力位门控后照此发送，落盘后缀随之走 -->
            <div class="gfield f-format">
              <label for="ig-output-format">{t.gen.outputFormat}</label>
              <select
                id="ig-output-format"
                value={imageaiStore.outputFormat}
                title={t.gen.outputFormatHint}
                onchange={(e) => (imageaiStore.outputFormat = e.currentTarget.value as 'png' | 'jpeg')}
              >
                <option value="png">PNG</option>
                <option value="jpeg">JPEG</option>
              </select>
            </div>
          {/if}
          {#if caps?.supportsWatermark}
            <!-- 水印：与其他参数同列同高（.gfield 复用字段的 label+控件同行布局）。
                 开关本体挂 .switch 复用全局轨道几何与 checked/focus 状态（不重写一份），
                 但**不用** .opt-row 的两行副标题结构 —— 副标题会让该字段高出一倍、拉歪整行基线；
                 说明文字改走 title 悬停。类名仍叫 track（.switch .track 才有几何） -->
            <label class="switch gfield f-watermark" title={t.gen.watermarkHint}>
              <input type="checkbox" bind:checked={imageaiStore.watermark} />
              <span class="track"></span>
              <span class="wm-text">{t.gen.watermark}</span>
            </label>
          {/if}
          {#if caps?.supportsPromptExtendMode}
            <!-- 改写模式：direct（DPE）/ agent（APE）。agent 官方仅支持文生图，说明走 title -->
            <div class="gfield f-extend-mode">
              <label for="ig-extend-mode">{t.gen.extendMode}</label>
              <select
                id="ig-extend-mode"
                value={imageaiStore.promptExtendMode}
                title={t.gen.extendModeAgentHint}
                onchange={(e) => (imageaiStore.promptExtendMode = e.currentTarget.value)}
              >
                <option value="direct">{t.gen.extendModeDirect}</option>
                <option value="agent">{t.gen.extendModeAgent}</option>
              </select>
            </div>
          {/if}
          {#if caps?.supportsEnableThinking}
            <!-- 思考模式官方「仅 prompt_extend=true 时生效」：改写关闭时置灰而非隐藏，
                 免得参数凭空消失；置灰态的值仍是用户上次的选择 -->
            <label class="switch gfield f-thinking" class:g-off={!imageaiStore.promptExtend} title={t.gen.thinkingHint}>
              <input type="checkbox" disabled={!imageaiStore.promptExtend} bind:checked={imageaiStore.enableThinking} />
              <span class="track"></span>
              <span class="wm-text">{t.gen.thinking}</span>
            </label>
          {/if}
          {#if caps?.supportsPromptExtend}
            <!-- 智能改写：改的是提示词文本，但仍属一次请求的开关，收拢到参数流末尾；
                 长说明走 title，免得把这个字段撑成两行高、拉歪整行基线 -->
            <label class="switch gfield f-extend" title={t.gen.promptExtendHint}>
              <input type="checkbox" bind:checked={imageaiStore.promptExtend} />
              <span class="track"></span>
              <span class="wm-text">{t.gen.promptExtend}</span>
            </label>
          {/if}
        </div>
      </div>
    </div>

    <!-- 尺寸错误/提示文案：不是参数项，横跨面板全宽（长句需要宽度，缩到右栏会折成多行） -->
    {#if imageaiStore.sizeError}
      <p class="field-error canvas-err" aria-live="polite">{imageaiStore.sizeError}</p>
    {:else if !pixelSizeMode && sizeSelectValue === SIZE_CUSTOM}
      <p class="hint canvas-err">{t.gen.sizeCustomHint}</p>
    {:else if pixelSizeMode && sizePresets.length > 0 && !sizePresets.includes(imageaiStore.size)}
      <!-- 像素模型：当前宽高不是官方推荐档时提示可改选比例回填 -->
      <p class="hint canvas-err">{t.gen.sizeCustomHint}</p>
    {/if}
  </div>
</section>

<style>
  /* ---------- 上半区：画面内容 ---------- */
  /* 模板应用栏：统一容器包无内框下拉（选择器自身的描边/底色移除，焦点走容器 :focus-within）。
     这是盒内内边距：图标与文本域的内文（26px）同一条线，不是行左缘缩进 */
  .tpl-bar {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    height: var(--control-h);
    padding: 0 var(--sp-1) 0 var(--sp-4);
    margin-bottom: var(--sp-3);
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
    transition:
      border-color var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out);
  }
  .tpl-bar:hover {
    border-color: var(--hairline-3);
  }
  .tpl-bar:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--focus-ring);
  }
  .tpl-bar :global(.bar-ic) {
    flex: none;
    color: var(--text-3);
  }
  .tpl-bar select {
    flex: 1;
    min-width: 0;
    border: none;
    background-color: transparent;
    box-shadow: none;
    padding-left: var(--sp-2);
  }
  .tpl-bar select:hover:not(:focus) {
    border-color: transparent;
  }
  .bar-btn {
    flex: none;
  }

  /* 正文标题行与负向条、调用方式同一档：纯文字行缩进 --sp-4（= 文本域内文左缘 26px）。
     盒型元素（模板栏 / 文本域 / 画幅盒）不缩进，外沿贴卡内容边 16px */
  .prompt-head {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: 0 var(--sp-4);
    margin-bottom: var(--sp-1);
  }
  .field-label {
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .head-right {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
  }
  /* 提示词计数 */
  .counter {
    font-size: var(--fs-xs);
    font-family: var(--mono);
    color: var(--text-3);
    white-space: nowrap;
  }
  .counter.near {
    color: var(--warn);
  }
  .counter.over {
    color: var(--danger);
  }
  /* 重掷胶囊：浮在输入框内右下角（textarea 内放不了元素，由 .prompt-wrap 定位）；
     仅数据源模式渲染。几何复用 pill 令牌，hover 走强调色 */
  .prompt-wrap {
    position: relative;
  }
  .reroll-btn {
    position: absolute;
    right: var(--sp-2);
    bottom: var(--sp-2);
    padding: var(--pill-padding-y) var(--pill-padding-x-sm);
    border-radius: var(--radius-pill);
    border: 1px solid var(--hairline);
    background: var(--bg-surface);
    font-size: var(--fs-xs);
    color: var(--text-2);
    white-space: nowrap;
    cursor: pointer;
    transition:
      color var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out);
  }
  .reroll-btn:hover {
    color: var(--accent);
    border-color: var(--accent-line);
  }
  /* 负向词折叠条：纯文字行，缩进 --sp-4 与正文标题行同左缘。
     按钮自身的描边/底色清掉，只剩一行文字 */
  .neg-bar {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    padding: 0 var(--sp-4);
    margin-top: var(--sp-3);
    background: none;
    border: none;
    color: var(--text-3);
    font-size: var(--fs-sm);
    cursor: pointer;
    white-space: nowrap;
    transition: color var(--dur-fast) var(--ease-out);
  }
  .neg-bar:hover {
    color: var(--accent);
  }
  .neg-bar :global(.ic) {
    flex: none;
  }
  .neg-right {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }
  .neg-sum {
    font-family: var(--mono);
    font-size: var(--fs-xs);
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .neg-dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: var(--radius-circle);
    background: var(--accent);
  }
  .neg-area {
    margin-top: var(--sp-2);
  }
  /* 调用方式行：位于参数区之上、与参数同族的「标签 + 控件」一行式；纯文字行，缩进 --sp-4。
     仅支持异步的模型（supportsAsync）才渲染本行，控件可选可用 */
  .call-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 0 var(--sp-4);
    margin-top: var(--sp-3);
  }
  .call-row select {
    flex: none;
    width: 124px;
  }

  /* ---------- 生成参数区（原「画幅规格」子面板） ---------- */
  /* 无底色 / 无描边 / 无分组标题（用户要求去掉内层拆分），也**不加横向内边距**：
     本层第一个元素是画幅盒，它的外沿要和上方文本域的外沿齐平（都在卡内容边），
     加内边距等于把画幅盒相对输入框缩进 10px。盒内的画幅帧只管在盒里按比例居中。
     上边距取 --sp-3：与负向条、调用方式行同一档 8px 节奏 */
  .canvas {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    margin-top: var(--sp-3);
  }

  /* ---- 画幅/参数两栏：画幅帧左栏独立，参数右栅格 ---- */
  /* 两栏容器：帧列定宽、参数列吃掉剩余宽度。frame-col/param-col 之间的竖分隔线是
     「预览 / 可编辑」的分界——不靠色块区分，避免面板里再多一层背景 */
  .canvas-body {
    display: flex;
    align-items: stretch;
    gap: var(--sp-5);
  }
  /* 左栏：画幅帧（输出预览）。帧在盒内居中按真实比例缩放。
     【宽度必须把右内边距与竖线加回来】本列是边框盒且自身承担 12px 右内边距 + 1px 竖分隔线，
     只写 120px 时内容盒仅剩 107px，120px 的画幅盒放不下，会被 align-items:center 左右各甩出
     6.5px —— 表现就是画幅盒比提示词输入框「探出来」一截。 */
  .frame-col {
    flex: none;
    width: calc(120px + var(--sp-5) + 1px);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-2);
    padding-right: var(--sp-5);
    border-right: 1px solid var(--hairline);
  }
  /* 固定画幅盒：圆角虚线框 + 近黑舞台底，明示「画幅区」 120×120 恒定，帧在内居中缩放 */
  .frame-box {
    width: 120px;
    height: 120px;
    display: grid;
    place-items: center;
    background: var(--stage);
    border: 1.5px dashed var(--hairline-2);
    border-radius: var(--radius-lg);
  }
  /* 画幅帧：默认中性虚线；命中标准比例 = accent 实线；自定义 = accent 虚线；违规 = danger。
     底色用 --bg-raised 而非 --bg-surface：面板底是 --bg-inset、框内舞台是 --stage，
     若帧与输入框同为 surface 会在深色下贴平，只剩虚线撑着边界 */
  .frm {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1.5px dashed var(--hairline-3);
    border-radius: var(--radius-md);
    background: var(--bg-raised);
    transition:
      width var(--dur-fast) var(--ease-out),
      height var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out);
  }
  .frm.active {
    border-style: solid;
    border-color: var(--accent);
  }
  .frm.custom {
    border-style: dashed;
    border-color: var(--accent);
  }
  .frm.invalid {
    border-color: var(--danger);
    background: var(--danger-soft);
  }
  .frm-ratio {
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  .frm.active .frm-ratio,
  .frm.custom .frm-ratio {
    color: var(--accent);
  }
  .frm.invalid .frm-ratio {
    color: var(--danger);
  }
  /* 张数角标：贴在画幅帧右上角，一眼带出「出几张」 */
  .frm-n {
    position: absolute;
    top: -9px;
    right: -9px;
    padding: 1px var(--sp-2);
    border-radius: var(--radius-pill);
    border: 1px solid var(--hairline-2);
    background: var(--bg-raised);
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text-2);
    line-height: 1.4;
  }
  .frm-cap {
    margin: 0;
    font-family: var(--mono);
    font-size: var(--fs-xs);
    color: var(--text-3);
    white-space: nowrap;
  }
  .frm-cap.err {
    color: var(--danger);
  }

  /* 右栏参数流：所有生成旋钮平等排布、自动折行；不分组、不加分隔条
     （字段之间靠 gap 与折行已能读出节奏，再切一刀是过度表达） */
  .param-col {
    flex: 1;
    min-width: 0;
  }
  /* 字段流：横向排布、可折行；字段内部始终 label 与控件同行（折行只发生在字段之间）。
     行距取 --sp-4（10px）而非 --sp-3（8px）：控件高 30px（质量分段更高），
     8px 折行时上下两行几乎贴死，读不出是两组字段。列距 --sp-5（12px）不变 */
  .gfields {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-4) var(--sp-5);
    min-width: 0;
  }
  .gfield {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }
  .gfield > label,
  .g-label {
    flex: none;
    white-space: nowrap;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .gfield input,
  .gfield select {
    flex: 1;
    width: auto;
    min-width: 0;
  }
  /* 比例：内容定宽，下拉 72px（选项只有 1:1/3:4 等短串；自定义占位仅展示） */
  .f-ratio {
    flex: none;
  }
  .f-ratio select {
    flex: none;
    width: 72px;
  }
  /* 尺寸：像素模型 = 宽×高两个 64px 定宽输入；
     档位串模型 = 124px 定宽下拉，选「自定义…」后在同一字段内追加宽×高双输入
     （此时整字段共 124 + 64 + 64 + 两个分隔符 ≈ 270px，.f-size 的 flex:none 保证
     下拉不吃 flex:1 而把双输入挤扁） */
  .f-size {
    flex: none;
  }
  .f-size input {
    flex: none;
    width: 64px;
  }
  .f-size select {
    flex: none;
    width: 124px;
  }
  .dim-x {
    flex: none;
    color: var(--text-3);
  }
  /* 张数：内容定宽（label + 步进器）；步进器显式锁宽，防 number 输入框内禀宽把字段撑开。
     单图模型渲染静态说明文字，字段按内容自然宽 */
  .f-n {
    flex: none;
  }
  .f-n > :global(.stepper) {
    flex: none;
    width: 112px;
  }
  /* Seed：弹性，basis 130px。
     min-width 130px 是硬下限 —— Seed 为 32 位数字（最多 10 字符）加「留空=随机」占位符，
     再窄占位符会被截成「留空=随」，读不出这框是干什么的；max-width 220px 防宽窗下拉出过长一行。
     【勿加外置清空/骰子按钮】留空即随机，清空靠选中删除；number 的步进器已被 app.css 全局隐去，
     130px 全部留给数字与占位符，不会被控件吃掉。 */
  .f-seed {
    flex: 1 1 130px;
    min-width: 130px;
    max-width: 220px;
  }
  /* 采样步数 / 引导系数：窄数字输入，内容定宽（与尺寸双输入同档宽度） */
  .f-steps,
  .f-guidance {
    flex: none;
  }
  .f-steps input,
  .f-guidance input {
    flex: none;
    width: 72px;
  }
  /* 单图模型静态说明：与 label 同行、与控件等高，文字较长允许换行不跳动 */
  .static-note {
    display: flex;
    align-items: center;
    min-height: var(--control-h);
    font-size: var(--fs-sm);
    color: var(--text-3);
  }
  /* 尺寸错误/提示：横跨全宽（不是参数项，长句需要宽度）。纯文字行，缩进 --sp-4 与其余文字行同左缘；
     它与画幅盒不同批 —— 画幅盒是盒，本案的盒一律不缩进 */
  .canvas-err {
    margin: 0;
    padding: 0 var(--sp-4);
  }
  /* 水印：与比例/尺寸/张数/Seed/质量同为一行字段。轨道几何与状态全在全局 .switch
     （34×19 胶囊轨道 + 13px 滑块），这里只补字段流的接入与文字样式：
     文字用 .g-label 同一套（12.5px / --text-2），使它与其他字段的标签视觉同族 */
  .f-watermark {
    flex: none;
    cursor: pointer;
  }
  /* 智能改写：与其余参数同列同高（说明走 title，不做两行副标题） */
  .f-extend {
    flex: none;
    cursor: pointer;
  }
  /* 开关字段的可见文字（水印 / 思考模式 / 智能改写）：与 .g-label 同族，
     悬停提到 --text-1，与 .gfield 内输入框的聚焦反馈同档 */
  .wm-text {
    font-size: var(--fs-sm);
    color: var(--text-2);
    white-space: nowrap;
  }
  .f-watermark:hover .wm-text,
  .f-thinking:hover .wm-text,
  .f-extend:hover .wm-text {
    color: var(--text-1);
  }
  /* 输出格式：与改写模式同档的短枚举下拉（选项只有 PNG / JPEG 两个短串） */
  .f-format {
    flex: none;
  }
  .f-format select {
    flex: none;
    width: 84px;
  }
  /* 改写模式：短枚举下拉，与比例同档定宽 */
  .f-extend-mode {
    flex: none;
  }
  .f-extend-mode select {
    flex: none;
    width: 124px;
  }
  /* 思考模式在改写关闭时置灰（开关本体已 disabled，这里只做视觉降档） */
  .g-off {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* 质量分段：与比例/尺寸/张数/Seed 同列。
     【等高】全局 .seg3 是弹窗用的宽松档（padding 3px + 按钮 padding 6px + 13px 文字 ≈ 37px），
     比输入框的 --control-h(30px) 高 7px，排在同一行时上下都不齐。
     这里只在画幅面板内压平：外层固定 control-h、内边距收到 2px、按钮竖向 padding 归 0
     改由 align-items 撑满 —— 不动全局 .seg3，弹窗里仍是宽松档。
     【宽度】外层与 .seg3 都 flex:none + 定宽：全局 button 已是 flex:1，
     外层再给弹性会让四个按钮最小内容宽叠加超过右栏，把整字段挤成独占一行。 */
  .f-quality {
    flex: none;
  }
  .f-quality .seg3 {
    flex: none;
    width: 296px;
    height: var(--control-h);
    padding: 2px;
    align-items: stretch;
  }
  /* 按钮撑满 .seg3 内腔（padding 归 0，高度由 stretch 给），文字垂直居中由 .seg3 的
     align-items:stretch + 按钮自身 flex 布局保证 */
  .f-quality .seg3 button {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 var(--sp-4);
  }

  /* ---------- 响应式：窄栏两栏降为单栏 ---------- */
  /* 断点 560px：低于此宽度，「120px 帧 + 竖分隔 + 参数列」会把参数列压到不足一行
     （比例 72 + 尺寸 2×64 + 张数 112 已近 320px，再加 Seed/质量必折）。此时帧横置在上、
     参数在下，竖分隔改横分隔（分隔方向随布局翻转，不留悬空边） */
  @media (max-width: 560px) {
    .canvas-body {
      flex-direction: column;
      gap: var(--sp-4);
    }
    .frame-col {
      width: auto;
      align-self: stretch;
      flex-direction: row;
      align-items: center;
      gap: var(--sp-4);
      padding-right: 0;
      padding-bottom: var(--sp-4);
      border-right: none;
      border-bottom: 1px solid var(--hairline);
    }
  }
</style>
