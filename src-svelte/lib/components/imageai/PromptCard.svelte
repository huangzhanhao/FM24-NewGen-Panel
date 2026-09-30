<!-- PromptCard · 提示词卡：模板选择 + 变量胶囊 + 正文（数据源只读预览 / 手填可编辑两态）+ 负向折叠 + 智能改写
     正文/负向/模板等状态住在 imageaiStore，本地只有折叠开关与预览文本；
     唯一外部触点是「管理模板」按钮经 onopentemplates 请父级打开 TemplateDialog -->
<script lang="ts">
  import { api } from "../../services/api";
  import Icon from "../../ui/Icon.svelte";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text";
  import {
    appendVariable,
    applyTemplate,
    imageaiStore,
    VARIABLE_HINTS,
  } from "../../stores/imageai.svelte";

  let { onopentemplates }: { onopentemplates: () => void } = $props();

  let showNeg = $state(true);

  const caps = $derived(imageaiStore.modelInfo?.capabilities);

  // ---- 数据源只读预览：有数据源时正文由 preview_prompt 渲染，文本框只读展示 ----
  let sourcePreview = $state("");

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

  // 字数计数：超限判定 promptOver 由 store 单点派生（跨组件共用），这里只渲染计数
  const promptLen = $derived(imageaiStore.prompt.length);
  const promptLimit = $derived(caps?.maxPromptChars ?? 0);
  const promptNear = $derived(
    promptLimit > 0 && promptLen > promptLimit * 0.9 && !imageaiStore.promptOver,
  );
</script>

<section class="pcard">
  <div class="pcard-h">
    <span>{t.gen.cardContent}</span>
    <button class="btn sm" onclick={onopentemplates}
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
          class:over={imageaiStore.promptOver}>{promptLen} / {promptLimit}</span
        >
      {/if}
    </div>
    <textarea
      id="ig-prompt"
      rows="7"
      bind:value={imageaiStore.prompt}
      class:invalid={imageaiStore.promptOver}
    ></textarea>
    {#if imageaiStore.promptOver}
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

<style>
  .field-label {
    display: block;
    font-size: 12px;
    color: var(--text-2);
    margin-bottom: 4px;
  }
  .tpl-row {
    display: flex;
    flex-direction: column;
  }
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
</style>
