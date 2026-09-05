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

<div class="row" style="align-items:flex-start">
  <div style="display:flex;flex-direction:column;align-items:center;gap:4px;flex-shrink:0">
    {#if blobUrl}
      <img src={blobUrl} alt="preview" style="width:180px;height:180px;object-fit:cover;border:1px solid var(--border);border-radius:6px" />
    {:else}
      <div style="width:180px;height:180px;display:grid;place-items:center;border:1px dashed var(--border);border-radius:6px;color:var(--muted)">No Image</div>
    {/if}
    <span class="muted" style="font-family:var(--mono)">
      {preview?.mapping ? `${preview.mapping.ethnicity}\\${preview.mapping.image}` : '...\\...'}
    </span>
  </div>

  <div style="flex:1">
    <div class="row">
      <label>UID:</label>
      <input type="text" bind:value={uid} style="width:140px" onkeydown={(e) => e.key === 'Enter' && showPreview()} />
      <label>Nation:</label><input type="text" readonly value={preview?.record?.primaryNat ?? ''} style="width:60px" />
      <label>2nd:</label><input type="text" readonly value={preview?.record?.secNat ?? ''} style="width:60px" />
      <label>Name:</label><input type="text" readonly value={preview?.record?.name ?? ''} style="flex:1" />
    </div>
    <div class="row">
      <label>Hair:</label><input type="text" readonly value={preview?.record?.hairLength ?? ''} style="width:50px" />
      <label>Color:</label><input type="text" readonly value={preview?.record?.hairColor ?? ''} style="width:50px" />
      <label>Eth:</label><input type="text" readonly value={preview?.record?.ethnicityCode ?? ''} style="width:50px" />
      <label>Skin:</label><input type="text" readonly value={d?.skinCode ?? ''} style="width:50px" />
      <label>isNewGAN:</label><input type="text" readonly value={d?.isNewgan ?? ''} style="width:50px" />
    </div>
    <div class="row">
      <label>Club:</label><input type="text" readonly value={d?.club ?? ''} style="flex:1" />
      <label>Age:</label><input type="text" readonly value={d?.age ?? ''} style="width:50px" />
      <label>Height:</label><input type="text" readonly value={d?.height ?? ''} style="width:60px" />
      <label>Weight:</label><input type="text" readonly value={d?.weight ?? ''} style="width:60px" />
    </div>
    <div class="row">
      <input type="text" placeholder="Image to replace..." bind:value={imgPathInput} style="flex:1" />
      <button class="btn" onclick={browseImage} disabled={!snap?.currentGroup}>Browse</button>
      <button class="btn primary" onclick={replaceIt} disabled={!uid.trim() || !imgPathInput.trim()}>Replace it</button>
    </div>
  </div>
</div>
