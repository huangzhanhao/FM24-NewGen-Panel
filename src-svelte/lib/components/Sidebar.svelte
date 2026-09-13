<!-- 左侧边栏：管理存档（Profile）与头像包（组），含新建/切换/删除存档、关联/选择/删除头像包与底部外部链接。 -->
<script lang="ts">
  import { open, ask } from '@tauri-apps/plugin-dialog'; // open=选目录；ask=确认框
  import { openUrl } from '@tauri-apps/plugin-opener';   // 用默认浏览器打开外部链接
  import { api } from '../api';
  import { showError } from '../dialog';
  import { t } from '../text'; // 全局文案常量
  import { appStore, applySnapshot } from '../stores/app.svelte';
  import { ui, toggleTheme } from '../stores/ui.svelte';
  import { icons } from '../icons';
  // Vite 静态资源导入：打包后得到带哈希的图片 URL（app-icon.png 在项目根）。
  import brandLogo from '../../../app-icon.png';

  let newProfileName = $state('');   // 新建存档输入框的值
  let creating = $state(false);      // 是否展开“新建存档”行
  // bind:this 把 <select> 的真实 DOM 元素引用存进这个变量，方便失败时手动回退选中项。
  let profileSelect = $state<HTMLSelectElement | null>(null);

  const snap = $derived(appStore.profilePreview ?? appStore.snapshot); // 展示层：浏览态优先
  const busy = $derived(!!snap?.replacing); // 替换中时禁用大部分操作
  // 浏览态：下拉选中的存档 ≠ 后端当前激活档（选中项未被激活）。
  const browseMode = $derived(
    !!snap && appStore.selectedProfile !== (appStore.snapshot?.currentProfile ?? ''),
  );
  // selectedIsActive：下拉选中的存档是否就是当前激活档（激活按钮灰/蓝态的依据）。
  const selectedIsActive = $derived(
    !!appStore.snapshot && appStore.selectedProfile === appStore.snapshot.currentProfile,
  );
  // hasProfile：选中项已激活且不是 'No Profile'——组管理与删除等写操作仅在此态可用。
  const hasProfile = $derived(selectedIsActive && appStore.selectedProfile !== 'No Profile');
  /** 新建存档：调后端创建（会立即激活），成功后选中项同步、清空输入、收起输入行。 */
  async function createProfile() {
    const name = newProfileName.trim();
    if (!name) return;
    try {
      await applySnapshot(await api.createProfile(name));
      appStore.selectedProfile = name;      // 新建即激活，下拉选中项同步
      appStore.profilePreview = null;       // 展示与激活态一致
      newProfileName = '';
      creating = false;
    } catch (e) { await showError(e); }
  }

  /** 下拉选择 = 仅浏览（组列表/当前任务卡随选中项刷新展示），不切换后端当前档；
      需要点「激活」按钮才真正切换。失败时把下拉选中项回退。 */
  async function onSelectChange() {
    const target = profileSelect?.value;
    const prev = appStore.selectedProfile;
    if (!target || target === prev) return;
    appStore.selectedProfile = target;
    try {
      appStore.profilePreview = await api.previewProfile(target);
    } catch (e) {
      appStore.selectedProfile = prev; // 预览失败：回退选中项
      appStore.profilePreview = null;
      await showError(e);
    }
  }

  /** 激活下拉选中的存档：真正切换后端当前档（快照交换），浏览态结束。 */
  async function activateProfile() {
    const target = appStore.selectedProfile;
    if (!target || target === (appStore.snapshot?.currentProfile ?? '')) return;
    try {
      await applySnapshot(await api.switchProfile(target));
      appStore.profilePreview = null; // 激活后展示与激活态一致
    } catch (e) { await showError(e); }
  }

  /** 删除下拉选中的存档（先弹确认框）。删除激活档会自动回到 No Profile；删未激活档不影响当前档。 */
  async function deleteProfile() {
    const target = appStore.selectedProfile;
    if (!target || target === 'No Profile') return;
    const ok = await ask(
      t.sidebar.deleteProfileConfirm(target),
      { title: t.sidebar.deleteProfileTitle },
    );
    if (!ok) return;
    try {
      await applySnapshot(await api.deleteProfile(target));
      // 下拉固定切回 No Profile 并预览其（空）资料；其他存档的激活状态不受影响
      appStore.selectedProfile = 'No Profile';
      appStore.profilePreview = await api.previewProfile('No Profile');
    } catch (e) { await showError(e); }
  }

  /** 关联头像包目录：选一个目录，后端自动建组并设为当前组。 */
  async function linkFacepack() {
    const dir = await open({ directory: true, title: t.sidebar.linkDialogTitle }); // directory:true 表示选文件夹
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
    const ok = await ask(t.sidebar.deleteGroupConfirm(imgDir), { title: t.sidebar.deleteGroupTitle });
    if (!ok) return;
    try { await applySnapshot(await api.deleteGroup(imgDir)); }
    catch (e) { await showError(e); }
  }

  // basename：从完整路径取最后一段（文件夹名），用于列表只显示短名。filter(Boolean) 去掉空段。
  const basename = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
</script>

