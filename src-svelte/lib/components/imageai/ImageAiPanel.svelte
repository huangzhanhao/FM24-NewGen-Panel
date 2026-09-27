<!-- AI 生图 Tab 内容：顶部「服务商/模型」条 + 子分段（工作台/台账）。
     工作台左栏：模板/提示词/参数/生成；右栏：结果区。复用全局类与 icons，不重写控件。 -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { message, save } from '@tauri-apps/plugin-dialog';
  import { api } from '../../services/api';
  import UsageLedger from './UsageLedger.svelte';
  import ProviderDialog from './ProviderDialog.svelte';
  import TemplateDialog from './TemplateDialog.svelte';
  import { t } from '../../text';
  import { showError } from '../../ui/dialog';
  import { icons } from '../../ui/icons';
  import {
    VARIABLE_HINTS, appendVariable, applyTemplate, generate, imageaiStore,
    loadImageState, removeResult, selectModel, selectProvider,
  } from '../../stores/imageai.svelte';

  let view = $state<'workbench' | 'usage'>('workbench');
  let providerOpen = $state(false);
  let templateOpen = $state(false);
  let showNeg = $state(true);

  // 首次进入懒加载配置；失败统一走错误弹窗。
  onMount(() => {
    if (!imageaiStore.config) void loadImageState().catch(showError);
  });

  async function onGenerate() {
    try {
      await generate();
    } catch (e) {
      await showError(e);
    }
  }

  async function onSave(imgId: string) {
    const target = await save({
      defaultPath: `${imgId}.png`,
      filters: [{ name: 'PNG', extensions: ['png'] }],
    });
    if (!target) return;
    try {
      await api.saveGeneratedImage(imgId, target);
      await message(t.gen.saved, { title: t.dialog.info });
    } catch (e) {
      await showError(e);
    }
  }
</script>

