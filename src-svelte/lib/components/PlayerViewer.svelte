<!-- 【新手导读】单人预览/微调：输入 UID 查看其头像与资料，或选一张新图单独替换。
     重点演示：把后端返回的图片字节（ArrayBuffer）转成 blob URL 供 <img> 显示。 -->
<script lang="ts">
  import { open, message } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { t } from '../text'; // 全局文案常量
  import { appStore } from '../stores/app.svelte';
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
</script>

<h3>{t.viewer.title} <span class="muted">{t.viewer.hint}</span></h3>

<!-- 左图右字的两列布局（窄窗口下会自动换行，见下方 style）。 -->
<div class="viewer-row">
  <div class="preview-col">
    <!-- 有 blob URL 就显示图片，否则显示“暂无图片”占位。 -->
    {#if blobUrl}
      <img src={blobUrl} alt={t.viewer.title} class="preview-img" />
    {:else}
      <div class="preview-placeholder">{t.viewer.noImage}</div>
    {/if}
    <!-- 图片下方显示当前映射的「种族\图片名」，无映射时显示占位。 -->
    <span class="muted mono-text">
      {preview?.mapping ? `${preview.mapping.ethnicity}\\${preview.mapping.image}` : t.viewer.noMappingPlaceholder}
    </span>
  </div>

  <div class="fields-col">
    <!-- 资料字段网格：每个 field 是“标签 + 只读输入框”。UID 框可输入并绑回车预览。 -->
    <div class="fgrid">
      <div class="field">
        <label for="pv-uid">{t.viewer.labels.uid}</label>
        <!-- onkeydown：按回车时触发预览（e.key === 'Enter' && ...）。 -->
        <input id="pv-uid" type="text" bind:value={uid} onkeydown={(e) => e.key === 'Enter' && showPreview()} />
      </div>
      <!-- 下面都是只读展示字段：value 绑预览里的对应值，?. 与 ?? '' 保证为空时不报错。 -->
      <div class="field"><label for="pv-nat">{t.viewer.labels.nat}</label><input id="pv-nat" type="text" readonly value={preview?.record?.primaryNat ?? ''} /></div>
      <div class="field"><label for="pv-2nd">{t.viewer.labels.nat2}</label><input id="pv-2nd" type="text" readonly value={preview?.record?.secNat ?? ''} /></div>
      <div class="field long"><label for="pv-name">{t.viewer.labels.name}</label><input id="pv-name" type="text" readonly value={preview?.record?.name ?? ''} /></div>
      <div class="field"><label for="pv-hair">{t.viewer.labels.hair}</label><input id="pv-hair" type="text" readonly value={preview?.record?.hairLength ?? ''} /></div>
      <div class="field"><label for="pv-color">{t.viewer.labels.color}</label><input id="pv-color" type="text" readonly value={preview?.record?.hairColor ?? ''} /></div>
      <div class="field"><label for="pv-eth">{t.viewer.labels.eth}</label><input id="pv-eth" type="text" readonly value={preview?.record?.ethnicityCode ?? ''} /></div>
      <div class="field"><label for="pv-skin">{t.viewer.labels.skin}</label><input id="pv-skin" type="text" readonly value={d?.skinCode ?? ''} /></div>
      <div class="field"><label for="pv-newgan">{t.viewer.labels.newgan}</label><input id="pv-newgan" type="text" readonly value={d?.isNewgan ?? ''} /></div>
      <div class="field long"><label for="pv-club">{t.viewer.labels.club}</label><input id="pv-club" type="text" readonly value={d?.club ?? ''} /></div>
      <div class="field"><label for="pv-age">{t.viewer.labels.age}</label><input id="pv-age" type="text" readonly value={d?.age ?? ''} /></div>
      <div class="field"><label for="pv-height">{t.viewer.labels.height}</label><input id="pv-height" type="text" readonly value={d?.height ?? ''} /></div>
      <div class="field"><label for="pv-weight">{t.viewer.labels.weight}</label><input id="pv-weight" type="text" readonly value={d?.weight ?? ''} /></div>
    </div>
    <!-- 替换行：图片路径输入框 + 浏览按钮 + 替换按钮。无选中组时禁用浏览；输入不全时禁用替换。 -->
    <div class="replace-row">
      <input type="text" placeholder={t.viewer.imgPlaceholder} bind:value={imgPathInput} aria-label={t.viewer.imgPathAria} />
      <button class="btn" onclick={browseImage} disabled={!snap?.currentGroup}>{t.viewer.browseBtn}</button>
      <button class="btn primary" onclick={replaceIt} disabled={!uid.trim() || !imgPathInput.trim()}>{t.viewer.replaceBtn}</button>
    </div>
  </div>
</div>

<style>
  /* 【说明】Svelte 的 <style> 默认“ scoped”（只作用于本组件），不会影响其他组件。
     var(--xxx) 是全局 CSS 变量（在 app.css 里定义的设计令牌），保证风格统一。 */
  /* 窄窗口下靠换行而不是横向滚动来保全 label 与按钮可见 */
  .viewer-row { display: flex; align-items: flex-start; gap: 12px; flex-wrap: wrap; }
  .preview-col { display: flex; flex-direction: column; align-items: center; gap: 4px; flex-shrink: 0; }
  .preview-img { width: 180px; height: 180px; object-fit: cover; border: 1px solid var(--border); border-radius: var(--radius-md); }
  .preview-placeholder { width: 180px; height: 180px; display: grid; place-items: center; border: 1px dashed var(--border); border-radius: var(--radius-md); color: var(--muted); }
  .mono-text { font-family: var(--mono); }
  .fields-col { flex: 1 1 380px; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  /* 字段网格用 flex 自动换行；每个 field 基础宽 170px，.long 更宽（放姓名/俱乐部）。 */
  .fgrid { display: flex; flex-wrap: wrap; gap: 6px 10px; }
  .field { display: flex; align-items: center; gap: 6px; flex: 1 1 170px; min-width: 0; }
  .field.long { flex-basis: 260px; }
  /* label 右对齐且不换行（符合项目的“选项设置标签右对齐”规范）。 */
  .field label { flex-shrink: 0; color: var(--muted); text-align: right; white-space: nowrap; }
  .field input { flex: 1 1 0; min-width: 48px; }
  .replace-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .replace-row input { flex: 1 1 220px; min-width: 0; }
</style>
