<!-- PlayerViewer · 单人预览/替换：搜索一名球员看其头像与资料，或选一张新图单独替换
     常驻替换台左栏（批量面板下方），切去别的工作台即卸载，预览现场不留存
     与批量替换共用 appStore.saveBackup 开关
     搜索与「名单预览」同口径（UID/姓名/国籍/俱乐部，整份名单在本地过滤）：命中多个时默认停第 1 个，
     由头像右侧的「上一个/下一个」切换；名单里找不到则退回按 UID 直查（未扫描名单时仍能预览映射）
     信息区统一为胶囊：基础字段空值以 — 占位，拓展字段按名单格式追加且空值跳过
     （球员取「所在地」、职员取「偏好工作」）
     头像图的「读字节 → 建 blob → 回收」交给与名单/快照两视图共用的 createAvatarBlobs，
     换人自动换代、卸载自动回收，本组件不持有释放账 -->
<script lang="ts">
  import { open, message } from "@tauri-apps/plugin-dialog";
  import { api } from "../../services/api";
  import { createAvatarBlobs } from "../../avatarBlobs.svelte";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text"; // 全局文案常量
  import { appStore, basedValue, filterPlayers, tip } from "../../stores/app.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { imageaiStore, setSourcePlayer } from "../../stores/imageai.svelte"; // 生图数据源
  import Icon from "../../ui/Icon.svelte";
  import type { PlayerPreview, PlayerRecord } from "../../types";

  const FACE = "face"; // 单人只有一张图：加载器里的固定 key

  let query = $state(""); // 搜索词（UID / 姓名 / 国籍 / 俱乐部）
  let preview = $state<PlayerPreview | null>(null); // 预览数据（资料+映射+图片路径）
  let imgPathInput = $state(""); // 用户选的替换图片路径
  // 卡片当前显示的是哪一名球员（数据源里的原始 UID，已补全 r- 前缀）；'' = 还没预览过
  let previewedUid = $state("");
  const face = createAvatarBlobs(); // 本卡头像的 blob 账，回收由加载器代管

  // 命中名单与停靠位置：「上一个/下一个」在这份命中里前后移动
  let matches = $state<string[]>([]);
  let matchIdx = $state(0);
  // 整份名单（后端一次给全量，与名单预览同一份数据源）：搜索在本地做，不打后端
  let roster = $state<PlayerRecord[] | null>(null);

  const snap = $derived(appStore.snapshot);
  const busy = $derived(appStore.busy);

  // 名单全量：扫描RTF / 替换收尾（rosterVersion）后重取；取不到就留空，搜索退回按 UID 直查。
  // 取数走流式接口（与名单预览同一实现），在这里攒成整份——本卡搜索要一次性在本地过全量。
  // 换代令牌：切档/切组可能连续两次 bump rosterVersion，两条流并发时后完成者不得覆盖新结果；
  // 流是几十块的异步序列，没有令牌这个窗口会被拉长到整条流跑完。
  let rosterToken = 0;
  $effect(() => {
    void appStore.rosterVersion;
    const mine = ++rosterToken;
    const acc: PlayerRecord[] = [];
    api
      .queryPlayersStream(
        (batch) => {
          if (mine !== rosterToken) return; // 换代后旧流的批次与收尾都不得写回
          if (batch === null) acc.length = 0; // 名单重新取数：上一代的批次作废
          else for (const r of batch) acc.push(r);
        },
        () => mine !== rosterToken, // 换代后让数据源停止后续分块
      )
      .then(() => {
        if (mine === rosterToken) roster = acc;
      })
      .catch(() => {
        if (mine === rosterToken) roster = [];
      });
  });

  // 【请求令牌】preview_player 是同步命令，缓存未命中时要整份重解析档景（15 万行量级、秒级），
  // 响应先后完全不可控。若把 await 的结果直接写回界面：「搜 A → 立刻点下一个」时，
  // A 的后到响应会把卡片盖回 A，用户接着点替换就改了 A 那一行 —— 静默改错人，事后无从发现
  let previewToken = 0;

  /** 拉某名球员的预览，并把图片装载为 blob URL
   *  返回是否命中并刷新了卡片：true = 卡片已对齐本次请求；false = 空/被更新者作废/后端报错
   *  （报错时错误弹窗已在此处弹出）。调用方据此决定是否继续落笔，避免预览失败仍拿旧 UID 写档 */
  async function loadPreview(want: string): Promise<boolean> {
    const target = want.trim();
    if (!target) return false;
    const token = ++previewToken;
    try {
      const got = await api.previewPlayer(target);
      if (token !== previewToken) return false; // 已有更新的请求发出 → 这份结果整份作废
      preview = got;
      // 命中以数据源里的原始 UID 为准（名单资料优先于映射表）：输入不带 r- 前缀也能命中随机人；
      // 两者都空 = 没命中任何数据源，保持原样
      previewedUid = got?.record?.uid ?? got?.mapping?.uid ?? target;
      // 【为何用 blob】WebView 不认本地磁盘路径，read_image 返回的原始字节要包成 blob URL；
      // 换人时旧图由加载器自动回收
      await face.load(got?.imagePath ? { [FACE]: got.imagePath } : {});
      return true;
    } catch (e) {
      if (token !== previewToken) return false; // 迟到的失败同样不该覆盖新请求已经呈现的界面
      // 未命中（UID 无数据、名单里也没有）与后端报错同走统一错误弹窗，与批量替换失败一个反馈口径
      await showError(e);
      return false;
    }
  }

  /** 搜索：命中多个时默认停第 1 个；名单里找不到就退回按 UID 直查（未扫描名单时仍可预览映射） */
  async function runSearch() {
    const word = query.trim();
    if (!word) return;
    // 命中口径 = store 的 filterPlayers（与「名单预览」同一份实现）
    const hits = filterPlayers(roster ?? [], word);
    matches = hits.map((r) => r.uid);
    matchIdx = 0;
    await loadPreview(hits.length ? matches[0] : word);
  }

  /** 上一个/下一个：在命中名单里前后移动（到头即禁用，不循环） */
  async function stepMatch(delta: number) {
    const i = matchIdx + delta;
    if (i < 0 || i >= matches.length) return;
    matchIdx = i;
    await loadPreview(matches[i]);
  }

  /** 浏览选择一张图片（png/jpg/jpeg）填到替换路径输入框：默认落在当前头像包目录，
   *  从源头减少选到包外图片（后端 replace_single_face 拒包外路径） */
  async function browseImage() {
    const file = await open({
      filters: [{ name: t.viewer.imageFilterName, extensions: ["png", "jpg", "jpeg"] }],
      title: t.viewer.imageDialogTitle,
      defaultPath: snap?.currentGroup ?? undefined,
    });
    if (typeof file === "string") imgPathInput = file;
  }

  /** 执行单人替换：写入目标只取当前预览的那名球员（previewedUid），不读搜索框 */
  async function replaceIt() {
    if (!previewedUid || !imgPathInput.trim()) {
      await message(t.viewer.missingInput, { title: t.dialog.error });
      return;
    }
    try {
      // 备份开关读「替换执行」面板的同一共享值：关掉后单人替换也不备份
      const hit = await api.replaceSingleFace(previewedUid, imgPathInput, appStore.saveBackup);
      // hit=true 命中并替换；false 表示 config.xml 里本来就没这个 UID，未做修改
      await message(hit ? t.viewer.replacedOk(previewedUid) : t.viewer.mappingNotFound(previewedUid), {
        title: hit ? t.dialog.info : t.dialog.warning,
      });
      imgPathInput = "";
      await loadPreview(previewedUid); // 重拉一次，让卡片与刚写入的映射对齐
    } catch (e) {
      await showError(e);
    }
  }

  /** 重掷替换：按标准规则给当前预览的球员重新抽一张图（后端固定重抽语义，见 reset_single_face），
   *  用于撤销手动指定、回到规则抽图结果。与 imgPathInput 无关：不读也不清 */
  async function resetReplace() {
    if (!previewedUid) return;
    try {
      const hit = await api.resetSingleFace(previewedUid, appStore.saveBackup);
      await message(hit ? t.viewer.resetOk(previewedUid) : t.viewer.mappingNotFound(previewedUid), {
        title: hit ? t.dialog.info : t.dialog.warning,
      });
      await loadPreview(previewedUid);
    } catch (e) {
      await showError(e);
    }
  }

  // 球员记录简写（未预览时为 null，模板用 ?? '—' 兜底）
  const rec = $derived(preview?.record ?? null);

  // ---- 「送生图」：把预览球员设为生图数据源 -------------------------------------------------------------------------
  // 当前预览的球员是否已是生图数据源（按钮文字/描边随之变化作为反馈）
  const isSource = $derived(!!previewedUid && imageaiStore.sourcePlayer?.uid === previewedUid);
  /** 把当前预览的球员设为生图数据源，并切到「AI 生成头像」工作台
   *  与名单预览行尾「送生图」同一机制（见 PlayersTable.sendToGenerate）：
   *  只切工作台、不碰预览区 Tab（共享的 appStore.rtfPreview 保留）；表内页码/搜索随卸载回初始 */
  function useForGenerate() {
    if (!previewedUid) return;
    // 姓名取名单记录（仅命中映射表时没有姓名，退回 UID 展示）
    setSourcePlayer(previewedUid, rec?.name?.trim() || previewedUid);
    ui.workbench = "ai";
  }

  // 第 15 列口径单点见 appStore（isStaffRoster/basedLabel/basedValue）

  /** 一个信息胶囊：标签 + 值；accent=强调色（随机人），mono=等宽（映射路径） */
  type Capsule = {
    label: string;
    value: string;
    accent?: boolean;
    mono?: boolean;
  };

  // 五个基础字段标签：顺序与「名单预览」表头对齐（国籍、第二国籍、发长、发色、种族）
  const BASE_FIELDS = [
    { label: t.table.colNat, key: "primaryNat" },
    { label: t.table.col2ndNat, key: "secNat" },
    { label: t.table.colHair, key: "hairLength" },
    { label: t.table.colColor, key: "hairColor" },
    { label: t.table.colEth, key: "ethnicityCode" },
  ] as const;

  /** 基础五枚胶囊（Basic/Ext 名单都有）：值缺失时以 — 占位，保证基础名单也有胶囊可看 */
  const baseCapsules = $derived.by<Capsule[]>(() => {
    const r = preview?.record;
    if (!r) return [];
    return BASE_FIELDS.map((f) => ({
      label: f.label,
      value: r[f.key].trim() || "—",
    }));
  });

  /** 拓展字段，顺序对齐「名单预览」表头列序：肤色 / 随机人 / 性别 / 年龄 / 身高 / 俱乐部 /
   *  所在地(球员)或偏好工作(职员) / 能力 / 潜力 / R.C.A / 自定义列
   *  仅拓展名单有；空值字段跳过，随机人胶囊（是/否）保持青绿强调色 */
  const extCapsules = $derived.by<Capsule[]>(() => {
    const d = preview?.record?.detailed;
    if (!d) return [];
    const out: Capsule[] = [];
    const push = (label: string, value: string, opts: Partial<Capsule> = {}) => {
      const v = value.trim();
      if (v) out.push({ label, value: v, ...opts });
    };
    push(t.table.colSkin, d.skinCode);
    if (d.isNewgan.trim()) push(t.table.colNewgan, d.isNewgan, { accent: true });
    push(t.table.colGender, d.gender);
    push(t.table.colAge, d.age);
    push(t.table.colHeight, d.height);
    push(t.table.colClub, d.club);
    push(appStore.basedLabel, basedValue(d));
    push(t.table.colCa, d.ca);
    push(t.table.colPa, d.pa);
    push(t.table.colRca, d.rca);
    // 第 19 列起的用户自定义列：后端无表头名，按「自定义N」命名，空列跳过
    d.extraFields.forEach((v, i) => push(t.viewer.extraField(i + 1), v));
    return out;
  });

  // 当前映射（等宽胶囊；无映射时不渲染）
  const mappingCapsule = $derived.by<Capsule | null>(() => {
    const m = preview?.mapping;
    if (!m) return null;
    return {
      label: t.viewer.colMapping,
      value: `${m.ethnicity}\\${m.image}`,
      mono: true,
    };
  });

  /** 全部胶囊并成一个池子：分组边界在 320 窄栏下会留出成片空白，折行交给模板的 .meta-row */
  const capsules = $derived.by<Capsule[]>(() => {
    const out = [...baseCapsules, ...extCapsules];
    if (mappingCapsule) out.push(mappingCapsule);
    return out;
  });
