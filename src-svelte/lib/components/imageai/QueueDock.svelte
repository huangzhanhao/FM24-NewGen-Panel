<!-- 生成队列坞（S9 起常驻 AI 台底部全宽）：可折叠批量任务面板。
     - open=false 且有条目：收成 38px 状态条（排队/生成中计数 + 迷你进度 + 入队作用域），点击展开；
     - open=true：完整工具条 + 进度 + 表格（条目行带头像/年龄/所在地，帮认出是谁——FM 同名同姓常见）；
     - open 状态由父级持有（结果工具条的「生成队列」按钮也能开合）。
     事件订阅是进程级的：gen:queue:* 由 imageai store 的 initQueueListeners（App 挂载调一次）统一接收，
     进度/收尾载荷写在 store 里——切走 AI 台期间事件不丢、回台无需重新 query，本组件只负责展示。 -->
<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { t } from "../../text";
  import Icon from "../../ui/Icon.svelte";
  import { saveGeneratedImageAs } from "../../ui/generatedImageSave"; // 生成图另存为（与画廊共用）
  import { showError } from "../../ui/dialog";
  import { api } from "../../services/api";
  import { loadImageBlobUrl } from "../../services/images";
  import { appStore, basedValue } from "../../stores/app.svelte"; // 第 15 列口径（basedLabel/basedValue）+ 气泡失效键
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 条目行头像缩略图 + 放大气泡
  import { createAvatarSource } from "../../avatarBlobs.svelte"; // 头像路径查询+缩略图装载（与名单预览/球员源卡共用）
  import {
    imageaiStore,
    queueCancel,
    queueClear,
    queueRemove,
    queueRetry,
    queueStart,
  } from "../../stores/imageai.svelte";
  import type {
    GenImage,
    PlayerRecord,
    QueueEntry,
    QueueStatus,
  } from "../../types";

  let { open = $bindable(false) }: { open?: boolean } = $props();

  // 进度/收尾载荷来自进程级订阅（imageai store），本组件卸载不再影响事件接收。
  const progress = $derived(imageaiStore.queueProgress); // 最近一次进度事件载荷（驱动进度条即时更新）
  const lastFinished = $derived(imageaiStore.queueFinished); // 最近一次批量收尾摘要
  let picked = $state<Record<string, boolean>>({}); // 队列条目勾选（key=条目 id，组件内会话态）
  let expandedId = $state<string | null>(null); // 展开缩略图的成功条目 id（同时只展一条）
  const thumbs = $state<Record<string, Array<{ id: string; url: string }>>>({}); // 条目 id → 缩略图 blob URL

  const entries = $derived(imageaiStore.queue);
  const dockScope = $derived(imageaiStore.enqueueScope);
  const queuedCount = $derived(
    entries.filter((e) => e.status === "queued").length,
  );
  const runningCount = $derived(
    entries.filter((e) => e.status === "running").length,
  );
  const pickedIds = $derived(
    entries.filter((e) => picked[e.id]).map((e) => e.id),
  );
  const progressPct = $derived(
    progress && progress.total > 0
      ? Math.round((progress.current / progress.total) * 100)
      : 0,
  );

  function statusText(s: QueueStatus) {
    return s === "queued"
      ? t.gen.queue.statusQueued
      : s === "running"
        ? t.gen.queue.statusRunning
        : s === "success"
          ? t.gen.queue.statusSuccess
          : t.gen.queue.statusFailed;
  }
  /** 详情列：失败显示原因，成功显示张数，其余留 —。 */
  function detailText(e: QueueEntry) {
    if (e.status === "failed") return e.error || "—";
    if (e.status === "success")
      return t.gen.queue.imagesDone(e.images?.length ?? 0);
    return "—";
  }

  // ---- 条目行身份信息（S9）：头像 + 年龄/所在地，帮认出同名同姓的球员 ----
  // 头像路径/缩略图走共用的 createAvatarSource（与名单预览同一实现，取失败静默降级为无图）。
  // 资料走 query_players_by_uids 批量取（暂缓①）：一次 IPC 替代原先逐 uid 的
  // query_players(search=uid, limit=1)——N 个条目 = N 次 IPC + N×M 线扫；批量失败整批
  // 降级为仅姓名/UID，不报错。不用 preview_player——它的映射兜底会在缓存未命中时
  // 整表解析存档快照（秒级且堵主线程）。条目来自上一次扫描时资料可能查不到：该行降级。
  const rowAvatars = createAvatarSource();
  let rowInfo = $state<Record<string, PlayerRecord | null>>({});
  const infoCache = new Map<string, PlayerRecord | null>(); // 非响应式缓存：避免逐条写入触发 effect 重跑
  let infoToken = 0;
  let loadedUidKey = ""; // 已拉过身份信息的 uid 稳定键（非响应式，仅 effect 内读写）
  $effect(() => {
    if (!open || entries.length === 0) {
      loadedUidKey = ""; // 收起/清空后再展开：重新拉一次
      return;
    }
    const uids = [...new Set(entries.map((e) => e.uid))];
    // 【护栏】队列运行中快照被节流刷新（store 的 scheduleQueueRefresh，最密约 120ms 一次）整组
    // 替换，条目集合其实没变；不挡的话 rowAvatars.load 每次都 revoke 全部旧 URL 并逐张重读
    // read_image——IPC/磁盘抖动 + 缩略图闪烁。uid 集合不变即跳过（资料侧本就有 infoCache 去重）。
    const uidKey = uids.join("|");
    if (uidKey === loadedUidKey) return;
    loadedUidKey = uidKey;
    void rowAvatars.load(uids); // 头像与资料并行查：各自的令牌防过期回写
    const token = ++infoToken;
    const missing = uids.filter((u) => !infoCache.has(u));
    for (const u of missing) infoCache.set(u, null); // 先占位，防并发期间重复查询
    void (async () => {
      if (missing.length > 0) {
        try {
          const got = await api.queryPlayersByUids(missing);
          const byUid = new Map(got.map((r) => [r.uid, r]));
          for (const u of missing) infoCache.set(u, byUid.get(u) ?? null);
        } catch {
          for (const u of missing) infoCache.set(u, null); // 批量失败：该批降级为仅姓名/UID
        }
      }
      if (token !== infoToken) return; // 期间条目已变：过期结果整批作废
      rowInfo = Object.fromEntries(uids.map((u) => [u, infoCache.get(u) ?? null]));
    })();
  });

  // ---- 展开缩略图的 blob 回收（泄漏防线）----
  // 条目从快照消失（重试换新 id / 移除 / 清空）时，其展开缩略图的 blob URL 立即回收——
  // blob URL 不随组件卸载自动释放，漏掉就是长会话的内存增长；读取侧（toggleThumbs）只管装载。
  // untrack：只跟踪 entries 的 id 集合，thumbs 自身的删键不回触本 effect。
  $effect(() => {
    const ids = new Set(entries.map((e) => e.id));
    untrack(() => {
      for (const [id, arr] of Object.entries(thumbs)) {
        if (!ids.has(id)) {
          for (const im of arr) URL.revokeObjectURL(im.url);
          delete thumbs[id];
        }
      }
    });
  });
  // 切工作台即卸载：兜底全量回收（含上方 effect 不覆盖的「卸载瞬间仍存活」的条目）。
  onDestroy(() => {
    for (const arr of Object.values(thumbs)) {
      for (const im of arr) URL.revokeObjectURL(im.url);
    }
  });
  // 第 15 列口径单点见 appStore（isStaffRoster/basedLabel/basedValue）。
  /** 行的年龄（拓展名单才有；缺资料/基础名单显示 —）。 */
  function rowAge(e: QueueEntry): string {
    return rowInfo[e.uid]?.detailed?.age?.trim() || "—";
  }
  /** 行的所在地（extStaff 名单显示偏好工作；缺资料显示 —）。 */
  function rowWhere(e: QueueEntry): string {
    const d = rowInfo[e.uid]?.detailed;
    if (!d) return "—";
    return basedValue(d).trim() || "—";
  }

  function toggleAll(v: boolean) {
    const next = { ...picked };
    for (const e of entries) next[e.id] = v;
    picked = next;
  }

  // 启动是「将产生费用」的确认（信息级）；取消/清空/移除是破坏性确认（警告级）——均走系统弹窗。
  async function onStart() {
    if (queuedCount === 0) return;
    if (
      !(await confirm(t.gen.queue.startConfirm(queuedCount), {
        title: t.dialog.info,
      }))
    )
      return;
    try {
      // 收尾摘要的复位在 queueStart action 内（store 单点维护，组件不再直写共享态）。
      await queueStart();
    } catch (e) {
      await showError(e);
    }
  }
  async function onCancel() {
    if (
      !(await confirm(t.gen.queue.cancelConfirm, { title: t.dialog.warning }))
    )
      return;
    try {
      await queueCancel();
    } catch (e) {
      await showError(e);
    }
  }
  async function onClear() {
    if (entries.length === 0) return;
    if (
      !(await confirm(t.gen.queue.clearConfirm(entries.length), {
        title: t.dialog.warning,
      }))
    )
      return;
    picked = {};
    try {
      await queueClear();
    } catch (e) {
      await showError(e);
    }
  }
  async function onRemove() {
    if (pickedIds.length === 0) return;
    if (
      !(await confirm(t.gen.queue.removeConfirm(pickedIds.length), {
        title: t.dialog.warning,
      }))
    )
      return;
    picked = {};
    try {
      await queueRemove(pickedIds);
    } catch (e) {
      await showError(e);
    }
  }

  /** 失败行「重试本条」：用原请求参数重新入队（旧失败条目一并移除）。 */
  async function onRetry(e: QueueEntry) {
    try {
      await queueRetry(e);
      expandedId = null;
    } catch (err) {
      await showError(err);
    }
  }

  /** 展开/收起成功行：首次展开时按 images[].path 懒载 blob URL。收起不回收是刻意的——
   *  条目存活期内重复展开直接复用；条目从快照消失或组件卸载时由上方 effect / onDestroy
   *  统一 revoke（旧注释写的「LRU 临时目录兜底」兜的是磁盘文件，兜不住 JS blob，勿再依赖）。 */
  async function toggleThumbs(e: QueueEntry) {
    if (expandedId === e.id) {
      expandedId = null;
      return;
    }
    expandedId = e.id;
    if (thumbs[e.id] || !e.images?.length) return;
    const loaded = await Promise.all(
      e.images.map(async (im: GenImage) => ({
        id: im.id,
        url: await loadImageBlobUrl(im.path),
      })),
    );
    if (!entries.some((en) => en.id === e.id)) {
      for (const im of loaded) URL.revokeObjectURL(im.url); // 展开期间条目已被移除/重试：立即回收，不回写
      return;
    }
    thumbs[e.id] = loaded;
  }
