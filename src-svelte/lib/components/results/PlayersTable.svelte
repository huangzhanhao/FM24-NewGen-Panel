<!-- 名单预览表格：分页、搜索、点表头排序；数据留在后端，每次只拉当前页。
     判据与快照预览同款：后端名单缓存还没被填上（既没「扫描RTF」也没跑过替换）时不渲染工具条
     （搜索框/筛选/分页器），只留引导空态。 -->
<script lang="ts">
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog"; // 查询失败要弹窗（原来裸 await 的 rejection 会静默吞掉错误）
  import { message } from "@tauri-apps/plugin-dialog"; // 「加入队列」成功提示（与 ImageAiPanel 保存成功同款）
  import { appStore } from "../../stores/app.svelte";
  import {
    enqueuePlayers,
    imageaiStore,
    setSourcePlayer,
    toggleSelect,
    toggleSelectPage,
  } from "../../stores/imageai.svelte"; // 勾选与批量入队（队列动作转发见 store）
  import { createAvatarBlobs } from "../../avatarBlobs.svelte"; // 头像 blob 加载器（三个标签页共用）
  import { t } from "../../text"; // 全局文案常量
  import { icons } from "../../ui/icons";
  import TablePager from "./TablePager.svelte"; // 分页器（与快照预览共用）
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 头像缩略图 + 放大气泡（与快照预览共用）
  import type { PageResult, PlayerRecord } from "../../types";

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

  let queryToken = 0; // 分页查询令牌：翻页/改筛选会乱序返回，过期响应不得写回 result
  /** 拉当前页数据（把页码/搜索/随机人筛选/排序传给后端）。 */
  async function load() {
    const token = ++queryToken;
    try {
      const got = await api.queryPlayers({
        offset: page * PAGE_SIZE,
        limit: PAGE_SIZE,
        search,
        sortKey,
        sortDesc,
        newgan: newganQuery === "any" ? null : newganQuery === "yes",
      });
      // 期间又发起了新的查询：这份过期结果整份作废（连头像也别取——它读的是同一批 UID）。
      // 不拦的话表体停在第 N 页、分页器却显示第 M 页，用户按页码翻回去还是错的。
      if (token !== queryToken) return;
      result = got;
    } catch (e) {
      if (token !== queryToken) return;
      result = null;
      await showError(e);
      return;
    }
    void loadAvatars(); // 本页头像另走一次批量查询（路径出处是后端快照/映射缓存）
  }
  // 名单版本变化（「扫描RTF」成功 / 替换结束）：回到第 1 页并清空搜索、排序、随机人筛选。
  // 与 SnapshotPreview 同款口径——新名单从头看，旧页码/旧筛选不残留；下方加载 effect
  // 已追踪 rosterVersion，会自动按重置后的参数重新请求。
  $effect(() => {
    void appStore.rosterVersion;
    result = null;
    page = 0;
    search = "";
    sortKey = "uid";
    sortDesc = false;
    newganFilter = "any";
    blobs.clear(); // 旧名单的缩略图一并回收（新名单的图在 load() 后重取）
    avatarPaths = {};
    queryToken++; // 使仍在途的旧名单分页响应作废（换名单后它不该再把上一页塞回表体）
    avatarToken++; // 使仍在途的旧名单路径响应作废（不能只靠 clear，它在途完成还会回写）
  });

  // 参数变化自动刷新：显式读取依赖以建立追踪，请求做 300ms 防抖。
  // rosterVersion：名单缓存被填充/清空时 +1（「扫描RTF」成功、替换结束），据此重载当前页。
  // snapshotVersion：快照缓存被填充时 +1（「读取快照」成功、替换收尾同步），头像列据此重取
  // ——它按 UID 查的就是快照缓存，读完快照后头像要能自己补上，不必等用户翻页。
  $effect(() => {
    void page;
    void search;
    void sortKey;
    void sortDesc;
    void newganQuery;
    void appStore.rosterVersion;
    void appStore.snapshotVersion;
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
  /** 可排序表头的键盘操作：Enter/Space 等价点击（th 本身不是按钮，
   *  靠 role=button + tabindex=0 + 本处理函数保证键盘可达）。 */
  function onSortKey(e: KeyboardEvent, key: string) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      sortBy(key);
    }
  }

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

  // ---------------- 勾选列与「加入队列」（P2 Task5） ----------------
  // 勾选状态存 imageaiStore.selectedInTable（key=uid，翻页/切组/换名单都不清空——勾选跟随球员）。
  // 【按钮口径】「加入队列（N）」的 N 与可用性都按**当前页勾选数**：提交动作永远只针对用户
  // 眼前这一页，语义最直白（store.selectedCount 是含跨页勾选的全局口径，不在这里用——
  // 避免按钮显示 5 实际提交了别页 12 条的错位）。勾选本身跨页保留，翻回来仍是勾上的。
  const pageSelectedCount = $derived(
    result
      ? result.items.filter((r) => imageaiStore.selectedInTable[r.uid]).length
      : 0,
  );
  const allPageSelected = $derived(
    (result?.items.length ?? 0) > 0 &&
      result!.items.every((r) => imageaiStore.selectedInTable[r.uid]),
  );
  const somePageSelected = $derived(
    !!result?.items.some((r) => imageaiStore.selectedInTable[r.uid]),
  );

  /** 表头全选/取消：只作用于当前页数据行（翻页后另一页不受影响）。 */
  function onPageAll(v: boolean) {
    toggleSelectPage(
      (result?.items ?? []).map((r) => r.uid),
      v,
    );
  }
  /** 把当前页勾选的球员按工作台当前参数加入批量队列；成功后弹提示（条数）。 */
  async function onEnqueue() {
    const rows = (result?.items ?? [])
      .filter((r) => imageaiStore.selectedInTable[r.uid])
      .map((r) => ({ uid: r.uid, name: r.name }));
    if (rows.length === 0) return;
    try {
      const n = await enqueuePlayers(rows);
      await message(t.gen.queue.enqueued(n), { title: t.dialog.info });
    } catch (e) {
      await showError(e); // 缺模型/占位符未填充等组装错误也走统一弹窗
    }
  }

  // ---------------- 行尾「送生图」（P3 数据源入口） ----------------
  /** 把该行球员设为生图数据源并切到 AI 生图 Tab。切 Tab 走 appStore.resultsView
   *  （结果区 Tab 的全局态，「读取快照」后跨组件切换用的同一机制，最小侵入）。 */
  function sendToGenerate(r: PlayerRecord) {
    setSourcePlayer(r.uid, r.name);
    appStore.resultsView = "generate";
  }

  // 第 15 列语义：按后端结构化协议码区分球员（所在地）/职员（偏好工作），不依赖中文展示文案。
  const isStaffRoster = $derived(
    appStore.lastRtfScan?.formatCode === "extStaff",
  );
  const basedHeader = $derived(
    isStaffRoster ? t.table.colJob : t.table.colBased,
  );
  /** 第 15 列取值：球员读 based，职员读 preferredJob（另一个在解析侧恒为空串）。 */
  const basedValue = (r: PlayerRecord) =>
    (isStaffRoster ? r.detailed?.preferredJob : r.detailed?.based) ?? "";

  // ---------------- 头像列：按球员 UID 从后端快照/映射缓存取头像图 ----------------
  // 【为什么按 UID 查】RTF 名单的「头像」列原文导出时恒为空（见 rtf_scanner 的 EXT_AVATAR_COL），
  // 头像只能按球员 UID 反查映射；解析（映射缓存 → 存档快照 → 快照文件）由命令层 query_avatars
  // 完成，出处与单人预览同源。
  // 【分工】本组件只负责「查这一页的路径」；读字节/建 blob/回收交共用的 createAvatarBlobs，
  // 缩略图与放大气泡交共用的 AvatarThumb（两处能力都与快照预览同一份实现）。
  let avatarPaths = $state<Record<string, string | null>>({}); // uid → 头像图片绝对路径（null = 无头像）
  let avatarToken = 0; // 路径查询令牌：翻页/换名单后使在途的旧响应作废
  const blobs = createAvatarBlobs(); // 本页缩略图的 blob 账（翻页换代、卸载回收都在其内部）

  let avatarError = $state(""); // 批量取路径失败的原因（挂在「头像」列头提示，见 loadAvatars）
  /** 取当前页头像：批量问后端要路径，再交给 blob 加载器读成可显示的缩略图。 */
  async function loadAvatars() {
    const token = ++avatarToken;
    const items = result?.items ?? [];
    // 空页或基础名单（非拓展格式，表格不渲染头像列）：只回收上一批，不必往返。
    if (items.length === 0 || !items.some((r) => r.detailed)) {
      avatarPaths = {};
      avatarError = "";
      blobs.clear();
      return;
    }
    let paths: Record<string, string | null>;
    try {
      paths = await api.queryAvatars(items.map((r) => r.uid));
    } catch (e) {
      // 头像列是辅助展示：取数失败就留白（也不弹窗抢焦点），但不留上一批的旧图。
      // 【批级失败必须留痕】单张图取不到本来就静默，所以「命令挂了/载荷形状变了/没选头像包」
      // 与「这份名单真的没脸」绝不能长成同一个样子——否则整列空了，用户没有任何线索可查。
      // 逐图静默保留，只在列头上挂一句原因。
      avatarPaths = {};
      avatarError = String(e);
      blobs.clear();
      return;
    }
    if (token !== avatarToken) return; // 期间已翻页/换名单：过期结果直接丢弃，不建 URL
    avatarError = "";
    avatarPaths = paths;
    // 只有确实找到图的条目才交给加载器；无映射/图片不在包内的留在占位。
    const wanted: Record<string, string> = {};
    for (const it of items) {
      const p = paths[it.uid];
      if (p) wanted[it.uid] = p;
    }
    void blobs.load(wanted);
  }
