<!-- SnapshotPreview · 快照预览：展示「读取快照」解析出的存档快照映射（UID / 头像相对路径 / 头像图）
     样式对齐名单预览（同款工具条与表格，行高由文本行决定，头像图不撑高行）
     后端一次给全量条目（uid + 相对路径），分页/搜索在前端；实际图片路径按当前页批量解析——
     整表几万条逐一探测文件开销太大。头像的「读字节 → 建 blob → 回收」走共用 createAvatarBlobs，
     缩略图与放大气泡走共用 AvatarThumb（两处实现都与名单预览同一份，本组件只把路径交出去） -->
<script lang="ts">
  import { api } from "../../services/api";
  import { appStore, tip } from "../../stores/app.svelte";
  import { createAvatarBlobs } from "../../avatarBlobs.svelte"; // 头像 blob 加载器（名单/快照/球员查看卡共用）
  import { t } from "../../text"; // 全局文案常量
  import { showError } from "../../ui/dialog";
  import { createPagedResource, type PagedResource } from "../../ui/pagedResource.svelte"; // 本地分页骨架
  import Icon from "../../ui/Icon.svelte";
  import TablePager from "./TablePager.svelte"; // 分页器（与名单预览共用）
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 头像缩略图 + 放大气泡（与名单预览共用）
  import type { SnapshotEntry } from "../../types";
  import { enqueuePlayers, imageaiStore, toggleSelect, toggleSelectPage } from "../../stores/imageai.svelte";
  import { message } from "@tauri-apps/plugin-dialog";

  const blobs = createAvatarBlobs(); // 本页头像图的 blob 账（翻页换代、卸载回收都在其内部）

  const snap = $derived(appStore.displaySnapshot); // 展示层：浏览态优先，统一取自 store getter
  const xmlPath = $derived(snap?.configXml?.path ?? ""); // 当前展示档的快照文件
  const imgDir = $derived(snap?.currentGroup ?? ""); // 头像相对路径的根（当前展示组的头像包）
  // 可读态：当前展示的快照正是最近一次「读取快照」的那份；切档/切组后路径不一致，回引导空态
  const readable = $derived(!!xmlPath && xmlPath === appStore.snapshotReadPath);

  // 随机人筛选（与名单预览同款，仅拓展名单显示）：'any' 任意 / 'yes' 仅随机人 / 'no' 仅真实球员。
  // 判据同为 uid 的 r- 前缀——config.xml 的 to 字段就是名单里的 UID 原文，两种表口径一致。
  let newganFilter = $state<"any" | "yes" | "no">("any");
  const isExtRoster = $derived(appStore.lastRtfScan?.detailed ?? false);
  // 生效筛选：非拓展名单（下拉被隐藏）时强制按「任意」，避免旧筛选残留在视图里
  const newganQuery = $derived<"any" | "yes" | "no">(isExtRoster ? newganFilter : "any");

  // ---- 本地分页骨架：后端一次给全量条目，UID 搜索与随机人筛选都在前端内存完成，翻页不产生请求 -----------------------
  //      （显式标注 paged 类型：select 闭包引用 paged，否则 TS 判定循环推断）
  let paneEl = $state<HTMLElement | undefined>(undefined);
  const paged: PagedResource<SnapshotEntry> = createPagedResource<SnapshotEntry>({
    fetch: () => api.querySnapshot(xmlPath),
    resetKey: () => `${xmlPath}|${appStore.snapshotVersion}`, // 换路径/重读快照都算换代
    pane: () => paneEl, // 每页行数随面板高度自适应（无行数常量）
    shouldLoad: () => readable, // 未读取时只留引导空态，不查询
    onReset: () => {
      blobs.clear(); // 换快照：旧头像图一并作废回收
      newganFilter = "any"; // 新快照也从「任意」看起
    },
    select: (entries) => {
      let out = entries;
      const search = paged.search.trim().toLowerCase();
      if (search) out = out.filter((e) => e.uid.toLowerCase().includes(search));
      if (isExtRoster && newganQuery !== "any") {
        const want = newganQuery === "yes";
        out = out.filter((e) => e.uid.startsWith("r-") === want);
      }
      return out;
    },
    onError: (e) => {
      showError(e);
      return [];
    },
  });

  // ---- 勾选列与「加入队列」（与名单预览同口径：全选跨页、提交按全部筛选结果中勾选行） -------------------------------
  // 快照条目只有 uid（无姓名），入队前需按 uid 回填姓名；查不到的条目以 uid 代替姓名展示。
  const filteredUids = $derived(paged.filtered.map((e) => e.uid));
  const selectedCount = $derived(
    filteredUids.filter((uid) => imageaiStore.selectedInTable[uid]).length,
  );
  const allSelected = $derived(
    filteredUids.length > 0 && filteredUids.every((uid) => imageaiStore.selectedInTable[uid]),
  );
  const someSelected = $derived(filteredUids.some((uid) => imageaiStore.selectedInTable[uid]));

  function onPageAll(v: boolean) {
    toggleSelectPage(filteredUids, v);
  }

  /** 把勾选的快照条目加入批量生成队列：先按 uid 回填姓名，再组装入队。 */
  async function onEnqueue() {
    const uids = filteredUids.filter((uid) => imageaiStore.selectedInTable[uid]);
    if (uids.length === 0) return;
    let nameByUid: Record<string, string> = {};
    try {
      const recs = await api.queryPlayersByUids(uids);
      for (const r of recs) nameByUid[r.uid] = r.name;
    } catch {
      // 名单缓存未就绪时查不到姓名：用 uid 代替，不阻断入队
    }
    const rows = uids.map((uid) => ({ uid, name: nameByUid[uid] || uid }));
    try {
      const n = await enqueuePlayers(rows);
      await message(t.gen.queue.enqueued(n), { title: t.dialog.info });
    } catch (e) {
      await showError(e);
    }
  }

  // 当前页条目的实际图片路径（按页向后端批量解析；找到图 → 绝对路径，找不到 → 无键）
  let imgPaths = $state<Record<string, string>>({});
  let resolveToken = 0;
  async function loadPaths(entries: SnapshotEntry[]) {
    const mine = ++resolveToken;
    try {
      const got = await api.querySnapshotImages(imgDir, entries.map((e) => e.relPath));
      if (mine !== resolveToken) return; // 页已翻走：晚到的解析结果作废
      const wanted: Record<string, string> = {};
      entries.forEach((e, i) => {
        const p = got[i];
        if (p) wanted[e.uid] = p; // 包内找不到图片的条目不发请求，留无图占位
      });
      imgPaths = wanted;
      void blobs.load(wanted); // 把本页头像路径交给共用 blob 加载器（就绪一张出现一张）
    } catch (e) {
      if (mine === resolveToken) showError(e);
    }
  }

  // 页码/筛选变化 → 当前页条目变化 → 只为这一页解析并装载头像；imgDir 切组也随之重解析
  $effect(() => {
    void loadPaths(paged.view.items);
  });
