<!-- QueueDock · 生成队列卡：左栏「球员源·单人」下方常驻的批量任务面板，空队列也显示空态提示。
     条目为纵向列表：勾选+姓名+头像+状态一行，
     UID·年龄·所在地一行，详情/操作仅成功与失败的条目渲染（排队/生成中没有可操作信息，摆「—」是噪音）。
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
  import { appStore, basedValue } from "../../stores/app.svelte"; // 第 15 列口径 basedValue + 气泡失效键
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 条目行头像缩略图 + 放大气泡
  import { createAvatarSource } from "../../avatarBlobs.svelte"; // 头像路径查询+缩略图装载（与名单预览/球员源卡共用）
  import {
    imageaiStore,
    queueCancel,
    queueRemove,
    queueRetry,
    queueStart,
    setSourcePlayer,
  } from "../../stores/imageai.svelte";
  import type { GenImage, PlayerRecord, QueueEntry, QueueStatus } from "../../types";

  // 进度/收尾载荷来自进程级订阅（imageai store），组件卸载不影响事件接收。
  const progress = $derived(imageaiStore.queueProgress); // 最近一次进度事件载荷（驱动进度条即时更新）
  const lastFinished = $derived(imageaiStore.queueFinished); // 最近一次批量收尾摘要
  let picked = $state<Record<string, boolean>>({}); // 队列条目勾选（key=条目 id，组件内会话态）
  let expandedId = $state<string | null>(null); // 展开缩略图的成功条目 id（同时只展一条）
  const thumbs = $state<Record<string, Array<{ id: string; url: string }>>>({}); // 条目 id → 缩略图 blob URL

  const entries = $derived(imageaiStore.queue);
  const queuedCount = $derived(entries.filter((e) => e.status === "queued").length);
  const runningCount = $derived(entries.filter((e) => e.status === "running").length);
  const pickedIds = $derived(entries.filter((e) => picked[e.id]).map((e) => e.id));
  const allPicked = $derived(entries.length > 0 && pickedIds.length === entries.length);
  /** 恰好勾选一条时的该条目（「查看单人」的操作对象；0 条或多选时为 null）。 */
  const singlePicked = $derived(pickedIds.length === 1 ? (entries.find((e) => e.id === pickedIds[0]) ?? null) : null);
  const progressPct = $derived(
    progress && progress.total > 0 ? Math.round((progress.current / progress.total) * 100) : 0,
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
  /** 详情行：失败显示原因，成功显示张数，其余留 —。 */
  function detailText(e: QueueEntry) {
    if (e.status === "failed") return e.error || "—";
    if (e.status === "success") return t.gen.queue.imagesDone(e.images?.length ?? 0);
    return "—";
  }

  // ---- 条目行身份信息：头像 + 年龄/所在地，帮认出同名同姓的球员 ----
  // 头像路径/缩略图走共用的 createAvatarSource（与名单预览同一实现，取失败静默降级为无图）。
  // 资料走 query_players_by_uids 批量取：一次 IPC 覆盖全部条目；批量失败整批降级为仅姓名/UID、不报错。
  // 不用 preview_player——它的映射兜底会在缓存未命中时整表解析存档快照（秒级且堵主线程）。
  // 条目来自上一次扫描时资料可能查不到：该行降级。
  const rowAvatars = createAvatarSource();
  let rowInfo = $state<Record<string, PlayerRecord | null>>({});
  const infoCache = new Map<string, PlayerRecord | null>(); // 非响应式缓存：避免逐条写入触发 effect 重跑
  let infoToken = 0;
  let loadedUidKey = ""; // 已拉过身份信息的 uid 稳定键（非响应式，仅 effect 内读写）
  $effect(() => {
    if (entries.length === 0) {
      loadedUidKey = ""; // 清空后再入队：重新拉一次
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
          for (const u of missing) infoCache.set(u, null);
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
  // 第 15 列口径单点在 appStore（isStaffRoster/basedLabel/basedValue）。
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
  /** 条目副行文本：UID 打头（唯一稳定标识），年龄/所在地有资料才追加——窄卡里不摆无资料的「—」。 */
  function subText(e: QueueEntry): string {
    const parts = [e.uid];
    const age = rowAge(e);
    if (age !== "—") parts.push(age);
    const where = rowWhere(e);
    if (where !== "—") parts.push(where);
    return parts.join(" · ");
  }

  function toggleAll(v: boolean) {
    const next = { ...picked };
    for (const e of entries) next[e.id] = v;
    picked = next;
  }

  // 启动是「将产生费用」的确认（信息级）；取消/移除是破坏性确认（警告级）——均走系统弹窗。
  async function onStart() {
    if (queuedCount === 0) return;
    if (
      !(await confirm(t.gen.queue.startConfirm(queuedCount), {
        title: t.dialog.info,
      }))
    )
      return;
    try {
      // 收尾摘要的复位在 queueStart action 内（store 单点维护，组件不直写共享态）。
      await queueStart();
    } catch (e) {
      await showError(e);
    }
  }
  async function onCancel() {
    if (!(await confirm(t.gen.queue.cancelConfirm, { title: t.dialog.warning }))) return;
    try {
      await queueCancel();
    } catch (e) {
      await showError(e);
    }
  }
  /** 「查看单人」：把唯一勾选条目的球员转入「球员源·单人」卡（单人卡只容一位，多选时按钮禁用）。
   *  只写 sourcePlayer 单点，档案/头像由数据源卡自行拉取；清空队列的职责由全选 + 移除选中覆盖。 */
  function onViewSingle() {
    if (!singlePicked) return;
    setSourcePlayer(singlePicked.uid, singlePicked.name);
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
   *  条目存活期内重复展开直接复用；条目从快照消失或组件卸载时由上方 effect / onDestroy 统一 revoke。 */
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

<div class="dock pcard">
  <div class="dock-head pcard-h">
    <span>{t.gen.queue.tabQueue}</span>
    {#if imageaiStore.queueRunning && progress}
      <small class="muted small mono">{progress.current} / {progress.total}</small>
    {/if}
  </div>

  <!-- 工具按钮 2×2：四个等宽按钮一行放不下窄卡（约 300px），网格布局两行稳定不换行抖动 -->
  <div class="toolbar">
    <button
      class="btn sm primary queue-start"
      disabled={imageaiStore.queueRunning || queuedCount === 0}
      onclick={onStart}
    >
      <Icon name="play" size={11} />
      <span class="queue-start-label">{t.gen.queue.startBtn}</span>
    </button>
    <button class="btn sm" disabled={!imageaiStore.queueRunning} onclick={onCancel}>{t.gen.queue.cancelBtn}</button>
    <button
      class="btn sm"
      disabled={!singlePicked}
      title={singlePicked ? t.gen.queue.viewSingleTitle(singlePicked.name) : t.gen.queue.viewSingleHint}
      onclick={onViewSingle}
    >
      {t.gen.queue.viewSingle}
    </button>
    <button class="btn sm danger" disabled={imageaiStore.queueRunning || pickedIds.length === 0} onclick={onRemove}>
      {t.gen.queue.removeBtn}
    </button>
  </div>

  {#if imageaiStore.queueRunning && progress}
    <div class="progress-row">
      <span class="muted small">{t.gen.queue.progressLabel}</span>
      <div
        class="progress-track"
        role="progressbar"
        aria-valuenow={progress.current}
        aria-valuemin={0}
        aria-valuemax={progress.total}
      >
        <div class="progress-fill" style={`width:${progressPct}%`}></div>
      </div>
    </div>
  {:else if lastFinished}
    <div class="muted small finished-row">
      {t.gen.queue.finishedSummary(lastFinished.finished, lastFinished.failed, lastFinished.total)}
    </div>
  {/if}

  {#if entries.length === 0}
    <div class="empty dock-empty">
      <Icon name="image" stroke={1.6} />
      <p>{t.gen.queue.empty}</p>
    </div>
  {:else}
    <div class="list-scroll">
      {#each entries as e (e.id)}
        <div class="q-item" class:picked={!!picked[e.id]}>
          <div class="q-top">
            <input
              type="checkbox"
              aria-label={t.gen.queue.selectRowAria(e.name)}
              checked={!!picked[e.id]}
              onchange={(ev) => (picked[e.id] = ev.currentTarget.checked)}
            />
            <div class="q-id">
              <span class="q-name" title={e.name}>{e.name}</span>
              <span class="q-sub mono" title={subText(e)}>{subText(e)}</span>
            </div>
            <AvatarThumb
              itemKey={e.uid}
              url={rowAvatars.urls[e.uid]}
              path={rowAvatars.paths[e.uid] ?? undefined}
              groupKey={appStore.snapshot?.currentGroup}
              placeholder=""
            />
            <span class="pill q-status {e.status}">{statusText(e.status)}</span>
          </div>
          {#if e.status === "success" || e.status === "failed"}
            <div class="q-detail">
              <span class="detail" title={detailText(e)}>{detailText(e)}</span>
              {#if e.status === "success" && e.images?.length > 0}
                <button
                  type="button"
                  class="btn sm link-btn"
                  aria-expanded={expandedId === e.id}
                  onclick={() => void toggleThumbs(e)}
                >
                  {expandedId === e.id ? t.gen.queue.collapseImages : t.gen.queue.viewImages}
                </button>
              {/if}
              {#if e.status === "failed" && !imageaiStore.queueRunning}
                <button type="button" class="btn sm link-btn" onclick={() => void onRetry(e)}>
                  {t.gen.queue.retryEntry}
                </button>
              {/if}
            </div>
          {/if}
          {#if expandedId === e.id && e.images?.length > 0}
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
          {/if}
        </div>
      {/each}
    </div>
    <div class="foot">
      <!-- 全选框并入计数行：窄卡里不另占一行 -->
      <label class="foot-all">
        <input
          type="checkbox"
          aria-label={t.gen.queue.selectAllAria}
          checked={allPicked}
          indeterminate={pickedIds.length > 0 && !allPicked}
          onchange={(e) => toggleAll(e.currentTarget.checked)}
        />
        <span class="muted small">{t.gen.queue.count(entries.length)}</span>
      </label>
    </div>
  {/if}
</div>

<style>
  /* 展开态：.pcard 壳 + flex:1 吃「球员源」卡下方剩余空白（最低 250px 保可用），
     列表自己滚，队列再长也不把左栏撑出滚动。 */
  .dock {
    flex: 1 1 0;
    min-height: 250px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .dock-head {
    margin-bottom: 0; /* .pcard-h 自带 margin-bottom，会与 .dock 的 gap 叠加过松 */
  }
  .toolbar {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--sp-2);
  }
  /* 启动生成：文字与其余三个纯文字按钮同中线，播放图标悬入左列留白
     （两侧 1fr + 对称列距保证中列居中；仅覆盖排版，几何/字号仍走 .btn.sm 契约） */
  .queue-start {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
  }
  .queue-start :global(svg) {
    grid-column: 1;
    justify-self: end;
  }
  .queue-start-label {
    grid-column: 2;
  }
  .progress-row {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
  }
  /* 进度条轨道/填充用全局 .progress-track/.progress-fill 唯一原语 */
  /* 窄卡里收尾摘要允许换行 */
  .finished-row {
    overflow-wrap: anywhere;
  }
  /* 行尾状态胶囊：几何直接复用全局 .pill 原语（标准档 3px/11px + fs-sm）；
     scoped 只补行内防压缩，四态变体仅覆盖色差 */
  .q-status {
    flex: none;
    white-space: nowrap;
  }
  .q-status.queued {
    color: var(--text-2);
    background: var(--bg-raised);
  }
  .q-status.running {
    color: var(--warn);
    background: var(--warn-soft);
  }
  .q-status.success {
    color: var(--ok);
    background: var(--ok-soft);
  }
  .q-status.failed {
    color: var(--danger);
    background: var(--danger-soft);
  }
  /* 条目列表：顶行 = 勾选 + 姓名/副行 + 头像 + 状态胶囊；次行 = 详情与操作（仅成功/失败） */
  .list-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .q-item {
    padding: var(--sp-3) var(--sp-2);
    border-radius: var(--radius-md);
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .q-item + .q-item {
    border-top: 1px solid var(--hairline-2);
  }
  .q-item.picked {
    background: var(--accent-soft);
  }
  .q-top {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
  }
  /* 头像在姓名列与状态胶囊之间：不参与压缩，空间紧张时由 .q-id 省略号让出宽度 */
  .q-top :global(.thumb-btn) {
    flex: none;
  }
  .q-id {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .q-name {
    font-size: var(--fs-md);
    font-weight: var(--fw-semibold);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .q-sub {
    font-size: var(--fs-xs);
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .q-detail {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
    padding-left: 22px; /* 与顶行姓名列对齐（勾选框 14px + 间距 8px） */
  }
  .q-detail .detail {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-sm);
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* 行内链接式小按钮（查看图片/重试本条） */
  .link-btn {
    flex: none;
    border: none;
    background: none;
    padding: 2px var(--sp-1);
    min-height: var(--control-h-xs);
    height: var(--control-h-xs);
    color: var(--accent);
    transition:
      color var(--dur-fast) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out);
  }
  .link-btn:hover:not(:disabled) {
    background: var(--accent-soft);
    color: var(--accent);
  }
  /* 缩略图展开：窄卡里一行最多两张 */
  .thumb-strip {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-4);
    padding: 2px 0 var(--sp-1) 22px;
  }
  .thumb {
    width: var(--thumb-queue-size);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .thumb img {
    width: var(--thumb-queue-size);
    height: var(--thumb-queue-size);
    object-fit: contain;
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    cursor: default;
  }
  .thumb-save {
    width: 100%;
  }
  .dock-empty {
    flex: 1;
    min-height: 0;
  }
  .foot {
    display: flex;
    justify-content: flex-start;
  }
  .foot-all {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    cursor: pointer;
  }
</style>
