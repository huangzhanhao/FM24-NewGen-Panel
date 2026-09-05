<script lang="ts">
  import { open, ask } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { showError } from '../dialog';
  import { pinScrollEnd } from '../actions';
  import { appStore, applySnapshot } from '../stores/app.svelte';

  let missingDirs = $state<string[] | null>(null); // null = 未检查

  const snap = $derived(appStore.snapshot);
  const busy = $derived(!!snap?.replacing);

  // 切档 / 换组后清掉上一次检查结论，不把旧组的结论显示到新组上
  let lastGroup = '';
  $effect(() => {
    const g = snap?.currentGroup ?? '';
    if (g !== lastGroup) { lastGroup = g; missingDirs = null; }
  });

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
    <div class="path-row">
      <label for="sc-group">头像包：</label>
      <input id="sc-group" type="text" readonly value={snap.currentGroup ?? ''}
             placeholder="未关联头像包目录" title={snap.currentGroup ?? ''}
             use:pinScrollEnd={snap.currentGroup ?? ''} />
      <button class="btn" onclick={checkIntegrity} disabled={busy || !snap.currentGroup}
              title={snap.currentGroup ? '校验 14 个种族目录是否齐全' : '请先关联头像包目录'}>检查完整性</button>
    </div>
    <div class="path-row">
      <label for="sc-rtf">RTF 名单：</label>
      <input id="sc-rtf" type="text" readonly value={snap.currentRtf || '未设置'} title={snap.currentRtf}
             use:pinScrollEnd={snap.currentRtf} />
      <button class="btn" onclick={browseRtf} disabled={busy || !snap.currentGroup}
              title={snap.currentGroup ? '选择 FM 导出的 RTF 名单' : '请先关联头像包目录'}>选择文件</button>
    </div>
    {#if !snap.currentGroup}
      <p class="muted small">先在左侧「头像包（组）」关联一个头像包目录，或点击已有组。</p>
    {:else if missingDirs !== null}
      {#if missingDirs.length === 0}
        <p class="ok-text small">✓ 头像包目录完整（14 个种族目录齐全）</p>
      {:else}
        <p class="warn-text small">缺少 {missingDirs.length} 个种族目录：{missingDirs.join(', ')}</p>
        <button class="btn primary" onclick={fixMissing} disabled={busy}>一键创建缺失目录</button>
      {/if}
    {/if}
  </div>
{/if}
