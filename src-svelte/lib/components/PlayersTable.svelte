<!-- 名单预览表格：分页、搜索、点表头排序；数据留在后端，每次只拉当前页。
     判据与快照预览同款：后端名单缓存还没被填上（既没「扫描RTF」也没跑过替换）时不渲染工具条
     （搜索框/筛选/分页器），只留引导空态。 -->
<script lang="ts">
  import { api } from "../api";
  import { appStore } from "../stores/app.svelte";
  import { loadImageBlobUrl } from "../images";
  import { t } from "../text"; // 全局文案常量
  import { icons } from "../icons";
  import TablePager from "./TablePager.svelte"; // 分页器（与快照预览共用）
  import AvatarPopover, {
    type AvatarEntry,
    type AvatarPopoverApi,
  } from "./AvatarPopover.svelte"; // 头像预览气泡（与快照预览共用）
  import type { PageResult, PlayerRecord } from "../types";

  const PAGE_SIZE = 8; // 每页条数（单页最多 8 行）
  let page = $state(0); // 当前页码（从 0 开始）
  let search = $state(""); // 搜索词
  let sortKey = $state("uid"); // 排序列
  let sortDesc = $state(false); // 是否降序
  let result = $state<PageResult<PlayerRecord> | null>(null); // 当前页结果
  let searchTimer: ReturnType<typeof setTimeout> | undefined; // 防抖定时器句柄

  // 随机人筛选（仅拓展名单显示）：'any'=任意（默认）/ 'yes'=仅随机人 / 'no'=仅真实球员。
  let newganFilter = $state<"any" | "yes" | "no">("any");

  // 是否为拓展名单：跟随最近一次「扫描RTF」的识别结果（detailed=true 即拓展球员/职员名单）。
  const isExtRoster = $derived(appStore.lastRtfScan?.detailed ?? false);
  // 发往后端的生效筛选：非拓展名单（下拉被隐藏）时强制按「任意」，避免旧筛选残留在请求里。
  const newganQuery = $derived<"any" | "yes" | "no">(
    isExtRoster ? newganFilter : "any",
  );

  // 总页数：由总数除以每页数向上取整（至少 1 页）。
  const totalPages = $derived(
    result ? Math.max(1, Math.ceil(result.total / PAGE_SIZE)) : 1,
  );

  // 可读态：与快照预览同款判据——只认 appStore 里记录「后端名单缓存被填上」的标记。
  // 填充者有两处：「扫描RTF」成功，以及替换结束（成功填上/失败取消清空），都由 store 统一维护。
  // 未填充过时工具条（搜索框/筛选/分页器）整块不渲染，只留引导空态。
  const readable = $derived(appStore.rosterReady);

  /** 拉当前页数据（把页码/搜索/随机人筛选/排序传给后端）。 */
  async function load() {
    result = await api.queryPlayers({
      offset: page * PAGE_SIZE,
      limit: PAGE_SIZE,
      search,
      sortKey,
      sortDesc,
      newgan: newganQuery === "any" ? null : newganQuery === "yes",
    });
  }
  // 参数变化自动刷新：显式读取依赖以建立追踪，请求做 300ms 防抖。
  // rosterVersion：名单缓存被填充/清空时 +1（「扫描RTF」成功、替换结束），据此重载当前页。
  $effect(() => {
    void page;
    void search;
    void sortKey;
    void sortDesc;
    void newganQuery;
    void appStore.rosterVersion;
    clearTimeout(searchTimer); // 取消上一次未触发的请求
    searchTimer = setTimeout(load, 300); // 300ms 后才真正请求（防抖：避免边输入边频繁请求）
    return () => clearTimeout(searchTimer); // 清理函数：依赖变化/组件销毁时取消定时器
  });

  /** 点表头排序：同一列则切换升/降序，不同列则切到该列并重置为升序。 */
  function sortBy(key: string) {
    if (sortKey === key) {
      sortDesc = !sortDesc;
    } else {
      sortKey = key;
      sortDesc = false;
    }
  }
  // 表头箭头指示：当前排序列显示 ↑/↓，其余列为空。
  const arrow = (key: string) =>
    sortKey === key ? (sortDesc ? " ↓" : " ↑") : "";

  // 是否含详细信息：当前页任一条有 detailed 就额外渲染那几列（适配基础/详细两种名单）。
  const hasDetailed = $derived(!!result?.items.some((r) => r.detailed));
  // 性别列：仅当当前页有记录带性别（拓展职员/新版拓展球员）才渲染，旧版 17 列名单不占位。
  const hasGender = $derived(!!result?.items.some((r) => r.detailed?.gender));

  /** 随机人列的标签配色：中文名单值为「是/否」，英文名单兼容 Yes/No；其余值原样显示不着色。 */
  function newganCls(v: string) {
    if (/否|no/i.test(v)) return "tag-no";
    if (/是|yes/i.test(v)) return "tag-yes";
    return "";
  }

  // 第 15 列语义：按扫描摘要的格式名区分球员（所在地）/职员（偏好工作），格式级信号比逐页猜值更可靠。
  const isStaffRoster = $derived(
    appStore.lastRtfScan?.format.includes("职员") ?? false,
  );
  const basedHeader = $derived(
    isStaffRoster ? t.table.colJob : t.table.colBased,
  );
  /** 第 15 列取值：球员读 based，职员读 preferredJob（另一个在解析侧恒为空串）。 */
  const basedValue = (r: PlayerRecord) =>
    (isStaffRoster ? r.detailed?.preferredJob : r.detailed?.based) ?? "";

  // ---------------- 头像列：悬停/点击「预览」气泡（气泡本体在 AvatarPopover 共用组件内） ----------------
  /** 气泡取头像：问后端要磁盘路径，再读字节包成 blob URL（取不到/出错由组件侧统一兜成 err 文案）。 */
  async function resolveAvatarEntry(name: string): Promise<AvatarEntry> {
    const path = await api.resolveAvatar(name);
    if (!path) return { url: null, err: t.table.avatarNotFound };
    return { url: await loadImageBlobUrl(path), err: "" };
  }

  // 头像气泡实例（bind:this）：行内触发按钮把鼠标/键盘事件接到它的 open/scheduleClose/close 上。
  let avatarPop = $state<AvatarPopoverApi | null>(null);
