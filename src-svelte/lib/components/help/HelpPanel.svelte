<!-- HelpPanel · 内置文档面板：侧栏「教程 / FAQ」入口打开同一浮层，左栏章节导航 + 右栏滚动正文，
     顶部搜索框做全文过滤与定位；内容来自 lib/help.ts，随应用版本统一维护 -->
<script lang="ts">
  import { onDestroy, tick } from "svelte";
  import { helpEntries, helpSearchText } from "../../help";
  import { tip } from "../../stores/app.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { t } from "../../text";
  import Icon from "../../ui/Icon.svelte";

  let query = $state("");
  let activeId = $state("");
  let contentEl = $state<HTMLDivElement | null>(null);
  let searchEl = $state<HTMLInputElement | null>(null);

  const tab = $derived(ui.helpTab ?? "tutorial");
  const entries = $derived(helpEntries(ui.lang, tab));
  // 搜索词统一小写做包含匹配；中文不受大小写影响，英文关键词按不区分大小写处理
  const q = $derived(query.trim().toLowerCase());
  const filtered = $derived(q ? entries.filter((e) => helpSearchText(e).includes(q)) : entries);

  // 打开面板或切换页签 / 语言时复位搜索与滚动位置；语言切换后条目整份替换，必须重新定位
  let lastOpenKey = "";
  $effect(() => {
    if (!ui.helpTab) {
      lastOpenKey = "";
      return;
    }
    const key = `${ui.helpTab}|${ui.lang}`;
    if (key === lastOpenKey) return;
    lastOpenKey = key;
    query = "";
    activeId = entries[0]?.id ?? "";
    void tick().then(() => contentEl?.scrollTo({ top: 0 }));
  });

  // 搜索词变化后定位到第一条命中；无命中时清空当前章节高亮
  let lastQuery = "";
  $effect(() => {
    if (!ui.helpTab || q === lastQuery) return;
    lastQuery = q;
    activeId = filtered[0]?.id ?? "";
    void tick().then(() => contentEl?.scrollTo({ top: 0 }));
  });

  function close() {
    ui.helpTab = null;
  }

  function jumpTo(id: string) {
    activeId = id;
    contentEl?.querySelector<HTMLElement>(`[data-help-id="${id}"]`)?.scrollIntoView({ block: "start", behavior: "smooth" });
  }

  // 滚动时用各章节顶边反查当前章节；rAF 合并高频滚动事件，避免长文档滚动卡顿
  let spyFrame = 0;
  function onScroll() {
    cancelAnimationFrame(spyFrame);
    spyFrame = requestAnimationFrame(() => {
      const c = contentEl;
      if (!c) return;
      const probe = c.getBoundingClientRect().top + 16;
      let current = filtered[0]?.id ?? "";
      for (const entry of filtered) {
        const node = c.querySelector<HTMLElement>(`[data-help-id="${entry.id}"]`);
        if (node && node.getBoundingClientRect().top <= probe) current = entry.id;
      }
      if (current) activeId = current;
    });
  }

  function onKey(e: KeyboardEvent) {
    if (!ui.helpTab) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    // Ctrl/Cmd+F 直接聚焦面板内搜索，不触发浏览器原生查找
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      searchEl?.focus();
      searchEl?.select();
    }
  }

  onDestroy(() => cancelAnimationFrame(spyFrame));

  type Segment = { text: string; hit: boolean };
  /** 把整段文字按搜索词切片，命中段交给 <mark> 渲染；用 indexOf 而非正则，避免关键词里的特殊字符被当成语法 */
  function segments(text: string): Segment[] {
    if (!q) return [{ text, hit: false }];
    const hay = text.toLowerCase();
    const out: Segment[] = [];
    let i = 0;
    let at = hay.indexOf(q, i);
    while (at !== -1) {
      if (at > i) out.push({ text: text.slice(i, at), hit: false });
      out.push({ text: text.slice(at, at + q.length), hit: true });
      i = at + q.length;
      at = hay.indexOf(q, i);
    }
    if (i < text.length) out.push({ text: text.slice(i), hit: false });
    return out;
  }
</script>

