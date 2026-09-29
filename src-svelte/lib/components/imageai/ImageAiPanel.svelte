<!-- AI 生图工作台：顶部全宽服务配置条；左栏球员源单人档案卡；右栏自上而下 = 提示词主区 +
     参数区 + 生成 CTA + 结果画廊，右栏底部可折叠 QueueDock。复用全局类与 icons，不重写控件。 -->
<script lang="ts">
  import { onMount } from "svelte";
  import { confirm, message } from "@tauri-apps/plugin-dialog";
  import { api } from "../../services/api";
  import QueueDock from "./QueueDock.svelte";
  import ProviderDialog from "./ProviderDialog.svelte";
  import TemplateDialog from "./TemplateDialog.svelte";
  import NumberStepper from "./NumberStepper.svelte";
  import RatioPicker from "./RatioPicker.svelte";
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 数据源条的头像缩略图 + 放大气泡（与名单预览共用）
  import { createAvatarSource } from "../../avatarBlobs.svelte"; // 头像路径查询+缩略图装载（与名单预览/队列坞共用）
  import { appStore, basedValue } from "../../stores/app.svelte"; // 气泡缓存失效键 + 第 15 列口径（basedLabel/basedValue）
  import { ui } from "../../stores/ui.svelte"; // 就绪卡「去存档资料台」切工作台
  import { t } from "../../text";
  import { showError } from "../../ui/dialog";
  import Icon from "../../ui/Icon.svelte";
  import { saveGeneratedImageAs } from "../../ui/generatedImageSave"; // 生成图另存为（与队列坞共用）
  import {
    VARIABLE_HINTS,
    appendVariable,
    applyTemplate,
    clearResults,
    clearSourcePlayer,
    enqueuePlayers,
    generate,
    imageaiStore,
    loadImageState,
    parseSize,
    removeResult,
    saveAndReplace,
    selectModel,
    selectProvider,
    setProviderKey,
    setSourcePlayer,
    validateSizeRules,
  } from "../../stores/imageai.svelte";
  import type { PlayerPreview } from "../../types";

  let providerOpen = $state(false);
  let templateOpen = $state(false);
  let showNeg = $state(true);
  let dockOpen = $state(false); // 队列坞开合（结果工具条按钮与坞内收起按钮共用）

  // ---- 生成反馈：内联错误条（含重试）+ 耗时计时（不再只弹系统窗）----
  let genError = $state("");
  let elapsed = $state(0);
  let timer: ReturnType<typeof setInterval> | undefined;

  // ---- 数据源只读预览：有数据源时提示词由 preview_prompt 渲染，文本框只读展示 ----
  let sourcePreview = $state("");

  // ---- 结果大图灯箱 ----
  let lightboxUrl = $state<string | null>(null);

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

  // 数据源变化时拉一次只读预览（与 generate 的数据源链路同一 IPC）；失败静默退回空预览，
  // 真正生成时 generate() 还会再渲染一次并把 missing 错误抛到内联错误条。
  $effect(() => {
    const sp = imageaiStore.sourcePlayer;
    if (!sp) {
      sourcePreview = "";
      return;
    }
    let cancelled = false;
    void api
      .previewPrompt(sp.uid, imageaiStore.templateId)
      .then((r) => {
        if (!cancelled) sourcePreview = r.text;
      })
      .catch(() => {
        if (!cancelled) sourcePreview = "";
      });
    return () => {
      cancelled = true;
    };
  });

  // ---- API Key 直达输入：掩码显示、不回显真值，成功即清空 ----
  let keyInput = $state("");
  let keyEl = $state<HTMLInputElement | null>(null);

  // ---- 连接就绪度（口径与顶栏 ③chip 同源：Key 已设置 + 模型已选）----
  // 项目没有真正的连通性测试命令，这里以「Key 已设置」为代理判据，措辞不冒充「已连通」。
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
  // 直到改选预设、或切服务商重置了尺寸才退出（切模型不动尺寸，见 store 注释）。
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

  /** 提示词字数计数。 */
  const promptLen = $derived(imageaiStore.prompt.length);
  const promptLimit = $derived(caps?.maxPromptChars ?? 0);
  const promptOver = $derived(promptLimit > 0 && promptLen > promptLimit);
  const promptNear = $derived(
    promptLimit > 0 && promptLen > promptLimit * 0.9 && !promptOver,
  );

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

  /** CTA 可用性：生成中 / 尺寸违规 / 提示词超长时禁用（空提示词等仍由后端校验兜底）。 */
  const genDisabled = $derived(
    imageaiStore.generating || !!sizeError || promptOver,
  );

  const queueBadge = $derived(
    imageaiStore.queue.filter(
      (e) => e.status === "queued" || e.status === "running",
    ).length,
  );

  // store.size 变化（初始挂载、切服务商重置、自定义输入写回）时把 WxH 回填进两个数字输入；
  // 解析失败（理论不会发生）保持现值。输入过程中的同值回填是幂等的，不会打断正在输入的数字。
  $effect(() => {
    const parsed = parseSize(imageaiStore.size);
    if (parsed) {
      sizeW = parsed[0];
      sizeH = parsed[1];
    }
  });

  // 切服务商会把 size 重置为新模型的首个预设 → 退出「自定义…」强制态；
  // 同时清空 Key 输入框（Key 跟随服务商，输入内容不跨服务商残留；已设置状态见 keysSet）。
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
      keyInput = ""; // 成功即清空，绝不回显真值
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

  // 首次进入懒加载配置；失败统一走错误弹窗。
  onMount(() => {
    if (!imageaiStore.config) void loadImageState().catch(showError);
  });

  // ---- 球员源档案卡：头像缩略图（有映射时）+ 资料胶囊 ----
  // 头像路径/缩略图走共用的 createAvatarSource（与名单预览同一实现）；
  // 取不到/取失败都静默降级为只显示姓名+UID（辅助展示不弹窗）。
  const srcAvatars = createAvatarSource();
  $effect(() => {
    const sp = imageaiStore.sourcePlayer;
    srcAvatars.clear(); // 换数据源立即清场：旧球员的头像不该闪现在新卡上
    if (sp) void srcAvatars.load([sp.uid]);
  });

  // ---- 球员源资料：preview_player 缓存未命中时要重解析整份档景（秒级），按 uid 记账防重拉 ----
  // 台内 UID 查询命中时已带回首份资料，effect 靠 srcProfileUid 跳过同 uid 的第二次查询。
  let srcProfile = $state<PlayerPreview | null>(null);
  let srcProfileUid = $state(""); // 资料已为哪个 uid 拉取过（失败也记账，避免 effect 反复重试）
  let srcProfileToken = 0;
  $effect(() => {
    const sp = imageaiStore.sourcePlayer;
    if (!sp) {
      srcProfile = null;
      srcProfileUid = "";
      return;
    }
    if (srcProfileUid === sp.uid) return;
    const token = ++srcProfileToken;
    api
      .previewPlayer(sp.uid)
      .then((got) => {
        if (token !== srcProfileToken) return;
        srcProfile = got;
        srcProfileUid = sp.uid;
      })
      .catch(() => {
        // uid 不在当前名单（数据源可能来自上一次扫描）：降级为只显示姓名/UID，不弹窗刷屏
        if (token === srcProfileToken) {
          srcProfile = null;
          srcProfileUid = sp.uid;
        }
      });
  });
  const srcRec = $derived(
    imageaiStore.sourcePlayer ? (srcProfile?.record ?? null) : null,
  );
  // 第 15 列口径单点见 appStore（isStaffRoster/basedLabel/basedValue）。

  // ---- 台内 UID 查询（球员源第三来源，与名单行「送生图」/单人替换「用此球员生成」同入口）----
  let srcUidInput = $state("");
  let srcQueryError = $state("");
  async function querySourceUid() {
    const want = srcUidInput.trim();
    if (!want) return;
    try {
      const got = await api.previewPlayer(want);
      // 回显命中的完整 UID（与 PlayerViewer 同规则），姓名缺失时退回 UID 展示
      const matched = got?.record?.uid ?? got?.mapping?.uid ?? want;
      srcProfile = got; // 资料顺手带入，免得 effect 再查一遍（preview_player 秒级）
      srcProfileUid = matched;
      setSourcePlayer(matched, got?.record?.name?.trim() || matched);
      srcUidInput = "";
      srcQueryError = "";
    } catch {
      // 未命中：数据源保持不变，就地提示（查询动作的失败不必弹系统窗）
      srcQueryError = t.gen.source.queryMiss(want);
    }
  }

  /** 把当前数据源球员单独投进队列（多人批量走名单工具行的「加入队列」，不经过本卡）。 */
  async function enqueueSource() {
    const sp = imageaiStore.sourcePlayer;
    if (!sp) return;
    try {
      const n = await enqueuePlayers([sp]);
      dockOpen = true; // 已落坞：展开让用户看见
      await message(t.gen.queue.enqueued(n), { title: t.dialog.info });
    } catch (e) {
      await showError(e); // 缺模型/占位符未填充等组装错误与名单表同口径
    }
  }

  /** meta（同参重生）：结果卡「再生成」传入该图记录的 {size, seed} 按原参重出；
   *  CTA 与内联重试不传 meta，用工作台当前值。 */
  async function onGenerate(meta?: { size: string; seed: string }) {
    // Key 框有未提交内容时先串行提交，避免 change 异步与生成竞态（提交失败中止生成）。
    if (keyInput.trim()) await commitKey();
    genError = "";
    try {
      await generate(meta);
    } catch (e) {
      // 生成错误走结果区内联错误条（可就地重试），不再用系统弹窗打断。
      genError = e instanceof Error ? e.message : String(e);
    }
  }

  /** 结果卡真实比例样式（由生成时快照尺寸推导；解析不出回退正方形）。 */
  function ratioStyle(size?: string): string {
    const p = size ? parseSize(size) : null;
    return p ? `aspect-ratio:${p[0]} / ${p[1]}` : "aspect-ratio:1";
  }

  // ---- 结果卡「保存并替换」（P3）：确认后写入 _generated 并走单人换脸链路 ----
  async function onSaveReplace(imgId: string) {
    const sp = imageaiStore.sourcePlayer;
    if (!sp) return; // 无数据源时按钮已禁用，这里只是防御
    if (
      !(await confirm(t.gen.saveReplace.confirm(sp.name), {
        title: t.dialog.info,
      }))
    )
      return;
    try {
      await saveAndReplace(imgId);
      await message(t.gen.saveReplace.ok(sp.name), { title: t.dialog.info });
    } catch (e) {
      await showError(e); // 批量替换进行中被拒等后端错误走统一弹窗
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

<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape") lightboxUrl = null;
  }}
