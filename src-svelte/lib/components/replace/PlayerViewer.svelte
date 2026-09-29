<!-- 单人预览/替换：输入 UID 查看其头像与资料，或选一张新图单独替换。
     常驻于替换台左栏（读写分栏的写侧下半；切去别的工作台即卸载，预览现场不留存），
     与上方批量替换共用 saveBackup 开关。
     信息区统一为胶囊，按名单格式动态组合：Basic 只有基础 7 字段；ExtPlayer/ExtStaff
     追加详细字段（球员取「所在地」，职员取「偏好工作」），空值字段不渲染。
     头像图的「读字节 → 建 blob → 回收」走与名单/快照两视图共用的 createAvatarBlobs
     （换人自动换代、卸载自动回收）。 -->
<script lang="ts">
  import { open, message } from "@tauri-apps/plugin-dialog";
  import { api } from "../../services/api";
  import { createAvatarBlobs } from "../../avatarBlobs.svelte"; // 头像 blob 加载器（名单/快照/本卡共用）
  import { showError } from "../../ui/dialog";
  import { t } from "../../text"; // 全局文案常量
  import { appStore, basedValue } from "../../stores/app.svelte";
  import { ui } from "../../stores/ui.svelte"; // 「用此球员生成」要切到 AI 生成头像工作台
  import { imageaiStore, setSourcePlayer } from "../../stores/imageai.svelte"; // P3 数据源入口
  import Icon from "../../ui/Icon.svelte";
  import type { PlayerPreview } from "../../types";

  const FACE = "face"; // 单人只有一张图：加载器里的固定 key

  let uid = $state(""); // 输入的 UID
  let preview = $state<PlayerPreview | null>(null); // 预览数据（资料+映射+图片路径）
  let imgPathInput = $state(""); // 用户选的替换图片路径
  // 卡片当前显示的是哪一名球员（数据源里的原始 UID，已补全 r- 前缀）；'' = 还没预览过。
  let previewedUid = $state("");
  const face = createAvatarBlobs(); // 头像图的 blob 账（复用名单预览/快照预览的同一实现）

  const snap = $derived(appStore.snapshot);
  const busy = $derived(appStore.busy);

  // 【请求令牌】preview_player 是同步命令，缓存未命中时要重解析整份档景（15 万行量级、秒级），
  // 响应先后完全不可控。旧写法把 await 的结果直接写回用户正在输入的 uid：
  // 「输 A 回车 → 立刻改成 B 回车」时，A 的后到响应会把输入框改回 A，用户接着点替换
  // 就改了 A 那一行，而弹窗回显的也是 A —— 静默改错人，事后无从发现。
  let previewToken = 0;

  /** 根据 UID 拉取预览，并把图片加载为 blob URL。
   *  返回是否成功命中并刷新了卡片：true=卡片已对齐到本次输入；false=输入为空/请求被更新者作废/后端报错
   *  （报错时错误弹窗已在此处弹出）。调用方据此决定是否继续落笔，避免预览失败仍拿旧 UID 写档。 */
  async function showPreview(): Promise<boolean> {
    const want = uid.trim();
    if (!want) return false;
    const token = ++previewToken;
    try {
      const got = await api.previewPlayer(want);
      if (token !== previewToken) return false; // 已有更新的请求发出 → 这份结果整份作废
      preview = got;
      // 回显命中的完整 UID：输入带不带 r- 前缀都能命中，展示以数据源里的原始形态为准
      // （输入 2000472008 命中随机人 r-2000472008 时，输入框补全前缀）。名单资料优先于映射表；
      // 两者都空 = 没命中任何数据源，保持用户输入不动。
      const matched = got?.record?.uid ?? got?.mapping?.uid ?? want;
      previewedUid = matched;
      if (uid === want) uid = matched; // 用户没再改过输入框，才安全地补全前缀
      // 加载图片为可显示的 blob URL（read_image 返回原始字节；换人时旧图由加载器自动回收）。
      // 【为何用 blob】WebView 不能直接用本地磁盘路径作为 <img src>，需把字节包成浏览器可识别的 blob URL。
      await face.load(got?.imagePath ? { [FACE]: got.imagePath } : {});
      return true;
    } catch (e) {
      if (token !== previewToken) return false; // 迟到的失败同样不该覆盖新请求已经呈现的界面
      // 搜索失败（UID 无数据/后端报错）走统一错误弹窗，与批量替换失败同一反馈口径。
      await showError(e);
      return false;
    }
  }

  /** 浏览选择一张图片（png/jpg/jpeg），填到替换路径输入框。默认落在当前头像包目录，
   *  从源头减少选到包外图片（后端会拒包外路径）。 */
  async function browseImage() {
    const file = await open({
      filters: [
        { name: t.viewer.imageFilterName, extensions: ["png", "jpg", "jpeg"] },
      ],
      title: t.viewer.imageDialogTitle,
      defaultPath: snap?.currentGroup ?? undefined,
    });
    if (typeof file === "string") imgPathInput = file;
  }

  /** 执行单人替换：校验输入 → 调后端 → 弹窗告知结果 → 刷新预览。 */
  async function replaceIt() {
    const target = uid.trim();
    if (!target || !imgPathInput.trim()) {
      await message(t.viewer.missingInput, { title: t.dialog.error });
      return;
    }
    // 【写前对齐】用户可能改了输入框却没回车，此刻卡片显示的还是别人。先把卡片刷成 target，
    // 让「屏幕上正在看的球员」与「config.xml 里要被改掉的那一行」确定是同一名，再落笔。
    // 刷新失败（后端报错/请求被作废）时立即中止：否则 previewedUid 仍是上一次成功预览的旧球员，
    // 会静默改错人（错误弹窗已由 showPreview 弹出，这里不再重复提示）。
    if (previewedUid !== target && !(await showPreview())) return;
    // 写入目标取自预览结果（数据源里的原始 UID），不再读那个随时可能被用户改动的输入框。
    const writeUid = previewedUid;
    try {
      // 备份开关读「替换执行」面板的同一共享值：关掉后单人替换也不备份。
      const hit = await api.replaceSingleFace(
        writeUid,
        imgPathInput,
        appStore.saveBackup,
      );
      // hit=true 表示命中并替换；false 表示 config.xml 里本来就没这个 UID，未做修改。
      await message(
        hit
          ? t.viewer.replacedOk(writeUid)
          : t.viewer.mappingNotFound(writeUid),
        { title: hit ? t.dialog.info : t.dialog.warning },
      );
      await showPreview(); // 刷新预览
    } catch (e) {
      await showError(e);
    }
  }

  // 球员记录简写（未预览时为 null，模板用 ?? '—' 兜底）。
  const rec = $derived(preview?.record ?? null);

  // ---------------- 「用此球员生成」（P3 数据源入口） ----------------
  // 当前预览的球员是否已是生图数据源（按钮文字/描边随之变化作为反馈）。
  const isSource = $derived(
    !!previewedUid && imageaiStore.sourcePlayer?.uid === previewedUid,
  );
  /** 把当前预览的球员设为生图数据源，并切到「AI 生成头像」工作台。
   *  与名单预览行尾「送生图」同一机制（详见 PlayersTable.sendToGenerate 的注释）：
   *  写 resultsView='generate' 会把本组件的预览结果换掉，改切工作台则不动这里的上下文。 */
  function useForGenerate() {
    if (!previewedUid) return;
    // 姓名取名单记录（仅命中映射表时没有姓名，退回 UID 展示）。
    setSourcePlayer(previewedUid, rec?.name?.trim() || previewedUid);
    ui.workbench = "ai";
  }

  // 第 15 列口径单点见 appStore（isStaffRoster/basedLabel/basedValue）。

  /** 一个信息胶囊：标签 + 值；accent=强调色（随机人），mono=等宽（映射路径）。 */
  type Capsule = {
    label: string;
    value: string;
    accent?: boolean;
    mono?: boolean;
  };

  // 固定的五个基础字段标签（顺序对齐「名单预览」表头：国籍、第二国籍、发长、发色、种族；
  // 值缺失时用 — 占位，保证基础名单也有胶囊可看）。
  const BASE_FIELDS = [
    { label: t.table.colNat, key: "primaryNat" },
    { label: t.table.col2ndNat, key: "secNat" },
    { label: t.table.colHair, key: "hairLength" },
    { label: t.table.colColor, key: "hairColor" },
    { label: t.table.colEth, key: "ethnicityCode" },
  ] as const;

  /** 基础五枚：国籍、第二国籍、发长、发色、种族（Basic/Ext 名单都有）；空值显示 — 不坍缩。 */
  const baseCapsules = $derived.by<Capsule[]>(() => {
    const r = preview?.record;
    if (!r) return [];
    return BASE_FIELDS.map((f) => ({
      label: f.label,
      value: r[f.key].trim() || "—",
    }));
  });

  /**
   * 拓展字段，顺序对齐「名单预览」表头列序——
   * 肤色 / 随机人 / 性别 / 年龄 / 身高 / 俱乐部 / 所在地(球员)或偏好工作(职员) / 能力 / 潜力 / R.C.A / 自定义列。
   * 仅拓展名单有；空值字段跳过。随机人胶囊（是/否）保持青绿强调色。
   */
  const extCapsules = $derived.by<Capsule[]>(() => {
    const d = preview?.record?.detailed;
    if (!d) return [];
    const out: Capsule[] = [];
    const push = (
      label: string,
      value: string,
      opts: Partial<Capsule> = {},
    ) => {
      const v = value.trim();
      if (v) out.push({ label, value: v, ...opts });
    };
    push(t.table.colSkin, d.skinCode);
    if (d.isNewgan.trim())
      push(t.table.colNewgan, d.isNewgan, { accent: true });
    push(t.table.colGender, d.gender);
    push(t.table.colAge, d.age);
    push(t.table.colHeight, d.height);
    push(t.table.colClub, d.club);
    push(appStore.basedLabel, basedValue(d));
    push(t.table.colCa, d.ca);
    push(t.table.colPa, d.pa);
    push(t.table.colRca, d.rca);
    // 第 19 列起的用户自定义列：后端无表头名，按「自定义N」命名，空列跳过。
    d.extraFields.forEach((v, i) => push(t.viewer.extraField(i + 1), v));
    return out;
  });

  // 当前映射（等宽胶囊；无映射时不渲染，头像下方另有占位简示）。
  const mappingCapsule = $derived.by<Capsule | null>(() => {
    const m = preview?.mapping;
    if (!m) return null;
    return {
      label: t.viewer.colMapping,
      value: `${m.ethnicity}\\${m.image}`,
      mono: true,
    };
  });

  /** 全部胶囊并成一个池子（用户定稿）：不再按「基础/拓展/映射」手动分行——分组边界在
   *  320 窄栏下会留出成片空白；单池交给 .meta-row 的 flex-wrap 按容器宽度自然折行，
   *  映射胶囊的「独占一行」语义一并取消（长路径靠 break-all 在胶囊内自折）。 */
  const capsules = $derived.by<Capsule[]>(() => {
    const out = [...baseCapsules, ...extCapsules];
    if (mappingCapsule) out.push(mappingCapsule);
    return out;
  });

  // 当前映射的「种族\图片名」展示串（头像下方简示用；无映射时为空串）。
  const mappingText = $derived(
    preview?.mapping
      ? `${preview.mapping.ethnicity}\\${preview.mapping.image}`
      : "",
  );
