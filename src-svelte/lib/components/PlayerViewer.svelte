<script lang="ts">
  import { open, message } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { appStore } from '../stores/app.svelte';
  import type { PlayerPreview } from '../types';

  let uid = $state('');
  let preview = $state<PlayerPreview | null>(null);
  let imgPathInput = $state('');
  let blobUrl = $state('');

  const snap = $derived(appStore.snapshot);
  const saveBackup = true; // 与主界面备份开关联动可后续扩展，先保持 v1 Viewer 行为

  async function showPreview() {
    if (!uid.trim()) return;
    preview = await api.previewPlayer(uid);
    // 加载图片为 blob URL（read_image 返回原始字节）
    if (blobUrl) { URL.revokeObjectURL(blobUrl); blobUrl = ''; }
    if (preview?.imagePath) {
      const bytes = await api.readImage(preview.imagePath);
      const ext = preview.imagePath.split('.').pop()?.toLowerCase() ?? 'png';
      const mime = ext === 'jpg' ? 'image/jpeg' : `image/${ext}`;
      blobUrl = URL.createObjectURL(new Blob([bytes], { type: mime }));
    }
  }

  async function browseImage() {
    const file = await open({
      filters: [{ name: 'Image', extensions: ['png', 'jpg', 'jpeg'] }],
      title: 'Select image file',
    });
    if (typeof file === 'string') imgPathInput = file;
  }

  async function replaceIt() {
    if (!uid.trim() || !imgPathInput.trim()) {
      await message('Please provide both UID and valid image path', { title: 'Error' });
      return;
    }
    try {
      const hit = await api.replaceSingleFace(uid, imgPathInput, saveBackup);
      await message(
        hit
          ? `Successfully replaced UID ${uid}`
          : `config.xml 中未找到 UID ${uid} 的现有映射，未做修改`,
        { title: hit ? 'Info' : 'Warning' },
      );
      await showPreview(); // 刷新预览
    } catch (e) {
      await message(String(e), { title: 'Error' });
    }
  }

  const d = $derived(preview?.record?.detailed);
</script>

<h3>Viewer <span class="muted">输入 UID 回车预览</span></h3>

<div class="viewer-row">
  <div class="preview-col">
    {#if blobUrl}
      <img src={blobUrl} alt="preview" class="preview-img" />
    {:else}
      <div class="preview-placeholder">No Image</div>
    {/if}
    <span class="muted mono-text">
      {preview?.mapping ? `${preview.mapping.ethnicity}\\${preview.mapping.image}` : '...\\...'}
    </span>
  </div>

  <div class="fields-col">
    <div class="fgrid">
      <div class="field">
        <label for="pv-uid">UID:</label>
        <input id="pv-uid" type="text" bind:value={uid} onkeydown={(e) => e.key === 'Enter' && showPreview()} />
      </div>
      <div class="field"><label for="pv-nat">Nation:</label><input id="pv-nat" type="text" readonly value={preview?.record?.primaryNat ?? ''} /></div>
      <div class="field"><label for="pv-2nd">2nd:</label><input id="pv-2nd" type="text" readonly value={preview?.record?.secNat ?? ''} /></div>
      <div class="field long"><label for="pv-name">Name:</label><input id="pv-name" type="text" readonly value={preview?.record?.name ?? ''} /></div>
      <div class="field"><label for="pv-hair">Hair:</label><input id="pv-hair" type="text" readonly value={preview?.record?.hairLength ?? ''} /></div>
      <div class="field"><label for="pv-color">Color:</label><input id="pv-color" type="text" readonly value={preview?.record?.hairColor ?? ''} /></div>
      <div class="field"><label for="pv-eth">Eth:</label><input id="pv-eth" type="text" readonly value={preview?.record?.ethnicityCode ?? ''} /></div>
      <div class="field"><label for="pv-skin">Skin:</label><input id="pv-skin" type="text" readonly value={d?.skinCode ?? ''} /></div>
      <div class="field"><label for="pv-newgan">isNewGAN:</label><input id="pv-newgan" type="text" readonly value={d?.isNewgan ?? ''} /></div>
      <div class="field long"><label for="pv-club">Club:</label><input id="pv-club" type="text" readonly value={d?.club ?? ''} /></div>
      <div class="field"><label for="pv-age">Age:</label><input id="pv-age" type="text" readonly value={d?.age ?? ''} /></div>
      <div class="field"><label for="pv-height">Height:</label><input id="pv-height" type="text" readonly value={d?.height ?? ''} /></div>
      <div class="field"><label for="pv-weight">Weight:</label><input id="pv-weight" type="text" readonly value={d?.weight ?? ''} /></div>
    </div>
    <div class="replace-row">
      <input type="text" placeholder="Image to replace..." bind:value={imgPathInput} aria-label="替换图片路径" />
      <button class="btn" onclick={browseImage} disabled={!snap?.currentGroup}>Browse</button>
      <button class="btn primary" onclick={replaceIt} disabled={!uid.trim() || !imgPathInput.trim()}>Replace it</button>
    </div>
  </div>
</div>

<style>
  /* 窄窗口下靠换行而不是横向滚动来保全 label 与按钮可见 */
  .viewer-row { display: flex; align-items: flex-start; gap: 12px; flex-wrap: wrap; }
  .preview-col { display: flex; flex-direction: column; align-items: center; gap: 4px; flex-shrink: 0; }
  .preview-img { width: 180px; height: 180px; object-fit: cover; border: 1px solid var(--border); border-radius: var(--radius-md); }
  .preview-placeholder { width: 180px; height: 180px; display: grid; place-items: center; border: 1px dashed var(--border); border-radius: var(--radius-md); color: var(--muted); }
  .mono-text { font-family: var(--mono); }
  .fields-col { flex: 1 1 380px; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .fgrid { display: flex; flex-wrap: wrap; gap: 6px 10px; }
  .field { display: flex; align-items: center; gap: 6px; flex: 1 1 170px; min-width: 0; }
  .field.long { flex-basis: 260px; }
  .field label { flex-shrink: 0; color: var(--muted); text-align: right; white-space: nowrap; }
  .field input { flex: 1 1 0; min-width: 48px; }
  .replace-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .replace-row input { flex: 1 1 220px; min-width: 0; }
</style>