{#snippet hl(text: string)}
  {#each segments(text) as seg}{#if seg.hit}<mark>{seg.text}</mark>{:else}{seg.text}{/if}{/each}
{/snippet}

<svelte:window onkeydown={onKey} />

{#if ui.helpTab}
  <div
    class="modal-overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) close();
    }}
  >
    <div class="modal help-modal" role="dialog" aria-modal="true" aria-label={t.help.title}>
      <header class="help-head">
        <div class="help-heading">
          <span class="help-mark"><Icon name="file" size={15} /></span>
          <div>
            <div class="help-title">{t.help.title}</div>
            <div class="help-sub">{t.help.subtitle} · {t.app.brandVer}</div>
          </div>
        </div>

        <div class="seg3 help-tabs" role="tablist">
          <button
            role="tab"
            aria-selected={tab === "tutorial"}
            class:active={tab === "tutorial"}
            onclick={() => (ui.helpTab = "tutorial")}
            >{t.help.tutorialTab}</button
          >
          <button
            role="tab"
            aria-selected={tab === "faq"}
            class:active={tab === "faq"}
            onclick={() => (ui.helpTab = "faq")}
            >{t.help.faqTab}</button
          >
        </div>

        <div class="input-wrap help-search">
          <Icon name="search" />
          <input
            type="text"
            bind:this={searchEl}
            bind:value={query}
            placeholder={t.help.searchPlaceholder}
            aria-label={t.help.searchPlaceholder}
          />
          {#if query}
            <button
              type="button"
              class="search-clear"
              aria-label={t.help.close}
              onclick={() => {
                query = "";
                void tick().then(() => searchEl?.focus());
              }}
            >
              <Icon name="x" size={12} />
            </button>
          {/if}
        </div>

        <button class="icon-btn" use:tip={t.help.close} aria-label={t.help.close} onclick={close}>
          <Icon name="x" size={14} />
        </button>
      </header>

      <div class="help-body">
        <nav class="help-nav" aria-label={t.help.title}>
          {#each filtered as entry (entry.id)}
            {@const n = entries.findIndex((e) => e.id === entry.id) + 1}
            <button class="nav-item" class:current={activeId === entry.id} onclick={() => jumpTo(entry.id)}>
              <span class="nav-num">{String(n).padStart(2, "0")}</span>
              <span class="nav-label">{@render hl(entry.title)}</span>
            </button>
          {:else}
            <p class="nav-empty">{t.help.noResults}</p>
          {/each}
        </nav>

        <div class="help-content" bind:this={contentEl} onscroll={onScroll}>
          {#each filtered as entry (entry.id)}
            {@const n = entries.findIndex((e) => e.id === entry.id) + 1}
            <article class="doc-entry" data-help-id={entry.id}>
              <h2 class="doc-h">
                <span class="doc-index">{String(n).padStart(2, "0")}</span>
                <span>{@render hl(entry.title)}</span>
              </h2>
              {#if entry.summary}
                <p class="doc-summary">{@render hl(entry.summary)}</p>
              {/if}

              {#each entry.blocks as block}
                {#if block.kind === "p"}
                  <p>{@render hl(block.text)}</p>
                {:else if block.kind === "list"}
                  {#if block.ordered}
                    <ol class="doc-list">
                      {#each block.items as item}<li>{@render hl(item)}</li>{/each}
                    </ol>
                  {:else}
                    <ul class="doc-list">
                      {#each block.items as item}<li>{@render hl(item)}</li>{/each}
                    </ul>
                  {/if}
                {:else if block.kind === "note"}
                  <div class="doc-note" class:warn={block.tone === "warn"}>
                    <span class="note-label">{block.tone === "warn" ? t.help.warning : t.help.note}</span>
                    <span>{@render hl(block.text)}</span>
                  </div>
                {:else if block.kind === "table"}
                  <div class="doc-table-wrap">
                    <table>
                      <thead>
                        <tr>
                          {#each block.head as cell}<th>{@render hl(cell)}</th>{/each}
                        </tr>
                      </thead>
                      <tbody>
                        {#each block.rows as row}
                          <tr>
                            {#each row as cell}<td>{@render hl(cell)}</td>{/each}
                          </tr>
                        {/each}
                      </tbody>
                    </table>
                  </div>
                {:else if block.kind === "code"}
                  <pre class="doc-code">{block.text}</pre>
                {/if}
              {/each}
            </article>
          {:else}
            <div class="help-empty">
              <Icon name="search" size={22} />
              <p>{t.help.noResults}</p>
            </div>
          {/each}
        </div>
      </div>

      <footer class="help-foot">
        <span>{t.help.resultCount(filtered.length, entries.length)}</span>
        <span>{t.help.searchHint}</span>
      </footer>
    </div>
  </div>
{/if}

<style>
  /* 宽面板覆盖全局 .modal 的默认 900×600；scoped 类选择器优先级高于全局同名类 */
  .help-modal {
    width: min(1060px, 94vw);
    height: min(740px, 90vh);
    padding: 0;
    overflow: hidden;
  }

  .help-head {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    flex: none;
    padding: var(--sp-4) var(--sp-5);
    border-bottom: 1px solid var(--hairline);
  }

  .help-heading {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
  }

  .help-mark {
    display: grid;
    place-items: center;
    flex: none;
    width: 30px;
    height: 30px;
    border: 1px solid var(--accent-line);
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    color: var(--accent);
  }

  .help-title {
    font-size: var(--fs-lg);
    font-weight: var(--fw-semibold);
    color: var(--text-1);
    line-height: 1.35;
  }

  .help-sub {
    margin-top: 2px;
    font-size: var(--fs-xs);
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .help-tabs {
    flex: none;
    width: auto;
  }

  .help-tabs button {
    flex: none;
    padding: var(--sp-2) var(--sp-5);
  }

  .help-search {
    flex: 1 1 200px;
    max-width: 300px;
    min-width: 140px;
    margin-left: auto;
  }

  .help-search input {
    padding-right: 32px;
  }

  .search-clear {
    position: absolute;
    right: var(--sp-1);
    display: grid;
    place-items: center;
    width: var(--control-h-xs);
    height: var(--control-h-xs);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-3);
    cursor: pointer;
  }

  .search-clear:hover {
    background: var(--bg-raised);
    color: var(--text-1);
  }

  .help-body {
    display: grid;
    grid-template-columns: 228px minmax(0, 1fr);
    flex: 1;
    min-height: 0;
  }

  .help-nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
    padding: var(--sp-3);
    overflow-y: auto;
    border-right: 1px solid var(--hairline);
    background: var(--bg-inset);
  }

  .nav-item {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    flex: none;
    width: 100%;
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-2);
    font-size: var(--fs-sm);
    line-height: 1.5;
    text-align: left;
    cursor: pointer;
  }

  .nav-item:hover {
    background: var(--bg-raised);
    color: var(--text-1);
  }

  .nav-item.current {
    border-color: var(--accent-line);
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: var(--fw-medium);
  }

  .nav-num {
    flex: none;
    color: var(--text-3);
    font-family: var(--mono);
    font-size: var(--fs-xs);
    line-height: 1.7;
  }

  .nav-item.current .nav-num {
    color: var(--accent);
  }

  .nav-label {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .nav-empty {
    margin: var(--sp-2) var(--sp-3);
    color: var(--text-3);
    font-size: var(--fs-sm);
    line-height: 1.6;
  }

  .help-content {
    min-width: 0;
    padding: var(--sp-6) var(--sp-8) var(--sp-8);
    overflow-y: auto;
    scroll-behavior: smooth;
  }

  .doc-entry {
    scroll-margin-top: var(--sp-4);
  }

  .doc-entry + .doc-entry {
    margin-top: var(--sp-8);
    padding-top: var(--sp-7);
    border-top: 1px solid var(--hairline);
  }

  .doc-h {
    display: flex;
    align-items: baseline;
    gap: var(--sp-3);
    margin: 0;
    color: var(--text-1);
    font-size: var(--fs-xl);
    font-weight: var(--fw-semibold);
    line-height: 1.4;
  }

  .doc-index {
    flex: none;
    padding: 1px var(--sp-2);
    border: 1px solid var(--accent-line);
    border-radius: var(--radius-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-family: var(--mono);
    font-size: var(--fs-xs);
    font-weight: var(--fw-medium);
    line-height: 1.5;
  }

  .doc-summary {
    margin: var(--sp-2) 0 0;
    color: var(--text-2);
    font-size: var(--fs-md);
    line-height: 1.7;
  }

  .help-content p {
    margin: var(--sp-3) 0;
    color: var(--text-2);
    font-size: var(--fs-md);
    line-height: 1.8;
  }

  .doc-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    margin: var(--sp-3) 0;
    padding-left: var(--sp-6);
    color: var(--text-2);
    font-size: var(--fs-md);
    line-height: 1.8;
  }

  .doc-list li::marker {
    color: var(--accent);
  }

  .doc-note {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    margin: var(--sp-4) 0;
    padding: var(--sp-3) var(--sp-4);
    border-left: 3px solid var(--accent);
    border-radius: 0 var(--radius-md) var(--radius-md) 0;
    background: var(--accent-soft);
    color: var(--text-2);
    font-size: var(--fs-md);
    line-height: 1.7;
  }

  .doc-note.warn {
    border-left-color: var(--warn);
    background: var(--warn-soft);
  }

  .note-label {
    flex: none;
    color: var(--accent);
    font-weight: var(--fw-semibold);
  }

  .doc-note.warn .note-label {
    color: var(--warn);
  }

  .doc-table-wrap {
    margin: var(--sp-4) 0;
    overflow-x: auto;
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
  }

  .doc-table-wrap table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-sm);
  }

  .doc-table-wrap th {
    padding: var(--sp-2) var(--sp-3);
    border-bottom: 1px solid var(--hairline-2);
    background: var(--bg-raised);
    color: var(--text-1);
    font-weight: var(--fw-medium);
    text-align: left;
    white-space: nowrap;
  }

  .doc-table-wrap td {
    padding: var(--sp-2) var(--sp-3);
    border-top: 1px solid var(--hairline);
    color: var(--text-2);
    line-height: 1.6;
    vertical-align: top;
    overflow-wrap: anywhere;
  }

  .doc-table-wrap tr:first-child td {
    border-top: none;
  }

  .doc-code {
    margin: var(--sp-4) 0;
    padding: var(--sp-4) var(--sp-5);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
    color: var(--text-2);
    font-family: var(--mono);
    font-size: var(--fs-sm);
    line-height: 1.65;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .help-content mark {
    padding: 0 1px;
    border-radius: 2px;
    background: var(--accent-soft);
    color: var(--accent);
  }

  .help-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    flex-direction: column;
    gap: var(--sp-3);
    height: 100%;
    min-height: 220px;
    color: var(--text-3);
    font-size: var(--fs-md);
  }

  .help-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
    flex: none;
    padding: var(--sp-2) var(--sp-5);
    border-top: 1px solid var(--hairline);
    color: var(--text-3);
    font-size: var(--fs-xs);
  }

  @media (max-width: 900px) {
    .help-body {
      grid-template-columns: 196px minmax(0, 1fr);
    }

    .help-content {
      padding: var(--sp-6);
    }
  }

  @media (max-width: 760px) {
    .help-modal {
      width: 96vw;
      height: 92vh;
    }

    .help-head {
      flex-wrap: wrap;
      row-gap: var(--sp-3);
    }

    .help-search {
      order: 4;
      flex: 1 1 100%;
      max-width: none;
    }

    .help-body {
      display: flex;
      flex-direction: column;
    }

    .help-nav {
      flex: none;
      flex-direction: row;
      max-height: 52px;
      padding: var(--sp-2) var(--sp-3);
      overflow-x: auto;
      overflow-y: hidden;
      border-right: none;
      border-bottom: 1px solid var(--hairline);
    }

    .nav-item {
      align-items: center;
      width: auto;
      flex: none;
    }

    .nav-label {
      max-width: 180px;
      overflow: hidden;
      white-space: nowrap;
      text-overflow: ellipsis;
    }

    .help-content {
      padding: var(--sp-5) var(--sp-4) var(--sp-7);
    }
  }
</style>
