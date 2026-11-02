<!-- NationalityEditor · 国籍对照表编辑对话框：在应用内调整/新增/删除「三字码 ↔ 各语言国籍全称」映射。
     一行 = 一条映射（同码同语言可以有多条全称/别名）；保存时后端统一校验并原子写回
     data/nationalities.json，成功后立即生效（下一次扫描/替换/提示词预览无需重启）。 -->
<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text";
  import { tip } from "../../stores/app.svelte";
  import type { NationalityDoc, NationalityMapping } from "../../types";

  let open = $state(false);
  let doc = $state<NationalityDoc>({ languages: [], mappings: [] });
  let baseline = $state(""); // 载入/保存后的内容（dirty 判定基准）
  let status = $state("");   // 成功提示（已保存）
  let notice = $state("");   // 编辑期校验提示（重复映射、字段未填等），与成功提示分开着色
  let query = $state("");
  let saving = $state(false);
  // 新增行草稿：语言跟随后端返回的语言列表，避免手输拼错后整份保存被拒
  let draft = $state<NationalityMapping>({ code: "", language: "", name: "" });
  let newLanguage = $state("");

  // 脏判定：与载入/保存时的序列化结果逐字比较（行内输入框直接改映射对象，没有统一 setter）。
  // 【成本】出厂表约 600 条、序列化约 36KB，每次按键重算；当前量级无感，数量再上一个量级
  // 应改成「编辑时置 dirty 标志 + 载入/保存时清除」。
  const dirty = $derived(JSON.stringify(doc) !== baseline);

  // 可见行只存源数组下标：删除时按下标 splice，绑定仍写回 doc.mappings 里的同一个响应式对象。
  // 搜索只影响显示，不改动源数组顺序（用户编辑时行不会因排序跳走；排序在后端保存时统一做）。
  const visible = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const indexes = doc.mappings.map((_, i) => i);
    if (!q) return indexes;
    return indexes.filter((i) => {
      const m = doc.mappings[i];
      return (
        m.code.toLowerCase().includes(q) ||
        m.language.toLowerCase().includes(q) ||
        m.name.toLowerCase().includes(q)
      );
    });
  });

  /** 父组件经 bind:this 调用：打开对话框并载入当前生效表 */
  export function openEditor() {
    open = true;
    void load();
  }

  async function load() {
    try {
      const got = await api.getNationalities();
      doc = got;
      baseline = JSON.stringify(got);
      // 语言下拉默认选第一项；新增行始终使用列表内的语言，避免用户手输了一个未登记的语言。
      draft = { code: "", language: got.languages[0] ?? "", name: "" };
      query = "";
      newLanguage = "";
      status = "";
      notice = "";
    } catch (e) {
      await showError(e);
      open = false;
    }
  }

  /** 新增一条映射（本地追加，点「保存」才落盘） */
  function addMapping() {
    const code = draft.code.trim().toUpperCase();
    const name = draft.name.trim();
    if (!code || !name || !draft.language) {
      notice = t.nationalityEditor.incomplete;
      status = "";
      return;
    }
    const exists = doc.mappings.some(
      (m) => m.code === code && m.language === draft.language && m.name === name,
    );
    if (exists) {
      notice = t.nationalityEditor.duplicate;
      status = "";
      return;
    }
    doc.mappings.push({ code, language: draft.language, name });
    draft.code = "";
    draft.name = "";
    status = "";
    notice = "";
  }

  /** 新增语言节点：只加进 languages 列表（可暂时没有映射），之后可在行内下拉里选用 */
  function addLanguage() {
    const lang = newLanguage.trim();
    if (!lang) return;
    if (doc.languages.includes(lang)) {
      notice = t.nationalityEditor.languageExists;
      status = "";
      return;
    }
    doc.languages.push(lang);
    // 新增语言后直接选中它，方便紧接着添加该语言下的映射
    draft.language = lang;
    newLanguage = "";
    status = "";
    notice = "";
  }

  function removeMapping(index: number) {
    doc.mappings.splice(index, 1);
    status = "";
    notice = "";
  }

  /** 保存：后端校验 → 原子写盘 → 立即换表；返回值是规范化/排序后的文档，用它重设界面 */
  async function save() {
    if (saving) return;
    saving = true;
    try {
      const saved = await api.saveNationalities({
        languages: doc.languages.map((l) => l.trim()).filter((l) => l.length > 0),
        mappings: doc.mappings.map((m) => ({
          code: m.code.trim(),
          language: m.language,
          name: m.name.trim(),
        })),
      });
      doc = saved;
      baseline = JSON.stringify(saved);
      if (!saved.languages.includes(draft.language)) {
        draft.language = saved.languages[0] ?? "";
      }
      status = t.nationalityEditor.savedStatus;
      notice = "";
    } catch (e) {
      await showError(e);
    } finally {
      saving = false;
    }
  }

  /** 关闭：有未保存修改时先弹确认 */
  async function close() {
    if (dirty && !(await ask(t.nationalityEditor.unsavedClose, { title: t.nationalityEditor.unsavedTitle }))) {
      return;
    }
    open = false;
  }

  /** Esc 等同点关闭（有未保存修改同样先确认） */
  function onWindowKey(e: KeyboardEvent) {
    if (e.key === "Escape" && open) void close();
  }

  /** 新增输入框里按 Enter 直接追加，省一次鼠标点击 */
  function onDraftKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      addMapping();
    }
  }
