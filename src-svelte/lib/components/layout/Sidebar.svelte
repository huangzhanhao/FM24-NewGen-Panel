<!-- Sidebar · 左侧边栏：管理存档（Profile）与头像包（组），新建/切换/删除存档、关联/选择/删除组，底部外部链接与语言切换 -->
<script lang="ts">
  import { open, ask } from "@tauri-apps/plugin-dialog"; // open=选目录；ask=确认框
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text";
  import { appStore, applySnapshot, tip } from "../../stores/app.svelte";
  import {
    imageaiStore,
    queueCancel,
  } from "../../stores/imageai.svelte"; // 队列运行中的作用域切换护栏
  import { ui, setLang, toggleTheme, type Lang } from "../../stores/ui.svelte";
  import Icon from "../../ui/Icon.svelte";
  // Vite 静态资源导入：打包后是带哈希的图片 URL（app-icon.png 在项目根）
  import brandLogo from "../../../../app-icon.png";

  // 语言菜单的 details 元素：选中语言后必须手动收起（原生开合不会随选项点击关闭）
  let langMenu = $state<HTMLDetailsElement | null>(null);
  function pickLang(lang: Lang) {
    setLang(lang);
    langMenu?.removeAttribute("open");
  }

  let newProfileName = $state("");
  let creating = $state(false); // 是否展开「新建存档」行
  // bind:this 存下 <select> 的 DOM 元素：select 的 value 是单向绑定，只能从元素读用户刚选的那一项
  let profileSelect = $state<HTMLSelectElement | null>(null);

  // 展示态统一取自 store getter（核心规则单点维护）
  const snap = $derived(appStore.displaySnapshot);
  const busy = $derived(appStore.busy);
  const browseMode = $derived(appStore.browseMode);
  // selectedIsActive：下拉选中项是否就是当前激活档（激活按钮灰/蓝态的依据）
  const selectedIsActive = $derived(
    !!appStore.snapshot &&
      appStore.selectedProfile === appStore.snapshot.currentProfile,
  );
  // hasProfile：已激活且不是 'No Profile'——关联新目录（建组）按钮只在此态可用
  const hasProfile = $derived(
    selectedIsActive && appStore.selectedProfile !== "No Profile",
  );
  /** 新建存档：后端创建即激活，成功后同步选中项、清空输入、收起输入行。
   *  「立即激活」属作用域变更，故与切档同款护栏；新建本身没有别的确认框，只问这一次 */
  async function createProfile() {
    const name = newProfileName.trim();
    if (!name) return;
    if (!(await confirmScopeChange())) return;
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

  /** 下拉选择 = 仅浏览（组列表/当前任务卡随选中项刷新展示），不切换后端当前档，
   *  要点「激活」才真正切换；预览失败时把下拉选中项回退 */
  let previewToken = 0; // 预览令牌：连点下拉时先发的响应不能盖掉后选的档
  async function onSelectChange() {
    const target = profileSelect?.value;
    const prev = appStore.selectedProfile;
    if (!target || target === prev) return;
    appStore.selectedProfile = target;
    const token = ++previewToken;
    try {
      const got = await api.previewProfile(target);
      // 【为什么要令牌】两个请求乱序返回时，profilePreview 会停在档A 的组/RTF/快照路径、
      // selectedProfile 已是档B，「读取快照」便按这份 preview 去读了另一个档的档景
      if (token !== previewToken) return;
      appStore.profilePreview = got;
    } catch (e) {
      if (token !== previewToken) return; // 已被更新的预览取代：这条错也不再打扰用户
      appStore.selectedProfile = prev;
      appStore.profilePreview = null;
      await showError(e);
    }
  }

  /** 作用域变更的统一确认（唯一实现处）：切档 / 切组 / 关联头像包 / 新建档 / 删档 / 删组都走这里。
   *  合成一处是防入口漏掉队列警告（createProfile / deleteProfile / deleteGroup 都曾漏过）。
   *  队列警告（queueScopeAppend）与 warning 样式只由本函数追加，调用方只决定两件事：
   *  message：这次具体做什么，默认值即切换类口径（切档/切组/新建/关联）的正文；
   *  askWhenIdle：队列空闲时要不要也问一句，删除不可逆 → true，切换类空闲无副作用 → false。
   *  删除类因此把队列警告并进同一个框，一次操作只弹一次。
   *  确认后若队列在跑则先取消；取消失败返回 false，调用方据此中止后续动作 */
  async function confirmScopeChange(opts: {
    message?: string;
    title?: string;
    askWhenIdle?: boolean;
  } = {}): Promise<boolean> {
    const message = opts.message ?? t.sidebar.queueScopeConfirm;
    const title = opts.title ?? t.sidebar.queueScopeTitle;
    const running = imageaiStore.queueRunning;
    if (!running && !opts.askWhenIdle) return true; // 空闲的切换类操作：不打扰
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

  /** 激活下拉选中的存档：真正切换后端当前档（快照交换），浏览态结束 */
  async function activateProfile() {
    const target = appStore.selectedProfile;
    if (!target || target === (appStore.snapshot?.currentProfile ?? "")) return;
    if (!(await confirmScopeChange())) return;
    try {
      await applySnapshot(await api.switchProfile(target));
      appStore.profilePreview = null; // 激活后展示与激活态一致
    } catch (e) {
      await showError(e);
    }
  }

  /** 删除下拉选中的存档：删激活档会自动回到 No Profile，删未激活档不影响当前档。
   *  确认走 confirmScopeChange（askWhenIdle: true），一次操作只打扰一次 */
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

  /** 关联头像包目录：选一个目录，后端自动建组并设为当前组 */
  async function linkFacepack() {
    const dir = await open({
      directory: true,
      title: t.sidebar.linkDialogTitle,
    });
    if (typeof dir === "string") {
      if (!(await confirmScopeChange())) return;
      try {
        await applySnapshot(await api.selectImageDirectory(dir));
      } catch (e) {
        await showError(e);
      }
    }
  }

  /** 选中某个已有组（切为当前组） */
  async function selectGroup(imgDir: string) {
    if (!(await confirmScopeChange())) return;
    try {
      await applySnapshot(await api.setCurrentGroup(imgDir));
    } catch (e) {
      await showError(e);
    }
  }

  /** 删除组：与 deleteProfile 同款走 confirmScopeChange（askWhenIdle: true） */
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

  // basename：取路径最后一段（文件夹名），列表只显示短名
  const basename = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
</script>

<aside class="sidebar">
  <!-- 品牌区：应用图标 + 名称 + 版本胶囊；第二行右侧为深/浅色主题切换 -->
  <div class="brand-row">
    <img class="brand-logo" src={brandLogo} alt="" />
    <div class="brand-text">
      <div class="brand-name">{t.brand}</div>
      <div class="brand-sub">
        <span class="brand-ver">{t.app.brandVer}</span>
        <!-- 主题切换：深色=月亮+「深色」，浅色=太阳+「浅色」；选择记忆在 localStorage -->
        <button
          class="theme-switch"
          onclick={toggleTheme}
          use:tip={t.theme.switchTo(ui.theme === "dark" ? "light" : "dark")}
        >
          <Icon name={ui.theme === "dark" ? "moon" : "sun"} />
          <span>{ui.theme === "dark" ? t.theme.dark : t.theme.light}</span>
        </button>
      </div>
    </div>
  </div>

  <!-- 区域一：存档（Profile）管理 -->
  <section>
    <div class="side-head">
      <span>{t.sidebar.profileSection}</span>
      <button
        class="icon-btn"
        use:tip={t.sidebar.newProfileBtn}
        onclick={() => (creating = !creating)}
      >
        <Icon name="plus" />
      </button>
    </div>
    {#if creating}
      <div class="row">
        <input
          type="text"
          placeholder={t.sidebar.newProfilePlaceholder}
          bind:value={newProfileName}
          class="w-full"
        />
        <button
          class="btn"
          onclick={createProfile}
          disabled={!newProfileName.trim()}>{t.sidebar.createBtn}</button
        >
      </div>
    {/if}
    <!-- 存档下拉框：选中即「浏览」（组列表/当前任务卡随选中项刷新），不切档；
         点「激活」才真正切换后端当前档；忙碌时禁用 -->
    <select
      bind:this={profileSelect}
      value={appStore.selectedProfile}
      onchange={onSelectChange}
      disabled={busy}
      class="w-full"
    >
      {#each snap?.profiles ?? [] as p}
        <option value={p}>{p}</option>
      {/each}
    </select>
    <div class="prf-actions">
      <!-- No Profile（缺省档）：固定未激活、不可删除 -->
      {#if appStore.selectedProfile === "No Profile"}
        <button class="btn" disabled use:tip={t.sidebar.noProfileNoActivate}
          >{t.sidebar.notActivated}</button
        >
        <button
          class="btn danger-solid"
          disabled
          use:tip={t.sidebar.noProfileNoDelete}
          >{t.sidebar.noProfileNoDelete}</button
        >
      {:else if browseMode}
        <!-- 浏览未激活存档：可激活、可删除 -->
        <button
          class="btn primary"
          onclick={activateProfile}
          disabled={busy}
          use:tip={t.sidebar.activateHint(appStore.selectedProfile)}
          >{t.sidebar.activateBtn}</button
        >
        <button class="btn danger-solid" onclick={deleteProfile} disabled={busy}
          >{t.sidebar.deleteCurrentProfile}</button
        >
      {:else}
        <!-- 选中项已激活：激活按钮置灰不可点，仍可删除 -->
        <button class="btn" disabled use:tip={t.sidebar.activeAlreadyHint}
          >{t.sidebar.activated}</button
        >
        <button class="btn danger-solid" onclick={deleteProfile} disabled={busy}
          >{t.sidebar.deleteCurrentProfile}</button
        >
      {/if}
    </div>
  </section>

  <!-- 区域二：头像包（组）管理 -->
  <section>
    <div class="side-head">
      <span>{t.sidebar.groupSection}</span>
      <!-- 组操作仅对激活档可用：忙碌或浏览态时禁用 -->
      <button
        class="icon-btn"
        use:tip={browseMode ? t.sidebar.linkBrowseHint : t.sidebar.linkNewHint}
        onclick={linkFacepack}
        disabled={busy || !hasProfile}
      >
        <Icon name="plus" />
      </button>
    </div>
    {#if snap && snap.groups.length > 0}
      <ul class="group-list">
        <!-- 以目录为键；current 高亮当前组 -->
        {#each snap.groups as g (g.imgDir)}
          <li class:current={snap.currentGroup === g.imgDir}>
            <!-- 点组名切为当前组，浏览态禁用（切组写后端当前档）；title 给完整路径，g-rtf 标 RTF -->
            <button
              class="group-item"
              onclick={() => selectGroup(g.imgDir)}
              disabled={busy || browseMode}
              use:tip={browseMode ? t.sidebar.selectGroupBrowseHint : g.imgDir}
            >
              <span class="g-name">{basename(g.imgDir)}</span>
              <span class="g-rtf" class:ok={!!g.rtf}
                >{g.rtf ? t.sidebar.rtfSet : t.sidebar.rtfUnset}</span
              >
            </button>
            <button
              class="icon-btn danger"
              use:tip={browseMode
                ? t.sidebar.deleteGroupBrowseHint
                : t.sidebar.deleteGroupBtn}
              onclick={() => deleteGroup(g.imgDir)}
              disabled={busy || browseMode}
            >
              <Icon name="x" />
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted small">{t.sidebar.emptyGroups}</p>
    {/if}
  </section>

  <!-- 底部：首页跳转仓库；教程/FAQ 打开应用内文档面板（离线可查，不跳外部页面）；
       「语言」弹出三语菜单，点选即切换整个界面语言并记忆到 localStorage -->
  <div class="side-foot">
    <button
      class="link small"
      onclick={() => openUrl("https://github.com/huangzhanhao/NewGAN-Manager-ByHzH")}
      >{t.sidebar.linkHome}</button
    >
    <button class="link small" onclick={() => (ui.helpTab = "tutorial")}
      >{t.sidebar.linkTutorial}</button
    >
    <button class="link small" onclick={() => (ui.helpTab = "faq")}
      >{t.sidebar.linkFaq}</button
    >
    <details class="lang-menu" bind:this={langMenu}>
      <summary class="link small" use:tip={t.sidebar.langMenuTitle}
        >{t.lang.label}</summary
      >
      <ul>
        <li>
          <button
            class="lang-opt"
            class:current={ui.lang === "zh-CN"}
            onclick={() => pickLang("zh-CN")}>{t.lang.zhCN}</button
          >
        </li>
        <li>
          <button
            class="lang-opt"
            class:current={ui.lang === "zh-TW"}
            onclick={() => pickLang("zh-TW")}>{t.lang.zhTW}</button
          >
        </li>
        <li>
          <button
            class="lang-opt"
            class:current={ui.lang === "en"}
            onclick={() => pickLang("en")}>{t.lang.en}</button
          >
        </li>
      </ul>
    </details>
  </div>
</aside>