</script>

<!-- 与名单预览同款布局：工具条 + 滚动表格；未读取时只显示引导空态 -->
<div class="table-pane" bind:this={paneEl}>
  {#if readable}
    <!-- 平铺 + 自然换行：放得下就同排，放不下才换行（与名单预览同款）；分页器自带贴右 -->
    <div class="table-toolbar">
      <div class="search">
        <Icon name="search" />
        <input
          type="search"
          placeholder={t.snapshot.searchPlaceholder}
          bind:value={paged.search}
          oninput={() => (paged.page = 0)}
          aria-label={t.snapshot.searchPlaceholder}
        />
      </div>
      <!-- 随机人筛选：与名单预览同款（仅拓展名单显示，默认「任意」）；改筛选即回第 1 页 -->
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
      <!-- 加入队列：按全部筛选结果中勾选数计数（与名单预览同口径），无勾选时禁用 -->
      <button class="btn sm" disabled={selectedCount === 0} onclick={onEnqueue}>
        {t.gen.queue.addBtn(selectedCount)}
      </button>
      <!-- 分页器（含快速跳页）与名单预览共用同一组件，样式/行为单点维护 -->
      <TablePager bind:page={paged.page} totalPages={paged.totalPages} />
    </div>
  {/if}

  {#if !readable}
    <div class="empty">
      <Icon name="file" stroke={1.8} />
      <p>{t.snapshot.readHint}</p>
    </div>
  {:else if paged.view.items.length > 0}
    <div class="table-scroll">
      <!-- fixed 布局（见 style）：列宽全由 colgroup 定，长路径只截断不撑宽表格；
           UID/头像列定宽，相对路径列吃满剩余宽度 -->
      <table class="grid fixed">
        <colgroup>
          <col class="w-sel" />
          <col class="w-uid" />
          <col />
          <col class="w-thumb" />
        </colgroup>
        <thead>
          <tr>
            <!-- 首列勾选框：表头复选 = 全选/取消全部筛选结果（跨页） -->
            <th class="static c sel-col">
              <input
                type="checkbox"
                aria-label={t.gen.queue.selectAllPlayersAria}
                checked={allSelected}
                indeterminate={someSelected && !allSelected}
                onchange={(e) => onPageAll(e.currentTarget.checked)}
              />
            </th>
            <th
              >{t.snapshot.colUid}<span class="muted">（{t.snapshot.recordCount(paged.view.total)}）</span></th
            >
            <th>{t.snapshot.colPath}</th>
            <th>{t.snapshot.colImage}</th>
          </tr>
        </thead>
        <tbody>
          <!-- 以行索引为键：快照里可能出现重复 UID，UID 作键会撞键 -->
          {#each paged.view.items as { uid, relPath }, i (i)}
            <tr>
              <td class="c sel-col">
                <input
                  type="checkbox"
                  aria-label={t.gen.queue.selectPlayerAria(uid)}
                  checked={!!imageaiStore.selectedInTable[uid]}
                  onchange={(e) => toggleSelect(uid, e.currentTarget.checked)}
                />
              </td>
              <td class="mono">{uid}</td>
              <td class="mono path" use:tip={relPath}>{relPath}</td>
              <td>
                <!-- 缩略图 + 悬停/点击放大气泡：加载与交互都在共用组件里（与名单预览头像列同款）；
                     path 来自本页批量解析结果（imgPaths），包内找不到图的条目无键 → 无图占位 -->
                <AvatarThumb
                  itemKey={uid}
                  url={blobs.urls[uid]}
                  path={imgPaths[uid]}
                  groupKey={imgDir}
                  placeholder={t.snapshot.noImage}
                />
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <div class="empty">
      <Icon name={paged.search ? "search" : "file"} stroke={1.8} />
      <p>{paged.search ? t.snapshot.noMatchHint : t.snapshot.emptyHint}</p>
    </div>
  {/if}
</div>

<style>
  /* 注：.table-pane 已收进 app.css 表格原语（与名单预览同款） */

  /* 固定表布局：列宽只认 colgroup（内容不再反过来影响列宽），长路径由 .path 截断，
     表格恒等于容器宽度、不产生横向滚动。不给宽度的第 2 列吃满剩余宽度。 */
  table.grid.fixed {
    table-layout: fixed;
  }
  /* 列宽（fixed 下即实际宽度）：UID 列要连表头的「（N 条）」一起装下（≈127px）；
     头像列容 60px 缩略图 + 单元格内边距 */
  .w-sel {
    width: 30px; /* 勾选框列：14px 复选框 + 单元格内边距（与名单预览同宽） */
  }
  .w-uid {
    width: 152px;
  }
  .w-thumb {
    width: 96px;
  }
  /* 路径列长串省略号，悬停看全文（fixed 布局下不需要 max-width 抑制撑宽） */
  td.path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