</script>

<svelte:window onkeydown={onWindowKey} />

{#if open}
  <!-- 遮罩：点空白处关闭（同样经未保存确认） -->
  <div
    class="modal-overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) void close();
    }}
  >
    <div class="modal nat-modal" role="dialog" aria-modal="true" aria-label={t.nationalityEditor.aria}>
      <div class="modal-title">
        <div class="nat-heading">
          <span class="nat-title">{t.nationalityEditor.title}</span>
          <span class="muted small">
            {t.nationalityEditor.countLabel(visible.length, doc.mappings.length)}
          </span>
        </div>
        <div class="modal-actions">
          {#if notice}<span class="notice small">{notice}</span>{/if}
          {#if status}<span class="ok-text small">{status}</span>{/if}
          <button class="btn primary" onclick={save} disabled={!dirty || saving} use:tip={t.nationalityEditor.saveHint}>
            {t.nationalityEditor.saveBtn}
          </button>
          <button class="btn" onclick={close}>{t.nationalityEditor.closeBtn}</button>
        </div>
      </div>
      <p class="nat-hint">{t.nationalityEditor.subtitle}</p>

      <div class="modal-body">
        <!-- 搜索 + 新增映射：一行内完成，减少弹窗滚动 -->
        <div class="nat-toolbar">
          <input
            type="text"
            bind:value={query}
            placeholder={t.nationalityEditor.searchPlaceholder}
            aria-label={t.nationalityEditor.searchPlaceholder}
          />
        </div>
        <div class="nat-add">
          <input
            type="text"
            class="code"
            bind:value={draft.code}
            maxlength="3"
            placeholder={t.nationalityEditor.addCodePlaceholder}
            aria-label={t.nationalityEditor.codeHeader}
            oninput={() => (notice = "")}
          />
          <select bind:value={draft.language} aria-label={t.nationalityEditor.languageHeader}>
            {#each doc.languages as lang (lang)}
              <option value={lang}>{lang}</option>
            {/each}
          </select>
          <input
            type="text"
            bind:value={draft.name}
            placeholder={t.nationalityEditor.addNamePlaceholder}
            aria-label={t.nationalityEditor.nameHeader}
            oninput={() => (notice = "")}
            onkeydown={onDraftKey}
          />
          <button class="btn" onclick={addMapping}>{t.nationalityEditor.addBtn}</button>
        </div>
        <!-- 语言节点管理：新增的语言会进入 languages（决定解析优先级与下拉选项） -->
        <div class="nat-langs">
          <span class="muted small">{t.nationalityEditor.languageLabel}</span>
          {#each doc.languages as lang (lang)}
            <span class="pill">{lang}</span>
          {/each}
          <input
            type="text"
            bind:value={newLanguage}
            placeholder={t.nationalityEditor.addLanguagePlaceholder}
            aria-label={t.nationalityEditor.addLanguagePlaceholder}
            onkeydown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                addLanguage();
              }
            }}
          />
          <button class="btn sm" onclick={addLanguage}>{t.nationalityEditor.addLanguageBtn}</button>
        </div>

        <div class="nat-list">
          <table>
            <thead>
              <tr>
                <th class="code-col">{t.nationalityEditor.codeHeader}</th>
                <th class="lang-col">{t.nationalityEditor.languageHeader}</th>
                <th>{t.nationalityEditor.nameHeader}</th>
                <th class="act-col"></th>
              </tr>
            </thead>
            <tbody>
              {#each visible as i}
                {@const m = doc.mappings[i]}
                <tr>
                  <td>
                    <input
                      type="text"
                      bind:value={m.code}
                      maxlength="3"
                      aria-label={t.nationalityEditor.codeHeader}
                      oninput={() => { status = ""; notice = ""; }}
                    />
                  </td>
                  <td>
                    <select
                      bind:value={m.language}
                      aria-label={t.nationalityEditor.languageHeader}
                      onchange={() => { status = ""; notice = ""; }}
                    >
                      {#each doc.languages as lang (lang)}
                        <option value={lang}>{lang}</option>
                      {/each}
                    </select>
                  </td>
                  <td>
                    <input
                      type="text"
                      bind:value={m.name}
                      aria-label={t.nationalityEditor.nameHeader}
                      oninput={() => { status = ""; notice = ""; }}
                    />
                  </td>
                  <td class="act-col">
                    <button
                      class="btn sm danger"
                      onclick={() => removeMapping(i)}
                      use:tip={t.nationalityEditor.deleteHint}
                      aria-label={t.nationalityEditor.deleteHint}
                    >
                      {t.nationalityEditor.deleteBtn}
                    </button>
                  </td>
                </tr>
              {:else}
                <tr>
                  <td colspan="4" class="empty muted small">{t.nationalityEditor.noMatch}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        <p class="nat-foot muted small">{t.nationalityEditor.emptyHint}</p>
      </div>
    </div>
  </div>
{/if}

<style>
  /* 对照表行数多（近 600 条），比通用规则弹窗再高/宽一些；其余几何仍用 app.css 的 .modal 原语 */
  :global(.modal.nat-modal) {
    width: min(1040px, 96vw);
    height: min(760px, 92vh);
  }
  .nat-heading {
    display: flex;
    align-items: baseline;
    gap: var(--sp-3);
    min-width: 0;
  }
  .nat-title {
    font-size: var(--fs-lg);
    font-weight: var(--fw-semibold);
  }
  .notice {
    color: var(--warn);
  }
  .nat-hint {
    margin: 0 0 var(--sp-3);
    color: var(--text-3);
    font-size: var(--fs-sm);
  }
  /* 搜索行/新增行/语言行：都贴着列表上方，间距紧一些，把高度留给列表 */
  .nat-toolbar {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    margin-bottom: var(--sp-2);
  }
  .nat-toolbar input {
    flex: 1;
    min-width: 0;
  }
  .nat-add {
    display: grid;
    grid-template-columns: 96px 132px minmax(0, 1fr) auto;
    gap: var(--sp-2);
    margin-bottom: var(--sp-2);
  }
  .nat-add .code {
    text-transform: uppercase;
  }
  .nat-langs {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-2);
    margin-bottom: var(--sp-3);
  }
  .nat-langs input {
    width: 170px;
  }
  /* 列表滚动区：sticky 表头 + 逐行输入框；表格宽度固定，长全称由输入框自身横向滚动 */
  .nat-list {
    flex: 1;
    min-height: 0;
    overflow: auto;
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    /* 表体用 surface、表头用 inset：与 app.css 的 table.grid 同一套明暗层次，
       避免表头在 inset 桌面上再铺一层 surface 色带、看起来像悬空的白条 */
    background: var(--bg-surface);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }
  th {
    position: sticky;
    top: 0;
    z-index: 1;
    /* 行高 = 输入框 30px + 上下 2px 内边距；表头同高，粘住滚动时不会突然变矮 */
    height: calc(var(--control-h) + 4px);
    /* 左内边距 = 单元格内边距 + 输入框边框 + 左内边距，使表头文字与行内文字左对齐 */
    padding: 0 calc(var(--sp-3) + var(--sp-4) + 1px);
    text-align: left;
    vertical-align: middle;
    font-size: var(--fs-sm);
    font-weight: var(--fw-semibold);
    color: var(--text-2);
    /* 与数据行同底色：表头不再是一条独立深色带；仍保持不透明，粘住滚动时不透出下面的行 */
    background: var(--bg-surface);
    border-bottom: 1px solid var(--hairline-2);
    user-select: none;
    white-space: nowrap;
  }
  td {
    padding: 2px var(--sp-3);
    border-bottom: 1px solid var(--hairline);
  }
  tbody tr:last-child td {
    border-bottom: none;
  }
  .code-col {
    width: 110px;
  }
  .lang-col {
    width: 150px;
  }
  .act-col {
    width: 88px;
    text-align: right;
  }
  td input,
  td select {
    width: 100%;
    min-width: 0;
  }
  .empty {
    padding: var(--sp-6);
    text-align: center;
  }
  .nat-foot {
    margin: var(--sp-2) 0 0;
  }
</style>
