<!-- 名单预览表格：分页、搜索、点表头排序；数据留在后端，每次只拉当前页。
     判据与快照预览同款：后端名单缓存还没被填上（既没「扫描RTF」也没跑过替换）时不渲染工具条
     （搜索框/筛选/分页器），只留引导空态。 -->
<script lang="ts">
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog"; // 查询失败要弹窗（原来裸 await 的 rejection 会静默吞掉错误）
  import { message } from "@tauri-apps/plugin-dialog"; // 「加入队列」成功提示（与 ImageAiPanel 保存成功同款）
  import { appStore, basedValue } from "../../stores/app.svelte";
  import { ui } from "../../stores/ui.svelte"; // 「送生图」要切到 AI 生成头像工作台
  import {
    enqueuePlayers,
    imageaiStore,
    setSourcePlayer,
    toggleSelect,
    toggleSelectPage,
  } from "../../stores/imageai.svelte"; // 勾选与批量入队（队列动作转发见 store）
  import { createPagedResource } from "../../ui/pagedResource.svelte"; // 分页骨架（D1）：页码/搜索/令牌/防抖/换代单点
  import { createAvatarSource } from "../../avatarBlobs.svelte"; // 头像路径查询+缩略图装载（与队列坞/球员源卡共用）
  import { t } from "../../text"; // 全局文案常量
  import Icon from "../../ui/Icon.svelte";
  import TablePager from "./TablePager.svelte"; // 分页器（与快照预览共用）
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 头像缩略图 + 放大气泡（与快照预览共用）
  import type { PlayerRecord } from "../../types";

  // ---- 列集模式（S11）：同一份组件两处职责分化 ----
  // 存档资料台全宽（≈940）= 完整 18 列审阅名单；替换台右栏（≈670）传 dense = 精简列看结果
  // （勾选/UID/姓名/头像/年龄/所在地/送生图）。列集必须用 prop 区分——同一个 1280 窗口里
  // 两处容器宽度不同，视口断点区分不了（设计方案 §3.3 / R1）。
  let { dense = false }: { dense?: boolean } = $props();
  // 精简模式下可勾选展开次要列（国籍/发色/肤色/随机人/性别/身高/俱乐部/能力等）
  let showSecondary = $state(false);
  const showAll = $derived(!dense || showSecondary);

  let sortKey = $state("uid"); // 排序列
  let sortDesc = $state(false); // 是否降序

  // 随机人筛选（仅拓展名单显示）：'any'=任意（默认）/ 'yes'=仅随机人 / 'no'=仅真实球员。
  let newganFilter = $state<"any" | "yes" | "no">("any");

  // 是否为拓展名单：跟随最近一次「扫描RTF」的识别结果（detailed=true 即拓展球员/职员名单）。
  const isExtRoster = $derived(appStore.lastRtfScan?.detailed ?? false);
  // 发往后端的生效筛选：非拓展名单（下拉被隐藏）时强制按「任意」，避免旧筛选残留在请求里。
  const newganQuery = $derived<"any" | "yes" | "no">(
    isExtRoster ? newganFilter : "any",
  );

  // 可读态：与快照预览同款判据——只认 appStore 里记录「后端名单缓存被填上」的标记。
  // 填充者有两处：「扫描RTF」成功，以及替换结束（成功填上/失败取消清空），都由 store 统一维护。
  // 未填充过时工具条（搜索框/筛选/分页器）整块不渲染，只留引导空态。
  const readable = $derived(appStore.rosterReady);

  // ---- 分页骨架（D1 收敛）：页码/搜索/令牌/防抖/换代复位在 createPagedResource 单点维护；
  //      本组件只剩排序键、随机人筛选与头像装载这些视图关切。 ----
  const paged = createPagedResource<PlayerRecord>({
    fetch: async ({ offset, limit, search }) =>
      api.queryPlayers({
        offset,
        limit,
        search,
        sortKey,
        sortDesc,
        newgan: newganQuery === "any" ? null : newganQuery === "yes",
      }),
    deps: () => {
      void sortKey;
      void sortDesc;
      void newganQuery;
      void appStore.rosterVersion; // 「扫描RTF」成功 / 替换结束 → 重载当前页
      void appStore.snapshotVersion; // 快照缓存填充 → 头像列按 UID 重查，顺带重取当前页
    },
    resetKey: () => appStore.rosterVersion,
    onReset: () => {
      sortKey = "uid";
      sortDesc = false;
      newganFilter = "any";
      avatars.clear(); // 旧名单的头像路径与缩略图一并作废回收（新名单的图在拉取后重取）
    },
    onLoaded: (r) => {
      // 本页头像另走一次批量查询（路径出处是后端快照/映射缓存）；空页或基础名单只清场不查询。
      if (r.items.some((x) => x.detailed)) void avatars.load(r.items.map((x) => x.uid));
      else avatars.clear();
    },
    onError: (e) => {
      showError(e); // 查询失败要弹窗（原来裸 await 的 rejection 会静默吞掉错误）
      return null;
    },
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
  const hasDetailed = $derived(!!paged.result?.items.some((r) => r.detailed));
  // 性别列：仅当当前页有记录带性别（拓展职员/新版拓展球员）才渲染，旧版 17 列名单不占位。
  const hasGender = $derived(!!paged.result?.items.some((r) => r.detailed?.gender));

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
    paged.result
      ? paged.result.items.filter((r) => imageaiStore.selectedInTable[r.uid]).length
      : 0,
  );
  const allPageSelected = $derived(
    (paged.result?.items.length ?? 0) > 0 &&
      paged.result!.items.every((r) => imageaiStore.selectedInTable[r.uid]),
  );
  const somePageSelected = $derived(
    !!paged.result?.items.some((r) => imageaiStore.selectedInTable[r.uid]),
  );

  /** 表头全选/取消：只作用于当前页数据行（翻页后另一页不受影响）。 */
  function onPageAll(v: boolean) {
    toggleSelectPage(
      (paged.result?.items ?? []).map((r) => r.uid),
      v,
    );
  }
  /** 把当前页勾选的球员按工作台当前参数加入批量队列；成功后弹提示（条数）。 */
  async function onEnqueue() {
    const rows = (paged.result?.items ?? [])
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
  /** 把该行球员设为生图数据源，并切到「AI 生成头像」工作台。
   *  【为什么是切工作台而不是切结果区 Tab】原实现写 appStore.resultsView='generate'，
   *  结果区整个被换成 AI 面板——用户勾了半天的搜索/页码/排序随组件卸载一起没了，
   *  而 Tab 还在同一个容器里，回不去。切工作台是顺着用户意图走：点了「送生图」就是要去生图，
   *  且顶栏三点之间可自由跳回，不会再被"半路劫持"。 */
  function sendToGenerate(r: PlayerRecord) {
    setSourcePlayer(r.uid, r.name);
    ui.workbench = "ai";
  }

  // 第 15 列口径单点见 appStore 的 isStaffRoster/basedLabel/basedValue（本组件只消费）。

  // ---------------- 头像列：按球员 UID 批量取路径 + 装载缩略图（共用 AvatarSource） ----------------
  // 查路径/令牌作废/blob 装载的骨架在 createAvatarSource 单点维护，本组件只剩两条本地策略：
  // 空页或基础名单（非拓展格式，表格不渲染头像列）不查询、只清场；
  // 批级失败的原因（avatars.error）挂在「头像」列头 title 上——单张图取不到本来就静默，
  // 「命令挂了/载荷形状变了/没选头像包」与「这份名单真的没脸」绝不能长成同一个样子，
  // 否则整列空了，用户没有任何线索可查。
  const avatars = createAvatarSource();
</script>

{#snippet sortIcon(key: string)}
  <!-- 当前排序列的方向指示：统一线性 chevron 图标（替代旧 ↑/↓ 字符） -->
  {#if sortKey === key}
    <Icon name={sortDesc ? "chevronDown" : "chevronUp"} class="sort-ic" stroke={2.4} />
  {/if}
{/snippet}

<!-- 工具条：搜索框（放大镜图标）+ 随机人筛选 + 总条数 + 右侧分页；缓存未填上时不渲染
     （与快照预览的 {#if readable} 同款）。组件根是纵向 flex，撑满卡片正文高度。 -->
<div class="table-pane">
  {#if readable}
    <div class="table-toolbar">
      <div class="search">
        <Icon name="search" />
        <input
          type="search"
          placeholder={t.table.searchPlaceholder}
          bind:value={paged.search}
          oninput={() => (paged.page = 0)}
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
            onchange={() => (paged.page = 0)}
            aria-label={t.table.newganFilterLabel}
          >
            <option value="any">{t.table.newganAny}</option>
            <option value="yes">{t.table.newganYes}</option>
            <option value="no">{t.table.newganNo}</option>
          </select>
        </label>
      {/if}
      <!-- 精简列集（dense）下的次要列开关：勾上展开完整列，取消收回精简列 -->
      {#if dense}
        <label class="switch">
          <input type="checkbox" bind:checked={showSecondary} />
          <span class="track"></span>{t.table.secondaryCols}
        </label>
      {/if}
      {#if paged.result}<span class="muted small record-count"
          >{t.table.recordCount(paged.result.total)}</span
        >{/if}
      <span class="spacer"></span>
      <!-- 分页器（含快速跳页）与快照预览共用同一组件，样式/行为单点维护。 -->
      <TablePager bind:page={paged.page} totalPages={paged.totalPages} />
    </div>
  {/if}

  <!-- 缓存未填上 → 引导空态；有数据 → 表格；已填上但当前筛选无结果 → 提示文案（与快照预览同款三层结构）。 -->
  {#if !readable}
    <div class="empty">
      <Icon name="folder" stroke={1.8} />
      <p>{t.table.emptyHint}</p>
    </div>
  {:else if paged.result && paged.result.items.length > 0}
    <div class="table-scroll">
      <table class="grid">
        <!-- 列宽：长文本列（姓名/俱乐部/所在地）给宽余量吃剩余空间；码值/数值列收窄并居中。
             首列是勾选框（P2 Task5）：不可排序、不参与筛选，宽度固定；
             末列是「送生图」（P3 数据源入口）：基础/拓展名单都有，固定窄列。
             列槽位与下方 thead/tbody 的 {#if} 条件一一对应（dense 精简模式收起次要列，R2）。 -->
        <colgroup>
          <col class="w-sel" />
          <col class="w-uid" />
          {#if showAll}
            <col class="w-nat" />
            <col class="w-nat2" />
          {/if}
          <col class="w-name" />
          {#if showAll}
            <col class="w-code" />
            <col class="w-code" />
            <col class="w-code" />
          {/if}
          {#if hasDetailed}
            {#if showAll}<col class="w-code" />{/if}
            {#if showAll || dense}<col class="w-avatar" />{/if}
            {#if showAll}<col class="w-tag" />{/if}
            {#if showAll && hasGender}<col class="w-code" />{/if}
            {#if showAll || dense}<col class="w-code" />{/if}
            {#if showAll}
              <col class="w-height" />
              <col class="w-club" />
            {/if}
            {#if showAll || dense}<col class="w-based" />{/if}
            {#if showAll}
              <col class="w-code" />
              <col class="w-code" />
              <col class="w-rca" />
            {/if}
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
            {#if showAll}
              <th
                role="button"
                tabindex="0"
                onclick={() => sortBy("primaryNat")}
                onkeydown={(e) => onSortKey(e, "primaryNat")}
                title={t.table.colNat}
                >{t.table.colNat}{@render sortIcon("primaryNat")}</th
              >
              <th class="static">{t.table.col2ndNat}</th>
            {/if}
            <th
              role="button"
              tabindex="0"
              onclick={() => sortBy("name")}
              onkeydown={(e) => onSortKey(e, "name")}
              title={t.table.colName}
              >{t.table.colName}{@render sortIcon("name")}</th
            >
            {#if showAll}
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
            {/if}
            <!-- 详细列（拓展名单）：顺序 肤色/头像/随机人/性别/年龄/身高/俱乐部/所在地(偏好工作)/能力/潜力/R.C.A。
                 dense 精简模式只留 头像/年龄/所在地 三列（次要列开关展开后恢复全部）。 -->
            {#if hasDetailed}
              {#if showAll}<th class="static c">{t.table.colSkin}</th>{/if}
              {#if showAll || dense}
                <th
                  class="static c"
                  title={avatars.error
                    ? t.table.avatarFailHint(avatars.error)
                    : undefined}
                >
                  {t.table.colAvatar}{#if avatars.error}<span
                      class="avatar-warn"
                      aria-hidden="true">!</span
                    >{/if}
                </th>
              {/if}
              {#if showAll}<th class="static c">{t.table.colNewgan}</th>{/if}
              {#if showAll && hasGender}
                <th class="static c">{t.table.colGender}</th>
              {/if}
              {#if showAll || dense}
                <th class="static c">{t.table.colAge}</th>
              {/if}
              {#if showAll}
                <th class="static c">{t.table.colHeight}</th>
                <th class="static">{t.table.colClub}</th>
              {/if}
              {#if showAll || dense}
                <th class="static">{appStore.basedLabel}</th>
              {/if}
              {#if showAll}
                <th class="static c">{t.table.colCa}</th>
                <th class="static c">{t.table.colPa}</th>
                <th class="static c">{t.table.colRca}</th>
              {/if}
            {/if}
            <!-- 末列「送生图」操作列：基础/拓展名单都有，表头留白（按钮自带读屏文案） -->
            <th class="static c" title={t.gen.source.sendBtn}></th>
          </tr>
        </thead>
        <tbody>
          <!-- 以 uid 为键，便于 Svelte 复用 DOM 行。 -->
          {#each paged.result.items as r (r.uid)}
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
              {#if showAll}
                <td>{r.primaryNat}</td>
                <td>{r.secNat}</td>
              {/if}
              <td>{r.name}</td>
              {#if showAll}
                <td class="c">{r.hairLength}</td>
                <td class="c">{r.hairColor}</td>
                <td class="c">{r.ethnicityCode}</td>
              {/if}
              <!-- 详细列：空值显示 —。头像列按 UID 从后端快照/映射缓存取图：有图显示缩略图
                   （本身即触发器），无映射或图片不在包内则显示 —。 -->
              {#if hasDetailed}
                {@const d = r.detailed}
                {#if showAll}<td class="c">{d?.skinCode || "—"}</td>{/if}
                {#if showAll || dense}
                  <td class="c imgcell">
                    <!-- 缩略图 + 悬停/点击放大气泡：加载与交互都在共用组件里，这里只交路径与图片 URL。 -->
                    <AvatarThumb
                      itemKey={r.uid}
                      url={avatars.urls[r.uid]}
                      path={avatars.paths[r.uid] ?? undefined}
                      groupKey={appStore.snapshot?.currentGroup}
                    />
                  </td>
                {/if}
                {#if showAll}
                  <td class="c">
                    {#if d?.isNewgan}
                      <span class={newganCls(d.isNewgan)}>{d.isNewgan}</span>
                    {:else}—{/if}
                  </td>
                {/if}
                {#if showAll && hasGender}<td class="c">{d?.gender || "—"}</td>{/if}
                {#if showAll || dense}<td class="c">{d?.age || "—"}</td>{/if}
                {#if showAll}
                  <td class="c">{d?.height || "—"}</td>
                  <td title={d?.club ?? ""}>{d?.club || "—"}</td>
                {/if}
                {#if showAll || dense}
                  <td title={basedValue(r.detailed)}>{basedValue(r.detailed) || "—"}</td>
                {/if}
                {#if showAll}
                  <td class="c">{d?.ca || "—"}</td>
                  <td class="c">{d?.pa || "—"}</td>
                  <td class="c">{d?.rca || "—"}</td>
                {/if}
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
  {:else if paged.result}
    <div class="empty">
      <Icon name="search" stroke={1.8} />
      <p>{paged.search ? t.table.noMatchHint : t.table.emptyHint}</p>
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
