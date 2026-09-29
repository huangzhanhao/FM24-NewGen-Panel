<!-- 左侧边栏：管理存档（Profile）与头像包（组），含新建/切换/删除存档、关联/选择/删除头像包与底部外部链接。 -->
<script lang="ts">
  import { open, ask } from "@tauri-apps/plugin-dialog"; // open=选目录；ask=确认框
  import { openUrl } from "@tauri-apps/plugin-opener"; // 用默认浏览器打开外部链接
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text"; // 全局文案常量
  import { appStore, applySnapshot } from "../../stores/app.svelte";
  import {
    imageaiStore,
    queueCancel,
  } from "../../stores/imageai.svelte"; // 队列运行中的作用域切换护栏（S10）
  import { ui, toggleTheme } from "../../stores/ui.svelte";
  import { icons } from "../../ui/icons";
  // Vite 静态资源导入：打包后得到带哈希的图片 URL（app-icon.png 在项目根）。
  import brandLogo from "../../../../app-icon.png";

  let newProfileName = $state(""); // 新建存档输入框的值
  let creating = $state(false); // 是否展开“新建存档”行
  // bind:this 把 <select> 的真实 DOM 元素引用存进这个变量，方便失败时手动回退选中项。
  let profileSelect = $state<HTMLSelectElement | null>(null);

  // 展示态统一取自 store getter（核心规则单点维护）。
  const snap = $derived(appStore.displaySnapshot);
  const busy = $derived(appStore.busy);
  const browseMode = $derived(appStore.browseMode);
  // selectedIsActive：下拉选中的存档是否就是当前激活档（激活按钮灰/蓝态的依据）。
  const selectedIsActive = $derived(
    !!appStore.snapshot &&
      appStore.selectedProfile === appStore.snapshot.currentProfile,
  );
  // hasProfile：选中项已激活且不是 'No Profile'——组管理与删除等写操作仅在此态可用。
  const hasProfile = $derived(
    selectedIsActive && appStore.selectedProfile !== "No Profile",
  );
  /** 新建存档：调后端创建（会立即激活），成功后选中项同步、清空输入、收起输入行。
   *  「立即激活」= 作用域变更，故与切档同款护栏（队列运行中先过确认）——新建本身没有别的
   *  确认框，所以这里是独立的一次询问，不存在双弹。 */
  async function createProfile() {
    const name = newProfileName.trim();
    if (!name) return;
    if (!(await confirmScopeChange())) return; // 队列运行中新建档（会立即激活）：先过确认
    try {
      await applySnapshot(await api.createProfile(name));
      appStore.selectedProfile = name; // 新建即激活，下拉选中项同步
      appStore.profilePreview = null; // 展示与激活态一致
      newProfileName = "";
      creating = false;
    } catch (e) {
      await showError(e);
    }
  }

  /** 下拉选择 = 仅浏览（组列表/当前任务卡随选中项刷新展示），不切换后端当前档；
      需要点「激活」按钮才真正切换。失败时把下拉选中项回退。 */
  let previewToken = 0; // 预览令牌：快速连点下拉时，先发的响应不能盖掉后选的档
  async function onSelectChange() {
    const target = profileSelect?.value;
    const prev = appStore.selectedProfile;
    if (!target || target === prev) return;
    appStore.selectedProfile = target;
    const token = ++previewToken;
    try {
      const got = await api.previewProfile(target);
      // 【为什么要令牌】下拉里连点两下（档A→档B）时两个请求会乱序返回：
      // profilePreview 停在档A 的组/RTF/快照路径，而 selectedProfile 显示档B——
      // 「读取快照」按这份 preview 的路径去读，读的就是**另一个档**的档景。
      if (token !== previewToken) return;
      appStore.profilePreview = got;
    } catch (e) {
      if (token !== previewToken) return; // 已被更新的预览取代：这条错也不再打扰用户
      appStore.selectedProfile = prev; // 预览失败：回退选中项
      appStore.profilePreview = null;
      await showError(e);
    }
  }

  /** 作用域变更的统一确认（唯一实现处）——切档 / 切组 / 关联头像包 / 新建档 / 删档 / 删组都走这里。
   *
   * 【为什么合成一个】这些操作对用户来说都是「当前作用域要变了」这一件事，队列跑着的时候
   * 该说的话完全一样。分散成多份实现，迟早有入口漏掉队列警告——本项目确实漏过
   * createProfile / deleteProfile / deleteGroup 三处。
   *
   * 【只有两处由调用方决定】
   * - `message`：这次具体要做什么。默认值即切换类口径（切档/切组/新建/关联）的正文；
   * - `askWhenIdle`：队列**空闲**时要不要也问一句。删除不可逆 → true；切换类空闲无副作用 → false。
   *
   * 队列警告（`queueScopeAppend`）与 warning 样式一律由本函数追加，调用方不要自己拼——
   * 这也保证了「一次操作只弹一次」：删除类把队列警告并进同一个框，而不是先问删除再问队列。
   * 确认后若队列在跑则先取消队列；**取消失败返回 false**，调用方据此中止后续动作。 */
  async function confirmScopeChange(opts: {
    message?: string;
    title?: string;
    askWhenIdle?: boolean;
  } = {}): Promise<boolean> {
    const message = opts.message ?? t.sidebar.queueScopeConfirm;
    const title = opts.title ?? t.sidebar.queueScopeTitle;
    const running = imageaiStore.queueRunning;
    if (!running && !opts.askWhenIdle) return true; // 空闲的切换类操作：不打扰（S10）
    const ok = await ask(message + (running ? t.sidebar.queueScopeAppend : ""), {
      title,
      ...(running ? { kind: "warning" as const } : {}),
    });
    if (!ok) return false;
    if (!running) return true; // 空闲删除：本就没有队列要取消
    try {
      await queueCancel();
      return true;
    } catch (e) {
      await showError(e);
      return false;
    }
  }

  /** 激活下拉选中的存档：真正切换后端当前档（快照交换），浏览态结束。 */
  async function activateProfile() {
    const target = appStore.selectedProfile;
    if (!target || target === (appStore.snapshot?.currentProfile ?? "")) return;
    if (!(await confirmScopeChange())) return; // 队列运行中切档：先过确认
    try {
      await applySnapshot(await api.switchProfile(target));
      appStore.profilePreview = null; // 激活后展示与激活态一致
    } catch (e) {
      await showError(e);
    }
  }

  /** 删除下拉选中的存档。删除激活档会自动回到 No Profile；删未激活档不影响当前档。
   *  确认统一走 confirmScopeChange：空闲时弹删除确认（askWhenIdle: true），
   *  队列运行中则把队列警告并进同一个框——一次操作只打扰一次。 */
  async function deleteProfile() {
    const target = appStore.selectedProfile;
    if (!target || target === "No Profile") return;
    const ok = await confirmScopeChange({
      message: t.sidebar.deleteProfileConfirm(target),
      title: t.sidebar.deleteProfileTitle,
      askWhenIdle: true,
    });
    if (!ok) return;
    try {
      await applySnapshot(await api.deleteProfile(target));
      // 下拉固定切回 No Profile 并预览其（空）资料；其他存档的激活状态不受影响
      appStore.selectedProfile = "No Profile";
      appStore.profilePreview = await api.previewProfile("No Profile");
    } catch (e) {
      await showError(e);
    }
  }

  /** 关联头像包目录：选一个目录，后端自动建组并设为当前组。 */
  async function linkFacepack() {
    const dir = await open({
      directory: true,
      title: t.sidebar.linkDialogTitle,
    }); // directory:true 表示选文件夹
    if (typeof dir === "string") {
      if (!(await confirmScopeChange())) return; // 队列运行中切组：先过确认
      try {
        await applySnapshot(await api.selectImageDirectory(dir));
      } catch (e) {
        await showError(e);
      }
    }
  }

  /** 选中某个已有组（切为当前组）。 */
  async function selectGroup(imgDir: string) {
    if (!(await confirmScopeChange())) return; // 队列运行中切组：先过确认
    try {
      await applySnapshot(await api.setCurrentGroup(imgDir));
    } catch (e) {
      await showError(e);
    }
  }

  /** 删除组。与 deleteProfile 同款：确认统一走 confirmScopeChange，
   *  空闲弹删除确认、运行中把队列警告并进同一个框。 */
  async function deleteGroup(imgDir: string) {
    const ok = await confirmScopeChange({
      message: t.sidebar.deleteGroupConfirm(imgDir),
      title: t.sidebar.deleteGroupTitle,
      askWhenIdle: true,
    });
    if (!ok) return;
    try {
      await applySnapshot(await api.deleteGroup(imgDir));
    } catch (e) {
      await showError(e);
    }
  }

  // basename：从完整路径取最后一段（文件夹名），用于列表只显示短名。filter(Boolean) 去掉空段。
  const basename = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
