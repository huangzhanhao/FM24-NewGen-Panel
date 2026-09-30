<!-- SourcePlayerCard · 数据源单人档案卡（左栏）：当前数据源球员的头像/资料胶囊 + 台内 UID 查询 + 「生成」
     三个来源都写 imageaiStore.sourcePlayer：名单行「送生图」、单人替换「用此球员生成」、本卡 UID 查询
     「生成」经 ongenerate 转发父级主 CTA 链路（单张接口）；批量入队只走名单工具行与常驻队列卡
     头像装载、资料 effect 与 UID 查询态都住在本地 -->
<script lang="ts">
  import { api } from "../../services/api";
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 缩略图 + 放大气泡，与名单预览共用
  import { createAvatarSource } from "../../avatarBlobs.svelte"; // 头像路径查询 + 缩略图装载
  import { appStore, basedValue } from "../../stores/app.svelte"; // rosterReady + 第 15 列口径 + 气泡失效键
  import { ui } from "../../stores/ui.svelte"; // 就绪卡「去存档资料台」切工作台
  import {
    clearSourcePlayer,
    imageaiStore,
    setSourcePlayer,
  } from "../../stores/imageai.svelte";
  import { t } from "../../text";
  import type { PlayerPreview } from "../../types";

  let { ongenerate }: { ongenerate: () => void | Promise<void> } = $props();

  // ---- 头像缩略图：走共用的 createAvatarSource；取不到/取失败静默降级为只显示姓名+UID，不弹窗 ----
  const srcAvatars = createAvatarSource();
  $effect(() => {
    const sp = imageaiStore.sourcePlayer;
    srcAvatars.clear(); // 换数据源立即清场：旧球员的头像不该闪现在新卡上
    if (sp) void srcAvatars.load([sp.uid]);
  });

  // ---- 球员源资料：preview_player 缓存未命中时要重解析整份档案（秒级），按 uid 记账防重拉 ----
  // 台内 UID 查询命中时已带回首份资料，effect 靠 srcProfileUid 跳过同 uid 的第二次查询
  let srcProfile = $state<PlayerPreview | null>(null);
  let srcProfileUid = $state(""); // 资料已为哪个 uid 拉取过（失败也记账，避免 effect 反复重试）
  let srcProfileToken = 0;
  $effect(() => {
    const sp = imageaiStore.sourcePlayer;
    if (!sp) {
      srcProfile = null;
      srcProfileUid = "";
      return;
    }
    if (srcProfileUid === sp.uid) return;
    const token = ++srcProfileToken;
    api
      .previewPlayer(sp.uid)
      .then((got) => {
        if (token !== srcProfileToken) return;
        srcProfile = got;
        srcProfileUid = sp.uid;
      })
      .catch(() => {
        // uid 不在当前名单（数据源可能来自上一次扫描）：降级为只显示姓名/UID，不弹窗刷屏
        if (token === srcProfileToken) {
          srcProfile = null;
          srcProfileUid = sp.uid;
        }
      });
  });
  const srcRec = $derived(
    imageaiStore.sourcePlayer ? (srcProfile?.record ?? null) : null,
  );

  // ---- 台内 UID 查询：命中即 setSourcePlayer 设为数据源 ----
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
    {#if imageaiStore.sourcePlayer}
      <button
        class="btn sm"
        title={t.gen.source.clearTitle}
        onclick={clearSourcePlayer}>{t.gen.source.clear}</button
      >
    {/if}
  </div>
  {#if !imageaiStore.sourcePlayer}
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
        <button
          class="btn"
          disabled={!srcUidInput.trim()}
          onclick={querySourceUid}>{t.gen.source.queryBtn}</button
        >
      </div>
      {#if srcQueryError}
        <p class="field-error">{srcQueryError}</p>
      {/if}
    {/if}
  {:else}
    {@const sp = imageaiStore.sourcePlayer}
    <div class="src-head">
      <AvatarThumb
        itemKey={sp.uid}
        url={srcAvatars.urls[sp.uid]}
        path={srcAvatars.paths[sp.uid] ?? undefined}
        groupKey={appStore.snapshot?.currentGroup}
      />
      <div class="src-id">
        <span class="src-name" title={sp.name}>{sp.name}</span>
        <span class="src-uid mono">{sp.uid}</span>
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
      <!-- 资料降级：uid 不在当前名单（可能来自上一次扫描），仅姓名/UID 可信 -->
      <p class="hint">{t.gen.source.noProfile}</p>
    {/if}
    <!-- 生成 = 单张接口：转发父级 onGenerate（未提交 Key 先串行提交、错误进右栏内联条）；
         disabled 读的 generating 与主 CTA 同源，两按钮互斥 -->
    <button
      class="btn primary src-generate"
      disabled={imageaiStore.generating}
      onclick={() => void ongenerate()}
    >
      {t.gen.generate}
    </button>
  {/if}
</section>

<style>
  .src-head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }
  .src-id {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .src-id .src-name {
    font-weight: 600;
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .src-id .src-uid {
    color: var(--text-2);
    font-size: 12px;
  }
  /* 台内 UID 查询：输入框吃余宽，回车与按钮等价 */
  .src-query {
    display: flex;
    gap: 8px;
  }
  .src-query input {
    flex: 1;
    min-width: 0;
  }
  .src-card .meta-row {
    margin-bottom: 9px;
  }
  .src-generate {
    width: 100%;
  }
</style>
