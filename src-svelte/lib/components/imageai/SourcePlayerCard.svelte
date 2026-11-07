<!-- SourcePlayerCard · 数据源单人档案卡（左栏）：当前数据源球员的头像/资料胶囊 + 台内 UID 查询 + 「生成」
     三个来源都写 imageaiStore.sourcePlayer：名单行「送生图」、单人替换「送生图」、本卡 UID 查询
     「生成」经 ongenerate 转发父级主 CTA 链路（单张接口）；批量入队只走名单工具行与常驻队列卡
     头像装载、资料 effect 与 UID 查询态都住在本地 -->
<script lang="ts">
  import { api } from "../../services/api";
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 缩略图 + 放大气泡，与名单预览共用
  import { createAvatarSource } from "../../avatarBlobs.svelte"; // 头像路径查询 + 缩略图装载
  import { appStore, basedValue, tip } from "../../stores/app.svelte"; // rosterReady + 第 15 列口径 + 气泡失效键
  import { ui } from "../../stores/ui.svelte"; // 就绪卡「去存档资料台」切工作台
  import { clearSourcePlayer, imageaiStore, runningQueueEntry, setSourcePlayer } from "../../stores/imageai.svelte";
  import { t } from "../../text";
  import type { PlayerPreview } from "../../types";

  let { ongenerate }: { ongenerate: () => void | Promise<void> } = $props();

  /** 队列正在跑的那条：在此期间卡片借用为「这条任务」的只读视图（生成/清除按钮一并收起，
   *  否则卡片显示 A、按钮作用于 B，用户点下去就生成错了人）。跑完自动回到自己的数据源。 */
  const queueTask = $derived(runningQueueEntry());
  const preview = $derived(queueTask !== null);
  /** 卡片实际展示的球员：队列任务优先。 */
  const shownUid = $derived(queueTask?.uid ?? imageaiStore.sourcePlayer?.uid ?? "");
  const shownName = $derived(queueTask?.name ?? imageaiStore.sourcePlayer?.name ?? "");

  // ---- 头像缩略图：走共用的 createAvatarSource；取不到/取失败静默降级为只显示姓名+UID，不弹窗 -----------------------
  const srcAvatars = createAvatarSource();
  $effect(() => {
    const uid = shownUid;
    srcAvatars.clear(); // 换数据源立即清场：旧球员的头像不该闪现在新卡上
    if (uid) void srcAvatars.load([uid]);
  });

  // ---- 球员源资料：preview_player 缓存未命中时要重解析整份档案（秒级），按 uid 记账防重拉 ---------------------------
  // 台内 UID 查询命中时已带回首份资料，effect 靠 srcProfileUid 跳过同 uid 的第二次查询
  let srcProfile = $state<PlayerPreview | null>(null);
  let srcProfileUid = $state(""); // 资料已为哪个 uid 拉取过（失败也记账，避免 effect 反复重试）
  let srcProfileToken = 0;
  $effect(() => {
    const uid = shownUid;
    if (!uid) {
      srcProfile = null;
      srcProfileUid = "";
      return;
    }
    if (srcProfileUid === uid) return;
    const token = ++srcProfileToken;
    api
      .previewPlayer(uid)
      .then((got) => {
        if (token !== srcProfileToken) return;
        srcProfile = got;
        srcProfileUid = uid;
      })
      .catch(() => {
        // uid 不在当前名单（数据源可能来自上一次扫描）：降级为只显示姓名/UID，不弹窗刷屏
        if (token === srcProfileToken) {
          srcProfile = null;
          srcProfileUid = uid;
        }
      });
  });
  const srcRec = $derived(shownUid ? (srcProfile?.record ?? null) : null);

  // ---- 台内 UID 查询：命中即 setSourcePlayer 设为数据源 -------------------------------------------------------------
  let srcUidInput = $state("");
  let srcQueryError = $state("");
  async function querySourceUid() {
    const want = srcUidInput.trim();
    if (!want) return;
    try {
      const got = await api.previewPlayer(want);
      // 回显命中的完整 UID（与 PlayerViewer 同规则），姓名缺失时退回 UID 展示
      const matched = got?.record?.uid ?? got?.mapping?.uid ?? want;
      srcProfile = got; // 资料顺手带入，免得 effect 再查一遍（preview_player 秒级）
      srcProfileUid = matched;
      setSourcePlayer(matched, got?.record?.name?.trim() || matched);
      srcUidInput = "";
      srcQueryError = "";
    } catch {
      // 未命中：数据源保持不变，就地提示（查询动作的失败不必弹系统窗）
      srcQueryError = t.gen.source.queryMiss(want);
    }
  }
</script>

