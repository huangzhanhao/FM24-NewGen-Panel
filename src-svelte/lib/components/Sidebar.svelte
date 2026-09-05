<!-- 【新手导读】左侧边栏：管理存档（Profile）与头像包（组）。
     含新建/切换/删除存档、关联/选择/删除头像包、以及底部外部链接。
     重点演示：bind:this（拿到 DOM 引用）、bind:value、{#each} 渲染列表。 -->
<script lang="ts">
  import { open, ask } from '@tauri-apps/plugin-dialog'; // open=选目录；ask=确认框
  import { openUrl } from '@tauri-apps/plugin-opener';   // 用默认浏览器打开外部链接
  import { api } from '../api';
  import { showError } from '../dialog';
  import { appStore, applySnapshot } from '../stores/app.svelte';

  let newProfileName = $state('');   // 新建存档输入框的值
  let creating = $state(false);      // 是否展开“新建存档”行
  // bind:this 把 <select> 的真实 DOM 元素引用存进这个变量，方便失败时手动回退选中项。
  let profileSelect = $state<HTMLSelectElement | null>(null);

  const snap = $derived(appStore.snapshot);
  const busy = $derived(!!snap?.replacing); // 替换中时禁用大部分操作
  // hasProfile：是否已激活真实存档（'No Profile' 视为未激活）。
  const hasProfile = $derived(!!snap && snap.currentProfile !== 'No Profile');
  const activeProfile = $derived(snap?.currentProfile ?? '');
  /** 下拉即切档，选中项永远等于激活档；旁边只是个状态标签。 */
  const stateHint = $derived(
    hasProfile ? `当前已激活存档：${activeProfile}` : '尚未激活任何存档，请先新建或在下拉框选择',
  );

  /** 新建存档：调后端创建（会立即激活），成功后用返回快照刷新、清空输入、收起输入行。 */
  async function createProfile() {
    const name = newProfileName.trim();
    if (!name) return;
    try {
      await applySnapshot(await api.createProfile(name));
      newProfileName = '';
      creating = false;
    } catch (e) { await showError(e); }
  }

  /** 下拉框一改就切档（v1 交互）；失败时把选项回退到当前激活档。 */
  async function switchProfile() {
    const target = profileSelect?.value; // 从 DOM 引用读当前选中值
    if (!target || target === activeProfile) return; // 没变就不做
    try {
      await applySnapshot(await api.switchProfile(target));
    } catch (e) {
      // 切档失败：手动把下拉框选中项改回激活档（避免 UI 与实际不一致）。
      if (profileSelect) profileSelect.value = activeProfile;
      await showError(e);
    }
  }

  /** 删除当前存档（先弹确认框）。 */
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

  /** 关联头像包目录：选一个目录，后端自动建组并设为当前组。 */
  async function linkFacepack() {
    const dir = await open({ directory: true, title: '关联头像包目录' }); // directory:true 表示选文件夹
    if (typeof dir === 'string') {
      try { await applySnapshot(await api.selectImageDirectory(dir)); }
      catch (e) { await showError(e); }
    }
  }

  /** 选中某个已有组（切为当前组）。 */
  async function selectGroup(imgDir: string) {
    try { await applySnapshot(await api.setCurrentGroup(imgDir)); }
    catch (e) { await showError(e); }
  }

  /** 删除组（先弹确认框）。 */
  async function deleteGroup(imgDir: string) {
    const ok = await ask(`删除组及其映射备份？\n${imgDir}`, { title: 'Delete Group' });
    if (!ok) return;
    try { await applySnapshot(await api.deleteGroup(imgDir)); }
    catch (e) { await showError(e); }
  }

  // basename：从完整路径取最后一段（文件夹名），用于列表只显示短名。filter(Boolean) 去掉空段。
  const basename = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
</script>

<aside class="sidebar">
  <div class="brand">NewGAN Manager</div>

  <!-- 区域一：存档（Profile）管理。 -->
  <section>
    <div class="side-head">
      <span>存档资料</span>
      <!-- ＋按钮切换 creating（展开/收起新建行）。 -->
      <button class="icon-btn" title="新建 Profile" onclick={() => (creating = !creating)}>＋</button>
    </div>
    <!-- 新建行：仅当 creating 为真时显示（输入框 + 创建按钮）。 -->
    {#if creating}
      <div class="row">
        <input type="text" placeholder="新 Profile 名称" bind:value={newProfileName} class="w-full" />
        <!-- 输入为空时禁用创建按钮。 -->
        <button class="btn" onclick={createProfile} disabled={!newProfileName.trim()}>创建</button>
      </div>
    {/if}
    <!-- 存档下拉框：bind:this 拿到 DOM 引用；onchange 一改就切档；忙碌时禁用。 -->
    <select bind:this={profileSelect} value={activeProfile} onchange={switchProfile} disabled={busy} class="w-full">
      <!-- {#each} 遍历所有存档名生成选项（snap 为空时用 ?? [] 兑底）。 -->
      {#each snap?.profiles ?? [] as p}
        <option value={p}>{p}</option>
      {/each}
    </select>
    <div class="prf-actions">
      <!-- 状态标签：class:off 在未激活时加上；role="status" 供读屏软件播报；title 显示详细提示。 -->
      <span class="prf-state" class:off={!hasProfile} role="status" title={stateHint}>
        {hasProfile ? '已激活' : '未激活'}
      </span>
      <!-- 只有已激活真实存档时才显示“删除当前 Profile”按钮。 -->
      {#if hasProfile}
        <button class="btn danger-solid" onclick={deleteProfile} disabled={busy}>删除当前 Profile</button>
      {/if}
    </div>
  </section>

  <!-- 区域二：头像包（组）管理。 -->
  <section>
    <div class="side-head">
      <span>头像包（组）</span>
      <!-- 关联新目录按钮：忙碌或未激活存档时禁用。 -->
      <button class="icon-btn" title="关联新的头像包目录" onclick={linkFacepack} disabled={busy || !hasProfile}>＋</button>
    </div>
    <!-- 有组时渲染列表；否则显示空状态提示。 -->
    {#if snap && snap.groups.length > 0}
      <ul class="group-list">
        <!-- {#each ... as g (g.imgDir)}：以目录为键；class:current 高亮当前组。 -->
        {#each snap.groups as g (g.imgDir)}
          <li class:current={snap.currentGroup === g.imgDir}>
            <!-- 点组名切为当前组；title 显示完整路径；g-rtf 显示是否已设 RTF（class:ok 变绿）。 -->
            <button class="group-item" onclick={() => selectGroup(g.imgDir)} disabled={busy} title={g.imgDir}>
              <span class="g-name">{basename(g.imgDir)}</span>
              <span class="g-rtf" class:ok={!!g.rtf}>{g.rtf ? 'RTF ✓' : '未设置 RTF'}</span>
            </button>
            <!-- 删除组按钮（×）。 -->
            <button class="icon-btn danger" title="删除组" onclick={() => deleteGroup(g.imgDir)} disabled={busy}>×</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted small">暂无头像包。点上方 ＋ 关联头像包目录。</p>
    {/if}
  </section>

  <!-- 底部外部链接：用 openUrl 在默认浏览器打开（教程/故障排查/FAQ/Discord）。 -->
  <div class="side-foot">
    <button class="link small" onclick={() => openUrl('https://www.bilibili.com/video/BV1ew411h759')}>教程</button>
    <button class="link small" onclick={() => openUrl('https://github.com/Maradonna90/NewGAN-Manager/wiki/Troubleshooting')}>故障排查</button>
    <button class="link small" onclick={() => openUrl('https://github.com/Maradonna90/NewGAN-Manager/wiki/FAQ')}>FAQ</button>
    <button class="link small" onclick={() => openUrl('https://discord.gg/UfRpJVc')}>Discord</button>
  </div>
</aside>
