<!-- ProviderDialog · 服务商设置弹窗：列表/新建/编辑/删除。
     服务商配置与它自己的 API Key 都在这里编辑（一家一把，密文随该行落盘）；
     工作台只展示 Key 状态，不再提供输入口。复用全局 modal 类 -->
<script lang="ts">
  import { confirm, message } from "@tauri-apps/plugin-dialog";
  import { t } from "../../text";
  import { tip } from "../../stores/app.svelte";
  import { deleteProvider, imageaiStore, saveProvider, setProviderKey } from "../../stores/imageai.svelte";
  import { showError } from "../../ui/dialog";
  import Icon from "../../ui/Icon.svelte";
  import type { Capabilities, ModelInfo, ProviderConfig, ProviderKindCode, SizeRules } from "../../types";

  /** 展开编辑时的能力集视图：normalizeCaps 已补齐全部字段，故按必填访问，便于直接 bind */
  type FullCaps = Required<Capabilities> & { sizeRules: Required<SizeRules> };

  let { open = $bindable(false) }: { open?: boolean } = $props();
  let editing = $state<ProviderConfig | null>(null);
  /** 展开编辑能力的模型下标（-1 = 全收起）；一次只展开一个，清单可能有十几个模型 */
  let expandedIdx = $state(-1);
  /** Key 输入（不回显已存值）：非空＝保存时写入/替换。 */
  let keyInput = $state("");
  /** 「清除 Key」的待提交标记：与弹窗其他编辑一致，点保存才真正写后端。 */
  let keyCleared = $state(false);
  /** 当前编辑行是否已有 Key：新建行（id 尚未生成）恒为 false。 */
  const keySet = $derived(!!editing?.id && !!imageaiStore.keysSet[editing.id]);

  function startEdit(p: ProviderConfig) {
    keyInput = "";
    keyCleared = false;
    expandedIdx = -1;
    editing = $state.snapshot(p);
  }

  function startNew() {
    keyInput = "";
    keyCleared = false;
    expandedIdx = -1;
    editing = blank();
  }

  function blank(): ProviderConfig {
    return {
      id: "",
      kind: "openai", // 默认走 OpenAI 协议（兼容服务改 baseUrl 即可）；另有 volcano / dashscope 可选
      name: "",
      baseUrl: "",
      defaultModel: "",
      models: [],
    };
  }

  /** 逗号/换行分隔文本 ↔ 字符串数组（尺寸、质量档共用） */
  function parseList(s: string): string[] {
    return s
      .split(/[,，\n]/)
      .map((x) => x.trim())
      .filter(Boolean);
  }

  /** 能力编辑字段的键类型：模板不写死字段名，全部由 CAP_ROWS 驱动。 */
  type RuleKey = "minSide" | "maxSide" | "totalMin" | "totalMax" | "ratioMin" | "ratioMax";
  type NumCapKey = "maxN" | "maxPromptChars" | "stepsDefault" | "stepsMax" | "guidanceDefault" | "guidanceMax";
  type BoolCapKey =
    | "supportsN"
    | "supportsSeed"
    | "supportsSteps"
    | "supportsGuidance"
    | "supportsQuality"
    | "supportsNegativePrompt"
    | "supportsPromptExtend"
    | "supportsPromptExtendMode"
    | "supportsEnableThinking"
    | "supportsWatermark"
    | "supportsResponseFormat"
    | "supportsOutputFormat"
    | "supportsAsync";
  type EnumCapKey = "promptExtendModeDefault";

  /** 附属项：挂在协议参数行下面，不占独立参数位（上限 / 推荐档 / 约束 / 默认档）。
   *  `prompt_extend` / `enable_thinking` / `watermark` 的官方默认值不在编辑区暴露：
   *  它们是模型出厂属性，由内置目录（或 JSON）维护，前端切模型时按此归位。 */
  type CapSub =
    | { t: "num"; label: string; bind: NumCapKey; hint?: string; step?: string }
    | { t: "list"; label: string; bind: "sizes" | "qualities"; placeholder: string; rows?: number }
    | { t: "pair"; label: string; lo: RuleKey; hi: RuleKey; hint: string }
    | { t: "enum"; label: string; bind: EnumCapKey; options: readonly { v: string; label: string }[]; hint?: string };

  /** 协议参数行：`api` 是官方字段原名（挂在 title 上，便于对着文档核验）；
   *  有 `bind` = 模型可选参数（支持位），无 = 该协议恒有（如正向提示词）。
   *  `bindStyle` 决定支持位的控件形态：switch（默认开关）｜mode（两态，false/true 各一个文案）。 */
  type CapRow = {
    key: string;
    label: string;
    api: string;
    hint?: string;
    bind?: BoolCapKey;
    bindStyle?: "switch" | "mode";
    bindLabels?: readonly [string, string];
    subs: CapSub[];
  };

  /** 原生 DashScope 多模态生成协议（文生图/图生图）的参数并集 + 另两套协议专有行。
   *  勾选只表达「这个模型收不收这个参数」，值域/约束是这条协议的固有属性。 */
  const CAP_ROWS: CapRow[] = [
    {
      key: "prompt",
      label: t.gen.capProtoPrompt,
      api: t.gen.capApi.prompt,
      subs: [{ t: "num", label: t.gen.capFieldMaxChars, bind: "maxPromptChars", hint: t.gen.capFieldMaxCharsHint }],
    },
    {
      key: "size",
      label: t.gen.capProtoSize,
      api: t.gen.capApi.size,
      subs: [
        { t: "list", label: t.gen.capFieldSizes, bind: "sizes", placeholder: t.gen.capFieldSizesPlaceholder, rows: 2 },
        { t: "pair", label: t.gen.capRuleSide, lo: "minSide", hi: "maxSide", hint: t.gen.capZeroHint },
        { t: "pair", label: t.gen.capRuleTotal, lo: "totalMin", hi: "totalMax", hint: t.gen.capZeroHint },
        { t: "pair", label: t.gen.capRuleRatio, lo: "ratioMin", hi: "ratioMax", hint: t.gen.capZeroHint },
      ],
    },
    {
      key: "n",
      label: t.gen.capProtoN,
      api: t.gen.capApi.n,
      bind: "supportsN",
      subs: [{ t: "num", label: t.gen.capFieldMaxN, bind: "maxN", hint: t.gen.capFieldMaxNHint }],
    },
    {
      key: "negative",
      label: t.gen.capProtoNegative,
      api: t.gen.capApi.negative,
      bind: "supportsNegativePrompt",
      subs: [],
    },
    { key: "seed", label: t.gen.capProtoSeed, api: t.gen.capApi.seed, bind: "supportsSeed", subs: [] },
    {
      key: "steps",
      label: t.gen.capProtoSteps,
      api: t.gen.capApi.steps,
      bind: "supportsSteps",
      subs: [
        { t: "num", label: t.gen.capFieldStepsDefault, bind: "stepsDefault" },
        { t: "num", label: t.gen.capFieldStepsMax, bind: "stepsMax" },
      ],
    },
    {
      key: "guidance",
      label: t.gen.capProtoGuidance,
      api: t.gen.capApi.guidance,
      bind: "supportsGuidance",
      subs: [
        { t: "num", label: t.gen.capFieldGuidanceDefault, bind: "guidanceDefault", step: "0.5" },
        { t: "num", label: t.gen.capFieldGuidanceMax, bind: "guidanceMax", step: "0.5" },
      ],
    },
    {
      key: "extend",
      label: t.gen.capProtoExtend,
      api: t.gen.capApi.extend,
      bind: "supportsPromptExtend",
      subs: [],
    },
    {
      key: "extendMode",
      label: t.gen.capProtoExtendMode,
      api: t.gen.capApi.extendMode,
      bind: "supportsPromptExtendMode",
      subs: [
        {
          t: "enum",
          label: t.gen.capFieldModeDefault,
          bind: "promptExtendModeDefault",
          options: [
            { v: "direct", label: t.gen.extendModeDirect },
            { v: "agent", label: t.gen.extendModeAgent },
          ],
          hint: t.gen.extendModeAgentHint,
        },
      ],
    },
    {
      key: "thinking",
      label: t.gen.capProtoThinking,
      api: t.gen.capApi.thinking,
      bind: "supportsEnableThinking",
      hint: t.gen.thinkingHint,
      subs: [],
    },
    {
      key: "watermark",
      label: t.gen.capProtoWatermark,
      api: t.gen.capApi.watermark,
      bind: "supportsWatermark",
      subs: [],
    },
    {
      key: "callMode",
      label: t.gen.capProtoCallMode,
      api: t.gen.capApi.callMode,
      bind: "supportsAsync",
      bindStyle: "mode",
      bindLabels: [t.gen.callModeSync, t.gen.callModeAsync],
      subs: [],
    },
    // ---- 非 DashScope 协议的专有行：由 KIND_ROWS 决定是否出现在编辑区 ----
    {
      key: "quality",
      label: t.gen.capProtoQuality,
      api: t.gen.capApi.quality,
      bind: "supportsQuality",
      subs: [
        {
          t: "list",
          label: t.gen.capFieldQualities,
          bind: "qualities",
          placeholder: t.gen.capFieldQualitiesPlaceholder,
        },
      ],
    },
    {
      key: "responseFormat",
      label: t.gen.capProtoResponseFormat,
      api: t.gen.capApi.responseFormat,
      bind: "supportsResponseFormat",
      subs: [],
    },
    // 火山 Seedream 专有：output_format（Pro/Flash/Lite 可设 png/jpeg；4.5/4.0 只回 jpeg、不发此参）
    {
      key: "outputFormat",
      label: t.gen.capProtoOutputFormat,
      api: t.gen.capApi.outputFormat,
      bind: "supportsOutputFormat",
      subs: [],
    },
  ];

  /** 行集合由「服务商协议」决定（适配器 build_* 实际发送的字段为准），模型只决定行上的勾选：
   *  dashscope = 原生多模态生成并集（含异步）；openai = OpenAI 官方参数 + 百炼兼容模式平铺在
   *  顶层的扩展字段（负向/seed/改写/改写模式/思考/水印），兼容模式**只有同步**故无「调用方式」行；
   *  volcano = Seedream 文生图官方字段（无 n/seed/负向/改写，有水印/返回格式/输出格式）。 */
  const KIND_ROWS: Record<ProviderKindCode, readonly string[]> = {
    dashscope: ["prompt", "size", "n", "negative", "seed", "extend", "extendMode", "thinking", "watermark", "callMode"],
    openai: [
      "prompt",
      "size",
      "n",
      "negative",
      "seed",
      "extend",
      "extendMode",
      "thinking",
      "watermark",
      "quality",
      "responseFormat",
    ],
    volcano: ["prompt", "size", "watermark", "responseFormat", "outputFormat"],
    // 硅基流动：字段名走 image_size / batch_size（见 t.gen.capApiByKind）；Qwen-Image 不支持 batch_size。
    siliconflow: ["prompt", "size", "n", "negative", "seed", "steps", "guidance", "outputFormat"],
    // 魔搭：单张无 n；size 字段名与 OpenAI 同名；有 steps / guidance。
    modelscope: ["prompt", "size", "negative", "seed", "steps", "guidance"],
    // 本地 ComfyUI：原生节点工作流（无鉴权）；张数走 batch_size、引导是 KSampler 的 cfg，
    // 采样器固定 euler/normal（不在能力面暴露）。
    localcomfyui: ["prompt", "size", "n", "negative", "seed", "steps", "guidance"],
  };

  /** 能力集默认值：新建模型行的起点，也是展开编辑前补齐缺失字段的兜底 */
  function newCaps() {
    return {
      sizes: [] as string[],
      maxN: 1,
      supportsQuality: false,
      supportsSeed: false,
      supportsSteps: false,
      stepsMax: 0,
      stepsDefault: 0,
      supportsGuidance: false,
      guidanceDefault: 0,
      guidanceMax: 0,
      supportsNegativePrompt: false,
      supportsResponseFormat: true,
      supportsOutputFormat: false,
      qualities: [] as string[],
      supportsPromptExtend: false,
      supportsWatermark: false,
      maxPromptChars: 0,
      sizeRules: { minSide: 0, maxSide: 0, totalMin: 0, totalMax: 0, ratioMin: 0, ratioMax: 0 },
      supportsN: true,
      promptExtendDefault: false,
      watermarkDefault: false,
      supportsPromptExtendMode: false,
      promptExtendModeDefault: "direct",
      supportsEnableThinking: false,
      enableThinkingDefault: true,
      supportsAsync: false,
    };
  }

  /** 当前编辑协议下的行集合（kind 变了行集合就变，模型只影响勾选）。
   *  官方字段名小字随协议换：DashScope 把参数嵌在 `parameters` 下、提示词在 `input.messages[]` 里，
   *  OpenAI 兼容与火山方舟一律平铺在请求体顶层（字段同名、位置不同）。 */
  const capRows = $derived.by(() => {
    const kind = editing?.kind ?? "openai";
    const flat = kind !== "dashscope";
    // 按服务商覆盖的官方字段名优先（硅基流动的 image_size/batch_size/num_inference_steps/guidance_scale
    // 与 OpenAI 不同）；未命中回落 flat（平铺）或 row.api（DashScope 嵌套）。
    const byKind = t.gen.capApiByKind[kind];
    return (KIND_ROWS[kind] ?? []).map((k) => {
      const row = CAP_ROWS.find((r) => r.key === k)!;
      const api = byKind?.[row.key] ?? (flat ? (t.gen.capApiFlat[row.key] ?? row.api) : row.api);
      return api === row.api ? row : { ...row, api };
    });
  });

  /** 展开前补齐能力集：编辑表单要绑定每个字段（含 sizeRules 的六个子项） */
  function normalizeCaps(m: ModelInfo) {
    const base = newCaps();
    m.capabilities = {
      ...base,
      ...m.capabilities,
      sizeRules: m.capabilities.sizeRules ?? base.sizeRules,
    };
  }

  function toggleCaps(i: number) {
    if (expandedIdx === i) {
      expandedIdx = -1;
      return;
    }
    const m = editing?.models[i];
    if (m) normalizeCaps(m);
    expandedIdx = i;
  }

  function addModel() {
    editing?.models.push({ model: "", capabilities: newCaps() });
    expandedIdx = editing ? editing.models.length - 1 : -1;
  }

  function removeModel(i: number) {
    if (!editing) return;
    const gone = editing.models[i];
    editing.models.splice(i, 1);
    // 删掉的正是默认模型 → 顺移到剩余首个，避免默认模型悬空指向已删除的行
    if (gone && editing.defaultModel === gone.model) {
      editing.defaultModel = editing.models[0]?.model ?? "";
    }
    // 展开下标跟随列表位移
    expandedIdx = expandedIdx === i ? -1 : expandedIdx > i ? expandedIdx - 1 : expandedIdx;
  }

  async function onSave() {
    if (!editing) return;
    try {
      // 丢掉模型名留空的行（点了「添加模型」但没填的），避免写入空模型名
      editing.models = editing.models.filter((m) => m.model.trim());
      if (!editing.id) editing.id = `p-${crypto.randomUUID()}`;
      if (!editing.defaultModel && editing.models[0]) editing.defaultModel = editing.models[0].model;
      // 配置先落盘（新建行此刻才拿到 id），再写/清 Key —— set_provider_key 按 id 找行，顺序不能反。
      const key = keyInput.trim();
      const needsKey = editing.kind !== "localcomfyui";
      await saveProvider($state.snapshot(editing)); // config 整包写回 + 悬空回退见 store action
      if (needsKey && key) await setProviderKey(editing.id, key);
      else if (needsKey && keyCleared) await setProviderKey(editing.id, "");
      keyInput = "";
      keyCleared = false;
      await message(t.gen.providerSaved, { title: t.dialog.info });
      editing = null;
    } catch (e) {
      await showError(e); // 保存失败必须弹窗，不能静默
    }
  }

  /** 只翻转待清除标记：真正的清空在保存时执行，点「返回」即随其他编辑一起丢弃。 */
  function toggleKeyClear() {
    keyCleared = !keyCleared;
    if (keyCleared) keyInput = "";
  }

  async function onDelete(p: ProviderConfig) {
    if (
      !(await confirm(t.gen.deleteProviderConfirm(p.name), {
        title: t.dialog.warning,
      }))
    )
      return;
    try {
      await deleteProvider(p.id);
    } catch (e) {
      await showError(e);
    }
  }

  /** Esc 关闭弹窗（编辑态的临时改动随弹窗丢弃，与点遮罩/「完成」同口径） */
  function onWindowKey(e: KeyboardEvent) {
    if (e.key === "Escape" && open) open = false;
  }
