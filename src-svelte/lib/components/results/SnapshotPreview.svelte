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
  import { createPagedResource } from '../../ui/pagedResource.svelte'; // 分页骨架（D1 收敛）
  import Icon from '../../ui/Icon.svelte';
  import TablePager from './TablePager.svelte'; // 分页器（与名单预览共用）
  import AvatarThumb from '../shared/AvatarThumb.svelte'; // 头像缩略图 + 放大气泡（与名单预览共用）
  import type { SnapshotEntry } from '../../types';

  const blobs = createAvatarBlobs(); // 本页头像图的 blob 账（翻页换代、卸载回收都在其内部）

  const snap = $derived(appStore.displaySnapshot); // 展示层：浏览态优先（统一取自 store getter）
  const xmlPath = $derived(snap?.configXml?.path ?? ''); // 当前展示档的快照文件
  const imgDir = $derived(snap?.currentGroup ?? '');     // 头像相对路径的根（当前展示组的头像包）
  // 可读态：当前展示的快照正是最近一次「读取快照」的那份。切档/切组后路径不一致，回到引导空态。
  const readable = $derived(!!xmlPath && xmlPath === appStore.snapshotReadPath);

  // 分页骨架（D1 收敛）：页码/搜索/令牌/防抖/换代在 createPagedResource 单点维护；
  // 本组件只剩快照特有语义：readable 门控、快照路径过期（wantPath）与头像装载。
  const paged = createPagedResource<SnapshotEntry>({
    fetch: async ({ offset, limit, search }) => {
      const wantPath = xmlPath; // 发出请求时的快照路径：返回时展示档可能已经换了
      const got = await api.querySnapshot(xmlPath, imgDir, { offset, limit, search });
      // 【为什么要再判 wantPath】切档连点时两个 querySnapshot 会乱序返回：晚到的旧档结果
      // 会盖在新档的表格上，而头像又按这份结果去读——展示的会是别档的映射。
      return wantPath === xmlPath ? got : null; // 展示档已换：作废不写（令牌防乱序由骨架兜底）
    },
    deps: () => {
      void readable; // 「读取快照」成功 → readable 翻 true，据此发起首次拉取
      void xmlPath;
      void imgDir;
      void appStore.snapshotVersion; // 替换收尾同步快照缓存 → 重取当前页（头像路径可能已变）
    },
    resetKey: () => `${xmlPath}|${appStore.snapshotVersion}`, // 换路径/重读快照都算换代
    shouldLoad: () => readable, // 未读取时只留引导空态，不查询
    onReset: () => blobs.clear(), // 换快照：旧头像图一并作废回收
    onLoaded: (r) => {
      // 把本页条目的头像路径交给共用的 blob 加载器（先回收上一页，再并发读图、就绪一张出现一张）。
      const wanted: Record<string, string> = {};
      for (const it of r.items) {
        if (it.imagePath) wanted[it.uid] = it.imagePath; // 包内找不到图片的条目不发请求，留无图占位
      }
      void blobs.load(wanted);
    },
    onError: (e) => {
      showError(e);
      return { total: 0, items: [] }; // 错误态显示空结果（与原行为一致：落到无匹配提示）
    },
  });
</script>

<!-- 与名单预览同款布局：工具条 + 滚动表格；未读取时只显示引导空态。 -->
<div class="table-pane">
  {#if readable}
    <div class="table-toolbar">
      <div class="search">
        <Icon name="search" />
        <input type="search" placeholder={t.snapshot.searchPlaceholder} bind:value={paged.search}
               oninput={() => (paged.page = 0)} aria-label={t.snapshot.searchPlaceholder} />
      </div>
      {#if paged.result}<span class="muted small record-count">{t.snapshot.recordCount(paged.result.total)}</span>{/if}
      <span class="spacer"></span>
      <!-- 分页器（含快速跳页）与名单预览共用同一组件，样式/行为单点维护。 -->
      <TablePager bind:page={paged.page} totalPages={paged.totalPages} />
    </div>
  {/if}

  {#if !readable}
    <div class="empty">
      <Icon name="file" stroke={1.8} />
      <p>{t.snapshot.readHint}</p>
    </div>
  {:else if paged.result && paged.result.items.length > 0}
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
          {#each paged.result.items as { uid, relPath, imagePath }, i (i)}
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
  {:else if paged.result}
    <div class="empty">
      <Icon name={paged.search ? "search" : "file"} stroke={1.8} />
      <p>{paged.search ? t.snapshot.noMatchHint : t.snapshot.emptyHint}</p>
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