<div class="panel">
  <div class="bar">
    <span class="bar-label">{t.gen.provider}</span>
    <select value={imageaiStore.providerId} onchange={(e) => selectProvider(e.currentTarget.value)}>
      {#each imageaiStore.config?.providers ?? [] as p}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
    <span class="bar-label">{t.gen.model}</span>
    <select value={imageaiStore.model} onchange={(e) => selectModel(e.currentTarget.value)}>
      {#each imageaiStore.provider?.models ?? [] as m}
        <option value={m.model}>{m.label}</option>
      {/each}
    </select>
    <button class="btn" onclick={() => (providerOpen = true)}>{t.gen.providerSettings}</button>
    <span class="spacer"></span>
    <div class="seg-tabs inner">
      <button class:active={view === 'workbench'} onclick={() => (view = 'workbench')}>{t.gen.tabWorkbench}</button>
      <button class:active={view === 'usage'} onclick={() => (view = 'usage')}>{t.gen.tabUsage}</button>
    </div>
  </div>

  {#if view === 'workbench'}
    <div class="wb-grid">
      <div class="wb-left">
        <!-- 模板 -->
        <div class="block">
          <div class="block-head">
            <span>{t.gen.template}</span>
            <button class="btn sm" onclick={() => (templateOpen = true)}>{t.gen.manageTemplate}</button>
          </div>
          <select value={imageaiStore.templateId} onchange={(e) => void applyTemplate(e.currentTarget.value).catch(showError)}>
            {#each imageaiStore.config?.templates ?? [] as tpl}
              <option value={tpl.id}>{tpl.name}</option>
            {/each}
          </select>
          <p class="hint">{t.gen.variables}</p>
          <div class="chips">
            {#each VARIABLE_HINTS as [key, label] (key)}
              <button class="chip" onclick={() => appendVariable(key)}>{label}</button>
            {/each}
          </div>
        </div>

        <!-- 提示词 -->
        <div class="block">
          <label class="field-label" for="ig-prompt">{t.gen.prompt}</label>
          <textarea id="ig-prompt" rows="7" bind:value={imageaiStore.prompt}></textarea>
          <button class="neg-toggle" onclick={() => (showNeg = !showNeg)}>
            <svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html (showNeg ? icons.chevronUp : icons.chevronDown).body}</svg>
            {t.gen.toggleNegative}
          </button>
          {#if showNeg}
            <textarea rows="2" placeholder={t.gen.negative} bind:value={imageaiStore.negative}></textarea>
          {/if}
        </div>

        <!-- 参数：能力集驱动 -->
        <div class="block">
          <div class="frow">
            <label for="ig-size">{t.gen.size}</label>
            <select id="ig-size" bind:value={imageaiStore.size}>
              {#each imageaiStore.modelInfo?.capabilities.sizes ?? [] as s}
                <option value={s}>{s}</option>
              {/each}
            </select>
          </div>
          <div class="frow">
            <label for="ig-n">{t.gen.n}</label>
            <input id="ig-n" type="number" min="1" max={imageaiStore.modelInfo?.capabilities.maxN ?? 1} bind:value={imageaiStore.n} />
          </div>
          {#if imageaiStore.modelInfo?.capabilities.supportsQuality}
            <div class="frow">
              <label for="ig-quality">{t.gen.quality}</label>
              <select id="ig-quality" bind:value={imageaiStore.quality}>
                {#each ['auto', ...imageaiStore.modelInfo?.capabilities.qualities.filter((q) => q !== 'auto')] as q}
                  <option value={q}>{q}</option>
                {/each}
              </select>
            </div>
          {/if}
          {#if imageaiStore.modelInfo?.capabilities.supportsSeed}
            <div class="frow">
              <label for="ig-seed">{t.gen.seed}</label>
              <input id="ig-seed" type="number" placeholder={t.gen.seedPlaceholder} bind:value={imageaiStore.seed} />
            </div>
          {/if}
        </div>

        <button class="btn primary" disabled={imageaiStore.generating} onclick={onGenerate}>
          {imageaiStore.generating ? t.gen.generating : t.gen.generate}
        </button>
      </div>

      <div class="wb-right">
        {#if imageaiStore.results.length === 0}
          <div class="empty">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">{@html icons.image.body}</svg>
            <p>{t.gen.empty}</p>
          </div>
        {:else}
          <div class="gallery">
            {#each imageaiStore.results as img (img.id)}
              <div class="img-card">
                <img src={img.url} alt="generated" />
                <div class="actions">
                  <button class="btn sm" onclick={() => onSave(img.id)}>
                    <svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.download.body}</svg>
                    {t.gen.save}
                  </button>
                  <button class="btn sm" onclick={onGenerate}>
                    <svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.refresh.body}</svg>
                  </button>
                  <button class="icon-btn" title={t.gen.removeImageTitle} onclick={() => removeResult(img.id)}>
                    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">{@html icons.x.body}</svg>
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {:else}
    <UsageLedger />
  {/if}
</div>

<ProviderDialog bind:open={providerOpen} />
<TemplateDialog bind:open={templateOpen} />

<style>
  /* 父级 .tab-pane.active 是 display:block（app.css L284），故这里不能用 flex:1 撑高，改 height:100%。 */
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 10px; }
  .bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .bar-label { color: var(--text-2); font-size: 12.5px; }
  /* 内嵌变体：.seg-tabs 全局只有 ::before 绿色标条 + 9px 下边距（没有 border），
     所以这里要显式去掉标条与下边距，并缩小字号以适配顶栏单行。 */
  .seg-tabs.inner { margin: 0; }
  .seg-tabs.inner::before { display: none; }
  .seg-tabs.inner button { font-size: 12.5px; padding-bottom: 2px; margin-right: 12px; }
  .wb-grid { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(300px, 360px) minmax(0, 1fr); gap: 14px; }
  .wb-left { display: flex; flex-direction: column; gap: 10px; overflow-y: auto; padding-right: 2px; }
  .wb-right { min-height: 0; display: flex; }
  /* .empty 是全局类（app.css L765），但没有独立 .grow；这里让它填满右栏。 */
  .wb-right .empty { flex: 1; }
  .block-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 6px; font-size: 12.5px; color: var(--text-2); }
  .hint { margin: 8px 0 6px; color: var(--text-3); font-size: 12px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .field-label { display: block; font-size: 12.5px; color: var(--text-2); margin-bottom: 6px; }
  textarea { width: 100%; resize: vertical; }
  .neg-toggle { display: inline-flex; align-items: center; gap: 5px; margin: 6px 0; background: none; border: none; color: var(--text-3); font-size: 12px; cursor: pointer; padding: 0; }
  .neg-toggle:hover { color: var(--accent); }
  .frow { margin: 6px 0; display: flex; align-items: center; gap: 8px; }
  .frow label { width: 48px; color: var(--text-2); font-size: 12.5px; text-align: right; }
  .frow input { width: 110px; }
  .wb-left .btn.primary { padding: 10px; font-size: 14px; }
  .gallery { flex: 1; display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 10px; align-content: start; overflow-y: auto; }
  .img-card { border: 1px solid var(--border); border-radius: var(--radius-md); overflow: hidden; background: var(--bg-surface); display: flex; flex-direction: column; }
  .img-card img { width: 100%; aspect-ratio: 1; object-fit: cover; display: block; }
  .actions { display: flex; gap: 6px; padding: 6px; align-items: center; }
  .actions .icon-btn { margin-left: auto; }
  .ic { width: 13px; height: 13px; }
</style>
