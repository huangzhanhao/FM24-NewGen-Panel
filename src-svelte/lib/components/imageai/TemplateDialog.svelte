<!-- 提示词模板 CRUD，复用全局 modal 类。 -->
<script lang="ts">
  import { confirm } from '@tauri-apps/plugin-dialog';
  import { api } from '../../services/api';
  import { imageaiStore } from '../../stores/imageai.svelte';
  import { showError } from '../../ui/dialog';
  import { t } from '../../text';
  import type { PromptTemplate } from '../../types';

  let { open = $bindable(false) }: { open?: boolean } = $props();
  let editing = $state<PromptTemplate | null>(null);

  function blank(): PromptTemplate {
    return { id: '', name: '', body: '', negative: '' };
  }

  async function onSave() {
    if (!editing) return;
    try {
      if (!editing.id) editing.id = `t-${Date.now()}`;
      imageaiStore.config = await api.savePromptTemplate($state.snapshot(editing));
      editing = null;
    } catch (e) {
      await showError(e);
    }
  }

  // 形参不要叫 t：会遮蔽 text.ts 的 t 对象。
  async function onDelete(tpl: PromptTemplate) {
    if (!(await confirm(t.gen.deleteTemplateConfirm(tpl.name), { title: t.dialog.warning }))) return;
    try {
      imageaiStore.config = await api.deletePromptTemplate(tpl.id);
    } catch (e) {
      await showError(e);
    }
  }
</script>

{#if open}
  <div class="modal-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) open = false; }}>
    <div class="modal" role="dialog" aria-modal="true">
      <div class="modal-title">{t.gen.templateManageTitle}</div>
      <div class="modal-body">
        {#if !editing}
          {#each imageaiStore.config?.templates ?? [] as tpl (tpl.id)}
            <div class="t-row">
              <span>{tpl.name}</span>
              <span class="btns">
                <button class="btn sm" onclick={() => (editing = $state.snapshot(tpl))}>{t.gen.edit}</button>
                {#if tpl.id !== 'default'}
                  <button class="btn sm danger" onclick={() => onDelete(tpl)}>{t.gen.remove}</button>
                {/if}
              </span>
            </div>
          {/each}
        {:else}
          <label for="tpl-name">{t.gen.templateNameLabel}</label>
          <input id="tpl-name" bind:value={editing.name} />
          <label for="tpl-body">{t.gen.templateBodyLabel}</label>
          <textarea id="tpl-body" rows="8" bind:value={editing.body}></textarea>
          <label for="tpl-neg">{t.gen.templateNegativeLabel}</label>
          <textarea id="tpl-neg" rows="2" bind:value={editing.negative}></textarea>
        {/if}
      </div>
      <div class="modal-foot">
        {#if editing}
          <button class="btn" onclick={() => (editing = null)}>{t.gen.back}</button>
          <button class="btn primary" onclick={onSave}>{t.gen.save}</button>
        {:else}
          <button class="btn" onclick={() => (editing = blank())}>{t.gen.addTemplate}</button>
          <button class="btn primary" onclick={() => (open = false)}>{t.gen.done}</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* 只写布局；紧凑按钮尺寸走 app.css 的 button.btn.sm */
  .t-row { display: flex; justify-content: space-between; align-items: center; padding: 4px 0; font-size: 13px; }
  .btns { display: flex; gap: 6px; }
  label { display: block; margin: 8px 0 4px; color: var(--text-2); font-size: 12.5px; }
  .modal-foot { display: flex; justify-content: flex-end; gap: 8px; padding-top: 10px; }
</style>