</script>

<!-- 卡片外壳由本组件自带（与 ReplacePanel/SetupCard 同一模式）；.viewer-card 同时是窄栏
     容器查询的锚点（app.css 的 .viewer-card + @container 块），动模板别丢类名 -->
<div class="card viewer-card">
  <div class="card-title">{t.viewer.section}</div>
  <div class="player-card">
    <!-- 左列：圆角头像 + 右侧搜索区（搜索框 / 搜索 / 上一个·下一个） -->
    <div class="face-col">
      {#if face.urls[FACE]}
        <img src={face.urls[FACE]} alt={t.viewer.title} class="face" />
      {:else}
        <div class="face-empty">{t.viewer.noImage}</div>
      {/if}
      <div class="face-side">
        <!-- 搜索框：外观由 app.css 的 .search 单点定义（与名单预览工具条同款）；
             这里回车/点搜索才提交，命中口径共用 store 里的 filterPlayers -->
        <div class="search">
          <Icon name="search" />
          <input
            type="search"
            bind:value={query}
            placeholder={t.table.searchPlaceholder}
            aria-label={t.table.searchPlaceholder}
            onkeydown={(e) => e.key === "Enter" && runSearch()}
          />
        </div>
        <button type="button" class="btn sm" onclick={runSearch} disabled={!query.trim()} use:tip={t.viewer.hint}>
          <Icon name="search" size={14} />
          {t.viewer.searchBtn}
        </button>
        <!-- 命中多个时在命中之间切换（到头即禁用，不循环）；位置计数只在多命中时出现 -->
        <div class="face-nav">
          <button type="button" class="btn sm" onclick={() => stepMatch(-1)} disabled={matchIdx <= 0}
            >{t.viewer.prevMatch}</button
          >
          {#if matches.length > 1}<span class="muted small">{matchIdx + 1}/{matches.length}</span>{/if}
          <button type="button" class="btn sm" onclick={() => stepMatch(1)} disabled={matchIdx >= matches.length - 1}
            >{t.viewer.nextMatch}</button
          >
        </div>
      </div>
    </div>

    <!-- 右列：姓名、信息胶囊单池、替换行 -->
    <div class="player-info">
      <div class="player-head">
        <!-- 姓名为头部视觉主体；极端窄宽下由省略号收口（悬停提示看全名） -->
        <h2 use:tip={rec?.name ?? undefined}>{rec?.name || "—"}</h2>
      </div>

      <!-- 信息胶囊单池：.meta-row 的 flex-wrap 按容器宽度自动折行（320 窄栏与堆叠全宽同一套
           流动逻辑）；空值显示 — 不坍缩，长路径靠胶囊内 break-all 自折 -->
      {#if capsules.length}
        <div class="meta-row">
          {#each capsules as c (c.label + c.value)}
            <span class="meta" class:accent={c.accent} class:mono={c.mono}>
              {c.label} <b>{c.value}</b>
            </span>
          {/each}
        </div>
      {/if}

      <!-- 替换区沉底，两行：上行 = 图片路径输入框 + 浏览；下行 = 三个动作按钮等分占满一行 -->
      <div class="replace-row">
        <div class="input-wrap">
          <Icon name="folder" />
          <input
            type="text"
            placeholder={t.viewer.imgPlaceholder}
            bind:value={imgPathInput}
            aria-label={t.viewer.imgPathAria}
          />
        </div>
        <!-- 批量替换进行中禁用这些按钮：本卡与批量替换同栏常驻，替换跑着时用户可就地操作；
             后端有同样的拒绝（replace_single_face/reset_single_face 的 running 守卫），这里先挡一层免得撞错误框 -->
        <button class="btn" onclick={browseImage} disabled={!snap?.currentGroup || busy}>{t.viewer.browseBtn}</button>
      </div>
      <div class="replace-actions">
        <!-- 替换目标 = 当前预览的球员（previewedUid），未选中球员时禁用 -->
        <button class="btn primary" onclick={replaceIt} disabled={!previewedUid || !imgPathInput.trim() || busy}
          >{t.viewer.replaceBtn}</button
        >
        <button class="btn" onclick={resetReplace} disabled={!previewedUid || busy}>{t.viewer.resetBtn}</button>
        <!-- 送生图：把当前预览的球员设为生图数据源并切到 AI 生图工作台（与名单行尾入口同名同语义）；
             已是该数据源时按钮文字/描边变化作为反馈；悬停提示用全称说明（按钮名受窄栏宽度限制取短名） -->
        <button
          class="btn gen-src-btn"
          class:on={isSource}
          onclick={useForGenerate}
          disabled={!previewedUid || busy}
          use:tip={isSource ? t.gen.source.selected : t.gen.source.sendTitle(rec?.name?.trim() || previewedUid)}
          >{isSource ? t.gen.source.selected : t.gen.source.useForGen}</button
        >
      </div>
    </div>
  </div>
</div>
