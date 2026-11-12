<!-- SetupCard · “当前任务”卡片：展示当前头像包与 RTF 路径，提供完整性检查与一键修复 -->
<script lang="ts">
  import { open, ask } from "@tauri-apps/plugin-dialog"; // open=文件/目录选择框；ask=确认框
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text"; // 全局文案常量
  import { pinScrollEnd } from "../../ui/actions"; // 把长路径钉到最右的 action
  import Icon from "../../ui/Icon.svelte";
  import { appStore, applySnapshot, markRosterFilled, markSnapshotRead, runTask, tip } from "../../stores/app.svelte";
  import type { IntegrityResult } from "../../types";
  import RuleEditor from "./RuleEditor.svelte";
  import NationalityEditor from "./NationalityEditor.svelte";

  let integrity = $state<IntegrityResult | null>(null); // null = 未检查
  let ruleEditor = $state<{ openEditor: () => void } | null>(null); // 规则编辑对话框实例
  let nationalityEditor = $state<{ openEditor: () => void } | null>(null); // 国籍对照表对话框实例

  // 展示态统一取自 store getter：核心规则单点维护，busy / browseMode 全局一致
  const snap = $derived(appStore.displaySnapshot);
  const busy = $derived(appStore.busy);
  const browseMode = $derived(appStore.browseMode);
  // 完整时按命中的标准号给出中文标注（目录总数来自标准的 FolderList，用户可配置）
  const completeLabel = $derived(
    integrity?.standard === 1
      ? t.setup.tpl1Complete
      : integrity && integrity.standard >= 2
        ? t.setup.tplComplete(integrity.standard, integrity.expectedTotal)
        : "",
  );

  // 当前组一变就清掉上一次检查结论：effect 只读 snap.currentGroup，
  // 切档后组名相同则结论保留（组名是这里能看到的唯一变更信号）
  let lastGroup = "";
  $effect(() => {
    const g = snap?.currentGroup ?? "";
    if (g !== lastGroup) {
      lastGroup = g;
      integrity = null;
    }
  });

  /** 浏览并选择 RTF：open 返回选中路径或 null（取消） */
  async function browseRtf() {
    const file = await open({
      filters: [{ name: "RTF", extensions: ["rtf"] }],
      title: t.setup.selectRtfDialogTitle,
    });
    if (typeof file === "string") {
      try {
        await applySnapshot(await api.setCurrentRtf(file));
        integrity = null;
      } catch (e) {
        await showError(e);
      }
    }
  }

  /** 浏览并选择 .fm 存档：open 返回选中路径或 null（取消） */
  async function browseFm() {
    const file = await open({
      filters: [{ name: "FM", extensions: ["fm"] }],
      title: t.setup.selectFmDialogTitle,
    });
    if (typeof file === "string") {
      try {
        await applySnapshot(await api.setCurrentFm(file));
        integrity = null;
      } catch (e) {
        await showError(e);
      }
    }
  }

  /** 检查完整性：按规则 json 的 FolderList 识别头像包归属哪个标准（用户改坏 json 会报错） */
  async function checkIntegrity() {
    if (!snap?.currentGroup) return;
    // 先取出目录到局部变量：闭包里 TS 的判空收窄会失效，且 runTask 包的正是这条命令
    const imgDir = snap.currentGroup;
    try {
      // runTask(任务名, 命令)：命令执行期间由底部状态栏接管进度条（后端 task:progress 事件实时驱动），
      // 传任务名让状态栏在第一条事件到达前就显示正确的名字，不残留上一个任务
      const got = await runTask(t.setup.checkIntegrityBtn, () => api.checkIntegrity(imgDir));
      // 【为什么要比对 currentGroup】检查一个 28 目录的大包要好几秒，期间用户可能已经换成另一个
      // 头像包：那份结论属于旧包，据此点「创建缺失目录」会在新包里凭空建出旧包的目录，
      // 还会把新包报成「完整」
      if (snap?.currentGroup !== imgDir) return;
      integrity = got;
    } catch (e) {
      if (snap?.currentGroup !== imgDir) return; // 已经换包：这条错误不再打扰用户
      await showError(e);
    }
  }

  /** 打开标准规则目录（data/rules/）：编辑 standard2/standard3.json，保存后下次替换即生效 */
  async function openRulesDir() {
    try {
      await api.openRulesDir();
    } catch (e) {
      await showError(e);
    }
  }

  /** 读取快照：解析当前展示档的存档 XML 快照并缓存（快照预览的数据源），读操作、浏览态自洽 */
  async function readSnapshot() {
    if (!snap?.configXml?.exists) return;
    const xmlPath = snap.configXml.path; // 取到局部变量：闭包里 TS 的判空收窄会失效
    try {
      await runTask(t.setup.viewConfigBtn, () => api.readSnapshot(xmlPath));
      markSnapshotRead(xmlPath); // 记已读路径 + 触发快照预览重载（标记单点见 store，与替换收尾共用）
      appStore.rtfPreview = "snapshot"; // 切到快照 Tab（两工作台共享：替换台也同步看到）
    } catch (e) {
      await showError(e);
    }
  }

  /** 查看位置：打开当前展示档的存档 XML 快照所在目录 */
  async function openSnapshot() {
    if (!snap?.configXml?.exists) return;
    try {
      await api.openSnapshotDir(snap.configXml.path);
    } catch (e) {
      await showError(e);
    }
  }

  /** 扫描头像：建图片索引缓存（供替换复用），结果已在后端写入日志面板 */
  async function scanImages() {
    if (!snap?.currentGroup) return;
    try {
      await runTask(t.setup.scanImagesBtn, () => api.scanImages());
    } catch (e) {
      await showError(e);
    }
  }

  /** 扫描RTF：解析当前 RTF 填充名单预览缓存，摘要存入全局态供「随机人」筛选 */
  async function scanRtf() {
    if (!snap?.currentRtf) return;
    try {
      const scan = await runTask(t.setup.scanRtfBtn, () => api.scanRtf());
      markRosterFilled(scan); // 名单缓存已填充：rosterVersion 触发名单预览重载（标记单点见 store，与替换收尾共用）
      appStore.rtfPreview = "table"; // 切到名单 Tab（两工作台共享：替换台也直接看到这份名单）
    } catch (e) {
      await showError(e);
    }
  }

  /** 扫描.fm：解析当前 .fm 存档填充名单预览缓存，摘要存入全局态供名单预览与替换共用 */
  async function scanFm() {
    if (!snap?.currentFm) return;
    try {
      const scan = await runTask(t.setup.scanFmBtn, () => api.scanFm());
      markRosterFilled(scan); // 名单缓存已填充：rosterVersion 触发名单预览重载（标记单点见 store，与扫描RTF/替换收尾共用）
      appStore.rtfPreview = "table"; // 切到名单 Tab（两工作台共享：替换台也直接看到这份名单）
    } catch (e) {
      await showError(e);
    }
  }

  /** 一键创建缺失目录（先弹确认），创建成功后重新检查拿最新结论 */
  async function fixMissing() {
    if (!snap?.currentGroup || !integrity || integrity.missing.length === 0) return;
    const ok = await ask(
      t.setup.missingDirsConfirm(integrity.standard, integrity.missing.length, integrity.missing.join(", ")),
      { title: t.setup.missingDirsTitle },
    );
    if (!ok) return;
    // 取到局部变量：闭包里 TS 的判空收窄会失效
    const imgDir = snap.currentGroup;
    try {
      await api.createMissingDirs(imgDir, integrity.missing);
      // 复检和「检查完整性」按钮是同一条命令，同样经 runTask 让状态栏接管进度：
      // 不包的话后端照样发 task:progress，进度条会动而状态文字停在「就绪」
      integrity = await runTask(t.setup.checkIntegrityBtn, () => api.checkIntegrity(imgDir));
    } catch (e) {
      await showError(e);
    }
  }
