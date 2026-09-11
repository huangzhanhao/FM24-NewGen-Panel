<!-- 【新手导读】单人预览/微调：输入 UID 查看其头像与资料，或选一张新图单独替换。
     重点演示：把后端返回的图片字节（ArrayBuffer）转成 blob URL 供 <img> 显示。
     外观为「球员卡」：左列 168 圆角头像 + 映射路径，右列姓名/UID/元信息胶囊/信息格/替换行。 -->
<script lang="ts">
  import { open, message } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { t } from '../text'; // 全局文案常量
  import { appStore } from '../stores/app.svelte';
  import { icons } from '../icons';
  import type { PlayerPreview } from '../types';

  let uid = $state('');                 // 输入的 UID
  let preview = $state<PlayerPreview | null>(null); // 预览数据（资料+映射+图片路径）
  let imgPathInput = $state('');        // 用户选的替换图片路径
  let blobUrl = $state('');             // 图片的 blob URL（供 <img src> 使用）

  const snap = $derived(appStore.snapshot);
  const saveBackup = true; // 与主界面备份开关联动可后续扩展，先保持 v1 Viewer 行为

  /** 根据 UID 拉取预览，并把图片加载为 blob URL。 */
  async function showPreview() {
    if (!uid.trim()) return;
    preview = await api.previewPlayer(uid);
    // 加载图片为 blob URL（read_image 返回原始字节）。
    // 【为何用 blob】WebView 不能直接用本地磁盘路径作为 <img src>，需把字节包成浏览器可识别的 blob URL。
    if (blobUrl) { URL.revokeObjectURL(blobUrl); blobUrl = ''; } // 先释放上一次的 blob，避免内存泄漏
    if (preview?.imagePath) {
      const bytes = await api.readImage(preview.imagePath); // 拿到 ArrayBuffer
      const ext = preview.imagePath.split('.').pop()?.toLowerCase() ?? 'png'; // 推断扩展名
      const mime = ext === 'jpg' ? 'image/jpeg' : `image/${ext}`; // jpg 的 MIME 是 image/jpeg
      blobUrl = URL.createObjectURL(new Blob([bytes], { type: mime })); // 生成 blob URL
    }
  }

  /** 浏览选择一张图片（png/jpg/jpeg），填到替换路径输入框。 */
  async function browseImage() {
    const file = await open({
      filters: [{ name: t.viewer.imageFilterName, extensions: ['png', 'jpg', 'jpeg'] }],
      title: t.viewer.imageDialogTitle,
    });
    if (typeof file === 'string') imgPathInput = file;
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

  // d = 预览里的详细信息（方便模板里简写）；可能为 undefined。
  const d = $derived(preview?.record?.detailed);
  // 球员记录简写（未预览时为 null，模板用 ?? '—' 兜底）。
  const rec = $derived(preview?.record ?? null);
  // 当前映射的「种族\图片名」展示串（无映射时为 null）。
  const mappingText = $derived(
    preview?.mapping ? `${preview.mapping.ethnicity}\\${preview.mapping.image}` : '',
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

  <!-- 右列：头部（姓名 + 元信息 + UID）、信息格、替换行。 -->
  <div class="player-info">
    <div class="player-head">
      <div>
        <h2>{rec?.name || '—'}</h2>
        <div class="meta-row" style="margin-top:8px">
          <span class="meta">{t.table.colNat} <b>{rec?.primaryNat || '—'}</b></span>
          {#if rec?.secNat}
            <span class="meta">{t.table.col2ndNat} <b>{rec.secNat}</b></span>
          {/if}
          <span class="meta">{t.table.colEth} <b>{rec?.ethnicityCode || '—'}</b></span>
          {#if d?.skinCode}
            <span class="meta">{t.table.colSkin} <b>{d.skinCode}</b></span>
          {/if}
          {#if d?.isNewgan}
            <span class="meta accent">{t.table.colNewgan} · {d.isNewgan}</span>
          {/if}
        </div>
      </div>
      <!-- UID 必须保持为可输入框：输入后回车即预览（不可改成只读标签）。 -->
      <input id="pv-uid" class="uid-chip" type="text" bind:value={uid}
             placeholder="UID" aria-label={t.viewer.hint}
             onkeydown={(e) => e.key === 'Enter' && showPreview()} />
    </div>

    <!-- 信息格：标签在上值在下（只读，基础名单缺详细信息时显示 —）。 -->
    <dl class="dl-grid">
      <div class="cell"><dt>{t.viewer.labels.hair}</dt><dd>{rec?.hairLength || '—'}</dd></div>
      <div class="cell"><dt>{t.viewer.labels.color}</dt><dd>{rec?.hairColor || '—'}</dd></div>
      <div class="cell"><dt>{t.viewer.labels.age}</dt><dd>{d?.age || '—'}</dd></div>
      <div class="cell"><dt>{t.viewer.labels.club}</dt><dd>{d?.club || '—'}</dd></div>
      <div class="cell"><dt>{t.viewer.labels.height}</dt><dd>{d?.height || '—'}</dd></div>
      <div class="cell wide"><dt>{t.viewer.colMapping}</dt>
        <dd class="mono" title={mappingText}>{mappingText || '—'}</dd></div>
    </dl>

    <!-- 替换行沉底：图片路径输入框（文件夹图标）+ 浏览 + 替换。无组禁用浏览；输入不全禁用替换。 -->
    <div class="replace-row">
      <div class="input-wrap">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.folder.body}</svg>
        <input type="text" placeholder={t.viewer.imgPlaceholder} bind:value={imgPathInput}
               aria-label={t.viewer.imgPathAria} />
      </div>
      <button class="btn" onclick={browseImage} disabled={!snap?.currentGroup}>{t.viewer.browseBtn}</button>
      <button class="btn primary" onclick={replaceIt}
              disabled={!uid.trim() || !imgPathInput.trim()}>{t.viewer.replaceBtn}</button>
    </div>
  </div>
</div>
