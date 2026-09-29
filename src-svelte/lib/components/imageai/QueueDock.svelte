<!-- 生成队列坞（S9 起常驻 AI 台底部全宽）：可折叠批量任务面板。
     - open=false 且有条目：收成 38px 状态条（排队/生成中计数 + 迷你进度 + 入队作用域），点击展开；
     - open=true：完整工具条 + 进度 + 表格（条目行带头像/年龄/所在地，帮认出是谁——FM 同名同姓常见）；
     - open 状态由父级持有（结果工具条的「生成队列」按钮也能开合）。
     事件订阅是进程级的：gen:queue:* 由 imageai store 的 initQueueListeners（App 挂载调一次）统一接收，
     进度/收尾载荷写在 store 里——切走 AI 台期间事件不丢、回台无需重新 query，本组件只负责展示。 -->
<script lang="ts">
  import { confirm, message, save } from "@tauri-apps/plugin-dialog";
  import { t } from "../../text";
  import { icons } from "../../ui/icons";
  import { showError } from "../../ui/dialog";
  import { api } from "../../services/api";
  import { loadImageBlobUrl } from "../../services/images";
  import { appStore } from "../../stores/app.svelte"; // 名单格式码（所在地/偏好工作列语义）+ 气泡失效键
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
  function detailText(e: (typeof entries)[number]) {
    if (e.status === "failed") return e.error || "—";
    if (e.status === "success")
      return t.gen.queue.imagesDone(e.images?.length ?? 0);
    return "—";
  }

  // ---- 条目行身份信息（S9）：头像 + 年龄/所在地，帮认出同名同姓的球员 ----
  // 头像路径/缩略图走共用的 createAvatarSource（与名单预览同一实现，取失败静默降级为无图）。
  // 资料走 query_players(search=uid)：只读后端 rtf_cache（线性扫，无 XML 兜底解析，逐条 IPC 可控）；
  // 不用 preview_player——它的映射兜底会在缓存未命中时整表解析存档快照（秒级且堵主线程）。
  // 条目来自上一次扫描时资料可能查不到：该行降级为仅姓名/UID，不报错。
  const rowAvatars = createAvatarSource();
  let rowInfo = $state<Record<string, PlayerRecord | null>>({});
  const infoCache = new Map<string, PlayerRecord | null>(); // 非响应式缓存：避免逐条写入触发 effect 重跑
  let infoToken = 0;
  $effect(() => {
    if (!open || entries.length === 0) return; // 收起状态条不需要身份信息，不拉
    const uids = [...new Set(entries.map((e) => e.uid))];
    void rowAvatars.load(uids); // 头像与资料并行查：各自的令牌防过期回写
    const token = ++infoToken;
    const missing = uids.filter((u) => !infoCache.has(u));
    for (const u of missing) infoCache.set(u, null); // 先占位，防并发期间重复查询
    void (async () => {
      if (missing.length > 0) {
        await Promise.all(
          missing.map(async (u) => {
            try {
              const page = await api.queryPlayers({
                offset: 0,
                limit: 1,
                search: u,
                sortKey: "",
                sortDesc: false,
                newgan: null,
              });
              infoCache.set(u, page.items[0] ?? null);
            } catch {
              infoCache.set(u, null);
            }
          }),
        );
      }
      if (token !== infoToken) return; // 期间条目已变：过期结果整批作废
      rowInfo = Object.fromEntries(uids.map((u) => [u, infoCache.get(u) ?? null]));
    })();
  });
  // 第 15 列语义与名单预览/单人替换同口径：extStaff 名单的「所在地」列实为「偏好工作」
  const dockIsStaff = $derived(appStore.lastRtfScan?.formatCode === "extStaff");
  /** 行的年龄（拓展名单才有；缺资料/基础名单显示 —）。 */
  function rowAge(e: QueueEntry): string {
    return rowInfo[e.uid]?.detailed?.age?.trim() || "—";
  }
  /** 行的所在地（extStaff 名单显示偏好工作；缺资料显示 —）。 */
  function rowWhere(e: QueueEntry): string {
    const d = rowInfo[e.uid]?.detailed;
    if (!d) return "—";
    return (dockIsStaff ? d.preferredJob : d.based).trim() || "—";
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

  /** 展开/收起成功行：首次展开时按 images[].path 懒载 blob URL（卸载/收起不主动回收，
   *  队列条目存活期内重复展开复用缓存；清空队列随条目消失由 LRU 临时目录兜底）。 */
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
    thumbs[e.id] = loaded;
  }

  async function onSaveThumb(imgId: string) {
    const target = await save({
      defaultPath: `${imgId}.png`,
      filters: [{ name: "PNG", extensions: ["png"] }],
    });
    if (!target) return;
    try {
      await api.saveGeneratedImage(imgId, target);
      await message(t.gen.saved, { title: t.dialog.info });
    } catch (e) {
      await showError(e);
    }
  }
</script>

{#if entries.length > 0 && !open}
  <!-- 收起态状态条：整条可点展开；运行中带迷你进度，否则显示排队/完成计数 -->
  <button
    class="dock-bar"
    onclick={() => (open = true)}
    title={t.gen.queue.expandTitle}
  >
    <svg
      class="db-ic"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true">{@html icons.play.body}</svg
    >
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
    <svg
      class="db-chev"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true">{@html icons.chevronUp.body}</svg
    >
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
        <svg
          viewBox="0 0 24 24"
          width="14"
          height="14"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true">{@html icons.chevronDown.body}</svg
        >
      </button>
    </div>

    <div class="toolbar">
      <button
        class="btn sm primary"
        disabled={imageaiStore.queueRunning || queuedCount === 0}
        onclick={onStart}
      >
        <svg
          viewBox="0 0 24 24"
          width="11"
          height="11"
          fill="currentColor"
          aria-hidden="true">{@html icons.play.body}</svg
        >
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
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          aria-hidden="true">{@html icons.image.body}</svg
        >
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
              <th class="static">{dockIsStaff ? t.table.colJob : t.table.colBased}</th>
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
                            onclick={() => void onSaveThumb(im.id)}
                          >
                            <svg
                              viewBox="0 0 24 24"
                              fill="none"
                              stroke="currentColor"
                              stroke-width="2"
                              stroke-linecap="round"
                              stroke-linejoin="round"
                              aria-hidden="true"
                              >{@html icons.download.body}</svg
                            >
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
  .db-ic {
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
  .db-chev {
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
