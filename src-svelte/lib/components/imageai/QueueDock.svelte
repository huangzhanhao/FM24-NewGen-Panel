<!-- QueueDock · 生成队列卡：左栏「球员源·单人」下方常驻的批量任务面板，空队列也显示空态提示。
     条目为纵向列表：勾选+姓名+头像+状态一行，
     UID·年龄·所在地一行，详情/操作只在「有可读信息」的条目上渲染（成功＝张数、失败＝原因+建议）；
     排队/生成中没有可操作信息，整行不渲染——摆个「—」是噪音；已取消同理（行内无待办，胶囊已写明状态）。
     事件订阅是进程级的：gen:queue:* 由 imageai store 的 initQueueListeners（App 挂载调一次）统一接收，
     进度/收尾载荷写在 store 里——切走 AI 台期间事件不丢、回台无需重新 query，本组件只负责展示。 -->
<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import { confirm, message } from "@tauri-apps/plugin-dialog";
  import { t } from "../../text";
  import Icon from "../../ui/Icon.svelte";
  import { saveGeneratedImageAs } from "../../ui/generatedImageSave"; // 生成图另存为（与画廊共用）
  import { showError } from "../../ui/dialog";
  import { api } from "../../services/api";
  import { loadImageBlobUrl } from "../../services/images";
  import { appStore, basedValue, tip } from "../../stores/app.svelte"; // 第 15 列口径 basedValue + 气泡失效键 + 悬停提示 action
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 条目行头像缩略图 + 放大气泡
  import { createAvatarSource } from "../../avatarBlobs.svelte"; // 头像路径查询+缩略图装载（与名单预览/球员源卡共用）
  import {
    imageaiStore,
    isRetryableErrorKind,
    queueCancel,
    queueBadgeKey,
    queueClearSelection,
    queuePendingConfirmIds,
    queueRemove,
    queueRetry,
    queueSelectEntry,
    queueStart,
    queueStartIds,
    setSourcePlayer,
    shouldShowRetriedBadge,
  } from "../../stores/imageai.svelte";
  import type { GenImage, PlayerRecord, QueueEntry } from "../../types";

  // 进度/收尾载荷来自进程级订阅（imageai store），组件卸载不影响事件接收。
  const progress = $derived(imageaiStore.queueProgress); // 最近一次进度事件载荷（驱动进度条即时更新）
  const lastFinished = $derived(imageaiStore.queueFinished); // 最近一次批量收尾摘要
  let picked = $state<Record<string, boolean>>({}); // 队列条目勾选（key=条目 id，组件内会话态）
  let expandedId = $state<string | null>(null); // 展开缩略图的成功条目 id（同时只展一条）
  const thumbs = $state<Record<string, Array<{ id: string; url: string }>>>({}); // 条目 id → 缩略图 blob URL

  const entries = $derived(imageaiStore.queue);
  const queuedCount = $derived(entries.filter((e) => e.status === "queued").length);
  /** 本次要提交的条目（判据在 store 的 queueStartIds，有单测）。 */
  const startIds = $derived(queueStartIds(entries, picked));
  const runningCount = $derived(entries.filter((e) => e.status === "running").length);
  const pickedIds = $derived(entries.filter((e) => picked[e.id]).map((e) => e.id));
  const allPicked = $derived(entries.length > 0 && pickedIds.length === entries.length);
  /** 恰好勾选一条时的该条目（「查看单人」的操作对象；0 条或多选时为 null）。 */
  const singlePicked = $derived(pickedIds.length === 1 ? (entries.find((e) => e.id === pickedIds[0]) ?? null) : null);
  const progressPct = $derived(
    progress && progress.total > 0 ? Math.round((progress.current / progress.total) * 100) : 0,
  );

  /** 该行是否处于「待确认」形态：只有手动模式下的排队条目才需要逐条核对参数。
   *  运行中/终态条目没有可改的东西，保持原外观即可。 */
  function needConfirm(e: QueueEntry): boolean {
    return !imageaiStore.queueAutoConfirm && e.status === "queued";
  }

  /** 选中行随队列形态收口：切回自动模式、队列开跑、被选中的那条已从快照里消失，
   *  都必须把选中清掉并归还表单——否则右侧工作台会继续显示一条已不该核对的参数。 */
  $effect(() => {
    const id = imageaiStore.queueSelectedId;
    if (id === null) return;
    if (imageaiStore.queueAutoConfirm || imageaiStore.queueRunning || !entries.some((e) => e.id === id)) {
      queueClearSelection();
    }
  });

  /** 状态标签与配色都取 [`queueBadgeKey`]（store 里唯一一份判据，已取消单独一档）。 */
  function statusText(e: QueueEntry) {
    switch (queueBadgeKey(e)) {
      case "queued":
        return t.gen.queue.statusQueued;
      case "running":
        return t.gen.queue.statusRunning;
      case "success":
        return t.gen.queue.statusSuccess;
      case "cancelled":
        return t.gen.queue.statusCancelled;
      default:
        return t.gen.queue.statusFailed;
    }
  }
  /** 运行中条目的阶段文案。只在**进度行**展示一次（挂在条目行会与状态胶囊并排出现
   *  「生成中 · 排队中」这种自相矛盾的读法）；串行下当前事件必属唯一在跑的那条。 */
  const stageSuffix = $derived(
    progress?.stage && imageaiStore.queueRunning ? ` · ${t.gen.queue.stageLabel(progress.stage)}` : "",
  );
  /** 详情行：只在**有可读信息**的终态渲染。失败显示原因 + 按类别的处置建议，成功显示张数；
   *  排队中/生成中没有可操作信息，返回 null＝整个详情块不渲染（摆「—」是噪音，见文件头注释）。
   *  已取消是用户自己按下的终态（status=failed + errorKind=cancelled）：行内既无待办也无按钮，
   *  状态胶囊已写明「已取消」，同样不渲染详情行。
   *  失败但后端没给原因（旧快照）时回落 "—"：那**确实**是「失败了却没有原因可读」这条信息本身。 */
  function detailText(e: QueueEntry): string | null {
    if (e.status === "failed") {
      // 已取消是我们自己置的终态，不是故障：不展示「原因（处置建议）」那一套。
      if (e.errorKind === "cancelled") return null;
      const base = e.error || "—";
      const action = e.errorKind ? t.gen.queue.errorAction(e.errorKind) : "";
      return action ? `${base}（${action}）` : base;
    }
    if (e.status === "success") return t.gen.queue.imagesDone(e.images?.length ?? 0);
    return null;
  }

  // ---- 条目行身份信息：头像 + 年龄/所在地，帮认出同名同姓的球员 -----------------------------------------------------
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

  // ---- 展开缩略图的 blob 回收（泄漏防线） ---------------------------------------------------------------------------
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

  /** 行上的完整人员信息（悬停提示用）：逐项一行，空项不占行。标签复用名单表头的三语键。
   *  只读已拉过的 rowInfo 与条目自带的 req（名单查不到时回落入队时的国籍/俱乐部），不发新 IPC。 */
  function personTip(e: QueueEntry): string {
    const rec = rowInfo[e.uid];
    const d = rec?.detailed;
    const col = t.table;
    const lines: string[] = [e.name];
    const uid = e.uid.trim();
    if (uid) lines.push(`${col.colUid} ${uid}`);
    // 国籍：名单未收录该球员时，只能展示入队那一刻记下的国籍。
    const first = rec?.primaryNat?.trim() ?? "";
    const second = rec?.secNat?.trim() ?? "";
    const nat = first ? (second && second !== first ? `${first} / ${second}` : first) : (e.req.nationality?.trim() ?? "");
    if (nat) lines.push(`${col.colNat} ${nat}`);
    const club = d?.club?.trim() || e.req.club?.trim() || "";
    if (club) lines.push(`${col.colClub} ${club}`);
    const where = basedValue(d ?? null).trim();
    if (where) lines.push(`${col.colBased} ${where}`);
    const age = d?.age?.trim() ?? "";
    if (age) lines.push(`${col.colAge} ${age}`);
    const height = d?.height?.trim() ?? "";
    if (height) lines.push(`${col.colHeight} ${height}cm`);
    const ca = d?.ca?.trim() ?? "";
    const pa = d?.pa?.trim() ?? "";
    if (ca && pa) lines.push(`${col.colCa} ${ca} / ${col.colPa} ${pa}`);
    // 名单与条目都没有人员信息（无 uid）时只剩姓名，不必给一个空气泡。
    return lines.filter(Boolean).join("\n");
  }

  // ---- 手动模式的逐条确认：选中落到 store（右侧工作台读同一份状态）。
  //      「确认」按钮住右侧参数卡底部（GenSettingsCard，仅手动模式渲染），行内只负责选中与状态展示 ----
  /** 行主体可键盘选中：Enter/Space 与点击等价（Space 默认会滚列表，先拦下）。 */
  function onKeySelect(ev: KeyboardEvent, e: QueueEntry) {
    if (ev.key !== "Enter" && ev.key !== " ") return;
    ev.preventDefault();
    void onSelect(e);
  }
  /** 点行/键盘选中都走这里：借用失败（该行入队后服务商或模型已从配置里删改）就地告知，
   *  不进借用态——能力位未知时表单会把高级参数显示成「不支持」，确认回去就等于把它们清空。 */
  async function onSelect(e: QueueEntry) {
    if (!queueSelectEntry(e.id)) {
      await message(t.gen.queue.borrowUnavailable(e.name), { title: t.dialog.info });
    }
  }

  function toggleAll(v: boolean) {
    const next = { ...picked };
    for (const e of entries) next[e.id] = v;
    picked = next;
  }

  // 启动是「将产生费用」的确认（信息级）；取消/移除是破坏性确认（警告级）——均走系统弹窗。
  async function onStart() {
    const ids = startIds;
    if (ids.length === 0) return;
    // 手动确认：本次要跑的每一条都必须先逐条确认过。有漏的就地拦下（不启动、不写队列），
    // 把「还有几条没核对」说清楚，用户回到列表逐条点确认即可。
    // 走信息级弹窗：这只是「还没点完确认」，不是出错，别用错误标题吓人。
    if (!imageaiStore.queueAutoConfirm) {
      const pending = queuePendingConfirmIds(entries, ids);
      if (pending.length > 0) {
        await message(t.gen.queue.needConfirm(pending.length), { title: t.dialog.info });
        return;
      }
    }
    // 文案必须与实际提交条数一致：只跑勾选时说明剩下几条会留在队列里。
    const rest = queuedCount - ids.length;
    const text = rest > 0 ? t.gen.queue.startConfirmPicked(ids.length, rest) : t.gen.queue.startConfirm(ids.length);
    if (!(await confirm(text, { title: t.dialog.info }))) return;
    try {
      // 收尾摘要的复位在 queueStart action 内（store 单点维护，组件不直写共享态）。
      await queueStart(rest > 0 ? ids : undefined);
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
      <small class="muted small mono">{t.gen.queue.progressCount(progress.current, progress.total)}</small>
    {/if}
  </div>

  <!-- 工具按钮 2×2：四个等宽按钮一行放不下窄卡（约 300px），网格布局两行稳定不换行抖动 -->
  <div class="toolbar">
    <button
      class="btn sm primary queue-start"
      disabled={imageaiStore.queueRunning || startIds.length === 0}
      onclick={onStart}
    >
      <Icon name="play" size={11} />
      <span class="queue-start-label">{t.gen.queue.startBtn}</span>
    </button>
    <button
      class="btn sm"
      disabled={!imageaiStore.queueRunning || imageaiStore.queueCancelling}
      onclick={onCancel}
    >
      {imageaiStore.queueCancelling ? t.gen.queue.cancellingBtn : t.gen.queue.cancelBtn}
    </button>
    <button
      class="btn sm"
      disabled={!singlePicked}
      use:tip={singlePicked ? t.gen.queue.viewSingleTitle(singlePicked.name) : t.gen.queue.viewSingleHint}
      onclick={onViewSingle}
    >
      {t.gen.queue.viewSingle}
    </button>
    <button class="btn sm danger" disabled={imageaiStore.queueRunning || pickedIds.length === 0} onclick={onRemove}>
      {t.gen.queue.removeBtn}
    </button>
  </div>

  <!-- 确认方式：两档全局分段控件（与「保留|覆写|生成」同款 .seg3）独占整行——
       工具栏 2×2 网格放不下第四档语义，混进去会让四个按钮都变窄。
       默认全自动＝点启动直接提交（原行为）；手动确认才要求逐条核对参数，日常批量不被打断。
       运行中两档都禁用：此刻改确认语义对正在跑的这批没有意义，只会让界面自相矛盾。 -->
  <div class="mode-row">
    <div class="seg3" role="group" aria-label={t.gen.queue.confirmModeAria}>
      <button
        type="button"
        class:active={!imageaiStore.queueAutoConfirm}
        disabled={imageaiStore.queueRunning}
        onclick={() => (imageaiStore.queueAutoConfirm = false)}>{t.gen.queue.confirmModeManual}</button
      >
      <button
        type="button"
        class:active={imageaiStore.queueAutoConfirm}
        disabled={imageaiStore.queueRunning}
        onclick={() => {
          imageaiStore.queueAutoConfirm = true;
          queueClearSelection(); // 切回全自动：不再有「逐条核对」这回事，选中与借用一并收口
        }}>{t.gen.queue.confirmModeAuto}</button
      >
    </div>
    {#if !imageaiStore.queueAutoConfirm}
      <p class="muted mode-hint">{t.gen.queue.confirmModeHintManual}</p>
    {/if}
  </div>

  {#if imageaiStore.queueRunning && progress}
    <div class="progress-row">
      <span class="muted small">{t.gen.queue.progressLabel}{stageSuffix}</span>
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

  <!-- 取消是协作式的：点完到真正收尾之间有真空期，这里明说「还在收尾」，
       否则用户会以为按钮没生效而反复点。 -->
  {#if imageaiStore.queueCancelling}
    <p class="muted small cancel-note">{t.gen.queue.cancellingHint}</p>
  {/if}

  {#if entries.length === 0}
    <div class="empty dock-empty">
      <Icon name="image" stroke={1.6} />
      <p>{t.gen.queue.empty}</p>
    </div>
  {:else}
    <!-- 表头在滚动区之外：长列表滚动时全选与列名不跟着滚走 -->
    <div class="q-head">
      <input
        type="checkbox"
        aria-label={t.gen.queue.selectAllAria}
        checked={allPicked}
        indeterminate={pickedIds.length > 0 && !allPicked}
        onchange={(e) => toggleAll(e.currentTarget.checked)}
      />
      <span class="q-head-player">{t.gen.queue.colPlayer}</span>
      <span class="q-head-count">{t.gen.queue.count(entries.length)}</span>
      <span class="q-head-status">{t.gen.colStatus}</span>
    </div>
    <div class="list-scroll">
      <!-- 姓名/副行两侧形态共用一份内容（手动模式多一层「可点选中」的外壳，自动模式是纯展示） -->
      {#snippet idText(e: QueueEntry)}
        <span class="q-name">{e.name}</span>
        <span class="q-sub mono">{subText(e)}</span>
      {/snippet}
      {#each entries as e (e.id)}
        {@const editMode = needConfirm(e)}
        {@const confirmed = !!imageaiStore.queueConfirmedIds[e.id]}
        {@const detail = detailText(e)}
        {@const canViewImages = e.status === "success" && e.images?.length > 0}
        {@const canRetry = e.status === "failed" && !imageaiStore.queueRunning && isRetryableErrorKind(e.errorKind)}
        <!-- 手动模式下排队中的行进入「待确认」形态：橙框=未确认、绿框=已确认、被选中再加一圈同色内描边。
             自动模式下这三档 class 都不出现，行外观不带状态框。
             悬停气泡挂在整行上（tip action）——窄卡里副行只放得下 UID·年龄·所在地，其余信息看气泡。 -->
        <div
          class="q-item"
          class:picked={!!picked[e.id]}
          class:pending={editMode}
          class:unconfirmed={editMode && !confirmed}
          class:confirmed={editMode && confirmed}
          class:selected={editMode && imageaiStore.queueSelectedId === e.id}
          use:tip={personTip(e)}
        >
          <div class="q-top">
            <input
              type="checkbox"
              aria-label={t.gen.queue.selectRowAria(e.name)}
              checked={!!picked[e.id]}
              onchange={(ev) => (picked[e.id] = ev.currentTarget.checked)}
            />
            <!-- 点主体=选中该行去右侧核对参数；复选框/头像/状态胶囊都不触发选中。 -->
            {#if editMode}
              <div
                class="q-id clickable"
                role="button"
                tabindex="0"
                aria-pressed={imageaiStore.queueSelectedId === e.id}
                onclick={() => void onSelect(e)}
                onkeydown={(ev) => onKeySelect(ev, e)}
              >
                {@render idText(e)}
              </div>
            {:else}
              <div class="q-id">
                {@render idText(e)}
              </div>
            {/if}
            <AvatarThumb
              itemKey={e.uid}
              url={rowAvatars.urls[e.uid]}
              path={rowAvatars.paths[e.uid] ?? undefined}
              groupKey={appStore.snapshot?.currentGroup}
              placeholder=""
            />
            <span class="pill q-status {queueBadgeKey(e)}">{statusText(e)}</span>
            {#if shouldShowRetriedBadge(e.attempts)}
              <span class="pill q-retried">{t.gen.queue.retriedBadge(e.attempts ?? 0)}</span>
            {/if}
          </div>
          <!-- 详情/操作行：来源（详情文本、查看图片/重试）全都为空时整行不渲染。
               排队/生成中/已取消的条目就是这种形态——不放一条只有「—」的空行占高度；
               核对参数用的「确认」按钮在右侧参数卡底部（仅手动确认模式）。 -->
          {#if detail !== null || canViewImages || canRetry}
            <div class="q-detail">
              {#if detail !== null}
                <span class="detail" use:tip={detail}>{detail}</span>
              {/if}
              {#if canViewImages}
                <button
                  type="button"
                  class="btn sm link-btn"
                  aria-expanded={expandedId === e.id}
                  onclick={() => void toggleThumbs(e)}
                >
                  {expandedId === e.id ? t.gen.queue.collapseImages : t.gen.queue.viewImages}
                </button>
              {/if}
              {#if canRetry}
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
                    use:tip={t.gen.queue.saveImage}
                    onclick={() => void saveGeneratedImageAs(im.id)}
                  >
                    <Icon name="download" size={12} />
                    {t.gen.queue.saveImage}
                  </button>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
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
  /* 「取消中」提示：与收尾摘要同款外观，但用警示色区分「尚在收尾」与「已结束」 */
  .cancel-note {
    margin: 0;
    color: var(--warn);
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
  /* 已取消：中性灰（用户自己按的取消，不是故障），与 failed 的危险色明确区分 */
  .q-status.cancelled {
    color: var(--text-3);
    background: var(--bg-raised);
  }
  /* 执行期重试徽标：几何复用 .pill，仅覆盖为警示色；与状态胶囊同排故在 Running 期间即可见
     （attempts 每次尝试前自增回写）。双类选择器提权以压过全局 .pill。 */
  .pill.q-retried {
    flex: none;
    white-space: nowrap;
    color: var(--warn);
    background: var(--warn-soft);
  }
  /* 运行阶段文案：与状态胶囊同排，不收缩不换行（窄卡下优先保持可读）。 */
  /* 条目列表：顶行 = 勾选 + 姓名/副行 + 头像 + 状态胶囊；次行 = 详情与操作（仅成功/失败等有信息的条目） */
  .list-scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .q-item {
    padding: var(--sp-3) var(--sp-2);
    border-radius: var(--radius-md);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out);
  }
  .q-item + .q-item {
    border-top: 1px solid var(--hairline-2);
  }
  .q-item.picked {
    background: var(--accent-soft);
  }
  /* ---- 手动模式：待确认 / 已确认（橙 / 绿） ----
     用 outline 而不是 border：outline 不占布局，行高与相邻行的分隔线都不受状态影响
     （border 会撑高 2px，且与 .q-item + .q-item 的灰线叠成双线）。-1px 偏移把描边收进行内。 */
  .q-item.unconfirmed {
    outline: 1px solid var(--warn);
    outline-offset: -1px;
  }
  .q-item.confirmed {
    outline: 1px solid var(--ok);
    outline-offset: -1px;
  }
  /* 选中的行：同色软底 + 内描边，视觉上明显强于「只描了边的未选中态」。
     颜色与描边同源（同一状态档），不会出现绿框配橙底。 */
  .q-item.unconfirmed.selected {
    background: var(--warn-soft);
    box-shadow: inset 0 0 0 1px var(--warn);
  }
  .q-item.confirmed.selected {
    background: var(--ok-soft);
    box-shadow: inset 0 0 0 1px var(--ok);
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
  /* 可点主体：只有手动模式下的排队条目能点（点它=去右侧核对这一条）。给手型与键盘焦点环，
     焦点环走全局 --focus-ring，与其余控件同款 */
  .q-id.clickable {
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .q-id.clickable:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3px var(--focus-ring);
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
    /* 详情文本缺席的行（成功/失败只剩操作按钮时）按钮贴右缘，与有详情文本时的位置一致——
       否则按钮会突然跳到左侧 22px 缩进处 */
    justify-content: flex-end;
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
  /* 表头：不参与列表滚动（.list-scroll 是唯一纵向滚动容器）；左内距与行内 .q-item 一致，
     勾选框左缘因此与每行对齐 */
  .q-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: 0 var(--sp-2) var(--sp-1);
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  .q-head-player {
    flex: none;
  }
  .q-head-count {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .q-head-status {
    flex: none;
  }
  /* 确认方式：全局 .seg3 分段控件占满整行（与「保留|覆写|生成」同款）。
     几何仍走全局档，只在这张窄卡里收窄左右内边距——否则「手动确认|全自动」两个标签
     叠加的最小内容宽会顶出容器。 */
  .mode-row {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    width: 100%;
  }
  .mode-row .seg3 {
    width: 100%;
  }
  .mode-row .seg3 button {
    padding: var(--sp-1) var(--sp-4);
  }
  .mode-hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-3);
    overflow-wrap: anywhere;
  }
</style>
