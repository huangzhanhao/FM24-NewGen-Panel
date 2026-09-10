<!-- 【新手导读】标准规则编辑对话框：直接查看/修改/保存 data/rules/ 下的 standard2|standard3.json。
     标准二/三用顶部 chip 切换；切换与关闭前若有未保存修改会弹确认；
     保存时后端先按该标准 schema 校验（非法 JSON 直接报错，不会写入坏规则）。 -->
<script lang="ts">
  import { ask } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { showError } from '../dialog';
  import { t } from '../text'; // 全局文案常量
  import { appStore, refreshSnapshot } from '../stores/app.svelte';

  type RuleName = 'standard2' | 'standard3';
  const LABELS: Record<RuleName, string> = {
    standard2: t.ruleEditor.std2Label,
    standard3: t.ruleEditor.std3Label,
  };
  const NAMES: RuleName[] = ['standard2', 'standard3'];

  let open = $state(false);
  let active = $state<RuleName>('standard2');
  let text = $state('');      // 编辑区内容
  let baseline = $state('');  // 载入时的内容（用于判断有无未保存修改）
  let status = $state('');    // 行内提示（保存成功等）

  const dirty = $derived(text !== baseline);

  /** 由父组件调用（bind:this 后 openEditor()）打开对话框并载入当前标准内容。 */
  export function openEditor() {
    open = true;
    void load(active);
  }

  async function load(name: RuleName) {
    try {
      text = await api.readRuleFile(name);
      baseline = text;
      status = '';
    } catch (e) { await showError(e); }
  }

  async function switchTo(name: RuleName) {
    if (name === active) return;
    if (dirty && !(await ask(t.ruleEditor.unsavedSwitch(LABELS[active]), { title: t.ruleEditor.unsavedTitle }))) return;
    active = name;
    await load(active);
  }

  async function save() {
    try {
      await api.saveRuleFile(active, text);
      baseline = text;
      status = t.ruleEditor.savedStatus;
      await refreshSnapshot(); // 快照携带其他状态，顺带保持全局一致
    } catch (e) { await showError(e); } // JSON 校验错误（后端拒绝写入坏规则）
  }

  async function close() {
    if (dirty && !(await ask(t.ruleEditor.unsavedClose(LABELS[active]), { title: t.ruleEditor.unsavedTitle }))) return;
    open = false;
  }
</script>

{#if open}
  <!-- 遮罩：点击空白处关闭（经确认）。 -->
  <div class="modal-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) close(); }}>
    <div class="modal">
      <div class="modal-title">
        <div class="seg3">
          {#each NAMES as name}
            <button class:active={active === name}
                    onclick={() => switchTo(name)}>{LABELS[name]}</button>
          {/each}
        </div>
        <div class="modal-actions">
          {#if status}<span class="ok-text small">{status}</span>{/if}
          <span class="muted small">{appStore.snapshot?.currentProfile ?? ''}</span>
          <button class="btn primary" onclick={save} disabled={!dirty} title={t.ruleEditor.saveHint}>{t.ruleEditor.saveBtn}</button>
          <button class="btn" onclick={close}>{t.ruleEditor.closeBtn}</button>
        </div>
      </div>
        <div class="modal-body">
          <textarea class="editor" bind:value={text} oninput={() => (status = '')}
                    spellcheck="false" wrap="off"
                    aria-label={t.ruleEditor.editorAria}></textarea>
      </div>
    </div>
  </div>
{/if}