</script>

<aside class="sidebar">
  <!-- 品牌区：柴犬图标（取自应用图标）+ 名称 + v2.0 胶囊；第二行右侧为深/浅色主题切换 -->
  <div class="brand-row">
    <img class="brand-logo" src={brandLogo} alt="" />
    <div class="brand-text">
      <div class="brand-name">{t.brand}</div>
      <div class="brand-sub">
        <span class="brand-ver">{t.app.brandVer}</span>
        <!-- 主题切换：深色显示月亮+「深色」，浅色显示太阳+「浅色」；选择记忆在 localStorage -->
        <button
          class="theme-switch"
          onclick={toggleTheme}
          title={t.theme.switchTo(ui.theme === "dark" ? "light" : "dark")}
        >
          <svg
            viewBox="0 0 22 22"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
            width="11"
            height="11"
          >
            {@html (ui.theme === "dark" ? icons.moon : icons.sun).body}
          </svg>
          <span>{ui.theme === "dark" ? t.theme.dark : t.theme.light}</span>
        </button>
      </div>
    </div>
  </div>

  <!-- 区域一：存档（Profile）管理。 -->
  <section>
    <div class="side-head">
      <span>{t.sidebar.profileSection}</span>
      <!-- ＋按钮切换 creating（展开/收起新建行）。 -->
      <button
        class="icon-btn"
        title={t.sidebar.newProfileBtn}
        onclick={() => (creating = !creating)}
      >
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true">{@html icons.plus.body}</svg
        >
      </button>
    </div>
    <!-- 新建行：仅当 creating 为真时显示（输入框 + 创建按钮）。 -->
    {#if creating}
      <div class="row">
        <input
          type="text"
          placeholder={t.sidebar.newProfilePlaceholder}
          bind:value={newProfileName}
          class="w-full"
        />
        <!-- 输入为空时禁用创建按钮。 -->
        <button
          class="btn"
          onclick={createProfile}
          disabled={!newProfileName.trim()}>{t.sidebar.createBtn}</button
        >
      </div>
    {/if}
    <!-- 存档下拉框：选中即「浏览」（组列表/当前任务卡随选中项刷新），不切档；
         点下方「激活」按钮才真正切换后端当前档。忙碌时禁用。 -->
    <select
      bind:this={profileSelect}
      value={appStore.selectedProfile}
      onchange={onSelectChange}
      disabled={busy}
      class="w-full"
    >
      <!-- 遍历所有存档名生成选项（快照未加载时兜底为空数组）。 -->
      {#each snap?.profiles ?? [] as p}
        <option value={p}>{p}</option>
      {/each}
    </select>
    <div class="prf-actions">
      <!-- No Profile（缺省档）：固定「未激活」+「无法删除No Profile」，均不可点。 -->
      {#if appStore.selectedProfile === "No Profile"}
        <button class="btn" disabled title={t.sidebar.noProfileNoActivate}
          >{t.sidebar.notActivated}</button
        >
        <button
          class="btn danger-solid"
          disabled
          title={t.sidebar.noProfileNoDelete}
          >{t.sidebar.noProfileNoDelete}</button
        >
      {:else if browseMode}
        <!-- 浏览未激活存档：蓝色「激活」可点 + 删除按钮。 -->
        <button
          class="btn primary"
          onclick={activateProfile}
          disabled={busy}
          title={t.sidebar.activateHint(appStore.selectedProfile)}
          >{t.sidebar.activateBtn}</button
        >
        <button class="btn danger-solid" onclick={deleteProfile} disabled={busy}
          >{t.sidebar.deleteCurrentProfile}</button
        >
      {:else}
        <!-- 选中项已激活：灰色「已激活」不可点 + 删除按钮。 -->
        <button class="btn" disabled title={t.sidebar.activeAlreadyHint}
          >{t.sidebar.activated}</button
        >
        <button class="btn danger-solid" onclick={deleteProfile} disabled={busy}
          >{t.sidebar.deleteCurrentProfile}</button
        >
      {/if}
    </div>
  </section>

  <!-- 区域二：头像包（组）管理。 -->
  <section>
    <div class="side-head">
      <span>{t.sidebar.groupSection}</span>
      <!-- 关联新目录按钮：忙碌或选中项未激活（浏览态）时禁用——组操作仅对激活档可用。 -->
      <button
        class="icon-btn"
        title={browseMode ? t.sidebar.linkBrowseHint : t.sidebar.linkNewHint}
        onclick={linkFacepack}
        disabled={busy || !hasProfile}
      >
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true">{@html icons.plus.body}</svg
        >
      </button>
    </div>
    <!-- 有组时渲染列表；否则显示空状态提示。 -->
    {#if snap && snap.groups.length > 0}
      <ul class="group-list">
        <!-- 以目录为键；class:current 高亮当前组。 -->
        {#each snap.groups as g (g.imgDir)}
          <li class:current={snap.currentGroup === g.imgDir}>
            <!-- 点组名切为当前组（浏览态禁用——切组写后端当前档）；title 显示完整路径；g-rtf 显示是否已设 RTF。 -->
            <button
              class="group-item"
              onclick={() => selectGroup(g.imgDir)}
              disabled={busy || browseMode}
              title={browseMode ? t.sidebar.selectGroupBrowseHint : g.imgDir}
            >
              <span class="g-name">{basename(g.imgDir)}</span>
              <span class="g-rtf" class:ok={!!g.rtf}
                >{g.rtf ? t.sidebar.rtfSet : t.sidebar.rtfUnset}</span
              >
            </button>
            <!-- 删除组按钮（×）：浏览态禁用。 -->
            <button
              class="icon-btn danger"
              title={browseMode
                ? t.sidebar.deleteGroupBrowseHint
                : t.sidebar.deleteGroupBtn}
              onclick={() => deleteGroup(g.imgDir)}
              disabled={busy || browseMode}
            >
              <svg
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true">{@html icons.x.body}</svg
              >
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted small">{t.sidebar.emptyGroups}</p>
    {/if}
  </section>

  <!-- 底部外部链接：用 openUrl 在默认浏览器打开（教程/故障排查/FAQ/Discord）。 -->
  <div class="side-foot">
    <button
      class="link small"
      onclick={() => openUrl("https://www.bilibili.com/video/BV1ew411h759")}
      >{t.sidebar.linkTutorial}</button
    >
    <button
      class="link small"
      onclick={() =>
        openUrl(
          "https://github.com/Maradonna90/NewGAN-Manager/wiki/Troubleshooting",
        )}>{t.sidebar.linkTroubleshooting}</button
    >
    <button
      class="link small"
      onclick={() =>
        openUrl("https://github.com/Maradonna90/NewGAN-Manager/wiki/FAQ")}
      >{t.sidebar.linkFaq}</button
    >
    <button
      class="link small"
      onclick={() => openUrl("https://discord.gg/UfRpJVc")}
      >{t.sidebar.linkDiscord}</button
    >
  </div>
</aside>
