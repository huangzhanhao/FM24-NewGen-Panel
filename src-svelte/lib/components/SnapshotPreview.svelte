<!-- 快照预览：展示「读取快照」解析出的存档快照映射（UID / 头像相对路径 / 头像图）。
     样式对齐名单预览（同款工具条与表格，行高同样由文本行决定，头像图不撑高行）。
     数据留在后端缓存，本组件每次只拉当前页 8 条；头像图也只加载当前页的 8 张，
     翻页/跳页/重新筛选时才取新页的图，避免大快照一次性加载全部图片。 -->
<script lang="ts">
  import { api } from '../api';
  import { appStore } from '../stores/app.svelte';
  import { bytesToBlobUrl, loadImageBlobUrl } from '../images';
  import { t } from '../text'; // 全局文案常量
  import { showError } from '../dialog';
  import { icons } from '../icons';
  import TablePager from './TablePager.svelte'; // 分页器（与名单预览共用）
  import AvatarPopover, { type AvatarEntry, type AvatarPopoverApi } from './AvatarPopover.svelte'; // 头像气泡（与名单预览共用）
  import type { PageResult, SnapshotEntry } from '../types';

  const PAGE_SIZE = 8;        // 每页条数（头像图同页加载：一次最多 8 张）
  let page = $state(0);       // 当前页码（从 0 开始）
  let search = $state('');    // UID 筛选词
  let result = $state<PageResult<SnapshotEntry> | null>(null); // 当前页结果
  let blobUrls = $state<Record<string, string>>({}); // uid → 头像图 blob URL（仅当前页在展示的）
  let searchTimer: ReturnType<typeof setTimeout> | undefined; // 防抖定时器句柄
  let loadToken = 0;           // 图片加载令牌：翻页/重读/卸载后使仍在途的旧页响应作废
  let liveUrls: string[] = []; // 在用的 blob URL 清单（释放用；刻意不用 $state，避免被 effect 追踪）

  const snap = $derived(appStore.displaySnapshot); // 展示层：浏览态优先（统一取自 store getter）
  const xmlPath = $derived(snap?.configXml?.path ?? ''); // 当前展示档的快照文件
  const imgDir = $derived(snap?.currentGroup ?? '');     // 头像相对路径的根（当前展示组的头像包）
  // 可读态：当前展示的快照正是最近一次「读取快照」的那份。切档/切组后路径不一致，回到引导空态。
  const readable = $derived(!!xmlPath && xmlPath === appStore.snapshotReadPath);
  const totalPages = $derived(result ? Math.max(1, Math.ceil(result.total / PAGE_SIZE)) : 1);

  // 快照路径变化或重新「读取快照」：回到第 1 页、清空筛选并释放图片（新数据从头看）。
  // 只读这两个依赖，刻意不读 readable——读取成功瞬间不能把刚要展示的数据清掉。
  $effect(() => {
    void xmlPath; void appStore.snapshotVersion;
    result = null;
    page = 0;
    search = '';
    releaseBlobs();
  });

  // 页码/筛选/读取状态任一变化后拉取当前页（筛选做 300ms 防抖，避免边输入边请求）。
  $effect(() => {
    void page; void search; void readable; void xmlPath; void imgDir; void appStore.snapshotVersion;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(load, 300);
    return () => clearTimeout(searchTimer);
  });

  // 卸载时使在途图片响应作废并释放全部 blob，防止长会话内存泄漏。
  $effect(() => () => { loadToken++; releaseBlobs(); });

  /** 拉当前页数据（UID 筛选 + 分页），随后只为本页条目加载头像图。 */
  async function load() {
    if (!readable) return;
    try {
      result = await api.querySnapshot(xmlPath, imgDir, {
        offset: page * PAGE_SIZE, limit: PAGE_SIZE, search,
      });
    } catch (e) {
      await showError(e);
      result = { total: 0, items: [] };
      return;
    }
    loadImages();
  }

  /** 为本页条目并发加载头像图：先释放上一页的 blob，再逐张把图片字节转成 blob URL（就绪一张显示一张）。 */
  function loadImages() {
    const token = ++loadToken;
    releaseBlobs();
    for (const it of result?.items ?? []) {
      const imagePath = it.imagePath;
      if (!imagePath) continue; // 包内找不到图片文件 → 留在无图占位
      api.readImage(imagePath).then((bytes) => {
        if (token !== loadToken) return; // 期间已翻页/重读/卸载：过期响应直接丢弃，不创建 URL
        const url = bytesToBlobUrl(imagePath, bytes);
        liveUrls.push(url);   // 先登记再显示，保证之后的 releaseBlobs 一定能释放到它
        blobUrls[it.uid] = url;
      }).catch(() => { /* 单张读取失败不阻塞整页，保持无图占位 */ });
    }
  }

  /** 释放当前在用的全部 blob URL（WebView 不能直接用磁盘路径当 <img> 源，blob 需手动回收）。 */
  function releaseBlobs() {
    for (const u of liveUrls) URL.revokeObjectURL(u);
    liveUrls = [];
    blobUrls = {};
  }

  // 头像气泡实例（bind:this）：行内缩略图把鼠标/键盘事件接到它的 open/scheduleClose/close 上。
  let avatarPop = $state<AvatarPopoverApi | null>(null);

  /** 气泡取头像：行内已解析出 imagePath，直接读字节包成 blob URL（缓存 key 即 imagePath）。 */
  async function resolveSnapshotAvatar(imagePath: string): Promise<AvatarEntry> {
    return { url: await loadImageBlobUrl(imagePath), err: '' };
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
    <div class="empty">{t.snapshot.readHint}</div>
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
                <!-- 缩略图本身即触发器：悬停/聚焦弹大气泡、点击钉住（与名单预览头像列同款交互）。 -->
                {#if blobUrls[uid] && imagePath}
                  <button type="button" class="thumb-btn" title={relPath} aria-label={t.table.avatarAria}
                          onmouseenter={(e) => avatarPop?.open(imagePath, e.currentTarget, false)}
                          onmouseleave={avatarPop?.scheduleClose}
                          onfocus={(e) => avatarPop?.open(imagePath, e.currentTarget, false)}
                          onblur={avatarPop?.scheduleClose}
                          onclick={(e) => avatarPop?.open(imagePath, e.currentTarget, true)}>
                    <img src={blobUrls[uid]} alt={uid} class="thumb" />
                  </button>
                {:else}
                  <span class="muted small">{t.snapshot.noImage}</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if result}
    <div class="empty">{search ? t.snapshot.noMatchHint : t.snapshot.emptyHint}</div>
  {/if}

  <!-- 头像大气泡（共用组件）：缓存按 imagePath，切组（imgDir 变化）自动失效并释放。 -->
  <AvatarPopover bind:this={avatarPop} resolve={resolveSnapshotAvatar} groupKey={imgDir} />
</div>

<style>
  /* 撑满父级 .card-body：工具条定高 + 表格滚动区吃剩余高度（与名单预览一致） */
  .table-pane { display: flex; flex-direction: column; height: 100%; min-height: 0; gap: 2px; }
  /* 头像缩略图：图片格去掉上下内边距后，30px 的图不超过文本格的盒子（约 32px），
     行高因此始终与名单预览的文本行一致，不会因图片被撑高 */
  .thumb { height: 30px; width: auto; max-width: 60px; object-fit: cover; border-radius: 4px; display: block; }
  /* 缩略图的触发按钮外壳：去掉按钮默认样式，让图贴齐行内基线 */
  .thumb-btn { appearance: none; border: 0; padding: 0; background: none; line-height: 0; cursor: zoom-in; }
  td.imgcell { padding-top: 0; padding-bottom: 0; }
  /* 路径列长串省略号，悬停看全文 */
  td.path { max-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