</script>

<svelte:window onkeydown={onWindowKey} />

{#if open}
  <div
    class="modal-overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) open = false;
    }}
  >
    <div class="modal" role="dialog" aria-modal="true">
      <div class="modal-title">{t.gen.providerSettings}</div>
      <div class="modal-body">
        {#if !editing}
          <div class="p-list">
            {#each imageaiStore.config?.providers ?? [] as p (p.id)}
              <div class="p-row">
                <span>{p.name} <em>{p.kind}</em></span>
                <span class="btns">
                  <button class="btn sm" onclick={() => startEdit(p)}>{t.gen.edit}</button>
                  <!-- 判据用 id 不用 kind：能否删讲「是不是内置」，与 kind 讲的「用什么协议」是两个维度。
                       必须与后端 remove_provider 同源（后端按 id 判），否则改 kind 会让按钮凭空消失 -->
                  {#if p.id !== "openai"}
                    <button class="btn sm danger" onclick={() => onDelete(p)}>{t.gen.remove}</button>
                  {/if}
                </span>
              </div>
            {/each}
          </div>
        {:else}
          <div class="form">
            <!-- ID 是 upsert_provider 的匹配键，编辑态不可改（改 ID 等于新增一条，旧条目与其 Key 留在旧 id 上）：
                 故编辑态给纯值展示，新建态才给输入框（留空则保存时自动生成 p-<uuid>）。
                 不可改的原因就地跟在 ID 后面，不另占一行 note -->
            <div class="f-row">
              {#if editing.id}
                <span class="f-label">{t.gen.providerIdLabel}</span>
                <span class="f-ro">{editing.id}</span>
              {:else}
                <label for="pv-id">{t.gen.providerIdLabel}</label>
                <input id="pv-id" type="text" bind:value={editing.id} placeholder={t.gen.providerIdPlaceholder} />
              {/if}
              <span class="f-hint">{t.gen.providerIdFixedNote}</span>
            </div>
            <div class="f-row">
              <label for="pv-kind">{t.gen.kind}</label>
              <!-- 联合字面量类型：不用 bind:value（svelte-check 会报类型不符），改显式赋值 -->
              <select
                id="pv-kind"
                value={editing.kind}
                onchange={(e) => (editing!.kind = e.currentTarget.value as ProviderConfig["kind"])}
              >
                <!-- 无 custom 档：OpenAI 兼容服务与官方是同一套 SDK，选 openai 后改 baseUrl 即可 -->
                <option value="openai">openai</option>
                <option value="volcano">volcano</option>
                <option value="dashscope">dashscope</option>
                <option value="siliconflow">siliconflow</option>
                <option value="modelscope">modelscope</option>
                <option value="localcomfyui">localcomfyui</option>
              </select>
            </div>
            <div class="f-row">
              <label for="pv-name">{t.gen.providerNameLabel}</label><input
                id="pv-name"
                type="text"
                bind:value={editing.name}
              />
            </div>
            <div class="f-row">
              <label for="pv-url">{t.gen.baseUrl}</label><input
                id="pv-url"
                type="text"
                bind:value={editing.baseUrl}
                placeholder="https://example.com/v1"
                use:tip={editing.kind === "dashscope" ? t.gen.baseUrlHintDashscope : ""}
              />
            </div>
            <!-- Key 跟着服务商走：一家一把，明文不回显。本地 ComfyUI 免鉴权，不显示这一行 -->
            {#if editing.kind !== "localcomfyui"}
              <div class="f-row">
                <label for="pv-key">{t.gen.keyBarLabel}</label>
                <input
                  id="pv-key"
                  type="password"
                  autocomplete="off"
                  bind:value={keyInput}
                  placeholder={keyCleared
                    ? t.gen.keyWillClear
                    : keySet
                      ? t.gen.keyBarSetPlaceholder
                      : t.gen.keyBarPlaceholder}
                  oninput={() => (keyCleared = false)}
                />
                {#if keySet}
                  <button class="btn sm" onclick={toggleKeyClear}>
                    {keyCleared ? t.gen.keyClearUndo : t.gen.keyClear}
                  </button>
                {:else}
                  <span class="f-hint">{t.gen.keyStatusUnset}</span>
                {/if}
              </div>
              <!-- Key 的存储口径：密文落盘、绑定当前系统账户，换机/重装需重填。 -->
              <p class="key-note">{t.gen.keyStorageHint}</p>
            {/if}
            <!-- 默认模型：有清单时从清单里选（避免填出清单外不存在的模型名）；
                 清单为空才退化为自由输入 —— 那是「只填一个模型名、能力走后端兜底」的轻量路径 -->
            <div class="f-row">
              <label for="pv-model">{t.gen.providerModelLabel}</label>
              {#if editing.models.length > 0}
                <select
                  id="pv-model"
                  value={editing.defaultModel}
                  onchange={(e) => (editing!.defaultModel = e.currentTarget.value)}
                >
                  <!-- 现值不在清单里时补一个选项保住它（旧配置/手工改过 JSON 的情形） -->
                  {#if editing!.defaultModel && !editing!.models.some((m) => m.model === editing!.defaultModel)}
                    <option value={editing!.defaultModel}>{editing!.defaultModel}</option>
                  {/if}
                  {#each editing!.models.filter((m) => m.model.trim()) as m (m.model)}
                    <option value={m.model}>{m.model}</option>
                  {/each}
                </select>
              {:else}
                <input id="pv-model" type="text" bind:value={editing.defaultModel} />
              {/if}
            </div>

            <!-- 模型清单：一套协议可挂多个模型。每行可展开编辑该模型的能力参数
                 （同一时刻只展开一个，避免长清单把弹窗撑得过高） -->
            <div class="f-row f-row-top">
              <span class="f-label">{t.gen.modelListLabel}</span>
              <div class="m-list">
                {#each editing.models as m, i (m)}
                  {@const caps = m.capabilities as FullCaps}
                  <div class="m-item">
                    <div class="m-head">
                      <button
                        type="button"
                        class="m-toggle"
                        aria-expanded={expandedIdx === i}
                        use:tip={t.gen.modelCapToggle}
                        onclick={() => toggleCaps(i)}
                      >
                        <Icon name={expandedIdx === i ? "chevronDown" : "chevronRight"} size={14} />
                      </button>
                      <input
                        type="text"
                        bind:value={m.model}
                        placeholder={t.gen.modelNamePlaceholder}
                        aria-label={t.gen.modelNamePlaceholder}
                      />
                      <button
                        class="icon-btn m-del"
                        use:tip={t.gen.removeModelTitle}
                        aria-label={t.gen.removeModelTitle}
                        onclick={() => removeModel(i)}
                      >
                        <Icon name="x" size={13} />
                      </button>
                    </div>

                    {#if expandedIdx === i}
                      <div class="m-cap">
                        <!-- 平铺参数表：行集合由协议（KIND_ROWS）决定，模型只决定行上的勾选。
                             一条参数 = 支持位（如有）+ 若干附属项；模板按 sub.t 分发，不写死字段名 -->
                        {#each capRows as row (row.key)}
                          {@const rowTitle = row.hint ? `${row.api} · ${row.hint}` : row.api}
                          <div class="cap-row">
                            <!-- 名称列只留中文（英文官方字段名走悬停），免得中文名被挤成两行 -->
                            <div class="cap-name" use:tip={rowTitle}>{row.label}</div>
                            <div class="cap-val">
                              {#if row.bind}
                                {#if row.bindStyle === "mode"}
                                  <!-- 两态控件：表达「选一种调用方式（同步/异步）」比开关的「开/关」更准确 -->
                                  <div class="seg3 cap-mode" role="group" aria-label={row.label} use:tip={rowTitle}>
                                    <button
                                      type="button"
                                      class:active={!caps[row.bind]}
                                      aria-pressed={!caps[row.bind]}
                                      onclick={() => (caps[row.bind!] = false)}
                                    >
                                      {row.bindLabels?.[0] ?? ""}
                                    </button>
                                    <button
                                      type="button"
                                      class:active={caps[row.bind]}
                                      aria-pressed={caps[row.bind]}
                                      onclick={() => (caps[row.bind!] = true)}
                                    >
                                      {row.bindLabels?.[1] ?? ""}
                                    </button>
                                  </div>
                                {:else}
                                  <label class="switch cap-sw" use:tip={rowTitle}>
                                    <input type="checkbox" bind:checked={caps[row.bind!]} />
                                    <span class="track"></span>
                                  </label>
                                {/if}
                              {/if}
                              {#each row.subs as s (s.label)}
                                {#if s.t === "num"}
                                  <span class="cap-sub">
                                    <span class="cap-k" use:tip={s.hint}>{s.label}</span>
                                    <input type="number" min="0" step={s.step ?? "1"} bind:value={caps[s.bind]} />
                                  </span>
                                {:else if s.t === "list"}
                                  <span class="cap-sub">
                                    <span class="cap-k">{s.label}</span>
                                    {#if s.rows}
                                      <textarea
                                        rows={s.rows}
                                        placeholder={s.placeholder}
                                        value={caps[s.bind].join(", ")}
                                        oninput={(e) => (caps[s.bind] = parseList(e.currentTarget.value))}
                                      ></textarea>
                                    {:else}
                                      <input
                                        type="text"
                                        placeholder={s.placeholder}
                                        value={caps[s.bind].join(", ")}
                                        oninput={(e) => (caps[s.bind] = parseList(e.currentTarget.value))}
                                      />
                                    {/if}
                                  </span>
                                {:else if s.t === "pair"}
                                  <span class="cap-sub">
                                    <span class="cap-k" use:tip={s.hint}>{s.label}</span>
                                    <span class="cap-pair">
                                      <span class="cap-op">≥</span>
                                      <input
                                        type="number"
                                        min="0"
                                        step="1"
                                        aria-label="{s.label} ≥"
                                        bind:value={caps.sizeRules[s.lo]}
                                      />
                                      <span class="cap-op">≤</span>
                                      <input
                                        type="number"
                                        min="0"
                                        step="1"
                                        aria-label="{s.label} ≤"
                                        bind:value={caps.sizeRules[s.hi]}
                                      />
                                    </span>
                                  </span>
                                {:else if s.t === "enum"}
                                  <span class="cap-sub">
                                    <span class="cap-k">{s.label}</span>
                                    <select
                                      use:tip={s.hint}
                                      value={caps[s.bind]}
                                      onchange={(e) => (caps[s.bind] = e.currentTarget.value)}
                                    >
                                      {#each s.options as o (o.v)}
                                        <option value={o.v}>{o.label}</option>
                                      {/each}
                                    </select>
                                  </span>
                                {/if}
                              {/each}
                            </div>
                          </div>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/each}
                <button class="btn sm m-add" onclick={addModel}>{t.gen.addModel}</button>
              </div>
            </div>
          </div>
        {/if}
      </div>
      <div class="modal-foot">
        {#if editing}
          <button class="btn" onclick={() => (editing = null)}>{t.gen.back}</button>
          <button class="btn primary" onclick={onSave}>{t.gen.save}</button>
        {:else}
          <button class="btn" onclick={() => startNew()}>{t.gen.addProvider}</button>
          <button class="btn primary" onclick={() => (open = false)}>{t.gen.done}</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* 本弹窗内容随模型数增长（每个模型还能展开能力表），必须让 body 滚动，
     否则超出 .modal 固定高度的部分直接被裁掉 */
  .modal-body {
    overflow-y: auto;
    padding-right: var(--sp-2);
  }
  /* 只写布局：紧凑按钮尺寸是控件规格，统一由 app.css 的 button.btn.sm 给 */
  .p-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .p-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: var(--fs-md);
  }
  .p-row em {
    color: var(--text-3);
    font-style: normal;
    font-size: var(--fs-sm);
    margin-left: var(--sp-2);
  }
  .btns {
    display: flex;
    gap: var(--sp-2);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .f-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }
  /* 行标签：只认行内的**直接子** label —— 用后代选择器会把模型清单里的
     开关 label（`.switch`）一并改成 150px 宽，滑钮与它的附属项之间被撑出一段空白。
     只读行没有可关联的控件、不能用 <label>，故补一个与它同为 150px 右对齐的 .f-label，
     保证所有行的标签左右对齐 */
  .f-row > label,
  .f-row > .f-label {
    width: 150px;
    text-align: right;
    color: var(--text-2);
    font-size: var(--fs-sm);
    flex-shrink: 0;
  }
  /* 只读值（当前仅 ID）：不可改的字段用纯值展示，字体字色与输入框内的文字同规格，
     min-height 取控件高度以保证与其他行基线一致 */
  .f-row .f-ro {
    flex: 1;
    min-width: 0;
    min-height: var(--control-h);
    display: flex;
    align-items: center;
    font-family: inherit;
    font-size: var(--fs-md);
    color: var(--text-1);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* 行内附注（当前仅 ID 的不可修改说明）：跟在值后面，不收缩、不折行 */
  .f-hint {
    flex: none;
    color: var(--text-3);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  /* 同上：只认直接子，避免把手伸进模型清单里的能力编辑控件 */
  .f-row > input,
  .f-row > select {
    flex: 1;
    min-width: 0;
  }
  /* 顶部对齐的行（模型清单）：标签与清单首行的输入框基线对齐 */
  .f-row-top {
    align-items: flex-start;
  }
  .f-row-top .f-label {
    padding-top: 6px;
  }
  .key-note {
    margin: 0;
    font-size: var(--fs-sm);
    color: var(--text-3);
  }
  /* 模型清单 */
  .m-list {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .m-item {
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  /* 每行头部：展开箭头 + 模型名 + 移除 */
  .m-head {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-2) var(--sp-1) var(--sp-1);
  }
  .m-toggle {
    flex: none;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    padding: 0;
    border: none;
    background: none;
    color: var(--text-3);
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: color var(--dur-fast) var(--ease-out);
  }
  .m-toggle:hover {
    color: var(--text-1);
  }
  .m-head input {
    flex: 1;
    min-width: 0;
  }
  .m-del {
    flex: none;
  }
  .m-add {
    align-self: flex-start;
  }
  /* 能力编辑区：inset 底与头部区分。
     四个宽度是本节控件的唯一定宽旋钮 —— 这些值都短，拉满整行只会显得空；
     同类型的控件取同一宽度，右边界才能连成列 */
  .m-cap {
    --cap-name-w: 96px; /* 参数名列：中文名右对齐即可，留白越少越紧凑 */
    --cap-num-w: 110px; /* 数字输入（约束上下限、张数/字数上限） */
    --cap-text-w: 260px; /* 文本输入与多行文本（尺寸清单、质量档） */
    --cap-sel-w: 124px; /* 枚举下拉（改写模式默认档） */
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    padding: var(--sp-3) var(--sp-4);
    background: var(--bg-inset);
    border-top: 1px solid var(--hairline);
  }
  /* 一条协议参数 = 参数名 | 值（支持位 + 附属项，按需折行）。
     平铺无分组：行集合由协议决定，模型只决定行上的勾选 */
  .cap-row {
    display: grid;
    grid-template-columns: var(--cap-name-w) minmax(0, 1fr);
    align-items: center;
    column-gap: var(--sp-3);
    row-gap: var(--sp-2);
  }
  /* 参数名列：中文名，右对齐；官方字段原名只在 title 里出现 */
  .cap-name {
    text-align: right;
    color: var(--text-2);
    font-size: var(--fs-sm);
    line-height: 1.35;
  }
  /* 值列：支持位开关在最前，其后按登记顺序紧跟附属项；窄窗自动折行。
     子项一律 flex:0 0 auto + 归零外边距 —— 任何继承来的 grow / margin
     都会在滑钮与它的附属项之间撑出空位，而这正是本布局最忌讳的 */
  .cap-val {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-2) var(--sp-3);
    min-width: 0;
  }
  .cap-val > * {
    flex: 0 0 auto;
    margin: 0;
  }
  .cap-sub {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }
  .cap-sw {
    flex: none;
    width: auto;
  }
  /* 两态支持位（调用方式 = 同步/异步）：复用全局 .seg3 的几何，
     在能力表里压到与输入框同高（外层 30px、内边距 2px、按钮竖向 padding 归零靠 stretch 撑满） */
  .cap-mode {
    flex: none;
    width: 124px;
    height: var(--control-h);
    padding: 2px;
    align-items: stretch;
  }
  .cap-mode button {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 var(--sp-3);
  }
  .cap-k {
    color: var(--text-3);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  /* 成对数值（≥ / ≤）：只锁不收缩，宽度由 type 选择器统一给 */
  .cap-pair {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }
  .cap-pair input {
    flex: none;
  }
  .cap-op {
    flex: none;
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  /* 按 type 定宽，刻意不写裸 `.m-cap input`：组件选择器带 scope 类，优先级压过
     app.css 的 input[type='checkbox']，裸选择器会把 14px 的勾选框一起拉宽 */
  .m-cap textarea,
  .m-cap input[type="text"] {
    width: var(--cap-text-w);
  }
  .m-cap input[type="number"] {
    width: var(--cap-num-w);
  }
  .m-cap select {
    width: var(--cap-sel-w);
  }
  /* .modal-foot 用 app.css 全局原语，本组件不重写 */
</style>