<aside class="sidebar">
  <!-- 品牌区：柴犬图标（取自应用图标）+ 名称 + v2.0 胶囊 -->
  <div class="brand-row">
    <img class="brand-logo" src={brandLogo} alt="" />
    <div>
      <div class="brand-name">{t.brand}</div>
      <span class="brand-ver">{t.app.brandVer}</span>
    </div>
  </div>

  <!-- 区域一：存档（Profile）管理。 -->
  <section>
    <div class="side-head">
      <span>{t.sidebar.profileSection}</span>
      <!-- ＋按钮切换 creating（展开/收起新建行）。 -->
      <button class="icon-btn" title={t.sidebar.newProfileBtn} onclick={() => (creating = !creating)}>＋</button>
    </div>
    <!-- 新建行：仅当 creating 为真时显示（输入框 + 创建按钮）。 -->
    {#if creating}
      <div class="row">
        <input type="text" placeholder={t.sidebar.newProfilePlaceholder} bind:value={newProfileName} class="w-full" />
        <!-- 输入为空时禁用创建按钮。 -->
        <button class="btn" onclick={createProfile} disabled={!newProfileName.trim()}>{t.sidebar.createBtn}</button>
      </div>
    {/if}
    <!-- 存档下拉框：选中即「浏览」（组列表/当前任务卡随选中项刷新），不切档；
         点下方「激活」按钮才真正切换后端当前档。忙碌时禁用。 -->
    <select bind:this={profileSelect} value={appStore.selectedProfile} onchange={onSelectChange} disabled={busy} class="w-full">
      <!-- 遍历所有存档名生成选项（快照未加载时兜底为空数组）。 -->
      {#each snap?.profiles ?? [] as p}
        <option value={p}>{p}</option>
      {/each}
    </select>
    <div class="prf-actions">
      <!-- No Profile（缺省档）：固定「未激活」+「无法删除No Profile」，均不可点。 -->
      {#if appStore.selectedProfile === 'No Profile'}
        <button class="btn" disabled title={t.sidebar.noProfileNoActivate}>{t.sidebar.notActivated}</button>
        <button class="btn danger-solid" disabled
                title={t.sidebar.noProfileNoDelete}>{t.sidebar.noProfileNoDelete}</button>
      {:else if browseMode}
        <!-- 浏览未激活存档：蓝色「激活」可点 + 删除按钮。 -->
        <button class="btn primary" onclick={activateProfile} disabled={busy}
                title={t.sidebar.activateHint(appStore.selectedProfile)}>{t.sidebar.activateBtn}</button>
        <button class="btn danger-solid" onclick={deleteProfile} disabled={busy}>{t.sidebar.deleteCurrentProfile}</button>
      {:else}
        <!-- 选中项已激活：灰色「已激活」不可点 + 删除按钮。 -->
        <button class="btn" disabled title={t.sidebar.activeAlreadyHint}>{t.sidebar.activated}</button>
        <button class="btn danger-solid" onclick={deleteProfile} disabled={busy}>{t.sidebar.deleteCurrentProfile}</button>
      {/if}
    </div>
  </section>

  <!-- 区域二：头像包（组）管理。 -->
  <section>
    <div class="side-head">
      <span>{t.sidebar.groupSection}</span>
      <!-- 关联新目录按钮：忙碌或选中项未激活（浏览态）时禁用——组操作仅对激活档可用。 -->
      <button class="icon-btn" title={browseMode ? t.sidebar.linkBrowseHint : t.sidebar.linkNewHint}
              onclick={linkFacepack} disabled={busy || !hasProfile}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
             stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{@html icons.plus.body}</svg>
      </button>
    </div>
    <!-- 有组时渲染列表；否则显示空状态提示。 -->
    {#if snap && snap.groups.length > 0}
      <ul class="group-list">
        <!-- 以目录为键；class:current 高亮当前组。 -->
        {#each snap.groups as g (g.imgDir)}
          <li class:current={snap.currentGroup === g.imgDir}>
            <!-- 点组名切为当前组（浏览态禁用——切组写后端当前档）；title 显示完整路径；g-rtf 显示是否已设 RTF。 -->
            <button class="group-item" onclick={() => selectGroup(g.imgDir)}
                    disabled={busy || browseMode}
                    title={browseMode ? t.sidebar.selectGroupBrowseHint : g.imgDir}>
              <span class="g-name">{basename(g.imgDir)}</span>
              <span class="g-rtf" class:ok={!!g.rtf}>{g.rtf ? t.sidebar.rtfSet : t.sidebar.rtfUnset}</span>
            </button>
            <!-- 删除组按钮（×）：浏览态禁用。 -->
            <button class="icon-btn danger" title={browseMode ? t.sidebar.deleteGroupBrowseHint : t.sidebar.deleteGroupBtn}
                    onclick={() => deleteGroup(g.imgDir)} disabled={busy || browseMode}>×</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted small">{t.sidebar.emptyGroups}</p>
    {/if}
  </section>

  <!-- 底部外部链接：用 openUrl 在默认浏览器打开（教程/故障排查/FAQ/Discord）。 -->
  <div class="side-foot">
    <button class="link small" onclick={() => openUrl('https://www.bilibili.com/video/BV1ew411h759')}>{t.sidebar.linkTutorial}</button>
    <button class="link small" onclick={() => openUrl('https://github.com/Maradonna90/NewGAN-Manager/wiki/Troubleshooting')}>{t.sidebar.linkTroubleshooting}</button>
    <button class="link small" onclick={() => openUrl('https://github.com/Maradonna90/NewGAN-Manager/wiki/FAQ')}>{t.sidebar.linkFaq}</button>
    <button class="link small" onclick={() => openUrl('https://discord.gg/UfRpJVc')}>{t.sidebar.linkDiscord}</button>
    <!-- 主题切换：深色显示月亮+「深色」，浅色显示太阳+「浅色」；选择记忆在 localStorage -->
    <button class="theme-switch" onclick={toggleTheme}
            title={t.theme.switchTo(ui.theme === 'dark' ? 'light' : 'dark')}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
           stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" width="13" height="13">
        {@html (ui.theme === 'dark' ? icons.moon : icons.sun).body}
      </svg>
      <span>{ui.theme === 'dark' ? t.theme.dark : t.theme.light}</span>
    </button>
  </div>
</aside>