</script>

<!-- 卡片外壳收进本组件（与 ReplacePanel/SetupCard 同一模式），App 不再包壳；
     .viewer-card 同时是窄栏容器查询的锚点（app.css 的 .viewer-card + @container 块），动模板别丢类名。 -->
<div class="card viewer-card">
  <div class="card-title">{t.viewer.section}</div>
  <div class="player-card">
    <!-- 左列：圆角头像 + 映射路径（有映射时小圆点着青绿）。 -->
    <div class="face-col">
      <!-- 有 blob URL 就显示图片，否则显示虚线占位。 -->
      {#if face.urls[FACE]}
        <img src={face.urls[FACE]} alt={t.viewer.title} class="face" />
      {:else}
        <div class="face-empty">{t.viewer.noImage}</div>
      {/if}
      <span class="mapping" class:set={!!preview?.mapping} title={mappingText}>
        {mappingText || t.viewer.noMappingPlaceholder}
      </span>
    </div>

    <!-- 右列：头部（UID 输入框在最左 + 姓名）、三行信息胶囊、替换行。 -->
    <div class="player-info">
      <div class="player-head">
        <!-- UID 搜索组：标准尺寸输入框 + 搜索按钮成组等高；回车与点按钮等价。 -->
        <div class="uid-search">
          <!-- UID 必须保持为可输入框：输入后回车即预览（不可改成只读标签）；标准输入框尺寸、等宽 semibold，置于最左。 -->
          <input
            id="pv-uid"
            class="uid-chip"
            type="text"
            bind:value={uid}
            placeholder={t.viewer.uidPlaceholder}
            aria-label={t.viewer.hint}
            onkeydown={(e) => e.key === "Enter" && showPreview()}
          />
          <button
            type="button"
            class="btn uid-search-btn"
            onclick={showPreview}
            disabled={!uid.trim()}
            title={t.viewer.hint}
          >
            <Icon name="search" size={14} />
            {t.viewer.searchBtn}
          </button>
        </div>
        <!-- 姓名为头部视觉主体；极端窄宽下由省略号收口（title 悬停看全名），不再把右侧提示挤换行。 -->
        <h2 title={rec?.name ?? undefined}>{rec?.name || "—"}</h2>
        <!-- 文本提示安静靠右：说明回车快捷键与 r- 前缀可省略；默认单行，仅 ≤900px 窄窗折到下一行（见 app.css）。 -->
        <span class="uid-tip">{t.viewer.uidTip}</span>
      </div>

      <!-- 信息胶囊单池：不再按「基础/拓展/映射」分三个 div，交给 .meta-row 的 flex-wrap
           按容器宽度自动折行（320 窄栏与堆叠全宽同一套流动逻辑）；空值显示 — 不坍缩。 -->
      {#if capsules.length}
        <div class="meta-row">
          {#each capsules as c (c.label + c.value)}
            <span class="meta" class:accent={c.accent} class:mono={c.mono}>
              {c.label} <b>{c.value}</b>
            </span>
          {/each}
        </div>
      {/if}

      <!-- 替换行沉底：图片路径输入框（文件夹图标）+ 浏览 + 替换。无组禁用浏览；输入不全禁用替换。 -->
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
        <!-- 批量替换进行中禁用这两个按钮：本卡与批量替换同栏常驻，替换跑着时用户可就地操作；
             后端也已加同样的拒绝（见 replace_single_face），这里先挡一层免得用户撞错误框。 -->
        <button
          class="btn"
          onclick={browseImage}
          disabled={!snap?.currentGroup || busy}>{t.viewer.browseBtn}</button
        >
        <button
          class="btn primary"
          onclick={replaceIt}
          disabled={!uid.trim() || !imgPathInput.trim() || busy}
          >{t.viewer.replaceBtn}</button
        >
        <!-- 用此球员生成（P3）：把当前预览的球员设为生图数据源并切到 AI 生图；
             已是该数据源时按钮文字/描边变化作为反馈。 -->
        <button
          class="btn gen-src-btn"
          class:on={isSource}
          onclick={useForGenerate}
          disabled={!previewedUid || busy}
          title={isSource ? t.gen.source.selected : t.gen.source.useForGen}
          >{isSource ? t.gen.source.selected : t.gen.source.useForGen}</button
        >
      </div>
    </div>
  </div>
</div>