</script>

<!-- {#if snap}：快照未加载时不渲染整张卡片，避免读 snap.xxx 报错 -->
{#if snap}
  <div class="card">
    <div class="card-title">
      {t.setup.cardTitle} <span class="muted small">{snap.currentProfile}</span>
    </div>
    <!-- 头像包路径行：只读输入框 + [检查完整性 | 扫描头像] 按钮组；use:pinScrollEnd 把长路径钉到最右 -->
    <div class="path-row">
      <label for="sc-group">{t.setup.imgDirLabel}</label>
      <div class="input-wrap">
        <Icon name="folder" />
        <input
          id="sc-group"
          type="text"
          readonly
          value={snap.currentGroup ?? ""}
          placeholder={t.setup.imgDirPlaceholder}
          use:tip={snap.currentGroup ?? ""}
          use:pinScrollEnd={snap.currentGroup ?? ""}
        />
      </div>
      <div class="path-actions">
        <button
          class="btn"
          onclick={checkIntegrity}
          disabled={busy || !snap.currentGroup}
          use:tip={snap.currentGroup ? t.setup.checkIntegrityHint : t.setup.noGroupHint}
          >{t.setup.checkIntegrityBtn}</button
        >
        <button class="btn" onclick={scanImages} disabled={busy || !snap.currentGroup} use:tip={t.setup.scanImagesHint}
          >{t.setup.scanImagesBtn}</button
        >
      </div>
    </div>
    <!-- RTF 名单路径行：只读输入框（前置文件图标）+ [选择RTF | 扫描RTF] 按钮组 -->
    <div class="path-row">
      <label for="sc-rtf">{t.setup.rtfLabel}</label>
      <div class="input-wrap">
        <Icon name="file" />
        <input
          id="sc-rtf"
          type="text"
          readonly
          value={snap.currentRtf || t.setup.rtfUnsetValue}
          use:tip={snap.currentRtf}
          use:pinScrollEnd={snap.currentRtf}
        />
      </div>
      <div class="path-actions">
        <button
          class="btn"
          onclick={browseRtf}
          disabled={busy || browseMode || !snap.currentGroup}
          use:tip={browseMode
            ? t.setup.selectRtfBrowseHint
            : snap.currentGroup
              ? t.setup.selectRtfHint
              : t.setup.noGroupHint}>{t.setup.selectRtfBtn}</button
        >
        <button class="btn" onclick={scanRtf} disabled={busy || !snap.currentRtf} use:tip={t.setup.scanRtfHint}
          >{t.setup.scanRtfBtn}</button
        >
      </div>
    </div>
    <!-- FM 存档路径行（与 RTF 行并立的名单来源）：只读输入框（前置文件图标）+ [选择.fm | 扫描.fm] 按钮组 -->
    <div class="path-row">
      <label for="sc-fm">{t.setup.fmLabel}</label>
      <div class="input-wrap">
        <Icon name="file" />
        <input
          id="sc-fm"
          type="text"
          readonly
          value={snap.currentFm || t.setup.fmUnsetValue}
          use:tip={snap.currentFm}
          use:pinScrollEnd={snap.currentFm}
        />
      </div>
      <div class="path-actions">
        <button
          class="btn"
          onclick={browseFm}
          disabled={busy || browseMode || !snap.currentGroup}
          use:tip={snap.currentGroup ? t.setup.selectFmHint : t.setup.noGroupHint}>{t.setup.selectFmBtn}</button
        >
        <button class="btn" onclick={scanFm} disabled={busy || !snap.currentFm} use:tip={t.setup.scanFmHint}
          >{t.setup.scanFmBtn}</button
        >
      </div>
    </div>
    <!-- FM 状态行：已选路径时可观测扫描状态（名单缓存是否已填充由 store 的 rosterReady 单点判定） -->
    {#if snap.currentFm}
      <div class="statusline">
        {#if appStore.rosterReady && appStore.lastRtfScan}
          <span class="pill ok"
            ><span class="dot"></span>{t.setup.fmScannedTag}（{t.table.recordCount(appStore.lastRtfScan.total)}）</span
          >
        {:else}
          <span class="pill warn"><span class="dot"></span>{t.setup.fmUnscannedTag}</span>
        {/if}
        <span class="note">{t.setup.fmRosterHelp}</span>
      </div>
    {/if}
    <!-- config.xml 行：当前展示档的存档 XML 快照路径（前置文件图标）+ [读取快照 | 查看位置]（左读取右位置）；
         两者都依赖快照文件已生成（执行替换后才有），未生成时一并禁用 -->
    <div class="path-row">
      <label for="sc-xml">{t.setup.xmlLabel}</label>
      <div class="input-wrap">
        <Icon name="file" />
        <input
          id="sc-xml"
          type="text"
          readonly
          value={snap.configXml?.path ?? ""}
          placeholder={t.setup.xmlPlaceholder}
          use:tip={snap.configXml?.path ?? ""}
          use:pinScrollEnd={snap.configXml?.path ?? ""}
        />
      </div>
      <div class="path-actions">
        <button
          class="btn"
          onclick={readSnapshot}
          disabled={busy || !snap.configXml?.exists}
          use:tip={t.setup.viewConfigHint}>{t.setup.viewConfigBtn}</button
        >
        <button
          class="btn"
          onclick={openSnapshot}
          disabled={busy || !snap.configXml?.exists}
          use:tip={t.setup.viewSnapshotHint}>{t.setup.viewSnapshotBtn}</button
        >
      </div>
    </div>
    <!-- 完整性状态行：无组→引导关联；完整→绿色胶囊（旁注多余目录）；
         缺失目录→琥珀胶囊 + 详情长句 + 一键创建按钮；无法识别→琥珀胶囊 + 说明长句 -->
    {#if !snap.currentGroup}
      <p class="muted small statusline">{t.setup.noGroupGuide}</p>
    {:else if integrity !== null}
      {#if integrity.status === "complete"}
        <div class="statusline">
          <span class="pill ok"><span class="dot"></span>{completeLabel}</span>
          {#if integrity.extra.length > 0}
            <span class="note">{t.setup.extraDirsNote(integrity.extra.length)}</span>
          {/if}
        </div>
      {:else if integrity.status === "fixable"}
        <div class="statusline">
          <span class="pill warn"
            ><span class="dot"></span>
            {t.setup.pillMissingDirs(integrity.missing.length, integrity.found, integrity.expectedTotal)}
          </span>
          <button class="btn primary" onclick={fixMissing} disabled={busy}>{t.setup.createMissingBtn}</button>
        </div>
        <p class="muted small sub-note">
          {t.setup.integrityFixable(
            integrity.standard,
            integrity.missing.length,
            integrity.found,
            integrity.expectedTotal,
            integrity.missing.join(", "),
          )}
        </p>
      {:else}
        <div class="statusline">
          <span class="pill warn"><span class="dot"></span>{t.setup.pillUnknown}</span>
        </div>
        <p class="muted small sub-note">
          {t.setup.integrityUnknown}
        </p>
      {/if}
    {/if}
    <!-- 可编辑数据入口：标准规则（RuleEditor）与国籍对照表（NationalityEditor）同居一行，
         两者都是 json 配置，只是打开不同的编辑对话框 -->
    <div class="rule-row">
      <div class="rule-actions">
        <button class="btn" onclick={() => ruleEditor?.openEditor()} disabled={busy} use:tip={t.setup.ruleConfigHint}
          >{t.setup.ruleConfigBtn}</button
        >
        <button
          class="btn"
          onclick={() => nationalityEditor?.openEditor()}
          disabled={busy}
          use:tip={t.setup.nationalityConfigHint}>{t.setup.nationalityConfigBtn}</button
        >
      </div>
    </div>
  </div>
{/if}

<RuleEditor bind:this={ruleEditor} />
<NationalityEditor bind:this={nationalityEditor} />

<style>
  /* 完整性详情长句：贴在状态胶囊行下方的弱化说明（与 .statusline 的行距同档） */
  .sub-note {
    margin: var(--sp-2) 0 0;
  }
  /* 规则配置 / 国籍对照表两个按钮同排；间距比 .rule-row 的通用 gap 更紧凑 */
  .rule-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
</style>