<section class="pcard src-card">
  <div class="pcard-h">
    <span>{t.gen.cardSource}</span>
    {#if preview}
      <!-- 队列占用期间卡片只读：清除/生成都会作用到「另一个球员」，一并收起 -->
      <span class="pill src-queue-pill">{t.gen.queue.queueRunningTag}</span>
    {:else if imageaiStore.sourcePlayer}
      <button class="btn sm" use:tip={t.gen.source.clearTitle} onclick={clearSourcePlayer}>{t.gen.source.clear}</button>
    {/if}
  </div>
  {#if !shownUid}
    {#if !appStore.rosterReady}
      <!-- 就绪卡：名单没扫，UID 查询无从命中 → 直达存档资料台 -->
      <p class="hint">{t.gen.source.scanFirst}</p>
      <button class="btn" onclick={() => (ui.workbench = "archive")}>
        {t.gen.source.goArchive}
      </button>
    {:else}
      <p class="hint">{t.gen.source.queryHint}</p>
      <div class="src-query">
        <input
          type="text"
          placeholder={t.gen.source.queryPlaceholder}
          bind:value={srcUidInput}
          onkeydown={(e) => e.key === "Enter" && querySourceUid()}
        />
        <button class="btn" disabled={!srcUidInput.trim()} onclick={querySourceUid}>{t.gen.source.queryBtn}</button>
      </div>
      {#if srcQueryError}
        <p class="field-error">{srcQueryError}</p>
      {/if}
    {/if}
  {:else}
    <div class="src-head">
      <AvatarThumb
        itemKey={shownUid}
        url={srcAvatars.urls[shownUid]}
        path={srcAvatars.paths[shownUid] ?? undefined}
        groupKey={appStore.snapshot?.currentGroup}
      />
      <div class="src-id">
        <span class="src-name" use:tip={shownName}>{shownName}</span>
        <span class="src-uid mono">{shownUid}</span>
      </div>
    </div>
    {#if srcRec}
      <!-- 信息胶囊单池自动折行（与 PlayerViewer 同语言），顺序对齐名单预览表头列序，空值以 — 占位不坍缩 -->
      <div class="meta-row">
        <span class="meta">{t.table.colNat} <b>{srcRec.primaryNat.trim() || "—"}</b></span>
        <span class="meta">{t.table.col2ndNat} <b>{srcRec.secNat.trim() || "—"}</b></span>
        <span class="meta">{t.table.colHair} <b>{srcRec.hairLength.trim() || "—"}</b></span>
        <span class="meta">{t.table.colColor} <b>{srcRec.hairColor.trim() || "—"}</b></span>
        <span class="meta">{t.table.colEth} <b>{srcRec.ethnicityCode.trim() || "—"}</b></span>
        {#if srcRec.detailed}
          <span class="meta">{t.table.colSkin} <b>{srcRec.detailed.skinCode.trim() || "—"}</b></span>
          <span class="meta">{t.table.colGender} <b>{srcRec.detailed.gender.trim() || "—"}</b></span>
          <span class="meta">{t.table.colAge} <b>{srcRec.detailed.age.trim() || "—"}</b></span>
          <span class="meta">{t.table.colHeight} <b>{srcRec.detailed.height.trim() || "—"}</b></span>
          <span class="meta">{t.table.colClub} <b>{srcRec.detailed.club.trim() || "—"}</b></span>
          <span class="meta">{appStore.basedLabel} <b>{basedValue(srcRec.detailed).trim() || "—"}</b></span>
          <span class="meta">{t.table.colCa} <b>{srcRec.detailed.ca.trim() || "—"}</b></span>
          <span class="meta">{t.table.colPa} <b>{srcRec.detailed.pa.trim() || "—"}</b></span>
        {/if}
      </div>
    {:else}
      <!-- 资料降级：uid 不在当前名单（可能来自上一次扫描）。队列占用时条目自带国籍/俱乐部
           （入队时记录，见 GenRequest.nationality/club），先用它顶着，别只剩一句「找不到」。 -->
      {#if queueTask && (queueTask.req.nationality || queueTask.req.club)}
        <div class="meta-row">
          {#if queueTask.req.nationality}
            <span class="meta">{t.table.colNat} <b>{queueTask.req.nationality}</b></span>
          {/if}
          {#if queueTask.req.club}
            <span class="meta">{t.table.colClub} <b>{queueTask.req.club}</b></span>
          {/if}
        </div>
      {/if}
      <p class="hint">{t.gen.source.noProfile}</p>
    {/if}
    <!-- 生成 = 单张接口：转发父级 onGenerate（未提交 Key 先串行提交、错误进右栏内联条）；
         disabled 读的 generating 与主 CTA 同源，两按钮互斥。队列占用期间不提供单张生成。 -->
    {#if !preview}
      <button class="btn primary src-generate" disabled={imageaiStore.generating} onclick={() => void ongenerate()}>
        {t.gen.generate}
      </button>
    {/if}
  {/if}
</section>

<style>
  /* 队列占用期间的只读标记：与队列坞状态胶囊同款配色（状态语义，不是错误） */
  .src-queue-pill {
    color: var(--warn);
    background: var(--warn-soft);
  }

  .src-head {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    margin-bottom: var(--sp-3);
  }
  .src-id {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .src-id .src-name {
    font-weight: var(--fw-semibold);
    font-size: var(--fs-lg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .src-id .src-uid {
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  /* 台内 UID 查询：输入框吃余宽，回车与按钮等价 */
  .src-query {
    display: flex;
    gap: var(--sp-3);
  }
  .src-query input {
    flex: 1;
    min-width: 0;
  }
  .src-card .meta-row {
    margin-bottom: var(--sp-4);
  }
  .src-generate {
    width: 100%;
  }
</style>