</script>

{#snippet sortIcon(key: string)}
  <!-- 当前排序列的方向指示：统一线性 chevron 图标（替代旧 ↑/↓ 字符） -->
  {#if sortKey === key}
    <svg
      class="sort-ic"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2.4"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
      >{@html (sortDesc ? icons.chevronDown : icons.chevronUp).body}</svg
    >
  {/if}
{/snippet}

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
      <!-- 加入队列：按当前页勾选数计数（见脚本内口径注释），无勾选时禁用。
           名单缓存可读但本页无数据行时同样无勾选，禁用态即引导。 -->
      <button
        class="btn sm"
        disabled={pageSelectedCount === 0}
        onclick={onEnqueue}
      >
        {t.gen.queue.addBtn(pageSelectedCount)}
      </button>
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
    <div class="empty">
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true">{@html icons.folder.body}</svg
      >
      <p>{t.table.emptyHint}</p>
    </div>
  {:else if result && result.items.length > 0}
    <div class="table-scroll">
      <table class="grid">
        <!-- 列宽：长文本列（姓名/俱乐部/所在地）给宽余量吃剩余空间；码值/数值列收窄并居中。
             首列是勾选框（P2 Task5）：不可排序、不参与筛选，宽度固定；
             末列是「送生图」（P3 数据源入口）：基础/拓展名单都有，固定窄列。 -->
        <colgroup>
          <col class="w-sel" />
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
          <col class="w-gen" />
        </colgroup>
        <thead>
          <tr>
            <!-- 可排序的表头绑 onclick=sortBy，sortIcon snippet 渲染升降序 chevron；
                 role=button + tabindex=0 + onkeydown 让键盘用户也能排序。
                 排序键（'uid'/'primaryNat'/'name'/'ethnicityCode'）是发往后端的协议值，勿改。
                 static 类=不可排序列，默认光标；c 类=紧凑列居中。 -->
            <!-- 首列：勾选框。表头复选 = 全选/取消当前页数据行（indeterminate 表达部分勾选）；
                 aria-label 提供读屏文案（不造 label/for，避免为每行生成唯一 id）。 -->
            <th class="static c sel-col">
              <input
                type="checkbox"
                aria-label={t.gen.queue.selectAllPlayersAria}
                checked={allPageSelected}
                indeterminate={somePageSelected && !allPageSelected}
                onchange={(e) => onPageAll(e.currentTarget.checked)}
              />
            </th>
            <th
              role="button"
              tabindex="0"
              onclick={() => sortBy("uid")}
              onkeydown={(e) => onSortKey(e, "uid")}
              title={t.table.colUid}
              >{t.table.colUid}{@render sortIcon("uid")}</th
            >
            <th
              role="button"
              tabindex="0"
              onclick={() => sortBy("primaryNat")}
              onkeydown={(e) => onSortKey(e, "primaryNat")}
              title={t.table.colNat}
              >{t.table.colNat}{@render sortIcon("primaryNat")}</th
            >
            <th class="static">{t.table.col2ndNat}</th>
            <th
              role="button"
              tabindex="0"
              onclick={() => sortBy("name")}
              onkeydown={(e) => onSortKey(e, "name")}
              title={t.table.colName}
              >{t.table.colName}{@render sortIcon("name")}</th
            >
            <th class="static c">{t.table.colHair}</th>
            <th class="static c">{t.table.colColor}</th>
            <th
              role="button"
              tabindex="0"
              class="c"
              onclick={() => sortBy("ethnicityCode")}
              onkeydown={(e) => onSortKey(e, "ethnicityCode")}
              title={t.table.colEth}
              >{t.table.colEth}{@render sortIcon("ethnicityCode")}</th
            >
            <!-- 仅拓展名单渲染第 8 列起；顺序：肤色/头像/随机人/性别/年龄/身高/俱乐部/所在地(偏好工作)/能力/潜力/R.C.A。 -->
            {#if hasDetailed}
              <th class="static c">{t.table.colSkin}</th>
              <th
                class="static c"
                title={avatarError
                  ? t.table.avatarFailHint(avatarError)
                  : undefined}
              >
                {t.table.colAvatar}{#if avatarError}<span
                    class="avatar-warn"
                    aria-hidden="true">!</span
                  >{/if}
              </th>
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
            <!-- 末列「送生图」操作列：基础/拓展名单都有，表头留白（按钮自带读屏文案） -->
            <th class="static c" title={t.gen.source.sendBtn}></th>
          </tr>
        </thead>
        <tbody>
          <!-- 以 uid 为键，便于 Svelte 复用 DOM 行。 -->
          {#each result.items as r (r.uid)}
            <tr>
              <td class="c sel-col">
                <input
                  type="checkbox"
                  aria-label={t.gen.queue.selectPlayerAria(r.name)}
                  checked={!!imageaiStore.selectedInTable[r.uid]}
                  onchange={(e) => toggleSelect(r.uid, e.currentTarget.checked)}
                />
              </td>
              <td class="mono">{r.uid}</td>
              <td>{r.primaryNat}</td>
              <td>{r.secNat}</td>
              <td>{r.name}</td>
              <td class="c">{r.hairLength}</td>
              <td class="c">{r.hairColor}</td>
              <td class="c">{r.ethnicityCode}</td>
              <!-- 详细列：空值显示 —。头像列按 UID 从后端快照/映射缓存取图：有图显示缩略图
                   （本身即触发器），无映射或图片不在包内则显示 —。 -->
              {#if hasDetailed}
                {@const d = r.detailed}
                <td class="c">{d?.skinCode || "—"}</td>
                <td class="c imgcell">
                  <!-- 缩略图 + 悬停/点击放大气泡：加载与交互都在共用组件里，这里只交路径与图片 URL。 -->
                  <AvatarThumb
                    itemKey={r.uid}
                    url={blobs.urls[r.uid]}
                    path={avatarPaths[r.uid] ?? undefined}
                    groupKey={appStore.snapshot?.currentGroup}
                  />
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
              <!-- 行尾「送生图」（P3）：设为数据源并切到 AI 生图；已是数据源的球员按钮变「已选」描边高亮 -->
              <td class="c">
                <button
                  class="btn sm gen-send"
                  class:on={imageaiStore.sourcePlayer?.uid === r.uid}
                  title={t.gen.source.sendTitle(r.name)}
                  onclick={() => sendToGenerate(r)}
                >
                  {imageaiStore.sourcePlayer?.uid === r.uid
                    ? t.gen.source.sent
                    : t.gen.source.sendBtn}
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if result}
    <div class="empty">
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        aria-hidden="true">{@html icons.search.body}</svg
      >
      <p>{search ? t.table.noMatchHint : t.table.emptyHint}</p>
    </div>
  {/if}
</div>

<style>
  /* 撑满父级 .card-body：工具条定高 + 表格滚动区吃剩余高度（粘顶表头在其中生效）。
     min-width:0：18 列宽表格首次挂载时，宽度约束在本层即被截断，
     只允许 .table-scroll 内部横向滚动，不把 .card-body / 网格列撑宽。 */
  .table-pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
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
  .w-sel {
    width: 36px; /* 勾选框列：固定窄列，不可排序 */
  }
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
    width: 36px;
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
  .w-gen {
    width: 64px; /* 行尾「送生图」按钮列：固定窄列，不可排序 */
  }
  /* 已是数据源的行按钮：主题色描边高亮（两主题各自取色），文字同步变「已选」 */
  .gen-send.on {
    border-color: var(--accent);
    color: var(--accent);
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
  /* 头像列批量取数失败的角标：整列留白看不出是「名单真没脸」还是「取不到」，
     原因挂在列头 title 上，这里给一个可见的落点。 */
  .avatar-warn {
    color: var(--warn);
    font-weight: 700;
    margin-left: 3px;
  }

  /* 头像格（AvatarThumb）：去掉上下内边距后 30px 的缩略图不超过文本格的盒子（约 32px），
     行高因此始终与其余文本行一致，不会因图片被撑高（与快照预览同款）。 */
  td.imgcell {
    padding-top: 0;
    padding-bottom: 0;
  }
</style>
