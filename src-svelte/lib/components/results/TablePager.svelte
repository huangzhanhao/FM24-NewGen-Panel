<!-- TablePager · 表格分页器（名单预览/快照预览共用）：‹/› 翻页 + 「第 X / Y 页」+ 快速跳页
     细分隔线后是「前往 [输入框] 页 [跳转]」；两处共用同一组件保证样式与行为一致
     page 从 0 计数、双向绑定 -->
<script lang="ts">
  import { t } from "../../text"; // 全局文案常量
  import { tip } from "../../stores/app.svelte";
  import Icon from "../../ui/Icon.svelte";

  // totalPages 至少为 1，由父组件按总条数算出
  let { page = $bindable(0), totalPages = 1 }: { page: number; totalPages?: number } = $props();

  // 跳页输入框文本（1 基页码），随 page 自动同步为当前页（含夹取后的值）
  let jumpInput = $state("1");
  $effect(() => {
    jumpInput = String(page + 1);
  });

  /** 快速跳页：解析 1 基页码，夹取到 [1, totalPages] 后切换；非法输入还原为当前页 */
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
    onclick={() => (page = Math.max(0, page - 1))}
    disabled={page === 0}
    use:tip={t.table.prevPage}
  >
    <Icon name="chevronLeft" size={14} />
  </button>
  <span>{t.table.pageOf(page + 1, totalPages)}</span>
  <button
    class="btn"
    onclick={() => (page += 1)}
    disabled={page + 1 >= totalPages}
    use:tip={t.table.nextPage}
  >
    <Icon name="chevronRight" size={14} />
  </button>
  <!-- 快速跳页：1 基页码，回车或点「跳转」提交，夹取规则见 gotoPage -->
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
     窗口变窄时由左侧搜索框收缩让位（.search 的 min-width 在 app.css），
     避免分页器自己被挤成上下两行 */
  .pager {
    flex-wrap: nowrap;
    flex-shrink: 0;
    justify-content: flex-end;
    gap: var(--sp-2);
  }
  /* 快速跳页组：与「第 X/Y 页」之间用细分隔线区隔，整组不换行；
     分隔线余量收到最紧（sp-2），给工具栏「一排放下」让几像素 */
  .pager-jump {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    white-space: nowrap;
    padding-left: var(--sp-2);
    border-left: 1px solid var(--hairline);
  }
  /* 页码输入框：紧凑居中（覆盖全局 text 输入框的纵向内边距、默认宽度与 30px 控件高度） */
  .pager-jump .jump-input {
    width: 44px;
    min-width: 0;
    min-height: 0;
    padding: 3px var(--sp-2);
    text-align: center;
    font-size: var(--fs-sm);
  }
</style>
