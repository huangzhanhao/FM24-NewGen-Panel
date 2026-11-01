<!-- RuleEditor · 标准规则编辑对话框：直接查看/修改/保存 data/rules/ 下的 standard2|standard3.json。
     顶部 chip 切换标准二/三，切换与关闭前若有未保存修改会弹确认；
     保存时后端先按该标准 schema 校验，非法 JSON 直接报错、不会写入坏规则 -->
<script lang="ts">
  import { ask } from '@tauri-apps/plugin-dialog';
  import { api } from '../../services/api';
  import { showError } from '../../ui/dialog';
  import { t } from '../../text'; // 全局文案常量
  import { appStore, refreshSnapshot, tip } from '../../stores/app.svelte';

  type RuleName = 'standard2' | 'standard3';
  const LABELS: Record<RuleName, string> = {
    standard2: t.ruleEditor.std2Label,
    standard3: t.ruleEditor.std3Label,
  };
  const NAMES: RuleName[] = ['standard2', 'standard3'];

  let open = $state(false);
  let active = $state<RuleName>('standard2');
  let text = $state('');
  let baseline = $state('');  // 载入时的内容（dirty 判定基准）
  let status = $state('');    // 行内提示（保存成功等）
  let editorEl = $state<HTMLTextAreaElement | null>(null); // 打开即聚焦的编辑区

  const dirty = $derived(text !== baseline);

  /** 父组件经 bind:this 调用：打开对话框并载入当前标准的内容 */
  export function openEditor() {
    open = true;
    void load(active).then(() => editorEl?.focus());
  }

  /** Esc 等同点关闭：有未保存修改时 close() 内部会先弹确认 */
  function onWindowKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && open) void close();
  }

  /** 载入指定标准的规则文件内容，并把载入值记为 baseline */
  async function load(name: RuleName) {
    try {
      const got = await api.readRuleFile(name);
      // 【必须校验 active】快速连点两个标准 chip 时两个请求会乱序返回：
      // 晚到的 standard3 内容会盖进编辑区，用户一按保存就把它写进 standard2.json
      //（后端只校目标文件的 schema 形状，语义错的内容照样被原子写入）
      if (name !== active) return;
      text = got;
      baseline = got;
      status = '';
    } catch (e) {
      if (name !== active) return; // 已被别的标准取代，这条错误不该再弹
      await showError(e);
    }
  }

  /** 切换标准：有未保存修改时先弹确认，用户放弃才切 */
  async function switchTo(name: RuleName) {
    if (name === active) return;
    if (dirty && !(await ask(t.ruleEditor.unsavedSwitch(LABELS[active]), { title: t.ruleEditor.unsavedTitle }))) return;
    active = name;
    await load(active);
  }

  /** 保存当前标准：后端按 schema 校验并原子写回，成功后 baseline 同步 */
  async function save() {
    try {
      await api.saveRuleFile(active, text);
      baseline = text;
      status = t.ruleEditor.savedStatus;
      await refreshSnapshot(); // 快照携带其他状态，顺带保持全局一致
    } catch (e) { await showError(e); }
  }

  /** 关闭对话框：有未保存修改时先弹确认 */
  async function close() {
    if (dirty && !(await ask(t.ruleEditor.unsavedClose(LABELS[active]), { title: t.ruleEditor.unsavedTitle }))) return;
    open = false;
  }
</script>

<!-- Esc 监听（组件常驻；处理器内部只在 open 时响应），故 svelte:window 必须在 {#if} 之外 -->
<svelte:window onkeydown={onWindowKey} />

{#if open}
  <!-- 遮罩：点空白处关闭（同样经未保存确认） -->
  <div class="modal-overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) close(); }}>
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-label={t.ruleEditor.editorAria}
    >
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
          <button class="btn primary" onclick={save} disabled={!dirty} use:tip={t.ruleEditor.saveHint}>{t.ruleEditor.saveBtn}</button>
          <button class="btn" onclick={close}>{t.ruleEditor.closeBtn}</button>
        </div>
      </div>
        <div class="modal-body">
          <textarea class="editor" bind:this={editorEl} bind:value={text} oninput={() => (status = '')}
                    spellcheck="false" wrap="off"
                    aria-label={t.ruleEditor.editorAria}></textarea>
      </div>
    </div>
  </div>
{/if}
