<!-- 快照预览：展示「读取快照」解析出的存档快照映射（UID / 头像相对路径 / 头像图）。
     样式对齐名单预览（同款工具条与表格，行高同样由文本行决定，头像图不撑高行）。
     数据留在后端缓存，本组件每次只拉当前页 8 条；头像图也只加载当前页的 8 张，
     翻页/跳页/重新筛选时才取新页的图，避免大快照一次性加载全部图片。
     头像的「读字节 → 建 blob → 回收」走共用的 createAvatarBlobs，缩略图与放大气泡走共用的
     AvatarThumb（两处都与名单预览同一份实现，本组件只把路径交出去）。 -->
<script lang="ts">
  import { api } from '../../services/api';
  import { appStore } from '../../stores/app.svelte';
  import { createAvatarBlobs } from '../../avatarBlobs.svelte'; // 头像 blob 加载器（三个标签页共用）
  import { t } from '../../text'; // 全局文案常量
  import { showError } from '../../ui/dialog';
  import { icons } from '../../ui/icons';
  import TablePager from './TablePager.svelte'; // 分页器（与名单预览共用）
  import AvatarThumb from '../shared/AvatarThumb.svelte'; // 头像缩略图 + 放大气泡（与名单预览共用）
  import type { PageResult, SnapshotEntry } from '../../types';

  const PAGE_SIZE = 8;        // 每页条数（头像图同页加载：一次最多 8 张）
  let page = $state(0);       // 当前页码（从 0 开始）
  let search = $state('');    // UID 筛选词
  let result = $state<PageResult<SnapshotEntry> | null>(null); // 当前页结果
  let searchTimer: ReturnType<typeof setTimeout> | undefined; // 防抖定时器句柄
  const blobs = createAvatarBlobs(); // 本页头像图的 blob 账（翻页换代、卸载回收都在其内部）

  const snap = $derived(appStore.displaySnapshot); // 展示层：浏览态优先（统一取自 store getter）
  const xmlPath = $derived(snap?.configXml?.path ?? ''); // 当前展示档的快照文件
  const imgDir = $derived(snap?.currentGroup ?? '');     // 头像相对路径的根（当前展示组的头像包）
  // 可读态：当前展示的快照正是最近一次「读取快照」的那份。切档/切组后路径不一致，回到引导空态。
  const readable = $derived(!!xmlPath && xmlPath === appStore.snapshotReadPath);
  const totalPages = $derived(result ? Math.max(1, Math.ceil(result.total / PAGE_SIZE)) : 1);

  let queryToken = 0; // 分页查询令牌：切档/翻页的在途响应不得写回当前展示的结果
  // 快照路径变化或重新「读取快照」：回到第 1 页、清空筛选并释放图片（新数据从头看）。
  // 只读这两个依赖，刻意不读 readable——读取成功瞬间不能把刚要展示的数据清掉。
  $effect(() => {
    void xmlPath; void appStore.snapshotVersion;
    queryToken++; // 上一个快照的分页查询若在途，回来时直接作废
    result = null;
    page = 0;
    search = '';
    blobs.clear();
  });

  // 页码/筛选/读取状态任一变化后拉取当前页（筛选做 300ms 防抖，避免边输入边请求）。
  $effect(() => {
    void page; void search; void readable; void xmlPath; void imgDir; void appStore.snapshotVersion;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(load, 300);
    return () => clearTimeout(searchTimer);
  });

  /** 拉当前页数据（UID 筛选 + 分页），随后只为本页条目加载头像图。 */
  async function load() {
    if (!readable) return;
    const token = ++queryToken;
    const wantPath = xmlPath; // 发出请求时的快照路径：返回时展示档可能已经换了
    try {
      const got = await api.querySnapshot(xmlPath, imgDir, {
        offset: page * PAGE_SIZE, limit: PAGE_SIZE, search,
      });
      // 【为什么要令牌】切档连点时两个 querySnapshot 会乱序返回：晚到的旧档结果会盖在
      // 新档的表格上，而头像又按这份结果去读——展示的会是别档的映射。
      if (token !== queryToken || wantPath !== xmlPath) return;
      result = got;
    } catch (e) {
      if (token !== queryToken || wantPath !== xmlPath) return;
      await showError(e);
      result = { total: 0, items: [] };
      return;
    }
    loadImages();
  }

  /** 把本页条目的头像路径交给共用的 blob 加载器（先回收上一页，再并发读图、就绪一张出现一张）。 */
  function loadImages() {
    const wanted: Record<string, string> = {};
    for (const it of result?.items ?? []) {
      if (it.imagePath) wanted[it.uid] = it.imagePath; // 包内找不到图片的条目不发请求，留无图占位
    }
    void blobs.load(wanted);
  }
</script>

<!-- 与名单预览同款布局：工具条 + 滚动表格；未读取时只显示引导空态。 -->
<div class="table-pane">
  {#if readable}
    <div class="table-toolbar">
      <div class="search">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" aria-hidden="true">{@html icons.search.body}</svg>
        <input type="search" placeholder={t.snapshot.searchPlaceholder} bind:value={search}
               oninput={() => (page = 0)} aria-label={t.snapshot.searchPlaceholder} />
      </div>
      {#if result}<span class="muted small record-count">{t.snapshot.recordCount(result.total)}</span>{/if}
      <span class="spacer"></span>
      <!-- 分页器（含快速跳页）与名单预览共用同一组件，样式/行为单点维护。 -->
      <TablePager bind:page={page} {totalPages} />
    </div>
  {/if}

  {#if !readable}
    <div class="empty">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
           stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.file.body}</svg>
      <p>{t.snapshot.readHint}</p>
    </div>
  {:else if result && result.items.length > 0}
    <div class="table-scroll">
      <table class="grid">
        <thead>
          <tr>
            <th>{t.snapshot.colUid}</th>
            <th>{t.snapshot.colPath}</th>
            <th>{t.snapshot.colImage}</th>
          </tr>
        </thead>
        <tbody>
          <!-- 以行索引为键：快照里理论上可能出现重复 UID，UID 作键会撞键。 -->
          {#each result.items as { uid, relPath, imagePath }, i (i)}
            <tr>
              <td class="mono">{uid}</td>
              <td class="mono path" title={relPath}>{relPath}</td>
              <td class="imgcell">
                <!-- 缩略图 + 悬停/点击放大气泡：加载与交互都在共用组件里（与名单预览头像列同款）。 -->
                <AvatarThumb
                  itemKey={uid}
                  url={blobs.urls[uid]}
                  path={imagePath ?? undefined}
                  groupKey={imgDir}
                  placeholder={t.snapshot.noImage}
                />
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if result}
    <div class="empty">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
           stroke-linecap="round" aria-hidden="true">
        {@html (search ? icons.search : icons.file).body}
      </svg>
      <p>{search ? t.snapshot.noMatchHint : t.snapshot.emptyHint}</p>
    </div>
  {/if}
</div>

<style>
  /* 撑满父级 .card-body：工具条定高 + 表格滚动区吃剩余高度（与名单预览一致）。
     min-width:0：宽表格只在 .table-scroll 内部横滚，不向上撑宽卡片与网格列。 */
  .table-pane { display: flex; flex-direction: column; height: 100%; min-height: 0; min-width: 0; gap: 2px; }
  /* 头像格（AvatarThumb）：去掉上下内边距后 30px 的缩略图不超过文本格的盒子（约 32px），
     行高因此始终与名单预览的文本行一致，不会因图片被撑高 */
  td.imgcell { padding-top: 0; padding-bottom: 0; }
  /* 路径列长串省略号，悬停看全文 */
  td.path { max-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
