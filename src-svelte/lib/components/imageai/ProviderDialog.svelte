<!-- 服务商设置：列表/新建/编辑/删除；Key 掩码输入仅发会话内存。复用全局 modal 类。 -->
<script lang="ts">
  import { confirm, message } from '@tauri-apps/plugin-dialog';
  import { api } from '../../services/api';
  import { t } from '../../text';
  import { imageaiStore } from '../../stores/imageai.svelte';
  import { showError } from '../../ui/dialog';
  import type { ProviderConfig } from '../../types';

  let { open = $bindable(false) }: { open?: boolean } = $props();
  let editing = $state<ProviderConfig | null>(null);
  let keyInput = $state('');

  function blank(): ProviderConfig {
    return { id: '', kind: 'custom', name: '', baseUrl: '', defaultModel: '', models: [] };
  }

  async function onSave() {
    if (!editing) return;
    try {
      if (!editing.id) editing.id = `p-${Date.now()}`;
      if (!editing.defaultModel && editing.models[0]) editing.defaultModel = editing.models[0].model;
      imageaiStore.config = await api.saveProvider($state.snapshot(editing));
      if (keyInput) await api.setProviderKey(editing.id, keyInput);
      await message(t.gen.providerSaved, { title: t.dialog.info });
      editing = null;
      keyInput = '';
    } catch (e) {
      await showError(e); // 保存/写 Key 失败必须弹窗，不能静默
    }
  }

  async function onDelete(p: ProviderConfig) {
    if (!(await confirm(t.gen.deleteProviderConfirm(p.name), { title: t.dialog.warning }))) return;
    try {
      imageaiStore.config = await api.deleteProvider(p.id);
    } catch (e) {
      await showError(e);
    }
  }
</script>

{#if open}
  <div class="modal-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) open = false; }}>
    <div class="modal" role="dialog" aria-modal="true">
      <div class="modal-title">{t.gen.providerSettings}</div>
      <div class="modal-body">
        {#if !editing}
          <div class="p-list">
            {#each imageaiStore.config?.providers ?? [] as p (p.id)}
              <div class="p-row">
                <span>{p.name} <em>{p.kind}</em></span>
                <span class="btns">
                  <button class="btn sm" onclick={() => (editing = $state.snapshot(p))}>{t.gen.edit}</button>
                  {#if p.kind !== 'openai' && p.kind !== 'custom'}
                    <button class="btn sm danger" onclick={() => onDelete(p)}>{t.gen.remove}</button>
                  {/if}
                </span>
              </div>
            {/each}
          </div>
        {:else}
          <div class="form">
            <!-- ID 是 upsert_provider 的匹配键：编辑态只读，否则改 ID 会「新增一条、旧条目仍在」（Key 也留在旧 id 上）。 -->
            <div class="f-row"><label for="pv-id">{t.gen.providerIdLabel}</label><input id="pv-id" readonly={editing.id !== ''} bind:value={editing.id} placeholder={t.gen.providerIdPlaceholder} /></div>
            <div class="f-row"><label for="pv-kind">{t.gen.kind}</label>
              <!-- 联合字面量类型：不用 bind:value（svelte-check 会报类型不符），改显式赋值 -->
              <select id="pv-kind" value={editing.kind} onchange={(e) => (editing!.kind = e.currentTarget.value as ProviderConfig['kind'])}>
                <option value="custom">custom</option>
                <option value="openai">openai</option>
                <option value="volcano" disabled>volcano（P2）</option>
                <option value="dashscope" disabled>dashscope（P2）</option>
              </select>
            </div>
            <div class="f-row"><label for="pv-name">{t.gen.providerNameLabel}</label><input id="pv-name" bind:value={editing.name} /></div>
            <div class="f-row"><label for="pv-url">{t.gen.baseUrl}</label><input id="pv-url" bind:value={editing.baseUrl} placeholder="https://example.com/v1" /></div>
            <div class="f-row"><label for="pv-model">{t.gen.providerModelLabel}</label><input id="pv-model" bind:value={editing.defaultModel} /></div>
            <div class="f-row"><label for="pv-key">{t.gen.keyLabel}</label><input id="pv-key" type="password" bind:value={keyInput} placeholder={t.gen.keySetHint} /></div>
            <p class="note">{t.gen.providerIdFixedNote}</p>
            <p class="note">{t.gen.modelRowsNote}</p>
          </div>
        {/if}
      </div>
      <div class="modal-foot">
        {#if editing}
          <button class="btn" onclick={() => (editing = null)}>{t.gen.back}</button>
          <button class="btn primary" onclick={onSave}>{t.gen.save}</button>
        {:else}
          <button class="btn" onclick={() => (editing = blank())}>{t.gen.addProvider}</button>
          <button class="btn primary" onclick={() => (open = false)}>{t.gen.done}</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* 只写布局：紧凑按钮尺寸是控件规格，已提到 app.css 的 button.btn.sm（§2 的复用原则） */
  .p-list { display: flex; flex-direction: column; gap: 8px; }
  .p-row { display: flex; justify-content: space-between; align-items: center; font-size: 13px; }
  .p-row em { color: var(--text-3); font-style: normal; font-size: 12px; margin-left: 6px; }
  .btns { display: flex; gap: 6px; }
  .form { display: flex; flex-direction: column; gap: 8px; }
  .f-row { display: flex; align-items: center; gap: 8px; }
  .f-row label { width: 150px; text-align: right; color: var(--text-2); font-size: 12.5px; flex-shrink: 0; }
  .f-row input, .f-row select { flex: 1; min-width: 0; }
  .note { color: var(--text-3); font-size: 12px; }
  .modal-foot { display: flex; justify-content: flex-end; gap: 8px; padding-top: 10px; }
</style>
