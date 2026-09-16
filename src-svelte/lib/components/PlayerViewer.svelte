<!-- 单人预览/替换：输入 UID 查看其头像与资料，或选一张新图单独替换。
     外观为「球员卡」：左列头像 + 映射路径，右列 UID 输入框 + 姓名 + 信息胶囊 + 替换行。
     信息区统一为胶囊，按名单格式动态组合：Basic 只有基础 7 字段；ExtPlayer/ExtStaff
     追加详细字段（球员取「所在地」，职员取「偏好工作」），空值字段不渲染。 -->
<script lang="ts">
  import { open, message } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import { loadImageBlobUrl } from "../images";
  import { t } from "../text"; // 全局文案常量
  import { appStore } from "../stores/app.svelte";
  import { icons } from "../icons";
  import type { PlayerPreview } from "../types";

  let uid = $state(""); // 输入的 UID
  let preview = $state<PlayerPreview | null>(null); // 预览数据（资料+映射+图片路径）
  let imgPathInput = $state(""); // 用户选的替换图片路径
  let blobUrl = $state(""); // 图片的 blob URL（供 <img src> 使用）

  const snap = $derived(appStore.snapshot);
  const saveBackup = true; // 与主界面备份开关联动可后续扩展，先保持 v1 Viewer 行为

  /** 根据 UID 拉取预览，并把图片加载为 blob URL。 */
  async function showPreview() {
    if (!uid.trim()) return;
    try {
      preview = await api.previewPlayer(uid);
      // 回显命中的完整 UID：输入带不带 r- 前缀都能命中，展示以数据源里的原始形态为准
      // （输入 2000472008 命中随机人 r-2000472008 时，输入框补全前缀）。名单资料优先于映射表；
      // 两者都空 = 没命中任何数据源，保持用户输入不动。
      const matched = preview?.record?.uid ?? preview?.mapping?.uid;
      if (matched) uid = matched;
      // 加载图片为 blob URL（read_image 返回原始字节）。
      // 【为何用 blob】WebView 不能直接用本地磁盘路径作为 <img src>，需把字节包成浏览器可识别的 blob URL。
      if (blobUrl) {
        URL.revokeObjectURL(blobUrl);
        blobUrl = "";
      } // 先释放上一次的 blob，避免内存泄漏
      if (preview?.imagePath) {
        blobUrl = await loadImageBlobUrl(preview.imagePath);
      }
    } catch (e) {
      // 搜索失败（UID 无数据/后端报错）走原生弹窗，与「替换」失败同一反馈口径。
      await message(String(e), { title: t.dialog.error });
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
    if (!uid.trim() || !imgPathInput.trim()) {
      await message(t.viewer.missingInput, { title: t.dialog.error });
      return;
    }
    try {
      const hit = await api.replaceSingleFace(uid, imgPathInput, saveBackup);
      // hit=true 表示命中并替换；false 表示 config.xml 里本来就没这个 UID，未做修改。
      await message(
        hit ? t.viewer.replacedOk(uid) : t.viewer.mappingNotFound(uid),
        { title: hit ? t.dialog.info : t.dialog.warning },
      );
      await showPreview(); // 刷新预览
    } catch (e) {
      await message(String(e), { title: t.dialog.error });
    }
  }

  // 球员记录简写（未预览时为 null，模板用 ?? '—' 兜底）。
  const rec = $derived(preview?.record ?? null);

  // 第 15 列语义：按扫描摘要的格式名区分球员（所在地）/职员（偏好工作），与名单预览表同一口径。
  const isStaffRoster = $derived(
    appStore.lastRtfScan?.format.includes("职员") ?? false,
  );

  /** 一个信息胶囊：标签 + 值；accent=强调色（随机人），mono=等宽（映射路径）。 */
  type Capsule = {
    label: string;
    value: string;
    accent?: boolean;
    mono?: boolean;
  };

  // 第一行固定的五个基础字段标签（顺序对齐「名单预览」表头：国籍、第二国籍、发长、发色、种族；
  // 值缺失时用 — 占位，保证该行结构恒定）。
  const BASE_FIELDS = [
    { label: t.table.colNat, key: "primaryNat" },
    { label: t.table.col2ndNat, key: "secNat" },
    { label: t.table.colHair, key: "hairLength" },
    { label: t.table.colColor, key: "hairColor" },
    { label: t.table.colEth, key: "ethnicityCode" },
  ] as const;

  /**
   * 第一行：国籍、第二国籍、发长、发色、种族（固定五枚，Basic/Ext 都有）。
   * 左边界对齐 UID 输入框；空值显示 — 不坍缩。
   */
  const baseCapsules = $derived.by<Capsule[]>(() => {
    const r = preview?.record;
    if (!r) return [];
    return BASE_FIELDS.map((f) => ({
      label: f.label,
      value: r[f.key].trim() || "—",
    }));
  });

  /**
   * 第二行：拓展字段，顺序对齐「名单预览」表头列序——
   * 肤色 / 随机人 / 性别 / 年龄 / 身高 / 俱乐部 / 所在地(球员)或偏好工作(职员) / 能力 / 潜力 / R.C.A / 自定义列。
   * 仅拓展名单有此行；空值字段跳过。随机人胶囊（是/否）保持青绿强调色。
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
    push(
      isStaffRoster ? t.table.colJob : t.table.colBased,
      isStaffRoster ? d.preferredJob : d.based,
    );
    push(t.table.colCa, d.ca);
    push(t.table.colPa, d.pa);
    push(t.table.colRca, d.rca);
    // 第 19 列起的用户自定义列：后端无表头名，按「自定义N」命名，空列跳过。
    d.extraFields.forEach((v, i) => push(t.viewer.extraField(i + 1), v));
    return out;
  });

  // 第三行：当前映射（等宽胶囊，独占一行；无映射时该行不渲染，头像下方另有占位简示）。
  const mappingCapsule = $derived.by<Capsule | null>(() => {
    const m = preview?.mapping;
    if (!m) return null;
    return {
      label: t.viewer.colMapping,
      value: `${m.ethnicity}\\${m.image}`,
      mono: true,
    };
  });

  // 当前映射的「种族\图片名」展示串（头像下方简示用；无映射时为空串）。
  const mappingText = $derived(
    preview?.mapping
      ? `${preview.mapping.ethnicity}\\${preview.mapping.image}`
      : "",
  );
</script>

<div class="player-card">
  <!-- 左列：圆角头像 + 映射路径（有映射时小圆点着青绿）。 -->
  <div class="face-col">
    <!-- 有 blob URL 就显示图片，否则显示虚线占位。 -->
    {#if blobUrl}
      <img src={blobUrl} alt={t.viewer.title} class="face" />
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
      <!-- UID 搜索组：输入框 + 搜索按钮成组等高；回车与点按钮等价。输入框保持大号 UID 视觉签名。 -->
      <div class="uid-search">
        <!-- UID 必须保持为可输入框：输入后回车即预览（不可改成只读标签）；字号与姓名同级、置于最左。 -->
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
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            aria-hidden="true">{@html icons.search.body}</svg
          >
          {t.viewer.searchBtn}
        </button>
      </div>
      <h2>{rec?.name || "—"}</h2>
      <!-- 文本提示安静靠右：说明回车快捷键与 r- 前缀可省略；窄宽折到下一行（见 app.css 媒体查询）。 -->
      <span class="uid-tip">{t.viewer.uidTip}</span>
    </div>

    <!-- 三行胶囊：整体作为 player-info 直接子级，左缘与 UID 输入框对齐（不跟随姓名缩进）。
         第一行固定五枚基础字段（国籍、第二国籍、发长、发色、种族）；第二行拓展字段（顺序对齐名单预览列序）；第三行当前映射。 -->
    {#if baseCapsules.length}
      <div class="capsule-rows">
        <div class="meta-row">
          {#each baseCapsules as c (c.label)}
            <span class="meta">{c.label} <b>{c.value}</b></span>
          {/each}
        </div>
        {#if extCapsules.length}
          <div class="meta-row">
            {#each extCapsules as c (c.label + c.value)}
              <span class="meta" class:accent={c.accent}>
                {c.label} <b>{c.value}</b>
              </span>
            {/each}
          </div>
        {/if}
        {#if mappingCapsule}
          <div class="meta-row">
            <span class="meta mono">
              {mappingCapsule.label} <b>{mappingCapsule.value}</b>
            </span>
          </div>
        {/if}
      </div>
    {/if}

    <!-- 替换行沉底：图片路径输入框（文件夹图标）+ 浏览 + 替换。无组禁用浏览；输入不全禁用替换。 -->
    <div class="replace-row">
      <div class="input-wrap">
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true">{@html icons.folder.body}</svg
        >
        <input
          type="text"
          placeholder={t.viewer.imgPlaceholder}
          bind:value={imgPathInput}
          aria-label={t.viewer.imgPathAria}
        />
      </div>
      <button class="btn" onclick={browseImage} disabled={!snap?.currentGroup}
        >{t.viewer.browseBtn}</button
      >
      <button
        class="btn primary"
        onclick={replaceIt}
        disabled={!uid.trim() || !imgPathInput.trim()}
        >{t.viewer.replaceBtn}</button
      >
    </div>
  </div>
</div>
