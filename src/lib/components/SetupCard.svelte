<script lang="ts">
  import { open, ask } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { showError } from '../dialog';
  import { appStore, applySnapshot } from '../stores/app.svelte';

  let missingDirs = $state<string[] | null>(null); // null = 未检查

  const snap = $derived(appStore.snapshot);
  const busy = $derived(!!snap?.replacing);

  async function browseRtf() {
    const file = await open({ filters: [{ name: 'RTF', extensions: ['rtf'] }], title: '选择 RTF 名单' });
    if (typeof file === 'string') {
      try { await applySnapshot(await api.setCurrentRtf(file)); missingDirs = null; }
      catch (e) { await showError(e); }
    }
  }

  async function checkIntegrity() {
    if (!snap?.currentGroup) return;
    missingDirs = await api.validateImageDirs(snap.currentGroup);
  }

  async function fixMissing() {
    if (!snap?.currentGroup || !missingDirs) return;
    const ok = await ask(
      `缺少 ${missingDirs.length} 个种族目录：\n${missingDirs.join(', ')}\n\n自动创建？`,
      { title: 'Missing Directories' },
    );
    if (!ok) return;
    try { await api.createMissingDirs(snap.currentGroup, missingDirs); missingDirs = []; }
    catch (e) { await showError(e); }
  }
</script>

{#if snap}
  <div class="card">
    <div class="card-title">当前任务 <span class="muted small">{snap.currentProfile}</span></div>
    {#if !snap.currentGroup}
      <p class="muted">先在左侧关联一个头像包目录，或点击已有组。</p>
    {:else}
      <div class="row">
        <label>头像包：</label>
        <input type="text" readonly value={snap.currentGroup} style="flex:1" title={snap.currentGroup} />
        <button class="btn" onclick={checkIntegrity} disabled={busy}>检查完整性</button>
      </div>
      <div class="row">
        <label>RTF 名单：</label>
        <input type="text" readonly value={snap.currentRtf || '未设置'} style="flex:1" />
        <button class="btn" onclick={browseRtf} disabled={busy}>选择文件</button>
      </div>
      {#if missingDirs !== null}
        {#if missingDirs.length === 0}
          <p class="ok-text small">✓ 头像包目录完整（14 个种族目录齐全）</p>
        {:else}
          <p class="warn-text small">缺少 {missingDirs.length} 个种族目录：{missingDirs.join(', ')}</p>
          <button class="btn primary" onclick={fixMissing} disabled={busy}>一键创建缺失目录</button>
        {/if}
      {/if}
    {/if}
  </div>
{/if}