</script>

{#if entries.length > 0 && !open}
  <!-- 收起态状态条：整条可点展开；运行中带迷你进度，否则显示排队/完成计数 -->
  <button
    class="dock-bar"
    onclick={() => (open = true)}
    title={t.gen.queue.expandTitle}
  >
    <Icon name="play" class="db-ic" filled={false} />
    <span class="db-title">{t.gen.queue.tabQueue}</span>
    {#if dockScope}
      <span class="db-scope">{t.gen.queue.scopeTitle(dockScope)}</span>
    {/if}
    {#if imageaiStore.queueRunning && progress}
      <span class="db-count">{progress.current}/{progress.total}</span>
      <span class="db-pbar"
        ><span class="db-pfill" style={`width:${progressPct}%`}></span></span
      >
    {:else}
      {#if queuedCount > 0}<span class="db-chip"
          >{t.gen.queue.queuedBadge(queuedCount)}</span
        >{/if}
      {#if runningCount > 0}<span class="db-chip run"
          >{t.gen.queue.runningBadge(runningCount)}</span
        >{/if}
      {#if lastFinished}
        <span class="db-note"
          >{t.gen.queue.finishedSummary(
            lastFinished.finished,
            lastFinished.failed,
            lastFinished.total,
          )}</span
        >
      {/if}
    {/if}
    <Icon name="chevronUp" class="db-chev" />
  </button>
{:else if open}
  <div class="dock">
    <div class="dock-head">
      <span class="dock-title">{t.gen.queue.tabQueue}</span>
      {#if dockScope}
        <span class="dock-scope">{t.gen.queue.scopeTitle(dockScope)}</span>
      {/if}
      {#if imageaiStore.queueRunning && progress}
        <span class="muted small mono"
          >{progress.current} / {progress.total}</span
        >
      {/if}
      <button
        class="icon-btn"
        title={t.gen.queue.collapseTitle}
        onclick={() => (open = false)}
      >
        <Icon name="chevronDown" size={14} />
      </button>
    </div>

    <div class="toolbar">
      <button
        class="btn sm primary"
        disabled={imageaiStore.queueRunning || queuedCount === 0}
        onclick={onStart}
      >
        <Icon name="play" size={11} />
        {t.gen.queue.startBtn}
      </button>
      <button
        class="btn sm"
        disabled={!imageaiStore.queueRunning}
        onclick={onCancel}>{t.gen.queue.cancelBtn}</button
      >
      <span class="spacer"></span>
      <button
        class="btn sm danger"
        disabled={imageaiStore.queueRunning || pickedIds.length === 0}
        onclick={onRemove}
      >
        {t.gen.queue.removeBtn}
      </button>
      <button
        class="btn sm danger"
        disabled={imageaiStore.queueRunning || entries.length === 0}
        onclick={onClear}
      >
        {t.gen.queue.clearBtn}
      </button>
    </div>

    {#if imageaiStore.queueRunning && progress}
      <div class="progress-row">
        <span class="muted small">{t.gen.queue.progressLabel}</span>
        <div
          class="pbar"
          role="progressbar"
          aria-valuenow={progress.current}
          aria-valuemin={0}
          aria-valuemax={progress.total}
        >
          <div class="pfill" style={`width:${progressPct}%`}></div>
        </div>
      </div>
    {:else if lastFinished}
      <div class="muted small finished-row">
        {t.gen.queue.finishedSummary(
          lastFinished.finished,
          lastFinished.failed,
          lastFinished.total,
        )}
      </div>
    {/if}

    {#if entries.length === 0}
      <div class="empty dock-empty">
        <Icon name="image" stroke={1.6} />
        <p>{t.gen.queue.empty}</p>
      </div>
    {:else}
      <div class="table-scroll">
        <table class="grid">
          <colgroup>
            <col class="w-sel" />
            <col class="w-player" />
            <col class="w-uid" />
            <col class="w-age" />
            <col class="w-where" />
            <col class="w-status" />
            <col class="w-detail" />
          </colgroup>
          <thead>
            <tr>
              <th class="static c">
                <input
                  type="checkbox"
                  aria-label={t.gen.queue.selectAllAria}
                  checked={entries.length > 0 &&
                    pickedIds.length === entries.length}
                  indeterminate={pickedIds.length > 0 &&
                    pickedIds.length < entries.length}
                  onchange={(e) => toggleAll(e.currentTarget.checked)}
                />
              </th>
              <th class="static">{t.gen.queue.colPlayer}</th>
              <th class="static">{t.table.colUid}</th>
              <th class="static">{t.table.colAge}</th>
              <th class="static">{appStore.basedLabel}</th>
              <th class="static c">{t.gen.colStatus}</th>
              <th class="static">{t.gen.queue.colDetail}</th>
            </tr>
          </thead>
          <tbody>
            {#each entries as e (e.id)}
              <tr>
                <td class="c">
                  <input
                    type="checkbox"
                    aria-label={t.gen.queue.selectRowAria(e.name)}
                    checked={!!picked[e.id]}
                    onchange={(ev) => (picked[e.id] = ev.currentTarget.checked)}
                  />
                </td>
                <td>
                  <div class="player-cell">
                    <AvatarThumb
                      itemKey={e.uid}
                      url={rowAvatars.urls[e.uid]}
                      path={rowAvatars.paths[e.uid] ?? undefined}
                      groupKey={appStore.snapshot?.currentGroup}
                    />
                    <span class="pc-name" title={e.name}>{e.name}</span>
                  </div>
                </td>
                <td class="mono">{e.uid}</td>
                <td>{rowAge(e)}</td>
                <td>{rowWhere(e)}</td>
                <td class="c"
                  ><span class="badge {e.status}">{statusText(e.status)}</span
                  ></td
                >
                <td class="detail-cell">
                  <span class="detail" title={detailText(e)}
                    >{detailText(e)}</span
                  >
                  {#if e.status === "success" && e.images?.length > 0}
                    <button
                      type="button"
                      class="btn sm link-btn"
                      aria-expanded={expandedId === e.id}
                      onclick={() => void toggleThumbs(e)}
                    >
                      {expandedId === e.id
                        ? t.gen.queue.collapseImages
                        : t.gen.queue.viewImages}
                    </button>
                  {/if}
                  {#if e.status === "failed" && !imageaiStore.queueRunning}
                    <button
                      type="button"
                      class="btn sm link-btn"
                      onclick={() => void onRetry(e)}
                    >
                      {t.gen.queue.retryEntry}
                    </button>
                  {/if}
                </td>
              </tr>
              {#if expandedId === e.id && e.images?.length > 0}
                <tr class="thumb-row">
                  <td colspan="7">
                    <div class="thumb-strip">
                      {#each thumbs[e.id] ?? [] as im (im.id)}
                        <div class="thumb">
                          <img src={im.url} alt={t.gen.queue.colPlayer} />
                          <button
                            type="button"
                            class="btn sm thumb-save"
                            title={t.gen.queue.saveImage}
                            onclick={() => void saveGeneratedImageAs(im.id)}
                          >
                            <Icon name="download" />
                            {t.gen.queue.saveImage}
                          </button>
                        </div>
                      {/each}
                    </div>
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      </div>
      <div class="foot">
        <span class="muted small">{t.gen.queue.count(entries.length)}</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  /* 收起态状态条：38px 高、整条是按钮；描边 + 软底，双主题取令牌。 */
  .dock-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 38px;
    padding: 0 10px 0 12px;
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    background: var(--bg-raised);
    color: var(--text-1);
    cursor: pointer;
    text-align: left;
    transition:
      border-color var(--dur-fast) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out);
  }
  .dock-bar:hover {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  :global(.db-ic) {
    width: 13px;
    height: 13px;
    color: var(--accent);
    flex: none;
  }
  .db-title {
    font-size: 13px;
    font-weight: 600;
    flex: none;
  }
  /* 入队作用域标注（S9 护栏）：与顶栏右端回显对齐，一眼看出这批在哪个档组入的队 */
  .db-scope,
  .dock-scope {
    font-size: 12px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .db-count {
    font-size: 12px;
    color: var(--text-2);
    font-family: var(--mono);
    flex: none;
  }
  .db-pbar {
    flex: 1;
    max-width: 240px;
    height: 5px;
    border-radius: 99px;
    background: var(--progress-bg);
    overflow: hidden;
  }
  .db-pfill {
    display: block;
    height: 100%;
    border-radius: 99px;
    background: var(--accent);
    transition: width var(--dur) var(--ease-out);
  }
  .db-chip {
    border-radius: 99px;
    padding: 1px 9px;
    font-size: 11.5px;
    white-space: nowrap;
    color: var(--text-2);
    background: var(--bg-surface);
    border: 1px solid var(--hairline-2);
  }
  .db-chip.run {
    color: var(--warn);
    background: var(--warn-soft);
    border-color: transparent;
  }
  .db-note {
    color: var(--text-3);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  :global(.db-chev) {
    width: 15px;
    height: 15px;
    color: var(--text-3);
    flex: none;
    margin-left: auto;
  }

  /* 展开态：固定高度的坞内面板，内部表格自己滚。 */
  .dock {
    display: flex;
    flex-direction: column;
    height: 248px;
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    box-shadow: var(--shadow-2);
    padding: 8px 10px;
    gap: 8px;
  }
  .dock-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .dock-title {
    font-size: 13px;
    font-weight: 600;
  }
  .dock-head .icon-btn {
    margin-left: auto;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  .progress-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .pbar {
    flex: 1;
    height: 6px;
    border-radius: 99px;
    background: var(--progress-bg);
    overflow: hidden;
  }
  .pfill {
    height: 100%;
    border-radius: 99px;
    background: var(--accent);
    transition: width var(--dur) var(--ease-out);
  }
  .finished-row {
    white-space: nowrap;
  }
  .badge {
    border-radius: 99px;
    padding: 1px 9px;
    font-size: 11.5px;
    white-space: nowrap;
  }
  .badge.queued {
    color: var(--text-2);
    background: var(--bg-raised);
  }
  .badge.running {
    color: var(--warn);
    background: var(--warn-soft);
  }
  .badge.success {
    color: var(--ok);
    background: var(--ok-soft);
  }
  .badge.failed {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .detail-cell {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .detail {
    min-width: 0;
    flex: 0 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
  }
  /* 行内链接式小按钮（查看图片/重试本条） */
  .link-btn {
    flex: none;
    border: none;
    background: none;
    padding: 2px 4px;
    min-height: 24px;
    height: 24px;
    color: var(--accent);
  }
  .link-btn:hover:not(:disabled) {
    background: var(--accent-soft);
    color: var(--accent);
  }
  /* 缩略图展开行 */
  .thumb-row > td {
    background: var(--bg-inset);
    padding: 8px 10px 10px;
  }
  .thumb-strip {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .thumb {
    width: 118px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .thumb img {
    width: 118px;
    height: 118px;
    object-fit: contain;
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    cursor: default;
  }
  .thumb-save {
    width: 100%;
  }
  .w-sel {
    width: 40px;
  }
  .w-player {
    width: 150px;
  }
  .w-uid {
    width: 110px;
  }
  .w-age {
    width: 56px;
  }
  .w-where {
    width: 110px;
  }
  .w-status {
    width: 72px;
  }
  /* 详情列（第 7 列，此前 colgroup 漏列、无宽约束）：给建议宽，长错误文本才有确定宽度走 .detail 的省略号 */
  .w-detail {
    width: 220px;
  }
  /* 条目行身份格：头像缩略 + 姓名（S9），资料缺列时该行仍只靠姓名/UID 认人 */
  .player-cell {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
  }
  .pc-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .table-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .dock-empty {
    flex: 1;
    min-height: 0;
  }
  .foot {
    display: flex;
    justify-content: flex-end;
  }
</style>