</script>

<!-- 工具条：搜索框（放大镜图标）+ 随机人筛选 + 总条数 + 右侧分页；缓存未填上时不渲染
     （与快照预览的 {#if readable} 同款）。组件根是纵向 flex，撑满卡片正文高度。 -->
<div class="table-pane">
  {#if readable}
    <div class="table-toolbar">
      <div class="search">
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          aria-hidden="true">{@html icons.search.body}</svg
        >
        <input
          type="search"
          placeholder={t.table.searchPlaceholder}
          bind:value={search}
          oninput={() => (page = 0)}
          aria-label={t.table.searchPlaceholder}
        />
      </div>
      <!-- 随机人筛选：仅拓展名单显示（默认「任意」）；切换后回到第 1 页再查询。 -->
      {#if isExtRoster}
        <label class="newgan-filter">
          <span class="muted small">{t.table.newganFilterLabel}</span>
          <select
            bind:value={newganFilter}
            onchange={() => (page = 0)}
            aria-label={t.table.newganFilterLabel}
          >
            <option value="any">{t.table.newganAny}</option>
            <option value="yes">{t.table.newganYes}</option>
            <option value="no">{t.table.newganNo}</option>
          </select>
        </label>
      {/if}
      {#if result}<span class="muted small record-count"
          >{t.table.recordCount(result.total)}</span
        >{/if}
      <span class="spacer"></span>
      <!-- 分页器（含快速跳页）与快照预览共用同一组件，样式/行为单点维护。 -->
      <TablePager bind:page {totalPages} />
    </div>
  {/if}

  <!-- 缓存未填上 → 引导空态；有数据 → 表格；已填上但当前筛选无结果 → 提示文案（与快照预览同款三层结构）。 -->
  {#if !readable}
    <div class="empty">{t.table.emptyHint}</div>
  {:else if result && result.items.length > 0}
    <div class="table-scroll">
      <table class="grid">
        <!-- 列宽：长文本列（姓名/俱乐部/所在地）给宽余量吃剩余空间；码值/数值列收窄并居中。 -->
        <colgroup>
          <col class="w-uid" />
          <col class="w-nat" />
          <col class="w-nat2" />
          <col class="w-name" />
          <col class="w-code" />
          <col class="w-code" />
          <col class="w-code" />
          {#if hasDetailed}
            <col class="w-code" />
            <col class="w-avatar" />
            <col class="w-tag" />
            {#if hasGender}<col class="w-code" />{/if}
            <col class="w-code" />
            <col class="w-height" />
            <col class="w-club" />
            <col class="w-based" />
            <col class="w-code" />
            <col class="w-code" />
            <col class="w-rca" />
          {/if}
        </colgroup>
        <thead>
          <tr>
            <!-- 可排序的表头绑 onclick=sortBy；arrow() 拼接升降序箭头。
                 排序键（'uid'/'primaryNat'/'name'/'ethnicityCode'）是发往后端的协议值，勿改。
                 static 类=不可排序列，去掉手型指针；c 类=紧凑列居中。 -->
            <th onclick={() => sortBy("uid")}>{t.table.colUid}{arrow("uid")}</th
            >
            <th onclick={() => sortBy("primaryNat")}
              >{t.table.colNat}{arrow("primaryNat")}</th
            >
            <th class="static">{t.table.col2ndNat}</th>
            <th onclick={() => sortBy("name")}
              >{t.table.colName}{arrow("name")}</th
            >
            <th class="static c">{t.table.colHair}</th>
            <th class="static c">{t.table.colColor}</th>
            <th onclick={() => sortBy("ethnicityCode")} class="c"
              >{t.table.colEth}{arrow("ethnicityCode")}</th
            >
            <!-- 仅拓展名单渲染第 8 列起；顺序：肤色/头像/随机人/性别/年龄/身高/俱乐部/所在地(偏好工作)/能力/潜力/R.C.A。 -->
            {#if hasDetailed}
              <th class="static c">{t.table.colSkin}</th>
              <th class="static c">{t.table.colAvatar}</th>
              <th class="static c">{t.table.colNewgan}</th>
              {#if hasGender}<th class="static c">{t.table.colGender}</th>{/if}
              <th class="static c">{t.table.colAge}</th>
              <th class="static c">{t.table.colHeight}</th>
              <th class="static">{t.table.colClub}</th>
              <th class="static">{basedHeader}</th>
              <th class="static c">{t.table.colCa}</th>
              <th class="static c">{t.table.colPa}</th>
              <th class="static c">{t.table.colRca}</th>
            {/if}
          </tr>
        </thead>
        <tbody>
          <!-- 以 uid 为键，便于 Svelte 复用 DOM 行。 -->
          {#each result.items as r (r.uid)}
            <tr>
              <td class="mono">{r.uid}</td>
              <td>{r.primaryNat}</td>
              <td>{r.secNat}</td>
              <td>{r.name}</td>
              <td class="c">{r.hairLength}</td>
              <td class="c">{r.hairColor}</td>
              <td class="c">{r.ethnicityCode}</td>
              <!-- 详细列：空值显示 —；头像列有值才出现「预览」小按钮，无值留空（不放占位符）。 -->
              {#if hasDetailed}
                {@const d = r.detailed}
                <td class="c">{d?.skinCode || "—"}</td>
                <td class="c">
                  {#if d?.avatar}
                    <button
                      type="button"
                      class="avatar-btn"
                      aria-label={t.table.avatarAria}
                      onmouseenter={(e) =>
                        avatarPop?.open(d.avatar, e.currentTarget, false)}
                      onmouseleave={avatarPop?.scheduleClose}
                      onfocus={(e) =>
                        avatarPop?.open(d.avatar, e.currentTarget, false)}
                      onblur={avatarPop?.scheduleClose}
                      onclick={(e) =>
                        avatarPop?.open(d.avatar, e.currentTarget, true)}
                    >
                      {t.table.avatarBtn}
                    </button>
                  {/if}
                </td>
                <td class="c">
                  {#if d?.isNewgan}
                    <span class={newganCls(d.isNewgan)}>{d.isNewgan}</span>
                  {:else}—{/if}
                </td>
                {#if hasGender}<td class="c">{d?.gender || "—"}</td>{/if}
                <td class="c">{d?.age || "—"}</td>
                <td class="c">{d?.height || "—"}</td>
                <td title={d?.club ?? ""}>{d?.club || "—"}</td>
                <td title={basedValue(r)}>{basedValue(r) || "—"}</td>
                <td class="c">{d?.ca || "—"}</td>
                <td class="c">{d?.pa || "—"}</td>
                <td class="c">{d?.rca || "—"}</td>
              {/if}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if result}
    <div class="empty">{search ? t.table.noMatchHint : t.table.emptyHint}</div>
  {/if}

  <!-- 头像预览气泡（共用组件）：缓存/钉住/延迟关闭/Esc 都在组件内；groupKey=当前组，切组自动失效。 -->
  <AvatarPopover
    bind:this={avatarPop}
    resolve={resolveAvatarEntry}
    groupKey={appStore.snapshot?.currentGroup}
  />
</div>

<style>
  /* 撑满父级 .card-body：工具条定高 + 表格滚动区吃剩余高度（粘顶表头在其中生效） */
  .table-pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    gap: 2px;
  }
  /* 随机人筛选：横向排列的“标签+下拉”，下拉用紧凑尺寸（全局 select 默认 width:100%） */
  .newgan-filter {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .newgan-filter select {
    width: auto;
    padding: 6px 26px 6px 8px;
  }

  /* ---- 列宽（auto 布局下为建议宽度，姓名/俱乐部/所在地会分吃剩余空间；总宽超出则横向滚动） ---- */
  .w-uid {
    width: 96px;
  }
  .w-nat {
    width: 56px;
  }
  .w-nat2 {
    width: 72px;
  }
  .w-name {
    width: 140px;
  }
  .w-code {
    width: 46px;
  }
  .w-avatar {
    width: 62px;
  }
  .w-tag {
    width: 64px;
  }
  .w-height {
    width: 76px;
  }
  .w-club {
    width: 150px;
  }
  .w-based {
    width: 178px;
  }
  .w-rca {
    width: 56px;
  }
  /* 不可排序列：去掉全局 table.grid th 的手型指针 */
  th.static {
    cursor: default;
  }
  /* 紧凑码值/数值列内容居中（text-align 不能设在 col 上，必须落在单元格） */
  th.c,
  td.c {
    text-align: center;
  }

  /* 头像列「预览」小按钮：胶囊样式，悬停描边变实色 */
  .avatar-btn {
    appearance: none;
    cursor: pointer;
    border: 1px solid transparent;
    border-radius: 99px;
    padding: 2px 12px;
    font-size: 11.5px;
    line-height: 1.6;
    font-family: inherit;
    color: var(--accent);
    background: var(--accent-soft);
    transition:
      border-color 120ms ease,
      background-color 120ms ease;
  }
  .avatar-btn:hover {
    border-color: var(--accent);
  }
</style>