/>

<div class="panel">
  <!-- ① 服务配置条（全宽单行，S6 压平自「服务连接卡」）：进台第一眼即见服务商 / 模型 / Key 就绪状态。
       Key 仍只进后端会话内存（setProviderKey），此处只是挪位置不改持久化语义；
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
        <!-- 默认模型不在明细行里时也要能选中（custom 服务商常无明细行，见 store.modelInfo 兜底） -->
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

  <!-- ② 主工作区：左栏 = 球员源单人档案卡，右栏 = 提示词 / 参数 / 结果（S7 起数据源不再用横幅） -->
  <div class="wb-grid">
    <div class="wb-left">
      <!-- 球员源 · 单人档案卡（S7 升级自数据源横幅）：字段对齐 15 个提示词变量，扫一眼可核对。
           三个来源都能填进来：名单行「送生图」、单人替换「用此球员生成」、台内 UID 查询。 -->
      <section class="pcard src-card">
        <div class="pcard-h">
          <span>{t.gen.cardSource}</span>
          {#if imageaiStore.sourcePlayer}
            <button
              class="btn sm"
              title={t.gen.source.clearTitle}
              onclick={clearSourcePlayer}>{t.gen.source.clear}</button
            >
          {/if}
        </div>
        {#if !imageaiStore.sourcePlayer}
          {#if !appStore.rosterReady}
            <!-- 就绪卡：名单没扫，UID 查询无从命中 → 直达存档资料台 -->
            <p class="hint">{t.gen.source.scanFirst}</p>
            <button class="btn" onclick={() => (ui.workbench = "archive")}>
              {t.gen.source.goArchive}
            </button>
          {:else}
            <p class="hint">{t.gen.source.queryHint}</p>
            <div class="src-query">
              <input
                type="text"
                placeholder={t.gen.source.queryPlaceholder}
                bind:value={srcUidInput}
                onkeydown={(e) => e.key === "Enter" && querySourceUid()}
              />
              <button
                class="btn"
                disabled={!srcUidInput.trim()}
                onclick={querySourceUid}>{t.gen.source.queryBtn}</button
              >
            </div>
            {#if srcQueryError}
              <p class="field-error">{srcQueryError}</p>
            {/if}
          {/if}
        {:else}
          {@const sp = imageaiStore.sourcePlayer}
          <div class="src-head">
            <AvatarThumb
              itemKey={sp.uid}
              url={srcAvatars.urls[sp.uid]}
              path={srcAvatars.paths[sp.uid] ?? undefined}
              groupKey={appStore.snapshot?.currentGroup}
            />
            <div class="src-id">
              <span class="src-name" title={sp.name}>{sp.name}</span>
              <span class="src-uid mono">{sp.uid}</span>
            </div>
          </div>
          {#if srcRec}
            <!-- 信息胶囊单池自动折行（与 PlayerViewer 同语言），顺序对齐名单预览表头列序，空值显示 — 不坍缩 -->
            <div class="meta-row">
              <span class="meta">{t.table.colNat} <b>{srcRec.primaryNat.trim() || "—"}</b></span>
              <span class="meta">{t.table.col2ndNat} <b>{srcRec.secNat.trim() || "—"}</b></span>
              <span class="meta">{t.table.colHair} <b>{srcRec.hairLength.trim() || "—"}</b></span>
              <span class="meta">{t.table.colColor} <b>{srcRec.hairColor.trim() || "—"}</b></span>
              <span class="meta">{t.table.colEth} <b>{srcRec.ethnicityCode.trim() || "—"}</b></span>
              {#if srcRec.detailed}
                <span class="meta">{t.table.colSkin} <b>{srcRec.detailed.skinCode.trim() || "—"}</b></span>
                <span class="meta">{t.table.colGender} <b>{srcRec.detailed.gender.trim() || "—"}</b></span>
                <span class="meta">{t.table.colAge} <b>{srcRec.detailed.age.trim() || "—"}</b></span>
                <span class="meta">{t.table.colHeight} <b>{srcRec.detailed.height.trim() || "—"}</b></span>
                <span class="meta">{t.table.colClub} <b>{srcRec.detailed.club.trim() || "—"}</b></span>
                <span class="meta">{appStore.basedLabel} <b>{basedValue(srcRec.detailed).trim() || "—"}</b></span>
                <span class="meta">{t.table.colCa} <b>{srcRec.detailed.ca.trim() || "—"}</b></span>
                <span class="meta">{t.table.colPa} <b>{srcRec.detailed.pa.trim() || "—"}</b></span>
              {/if}
            </div>
          {:else}
            <!-- 资料降级：uid 不在当前名单（可能来自上一次扫描），仅姓名/UID 可信 -->
            <p class="hint">{t.gen.source.noProfile}</p>
          {/if}
          <button class="btn primary src-enqueue" onclick={enqueueSource}>
            {t.gen.source.enqueue}
          </button>
        {/if}
      </section>

    </div>

    <div class="wb-right">
      <!-- 提示词主区（S8 升为右栏顶部）：模板选择 + 变量 + 正文。有球员源时正文按其名单记录渲染
           （只读预览），无球员源时手填可编辑；两种模式的生成链路差异见 store.generate 注释 -->
      <section class="pcard">
        <div class="pcard-h">
          <span>{t.gen.cardContent}</span>
          <button class="btn sm" onclick={() => (templateOpen = true)}
            >{t.gen.manageTemplate}</button
          >
        </div>
        <div class="tpl-row">
          <label for="ig-template" class="field-label">{t.gen.template}</label>
          <select
            id="ig-template"
            value={imageaiStore.templateId}
            onchange={(e) =>
              void applyTemplate(e.currentTarget.value).catch(showError)}
          >
            {#each imageaiStore.config?.templates ?? [] as tpl}
              <option value={tpl.id}>{tpl.name}</option>
            {/each}
          </select>
        </div>
        {#if imageaiStore.sourcePlayer}
          <!-- 数据源模式：变量按球员数据渲染，胶囊仅作清单展示（不可点追加） -->
          <p class="hint">{t.gen.variablesSource}</p>
          <div class="chips static-chips">
            {#each VARIABLE_HINTS as [, label] (label)}
              <span class="chip chip-static">{label}</span>
            {/each}
          </div>
          <label class="field-label" for="ig-prompt" style="margin-top:8px"
            >{t.gen.promptPreview}</label
          >
          <textarea
            id="ig-prompt"
            rows="7"
            readonly
            value={sourcePreview}
            placeholder={t.gen.promptPreviewing}
          ></textarea>
          <p class="hint src-note">{t.gen.sourceReadonlyNote}</p>
        {:else}
          <p class="hint">{t.gen.variables}</p>
          <div class="chips">
            {#each VARIABLE_HINTS as [key, label] (key)}
              <button class="chip" onclick={() => appendVariable(key)}
                >{label}</button
              >
            {/each}
          </div>
          <div class="prompt-head">
            <label for="ig-prompt" class="field-label">{t.gen.prompt}</label>
            {#if promptLimit > 0}
              <span
                class="counter"
                class:near={promptNear}
                class:over={promptOver}>{promptLen} / {promptLimit}</span
              >
            {/if}
          </div>
          <textarea
            id="ig-prompt"
            rows="7"
            bind:value={imageaiStore.prompt}
            class:invalid={promptOver}
          ></textarea>
          {#if promptOver}
            <p class="field-error">{t.gen.promptTooLong(promptLimit)}</p>
          {/if}
        {/if}
        {#if caps?.supportsPromptExtend}
          <label class="switch opt-row">
            <input type="checkbox" bind:checked={imageaiStore.promptExtend} />
            <span class="track"></span>
            <span class="opt-text">
              {t.gen.promptExtend}
              <small>{t.gen.promptExtendHint}</small>
            </span>
          </label>
        {/if}
        <button class="neg-toggle" onclick={() => (showNeg = !showNeg)}>
          <Icon name={showNeg ? "chevronUp" : "chevronDown"} class="ic" />
          {t.gen.toggleNegative}
          {#if caps && !caps.supportsNegativePrompt}
            <span class="neg-off">{t.gen.negativeNotSent}</span>
          {/if}
        </button>
        {#if showNeg}
          <textarea
            rows="2"
            placeholder={t.gen.negative}
            bind:value={imageaiStore.negative}
          ></textarea>
        {/if}
      </section>

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
          (promptOver ? t.gen.promptTooLong(promptLimit) : "")}
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
          class:on={dockOpen}
          onclick={() => (dockOpen = !dockOpen)}
        >
          <Icon name="play" class="ic" filled={false} />
          {t.gen.queue.tabQueue}
          {#if queueBadge > 0}<span class="queue-badge">{queueBadge}</span>{/if}
        </button>
        <button
          class="btn sm"
          disabled={imageaiStore.results.length === 0}
          title={t.gen.clearResultsTitle}
          onclick={onClearResults}
        >
          {t.gen.clearResults}
        </button>
      </div>

      <!-- 内联错误条：生成失败就地展示 + 重试（不再弹系统窗） -->
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

      {#if imageaiStore.results.length === 0 && !imageaiStore.generating}
        <div class="empty res-empty">
          <Icon name="image" stroke={1.6} />
          <p>{t.gen.empty}</p>
        </div>
      {:else}
        <div class="gallery">
          {#if imageaiStore.generating}
            {#each Array(Math.max(1, imageaiStore.n)) as _, i (i)}
              <div class="img-card skel-card">
                <div
                  class="skel-img"
                  aria-hidden="true"
                  style={ratioStyle(imageaiStore.size)}
                ></div>
                <div class="skel-meta">
                  <span class="skel-line"></span>
                  {#if i === 0}<span class="skel-hint"
                      >{t.gen.skeletonHint}</span
                    >{/if}
                </div>
              </div>
            {/each}
          {/if}
          {#each imageaiStore.results as img (img.id)}
            <div class="img-card">
              <button
                type="button"
                class="img-frame"
                style={ratioStyle(img.size)}
                title={t.gen.viewLarge}
                onclick={() => (lightboxUrl = img.url)}
              >
                <img src={img.url} alt={t.gen.resultAlt} loading="lazy" decoding="async" />
              </button>
              <div class="img-meta mono">
                <span>{img.size ? img.size.replace("*", "×") : ""}</span>
                {#if img.seed}<span>{t.gen.metaSeed(img.seed)}</span>{/if}
              </div>
              <div class="actions">
                <button
                  class="btn sm"
                  onclick={() => saveGeneratedImageAs(img.id)}
                  title={t.gen.save}
                >
                  <Icon name="download" class="ic" />
                  {t.gen.save}
                </button>
                <!-- 保存并替换：需数据源球员，无数据源禁用并提示入口 -->
                <button
                  class="btn sm"
                  disabled={!imageaiStore.sourcePlayer}
                  title={imageaiStore.sourcePlayer
                    ? t.gen.saveReplace.btn
                    : t.gen.saveReplace.needPlayer}
                  onclick={() => onSaveReplace(img.id)}
                >
                  {t.gen.saveReplace.btn}
                </button>
                <button
                  class="btn sm"
                  title={t.gen.regenTitle}
                  onclick={() => onGenerate({ size: img.size ?? '', seed: img.seed ?? '' })}
                >
                  <Icon name="refresh" class="ic" />
                </button>
                <button
                  class="btn sm icon-as-btn"
                  title={t.gen.viewLarge}
                  onclick={() => (lightboxUrl = img.url)}
                >
                  <Icon name="zoom" class="ic" />
                </button>
                <button
                  class="icon-btn"
                  title={t.gen.removeImageTitle}
                  onclick={() => removeResult(img.id)}
                >
                  <Icon name="x" size={13} />
                </button>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>

  <!-- ④ 队列坞（S9 移至台底全宽）：有条目时自动浮出状态条（空队列且未手动展开时不占空间），
       不再随右栏滚动，长任务进度常驻可见；切走工作台由顶栏 ③chip 角标接力 -->
  <QueueDock bind:open={dockOpen} />

  <!-- 结果大图灯箱：点遮罩空白处/关闭钮/Esc 退出（遮罩写法对齐项目内其他模态） -->
  {#if lightboxUrl}
    <div
      class="lightbox"
      role="presentation"
      onclick={(e) => {
        if (e.target === e.currentTarget) lightboxUrl = null;
      }}
    >
      <div
        class="lb-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={t.gen.viewLarge}
      >
        <img src={lightboxUrl} alt={t.gen.previewAlt} />
        <button
          class="icon-btn lb-close"
          title={t.gen.closePreview}
          onclick={() => (lightboxUrl = null)}
        >
          <Icon name="x" size={16} />
        </button>
      </div>
    </div>
  {/if}
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

  /* ---------- 服务配置条（S6 压平：全宽单行、控件内联标签） ---------- */
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
  .field-label {
    display: block;
    font-size: 12px;
    color: var(--text-2);
    margin-bottom: 4px;
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

  /* ---------- 球员源 · 单人档案卡（S7 升级自数据源横幅） ---------- */
  .src-head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }
  .src-id {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .src-id .src-name {
    font-weight: 600;
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .src-id .src-uid {
    color: var(--text-2);
    font-size: 12px;
  }
  /* 台内 UID 查询：输入框吃余宽，回车与按钮等价 */
  .src-query {
    display: flex;
    gap: 8px;
  }
  .src-query input {
    flex: 1;
    min-width: 0;
  }
  .src-card .meta-row {
    margin-bottom: 9px;
  }
  .src-enqueue {
    width: 100%;
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

  /* 参数卡：描边卡 + 标题行 */
  .pcard {
    border: 1px solid var(--hairline);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    padding: 9px 12px 10px;
    box-shadow: var(--shadow-1);
  }
  .pcard-h {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }
  .pcard-h > span {
    display: inline-flex;
    align-items: center;
    font-size: 13px;
    font-weight: 600;
  }
  .pcard-h > span::before {
    content: "";
    width: 3px;
    height: 14px;
    border-radius: 2px;
    background: var(--accent);
    margin-right: 8px;
  }
  .tpl-row {
    display: flex;
    flex-direction: column;
  }
  /* .hint 已提升为全局工具类（app.css），此处不再重复定义 */
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  textarea {
    width: 100%;
    resize: vertical;
  }
  .neg-toggle {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin: 7px 0 5px;
    background: none;
    border: none;
    color: var(--text-3);
    font-size: 12px;
    cursor: pointer;
    padding: 0;
  }
  .neg-toggle:hover {
    color: var(--accent);
  }
  /* 当前模型不发送负向提示词时的灰徽标 */
  .neg-off {
    margin-left: 4px;
    border-radius: 99px;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-3);
    background: var(--bg-raised);
    border: 1px solid var(--hairline-2);
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
  /* 自定义尺寸行：宽/高两个小数字输入，中间 × 分隔 */
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
  /* 开关行（智能改写/水印）：文字主副两行 */
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
  /* 提示词计数 */
  .prompt-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-top: 8px;
  }
  .prompt-head .field-label {
    margin-bottom: 0;
  }
  .counter {
    font-size: 11.5px;
    font-family: var(--mono);
    color: var(--text-3);
  }
  .counter.near {
    color: var(--warn);
  }
  .counter.over {
    color: var(--danger);
  }
  textarea.invalid,
  input.invalid {
    border-color: var(--danger);
  }
  textarea.invalid:focus,
  input.invalid:focus {
    box-shadow: 0 0 0 3px var(--danger-soft);
  }
  .field-error {
    margin: 5px 0 2px;
    font-size: 12px;
    color: var(--danger);
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
  .res-bar .btn.on {
    border-color: var(--accent-line);
    color: var(--accent);
    background: var(--accent-soft);
  }
  .queue-badge {
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 99px;
    background: var(--accent);
    color: var(--accent-contrast);
    font-size: 10.5px;
    font-weight: 600;
    display: inline-grid;
    place-items: center;
  }
  .res-empty {
    flex: 1;
  }
  .gallery {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 10px;
    align-content: start;
    overflow-y: auto;
  }
  .img-card {
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--bg-surface);
    display: flex;
    flex-direction: column;
  }
  /* 真实比例框：图片 contain 不裁剪；inset 底避免透明 PNG 看不清边界 */
  .img-frame {
    display: block;
    width: 100%;
    padding: 8px;
    border: none;
    background: var(--bg-inset);
    cursor: zoom-in;
  }
  .img-frame img {
    width: 100%;
    height: auto;
    object-fit: contain;
    display: block;
    border-radius: var(--radius-sm);
  }
  .img-meta {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
  }
  .actions {
    display: flex;
    gap: 6px;
    padding: 6px 8px 8px;
    align-items: center;
  }
  .actions .icon-btn {
    margin-left: auto;
  }
  .icon-as-btn {
    width: 30px;
    padding-left: 0;
    padding-right: 0;
  }
  :global(.ic) {
    width: 13px;
    height: 13px;
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

  /* 骨架卡 */
  .skel-card {
    border-style: dashed;
  }
  .skel-img {
    box-sizing: border-box;
    width: calc(100% - 16px);
    min-height: 90px;
    border-radius: var(--radius-sm);
    margin: 8px;
    background: linear-gradient(
      100deg,
      var(--bg-inset) 40%,
      var(--bg-raised) 50%,
      var(--bg-inset) 60%
    );
    background-size: 200% 100%;
    animation: skel-shimmer 1.3s linear infinite;
  }
  .skel-meta {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 0 8px 8px;
  }
  .skel-line {
    height: 10px;
    border-radius: 5px;
    background: var(--bg-raised);
    width: 55%;
  }
  .skel-hint {
    font-size: 11px;
    color: var(--text-3);
  }
  @keyframes skel-shimmer {
    from {
      background-position: 200% 0;
    }
    to {
      background-position: -200% 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .skel-img {
      animation: none;
    }
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

  /* 灯箱 */
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: var(--z-modal);
    background: rgba(0, 0, 0, 0.72);
    display: grid;
    place-items: center;
    padding: 5vh 8vw;
    animation: lb-fade var(--dur-fast) var(--ease-out);
  }
  .lb-dialog {
    position: relative;
    display: grid;
    place-items: center;
    max-width: 100%;
    max-height: 100%;
  }
  .lb-dialog img {
    max-width: 84vw;
    max-height: 88vh;
    object-fit: contain;
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-pop);
  }
  .lb-close {
    position: absolute;
    top: -12px;
    right: -12px;
    width: 32px;
    height: 32px;
    border-color: rgba(255, 255, 255, 0.4);
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
  }
  .lb-close:hover:not(:disabled) {
    background: rgba(0, 0, 0, 0.8);
    color: #fff;
    border-color: #fff;
  }
  @keyframes lb-fade {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .lightbox {
      animation: none;
    }
  }

  /* 数据源只读模式小样式 */
  textarea[readonly] {
    color: var(--text-2);
    cursor: default;
  }
  .static-chips .chip-static {
    background: var(--bg-raised);
    border-color: var(--hairline-2);
    color: var(--text-3);
    cursor: default;
  }
  .src-note {
    margin-bottom: 0;
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
    .gallery {
      min-height: 280px;
    }
  }
</style>
