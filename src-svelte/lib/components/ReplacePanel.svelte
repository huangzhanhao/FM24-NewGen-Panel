<script lang="ts">
  import { ask } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { showError } from '../dialog';
  import { appStore, refreshSnapshot } from '../stores/app.svelte';

  const MODE_INFO: Record<string, string> = {
    Preserve: '保留已换过的球员，只为新球员补图（推荐日常增量）',
    Overwrite: '名单内球员全部重新抽图，名单外保留',
    Generate: '无视现有 config.xml，从零生成全部映射',
  };

  let mode = $state<'Preserve' | 'Overwrite' | 'Generate'>('Preserve');
  let allowDuplicates = $state(true);
  let filterNewgan = $state(true);
  let saveBackup = $state(true);

  const snap = $derived(appStore.snapshot);
  const busy = $derived(!!snap?.replacing);
  const ready = $derived(
    !!snap && snap.currentProfile !== 'No Profile' && !!snap.currentGroup && !!snap.currentRtf,
  );
  // 未就绪原因（引导用户补全，对等 v1 按钮三级禁用逻辑）
  const missing = $derived([
    !snap || snap.currentProfile === 'No Profile' ? '选择 Profile' : '',
    snap && !snap.currentGroup ? '选择头像包' : '',
    snap && !snap.currentRtf ? '设置 RTF 名单' : '',
  ].filter(Boolean));

  async function startReplace() {
    if (!snap?.currentGroup) return;
    try {
      // 兜底校验：用户可能跳过配置卡的完整性检查
      const lacking = await api.validateImageDirs(snap.currentGroup);
      if (lacking.length > 0) {
        const ok = await ask(
          `头像包缺少 ${lacking.length} 个种族目录，自动创建并继续？`,
          { title: 'Missing Directories' },
        );
        if (!ok) return;
        await api.createMissingDirs(snap.currentGroup, lacking);
      }
      await api.startReplace({ mode, filterNewgan, allowDuplicates, saveBackup });
      await refreshSnapshot(); // replacing=true → 全局禁用
    } catch (e) { await showError(e); }
  }
</script>

<div class="card">
  <div class="card-title">替换执行</div>
  <div class="row">
    <span class="row-label">模式：</span>
    {#each Object.keys(MODE_INFO) as m}
      <button
        class="mode-chip"
        class:active={mode === m}
        onclick={() => (mode = m as typeof mode)}
        disabled={busy}
      >{m}</button>
    {/each}
    <span class="muted small flex-1">{MODE_INFO[mode]}</span>
  </div>
  <div class="row checks">
    <span class="row-label">选项设置：</span>
    <label><input type="checkbox" bind:checked={allowDuplicates} disabled={busy} /> 允许复用图片</label>
    <label><input type="checkbox" bind:checked={filterNewgan} disabled={busy} /> 仅随机人</label>
    <label><input type="checkbox" bind:checked={saveBackup} disabled={busy} /> 写入前备份</label>
  </div>
  <div class="row">
    <button class="btn primary big flex-1" onclick={startReplace} disabled={busy || !ready}>
      {busy ? '替换中…' : 'Replace Faces'}
    </button>
    <button class="btn" onclick={() => api.cancelReplace()} disabled={!busy}>取消</button>
  </div>
  {#if busy || appStore.progress.value > 0}
    <div class="row">
      <div class="progress-track"><div class="progress-fill" style="width:{appStore.progress.value}%"></div></div>
      <span class="muted" style="min-width:180px">{appStore.progress.status}</span>
    </div>
  {/if}
  {#if !ready && missing.length > 0}
    <p class="muted small">还需完成：{missing.join(' · ')}</p>
  {/if}
</div>
