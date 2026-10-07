<!-- TemplateDialog · 提示词模板管理弹窗：模板 CRUD，复用全局 modal 类 -->
<script lang="ts">
  import { tick } from "svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { deleteTemplate, imageaiStore, saveTemplate, VARIABLE_HINTS } from "../../stores/imageai.svelte";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text";
  import type { PromptTemplate } from "../../types";

  let { open = $bindable(false) }: { open?: boolean } = $props();
  let editing = $state<PromptTemplate | null>(null);

  // 正文 textarea 引用 + 是否已聚焦过：聚焦过就在光标选区插入占位符，从未聚焦则追加末尾
  let bodyEl = $state<HTMLTextAreaElement | null>(null);
  let bodyFocused = $state(false);

  function blank(): PromptTemplate {
    bodyFocused = false;
    // 新模板继承当前模板的码值映射与词库：否则它在文件里不带这两块，用户无从下手改。
    const cur = imageaiStore.config?.templates.find((x) => x.id === imageaiStore.templateId);
    return { id: "", name: "", body: "", negative: "", codes: cur?.codes, pools: cur?.pools };
  }

  /** 点变量胶囊：在模板正文光标处插入 {{key}}（失焦后浏览器仍保留上次选区）；
   *  正文从未聚焦时追加到末尾。插入后 tick 再聚焦并定位光标，支持连续点击多个变量。 */
  function insertVar(key: string) {
    if (!editing) return;
    const token = `{{${key}}}`;
    const start = bodyEl && bodyFocused ? bodyEl.selectionStart : editing.body.length;
    const end = bodyEl && bodyFocused ? bodyEl.selectionEnd : start;
    editing.body = editing.body.slice(0, start) + token + editing.body.slice(end);
    const caret = start + token.length;
    void tick().then(() => {
      bodyEl?.focus();
      bodyEl?.setSelectionRange(caret, caret);
    });
  }

  async function onSave() {
    if (!editing) return;
    try {
      if (!editing.id) editing.id = `t-${crypto.randomUUID()}`;
      await saveTemplate($state.snapshot(editing)); // config 整包写回 + 悬空回退见 store action
      editing = null;
    } catch (e) {
      await showError(e);
    }
  }

  // 形参不要叫 t：会遮蔽 text.ts 的 t 对象
  async function onDelete(tpl: PromptTemplate) {
    if (!(await confirm(t.gen.deleteTemplateConfirm(tpl.name), { title: t.dialog.warning }))) return;
    try {
      await deleteTemplate(tpl.id);
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
      <div class="modal-title">{t.gen.templateManageTitle}</div>
      <div class="modal-body">
        {#if !editing}
          {#each imageaiStore.config?.templates ?? [] as tpl (tpl.id)}
            <div class="t-row">
              <span>{tpl.name}</span>
              <span class="btns">
                <button
                  class="btn sm"
                  onclick={() => {
                    bodyFocused = false;
                    editing = $state.snapshot(tpl);
                  }}>{t.gen.edit}</button
                >
                {#if tpl.id !== "default"}
                  <button class="btn sm danger" onclick={() => onDelete(tpl)}>{t.gen.remove}</button>
                {/if}
              </span>
            </div>
          {/each}
        {:else}
          <label for="tpl-name">{t.gen.templateNameLabel}</label>
          <!-- type 必写：app.css 的控件样式走 input[type='text'] 属性选择器，漏写 type 会匹配不上、走浏览器默认外观 -->
          <input id="tpl-name" type="text" bind:value={editing.name} />
          <label for="tpl-body">{t.gen.templateBodyLabel}</label>
          <!-- 变量胶囊：点击在正文光标处插入占位符（仅正文参与渲染，负向不加） -->
          <p class="vars-hint">{t.gen.templateVariables}</p>
          <div class="var-chips">
            {#each VARIABLE_HINTS as [key, label] (key)}
              <button type="button" class="chip" title={`{{${key}}}`} onclick={() => insertVar(key)}>
                {label}
              </button>
            {/each}
          </div>
          <textarea
            id="tpl-body"
            rows="8"
            bind:this={bodyEl}
            bind:value={editing.body}
            onfocus={() => (bodyFocused = true)}
          ></textarea>
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
  /* 只写布局：紧凑按钮尺寸是控件规格，统一由 app.css 的 button.btn.sm 给 */
  .t-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: var(--sp-1) 0;
    font-size: var(--fs-md);
  }
  .btns {
    display: flex;
    gap: var(--sp-2);
  }
  label {
    display: block;
    margin: var(--sp-3) 0 var(--sp-1);
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  /* 变量胶囊区：提示句 + 可折行胶囊（.chip 悬停/色语用全局原语），与下方正文框留一行间距 */
  .vars-hint {
    margin: 0 0 var(--sp-2);
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  .var-chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
    margin-bottom: var(--sp-2);
  }
  /* .modal-foot 用 app.css 全局原语，本组件不重写 */
</style>
