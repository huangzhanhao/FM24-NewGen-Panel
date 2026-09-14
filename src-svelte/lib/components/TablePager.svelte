<!-- 表格分页器（名单预览/快照预览共用）：‹/› 翻页 + 「第 X / Y 页」，
     细分隔线后是「前往 [输入框] 页 [跳转]」快速跳页。两处共用同一组件保证样式与行为一致；
     page 从 0 计数、双向绑定；文案复用 t.table.* 的分页文案。 -->
<script lang="ts">
  import { t } from "../text"; // 全局文案常量
  import { icons } from "../icons";

  // page：当前页码（从 0 开始，双向绑定）；totalPages：总页数（至少 1，由父组件按总条数算出）。
  let { page = $bindable(0), totalPages = 1 }: { page: number; totalPages?: number } = $props();

  // 快速跳页输入框的文本（1 基页码）；随 page 变化自动同步为当前页（含夹取后的值）。
  let jumpInput = $state("1");
  $effect(() => {
    jumpInput = String(page + 1);
  });

  /** 快速跳页：解析输入的 1 基页码，夹取到 [1, totalPages] 后切换；非法输入还原为当前页。 */
  function gotoPage() {
    const n = Number.parseInt(jumpInput, 10);
    if (!Number.isFinite(n)) {
      jumpInput = String(page + 1);
      return;
    }
    page = Math.min(Math.max(n, 1), totalPages) - 1;
  }
</script>

<div class="pager">
  <button
    class="btn"
    onclick={() => (page = 0)}
    disabled={page === 0}
    title={t.table.prevPage}
  >
    <svg
      viewBox="0 0 24 24"
      width="14"
      height="14"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true">{@html icons.chevronLeft.body}</svg
    >
  </button>
  <span>{t.table.pageOf(page + 1, totalPages)}</span>
  <button
    class="btn"
    onclick={() => (page += 1)}
    disabled={page + 1 >= totalPages}
    title={t.table.nextPage}
  >
    <svg
      viewBox="0 0 24 24"
      width="14"
      height="14"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true">{@html icons.chevronRight.body}</svg
    >
  </button>
  <!-- 快速跳页：输入 1 基页码后回车或点「跳转」；gotoPage 会夹取到 [1, 总页数]。 -->
  <span class="pager-jump">
    <span>{t.table.gotoPrefix}</span>
    <input
      class="jump-input mono"
      type="text"
      inputmode="numeric"
      bind:value={jumpInput}
      onkeydown={(e) => {
        if (e.key === "Enter") gotoPage();
      }}
      aria-label={t.table.gotoAria}
    />
    <span>{t.table.gotoSuffix}</span>
    <button class="btn" onclick={gotoPage}>{t.table.gotoBtn}</button>
  </span>
</div>

<style>
  /* 分页器：整组始终排在同一行（nowrap）并靠右；flex-shrink: 0 让它不被压缩，
     窗口变窄时由左侧搜索框收缩让位（见 app.css 的 .search min-width），
     避免分页器自己被挤成上下两行。 */
  .pager {
    flex-wrap: nowrap;
    flex-shrink: 0;
    justify-content: flex-end;
    gap: 6px;
  }
  /* 快速跳页组：与「第 X/Y 页」之间用细分隔线区隔，整组不换行 */
  .pager-jump {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
    margin-left: 2px;
    padding-left: 8px;
    border-left: 1px solid var(--hairline);
  }
  /* 页码输入框：紧凑居中（覆盖全局 text 输入框的 7px 纵向内边距与默认宽度） */
  .pager-jump .jump-input {
    width: 44px;
    min-width: 0;
    padding: 3px 6px;
    text-align: center;
    font-size: 12px;
  }
</style>
