<script lang="ts">
  import { open, ask } from '@tauri-apps/plugin-dialog';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { api } from '../api';
  import { showError } from '../dialog';
  import { appStore, applySnapshot } from '../stores/app.svelte';

  let newProfileName = $state('');
  let creating = $state(false);

  const snap = $derived(appStore.snapshot);
  const busy = $derived(!!snap?.replacing);
  const hasProfile = $derived(!!snap && snap.currentProfile !== 'No Profile');

  async function createProfile() {
    try {
      await applySnapshot(await api.createProfile(newProfileName));
      newProfileName = '';
      creating = false;
    } catch (e) { await showError(e); }
  }

  async function switchProfile(e: Event) {
    try { await applySnapshot(await api.switchProfile((e.target as HTMLSelectElement).value)); }
    catch (e) { await showError(e); }
  }

  async function deleteProfile() {
    if (!snap) return;
    const ok = await ask(
      `删除 Profile「${snap.currentProfile}」及其全部组配置与映射备份？`,
      { title: 'Delete Profile' },
    );
    if (!ok) return;
    try { await applySnapshot(await api.deleteProfile(snap.currentProfile)); }
    catch (e) { await showError(e); }
  }

  async function linkFacepack() {
    const dir = await open({ directory: true, title: '关联头像包目录' });
    if (typeof dir === 'string') {
      try { await applySnapshot(await api.selectImageDirectory(dir)); }
      catch (e) { await showError(e); }
    }
  }

  async function selectGroup(imgDir: string) {
    try { await applySnapshot(await api.setCurrentGroup(imgDir)); }
    catch (e) { await showError(e); }
  }

  async function deleteGroup(imgDir: string) {
    const ok = await ask(`删除组及其映射备份？\n${imgDir}`, { title: 'Delete Group' });
    if (!ok) return;
    try { await applySnapshot(await api.deleteGroup(imgDir)); }
    catch (e) { await showError(e); }
  }

  const basename = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
</script>

<aside class="sidebar">
  <div class="brand">NewGAN Manager</div>

  <section>
    <div class="side-head">
      <span>Profile</span>
      <button class="icon-btn" title="新建 Profile" onclick={() => (creating = !creating)}>＋</button>
    </div>
    {#if creating}
      <div class="row">
        <input type="text" placeholder="新 Profile 名称" bind:value={newProfileName} style="width:100%" />
        <button class="btn" onclick={createProfile} disabled={!newProfileName.trim()}>创建</button>
      </div>
    {/if}
    <select value={snap?.currentProfile ?? ''} onchange={switchProfile} disabled={busy} style="width:100%">
      {#each snap?.profiles ?? [] as p}
        <option value={p}>{p}</option>
      {/each}
    </select>
    {#if hasProfile}
      <button class="link danger small" onclick={deleteProfile} disabled={busy}>删除当前 Profile</button>
    {/if}
  </section>

  <section>
    <div class="side-head">
      <span>头像包（组）</span>
      <button class="icon-btn" title="关联新的头像包目录" onclick={linkFacepack} disabled={busy || !hasProfile}>＋</button>
    </div>
    {#if snap && snap.groups.length > 0}
      <ul class="group-list">
        {#each snap.groups as g (g.imgDir)}
          <li class:current={snap.currentGroup === g.imgDir}>
            <button class="group-item" onclick={() => selectGroup(g.imgDir)} disabled={busy} title={g.imgDir}>
              <span class="g-name">{basename(g.imgDir)}</span>
              <span class="g-rtf" class:ok={!!g.rtf}>{g.rtf ? 'RTF ✓' : '未设置 RTF'}</span>
            </button>
            <button class="icon-btn danger" title="删除组" onclick={() => deleteGroup(g.imgDir)} disabled={busy}>×</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted small">暂无头像包。点上方 ＋ 关联头像包目录。</p>
    {/if}
  </section>

  <div class="side-foot">
    <button class="link small" onclick={() => openUrl('https://www.bilibili.com/video/BV1ew411h759')}>教程</button>
    <button class="link small" onclick={() => openUrl('https://github.com/Maradonna90/NewGAN-Manager/wiki/Troubleshooting')}>故障排查</button>
    <button class="link small" onclick={() => openUrl('https://github.com/Maradonna90/NewGAN-Manager/wiki/FAQ')}>FAQ</button>
    <button class="link small" onclick={() => openUrl('https://discord.gg/UfRpJVc')}>Discord</button>
  </div>
</aside>
