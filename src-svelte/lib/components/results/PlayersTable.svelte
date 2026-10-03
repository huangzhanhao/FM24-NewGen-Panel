<!-- PlayersTable · 名单预览表格：分页、搜索、点表头排序；后端一次给全量，分页/筛选/排序都在前端
     可读判据与快照预览同款：后端名单缓存未填上（既没「扫描RTF」也没跑过替换）时
     不渲染工具条（搜索框/筛选/分页器），只留引导空态 -->
<script lang="ts">
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog"; // 查询失败要弹窗，否则 rejection 被静默吞掉
  import { message } from "@tauri-apps/plugin-dialog"; // 「加入队列」成功提示（与 ImageAiPanel 保存同款）
  import { appStore, basedValue, filterPlayers } from "../../stores/app.svelte";
  import { ui } from "../../stores/ui.svelte"; // 「送生图」要切到 AI 生成头像工作台
  import {
    enqueuePlayers,
    imageaiStore,
    setSourcePlayer,
    toggleSelect,
    toggleSelectPage,
  } from "../../stores/imageai.svelte"; // 勾选与批量入队（队列动作转发见 store）
  import { createPagedResource, type PagedResource } from "../../ui/pagedResource.svelte"; // 本地分页骨架
  import { createAvatarSource } from "../../avatarBlobs.svelte"; // 头像路径查询 + 缩略图装载（与队列坞/球员源卡共用）
  import { t } from "../../text"; // 全局文案常量
  import Icon from "../../ui/Icon.svelte";
  import TablePager from "./TablePager.svelte"; // 分页器（与快照预览共用）
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 头像缩略图 + 放大气泡（与快照预览共用）
  import type { PlayerRecord } from "../../types";

  // ---- 列集模式：同一份组件两处职责分化 ----
  // 存档资料台全宽（≈940）= 完整 18 列审阅名单；替换台右栏（≈670）传 dense = 精简列看结果
  // （勾选/UID/姓名/头像/年龄/所在地/送生图）。列集只能用 prop 区分——同一个 1280 窗口里
  // 两处容器宽度不同，视口断点区分不了
  let { dense = false }: { dense?: boolean } = $props();
  // 拓展列开关（原「次要列」）：打开补齐拓展字段列（拓展名单共 18 列），收起只留基础字段列。
  // 基础字段列（UID/姓名/国籍/第二国籍/发色/肤色/族裔码）与勾选/头像/送生图恒显示，不受它影响
  let extCols = $state(false);
  // 拓展字段列是否上屏：存档台全宽（审阅）恒展开；替换台窄栏由「拓展列」开关控制
  const showExt = $derived(!dense || extCols);

  // 排序：null=不排序，保持 RTF 解析自然顺序（初始态）；点表头后按列名在前端本地排序
  // （uid/name/primaryNat/ethnicityCode，只在本组件的比较器里用，不再是后端协议值）
  let sortKey = $state<string | null>(null);
  let sortDesc = $state(false);

  // 随机人筛选（仅拓展名单显示）：'any' 任意（默认）/ 'yes' 仅随机人 / 'no' 仅真实球员
  let newganFilter = $state<"any" | "yes" | "no">("any");

  // 是否为拓展名单：跟随最近一次「扫描RTF」的识别结果（detailed=true 即拓展球员/职员名单）
  const isExtRoster = $derived(appStore.lastRtfScan?.detailed ?? false);
  // 生效筛选：非拓展名单（下拉被隐藏）时强制按「任意」，避免旧筛选残留在视图里
  const newganQuery = $derived<"any" | "yes" | "no">(isExtRoster ? newganFilter : "any");

  // 可读态：与快照预览同款判据——只认 store 的「后端名单缓存被填上」单点标记 rosterReady
  // 置真有两处（扫描RTF 成功 / 替换成功）；切档、切组、换名单导致作用域变化时由 store 作废
  const readable = $derived(appStore.rosterReady);

  // ---- 本地分页骨架：后端一次给全量（换代时取一次），搜索/随机人筛选/排序/翻页都在前端内存完成 ----
  //      select 读组件本地状态（搜索词/筛选/排序），变化只重算 view，不产生任何请求
  //      （显式标注 paged 类型：select 闭包引用 paged，否则 TS 判定循环推断）
  let paneEl = $state<HTMLElement | undefined>(undefined);
  const paged: PagedResource<PlayerRecord> = createPagedResource<PlayerRecord>({
    fetch: () => api.queryPlayers(),
    resetKey: () => appStore.rosterVersion,
    pane: () => paneEl, // 每页行数随面板高度自适应（无行数常量）
    onReset: () => {
      sortKey = null; // 新名单也从自然顺序看起
      sortDesc = false;
      newganFilter = "any";
      extCols = false;
      avatars.clear(); // 旧名单的头像路径与缩略图一并作废回收（新名单的图在装载 effect 里重取）
    },
    select: (rows) => {
      let out = rows;
      const search = paged.search.trim();
      // 命中口径与单人替换卡共用 store 里的 filterPlayers（UID/姓名/国籍/俱乐部）
      if (search) out = filterPlayers(out, search);
      if (isExtRoster && newganQuery !== "any") {
        const want = newganQuery === "yes";
        out = out.filter((r) => r.uid.startsWith("r-") === want);
      }
      if (sortKey) {
        const key = sortKey;
        const val = (r: PlayerRecord) =>
          key === "name"
            ? r.name
            : key === "primaryNat"
              ? r.primaryNat
              : key === "ethnicityCode"
                ? r.ethnicityCode
                : r.uid;
        out = [...out].sort((a, b) => {
          const ord = val(a).localeCompare(val(b));
          return sortDesc ? -ord : ord;
        });
      }
      return out;
    },
    onError: (e) => {
      showError(e);
      return [];
    },
  });

  /** 点表头排序：同一列切换升/降序，不同列切到该列并重置为升序（初始 null 时点任意列即按该列升序） */
  function sortBy(key: string) {
    if (sortKey === key) {
      sortDesc = !sortDesc;
    } else {
      sortKey = key;
      sortDesc = false;
    }
  }
  /** 可排序表头的键盘操作：Enter/Space 等价点击（th 靠 role=button + tabindex=0 保证键盘可达） */
  function onSortKey(e: KeyboardEvent, key: string) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      sortBy(key);
    }
  }

  // 是否含详细信息：当前页任一条有 detailed 就额外渲染那几列（适配基础/详细两种名单）
  const hasDetailed = $derived(paged.view.items.some((r) => r.detailed));
  // 性别列：仅当当前页有记录带性别（拓展职员/新版拓展球员）才渲染，无性别字段时不占位
  const hasGender = $derived(paged.view.items.some((r) => r.detailed?.gender));

  /** 随机人列的标签配色：中文名单值为「是/否」，英文名单兼容 Yes/No；其余值原样显示不着色 */
  function newganCls(v: string) {
    if (/否|no/i.test(v)) return "tag-no";
    if (/是|yes/i.test(v)) return "tag-yes";
    return "";
  }

  // ---------------- 勾选列与「加入队列」 ----------------
  // 勾选状态存 imageaiStore.selectedInTable（key=uid，翻页/切组/换名单都不清空——勾选跟随球员）
  // 【按钮口径】「加入队列（N）」的 N 与可用性只按当前页勾选数：提交只针对用户眼前这一页，
  // 语义最直白（store.selectedCount 是含跨页的全局口径，用它会出现显示 5、实际提交别页 12 条）
  const pageSelectedCount = $derived(
    paged.view.items.filter((r) => imageaiStore.selectedInTable[r.uid]).length,
  );
  const allPageSelected = $derived(
    paged.view.items.length > 0 && paged.view.items.every((r) => imageaiStore.selectedInTable[r.uid]),
  );
  const somePageSelected = $derived(paged.view.items.some((r) => imageaiStore.selectedInTable[r.uid]));

  /** 表头全选/取消：只作用于当前页数据行（翻页后另一页不受影响） */
  function onPageAll(v: boolean) {
    toggleSelectPage(
      paged.view.items.map((r) => r.uid),
      v,
    );
  }
  /** 把当前页勾选的球员按工作台当前参数加入批量队列；成功后弹提示（条数） */
  async function onEnqueue() {
    const rows = paged.view.items
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

  // ---------------- 行尾「送生图」数据源入口 ----------------
  /** 设为生图数据源并切 ui.workbench 去 AI 工作台；不碰 appStore.rtfPreview（Tab 选择是两工作台共享态，
   *  切回来仍停在原 Tab；但表内页码/搜索/排序是组件本地态，随本组件卸载回到初始） */
  function sendToGenerate(r: PlayerRecord) {
    setSourcePlayer(r.uid, r.name);
    ui.workbench = "ai";
  }

  // 第 15 列口径单点在 appStore 的 isStaffRoster/basedLabel/basedValue（本组件只消费）

  // ---------------- 头像列：按球员 UID 批量取路径 + 装载缩略图（共用 createAvatarSource） ----------------
  // 查路径/令牌作废/blob 装载的骨架在 createAvatarSource 单点维护，本组件只剩两条本地策略：
  // 空页只清场不查询；批级失败（avatars.error）挂在「头像」列头 title 上——单张图取不到本来就静默，
  // 「命令挂了/载荷形状变了/没选头像包」与「这份名单真的没脸」不能长成同一个样子，
  // 否则整列空了用户没有任何线索可查
  const avatars = createAvatarSource();

  // 头像装载跟随当前页视图：页码/筛选/搜索变化都会重算 view，这里随之只装载本页的 uid；
  // 快照缓存收尾（snapshotVersion）后头像路径可能已变，一并重取。
  // 头像列恒显示（基础名单也有），故不再按 detailed 门控——头像路径出自快照/映射缓存，与名单格式无关
  $effect(() => {
    void appStore.snapshotVersion;
    const uids = paged.view.items.map((r) => r.uid);
    if (uids.length) void avatars.load(uids);
    else avatars.clear();
  });
</script>

{#snippet sortIcon(key: string)}
  <!-- 当前排序列的方向指示：线性 chevron 图标（初始不排序，无任何箭头） -->
  {#if sortKey === key}
    <Icon name={sortDesc ? "chevronDown" : "chevronUp"} class="sort-ic" stroke={2.4} />
  {/if}
{/snippet}

<!-- 工具条：搜索框 + 加入队列 + 随机人筛选 + 总条数 + 右侧分页；readable 为假时整块不渲染
     （与快照预览的 {#if readable} 同款）。组件根是纵向 flex，撑满卡片正文高度 -->
<!-- 表格面板：工具条 + 滚动表格（面板交给 paged 做行数自适应） -->
<div class="table-pane" bind:this={paneEl}>
  {#if readable}
    <!-- 平铺 + 自然换行：放得下就同排（存档台一整排），放不下才换行；分页器自带贴右 -->
    <div class="table-toolbar">
      <!-- 搜索框：外观由 app.css 的 .search 单点定义（与单人替换卡同款），命中口径见 store.filterPlayers -->
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
      <!-- 随机人筛选：仅拓展名单显示，默认「任意」 -->
      {#if isExtRoster}
        <label class="newgan-filter">
          <span class="muted small">{t.table.newganFilterLabel}</span>
          <select bind:value={newganFilter} onchange={() => (paged.page = 0)} aria-label={t.table.newganFilterLabel}>
            <option value="any">{t.table.newganAny}</option>
            <option value="yes">{t.table.newganYes}</option>
            <option value="no">{t.table.newganNo}</option>
          </select>
        </label>
      {/if}
      <!-- 加入队列：按当前页勾选数计数（口径见脚本内注释），无勾选时禁用；排在随机人筛选右侧 -->
      <button class="btn sm" disabled={pageSelectedCount === 0} onclick={onEnqueue}>
        {t.gen.queue.addBtn(pageSelectedCount)}
      </button>
      <!-- 拓展列开关（仅替换台窄栏 + 拓展名单显示）：打开补齐拓展字段列（共 18 列），
           收起只留基础字段列；基础字段列与勾选/头像/送生图不受开关影响 -->
      {#if dense && hasDetailed}
        <label class="switch">
          <input type="checkbox" bind:checked={extCols} />
          <span class="track"></span>{t.table.extCols}
        </label>
      {/if}
      <!-- 分页器（含快速跳页）与快照预览共用同一组件，样式/行为单点维护 -->
      <TablePager bind:page={paged.page} totalPages={paged.totalPages} />
    </div>
  {/if}

  <!-- 三层结构：缓存未填上 → 引导空态；有数据 → 表格；已填上但当前筛选无结果 → 提示文案（与快照预览同款） -->
  {#if !readable}
    <div class="empty">
      <Icon name="folder" stroke={1.8} />
      <p>{t.table.emptyHint}</p>
    </div>
  {:else if paged.view.items.length > 0}
    <div class="table-scroll">
      <!-- dense 类只服务替换台窄栏（≈670）的列宽收敛策略（见 style 末尾） -->
      <table class="grid" class:dense>
        <!-- 列宽档位见 style 的列宽分区：长文本列（姓名/俱乐部/所在地）吃剩余空间，码值/数值列收窄居中
             首列勾选框与末列「送生图」都是固定窄列；
             列槽位与下方 thead/tbody 的条件一一对应（拓展字段列由「拓展列」开关控制） -->
        <colgroup>
          <col class="w-sel" />
          <col class="w-uid" />
          <col class="w-nat" />
          <col class="w-nat2" />
          <col class="w-name" />
          <!-- 头像列恒显示（应用列，与名单格式无关）：紧跟姓名列 -->
          <col class="w-avatar" />
          <col class="w-code" />
          <col class="w-code" />
          <col class="w-code" />
          {#if hasDetailed && showExt}
            <col class="w-code" />
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
            <!-- 可排序表头：onclick=sortBy + sortIcon snippet 渲染升降序 chevron；
                 role=button + tabindex=0 + onkeydown 让键盘用户也能排序
                 排序键（'uid'/'primaryNat'/'name'/'ethnicityCode'）是发往后端的协议值，勿改
                 static 类=不可排序列，默认光标；c 类=紧凑列居中 -->
            <!-- 首列勾选框：表头复选 = 全选/取消当前页数据行（indeterminate 表达部分勾选）；
                 aria-label 提供读屏文案（不造 label/for，避免为每行生成唯一 id） -->
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
                >{t.table.colUid}<span class="muted">（{t.table.recordCount(paged.view.total)}）</span>{@render sortIcon(
                  "uid",
                )}</th
              >
            <th
              role="button"
              tabindex="0"
              onclick={() => sortBy("primaryNat")}
              onkeydown={(e) => onSortKey(e, "primaryNat")}
              title={t.table.colNat}>{t.table.colNat}{@render sortIcon("primaryNat")}</th
            >
            <th class="static">{t.table.col2ndNat}</th>
            <th
              role="button"
              tabindex="0"
              class="clamp clamp-name"
              onclick={() => sortBy("name")}
              onkeydown={(e) => onSortKey(e, "name")}
              title={t.table.colName}>{t.table.colName}{@render sortIcon("name")}</th
            >
            <!-- 头像列恒显示（紧跟姓名） -->
            <th class="static c" title={avatars.error ? t.table.avatarFailHint(avatars.error) : undefined}>
              {t.table.colAvatar}{#if avatars.error}<span class="avatar-warn" aria-hidden="true">!</span>{/if}
            </th>
            <th class="static c">{t.table.colHair}</th>
            <th class="static c">{t.table.colColor}</th>
            <th
              role="button"
              tabindex="0"
              class="c"
              onclick={() => sortBy("ethnicityCode")}
              onkeydown={(e) => onSortKey(e, "ethnicityCode")}
              title={t.table.colEth}>{t.table.colEth}{@render sortIcon("ethnicityCode")}</th
            >
            <!-- 拓展字段列（仅拓展名单有，由「拓展列」开关控制）：顺序=肤色/随机人/性别/年龄/
                 身高/俱乐部/所在地(偏好工作)/能力/潜力/R.C.A。基础字段列与勾选/头像/送生图恒显示 -->
            {#if hasDetailed && showExt}
              <th class="static c">{t.table.colSkin}</th>
              <th class="static c">{t.table.colNewgan}</th>
              {#if hasGender}
                <th class="static c">{t.table.colGender}</th>
              {/if}
              <th class="static c">{t.table.colAge}</th>
              <th class="static c">{t.table.colHeight}</th>
              <th class="static">{t.table.colClub}</th>
              <th class="static clamp clamp-based">{appStore.basedLabel}</th>
              <th class="static c">{t.table.colCa}</th>
              <th class="static c">{t.table.colPa}</th>
              <th class="static c">{t.table.colRca}</th>
            {/if}
            <!-- 末列「送生图」操作列：基础/拓展名单都有，表头留白（按钮自带读屏文案） -->
            <th class="static c" title={t.gen.source.sendBtn}></th>
          </tr>
        </thead>
        <tbody>
          <!-- 以 uid 为键，便于 Svelte 复用 DOM 行 -->
          {#each paged.view.items as r (r.uid)}
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
              <td class="clamp clamp-name" title={r.name}>{r.name}</td>
              <!-- 头像列恒显示（紧跟姓名）：按 UID 从后端快照/映射缓存取图，无映射或图不在包内显示 — -->
              <td class="c">
                <!-- 缩略图 + 悬停/点击放大气泡：加载与交互都在共用组件里，这里只交路径与图片 URL -->
                <AvatarThumb
                  itemKey={r.uid}
                  url={avatars.urls[r.uid]}
                  path={avatars.paths[r.uid] ?? undefined}
                  groupKey={appStore.snapshot?.currentGroup}
                />
              </td>
              <td class="c">{r.hairLength}</td>
              <td class="c">{r.hairColor}</td>
              <td class="c">{r.ethnicityCode}</td>
              <!-- 拓展字段列（空值显示 —） -->
              {#if hasDetailed && showExt}
                {@const d = r.detailed}
                <td class="c">{d?.skinCode || "—"}</td>
                <td class="c">
                  {#if d?.isNewgan}
                    <span class={newganCls(d.isNewgan)}>{d.isNewgan}</span>
                  {:else}—{/if}
                </td>
                {#if hasGender}<td class="c">{d?.gender || "—"}</td>{/if}
                <td class="c">{d?.age || "—"}</td>
                <td class="c">{d?.height || "—"}</td>
                <td title={d?.club ?? ""}>{d?.club || "—"}</td>
                <td class="clamp clamp-based" title={basedValue(r.detailed)}>{basedValue(r.detailed) || "—"}</td>
                <td class="c">{d?.ca || "—"}</td>
                <td class="c">{d?.pa || "—"}</td>
                <td class="c">{d?.rca || "—"}</td>
              {/if}
              <!-- 行尾「送生图」：设为数据源并切到 AI 生图；已是数据源的球员按钮变「已选」描边高亮 -->
              <td class="c">
                <button
                  class="btn sm gen-send"
                  class:on={imageaiStore.sourcePlayer?.uid === r.uid}
                  title={t.gen.source.sendTitle(r.name)}
                  onclick={() => sendToGenerate(r)}
                >
                  {imageaiStore.sourcePlayer?.uid === r.uid ? t.gen.source.sent : t.gen.source.sendBtn}
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <div class="empty">
      <Icon name="search" stroke={1.8} />
      <p>{paged.search ? t.table.noMatchHint : t.table.emptyHint}</p>
    </div>
  {/if}
</div>

<style>
  /* 注：.newgan-filter（随机人筛选）已收进 app.css 表格章节——名单/快照两表共用 */

  /* ---- 列宽（auto 布局下为建议宽度，姓名/俱乐部/所在地会分吃剩余空间；总宽超出则横向滚动）----
     恒显示 10 列（勾选/UID/国籍/第二国籍/姓名/发色/肤色/族裔码/头像/送生图）建议宽合计 ≈598
     （码值三列表头即 40/40/52，实际 ≈610）≤ 窄栏 ≈621，替换台默认不横滚——为此把勾选/国籍/第二国籍/
     姓名/头像各收了几个像素（头像列由拓展列转为恒显示）；打开「拓展列」后追加拓展字段列（共 18 列），
     此时横滚是预期行为。UID 建议宽按表头「UID（N 条）」实测需求定（文字 ≈87 + 内边距 16 + 排序箭头 14），不能再收 */
  .w-sel {
    width: 30px; /* 勾选框列：14px 复选框 + 单元格内边距 */
  }
  .w-uid {
    width: 120px;
  }
  .w-nat {
    width: 44px;
  }
  .w-nat2 {
    width: 64px; /* 表头「第二国籍」四个字即为下限 */
  }
  .w-name {
    width: 120px;
  }
  .w-code {
    width: 36px;
  }
  .w-avatar {
    width: 48px; /* 30px 缩略图 + 单元格内边距 */
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
    width: 158px;
  }
  .w-rca {
    width: 56px;
  }
  .w-gen {
    width: 64px; /* 行尾「送生图」按钮列：固定窄列 */
  }
  /* 已是数据源的行按钮：主题色描边高亮（两主题各自取色） */
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
  /* 头像列批量取数失败的角标：给列头 title 里那个原因一个可见落点 */
  .avatar-warn {
    color: var(--warn);
    font-weight: var(--fw-bold);
    margin-left: 3px;
  }

  /* ---- dense 精简列集（替换台右栏 ≈670）：默认 1280 窗口一屏放全 7 列，不产生横向滚动 ----
     背景：全局单元格横向内边距 sp-5(12px) 是为存档台 18 列宽表定的宽松节奏；窄栏里
     7 列 col 建议宽合计 612 已逼近容器（约 621，含纵向滚动条占位），而姓名/所在地是
     nowrap 长文本列，遇长姓名/长地名（英文名单尤甚）auto 布局按 min-content 把整表撑出
     横滚，末列「送生图」被推出视口。收敛只挂 table.grid.dense，存档台全宽表不戴此类：
     ① 横向内边距收到 sp-3(8px)，降各列内容地板；
     ② 姓名/所在地溢出省略、全文走 title 悬停，max-width 与各自 col 建议宽同值——
        窗口变宽时两列仍可随 auto 布局分吃剩余空间，仅长内容不再撑破窄栏；
     1100–1280 区间右栏更窄时两列向地板收缩并省略，同样不横滚；<1100 整栏堆叠成全宽。 */
  table.grid.dense th,
  table.grid.dense td {
    padding-left: var(--sp-3);
    padding-right: var(--sp-3);
  }
  table.grid.dense .clamp {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  table.grid.dense .clamp-name {
    max-width: 120px; /* 与 .w-name 同值：列建议宽即截断点 */
  }
  table.grid.dense .clamp-based {
    max-width: 158px; /* 与 .w-based 同值：列建议宽即截断点 */
  }
  /* 注：.table-pane 已收进 app.css 表格原语（名单、快照两卡同款） */
</style>
