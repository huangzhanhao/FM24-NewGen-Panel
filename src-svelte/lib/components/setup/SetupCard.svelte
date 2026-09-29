<!-- “当前任务”卡片：展示当前头像包与 RTF 路径，提供完整性检查与一键修复。 -->
<script lang="ts">
  import { open, ask } from "@tauri-apps/plugin-dialog"; // open=文件/目录选择框；ask=确认框
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text"; // 全局文案常量
  import { pinScrollEnd } from "../../ui/actions"; // 把长路径钉到最右的 action
  import { icons } from "../../ui/icons";
  import { appStore, applySnapshot, runTask } from "../../stores/app.svelte";
  import { ui } from "../../stores/ui.svelte"; // 本卡已在存档资料台：扫描结果切的是**本台**的验证视图
  import type { IntegrityResult } from "../../types";
  import RuleEditor from "./RuleEditor.svelte";

  let integrity = $state<IntegrityResult | null>(null); // null = 未检查
  let ruleEditor = $state<{ openEditor: () => void } | null>(null); // 规则编辑对话框实例

  // 展示态统一取自 store getter（核心规则单点维护）：snap 浏览态优先，busy/browseMode 全局一致。
  const snap = $derived(appStore.displaySnapshot);
  const busy = $derived(appStore.busy);
  const browseMode = $derived(appStore.browseMode);
  // 完整时按命中的标准号给出中文标注（目录总数来自标准 FolderList，用户可配置）。
  const completeLabel = $derived(
    integrity?.standard === 1
      ? t.setup.tpl1Complete
      : integrity && integrity.standard >= 2
        ? t.setup.tplComplete(integrity.standard, integrity.expectedTotal)
        : "",
  );

  // 切档 / 换组后清掉上一次检查结论，不把旧组的结论显示到新组上。
  // 该 effect 读取了 snap.currentGroup，因此当前组一变就会重新执行。
  let lastGroup = "";
  $effect(() => {
    const g = snap?.currentGroup ?? "";
    if (g !== lastGroup) {
      lastGroup = g;
      integrity = null;
    }
  });

  /** 浏览并选择 RTF 文件。open 返回选中路径（字符串）或 null（取消）。 */
  async function browseRtf() {
    const file = await open({
      filters: [{ name: "RTF", extensions: ["rtf"] }],
      title: t.setup.selectRtfDialogTitle,
    });
    if (typeof file === "string") {
      // 调后端设置 RTF，并用返回的快照刷新界面；成功后清掉检查结论。
      try {
        await applySnapshot(await api.setCurrentRtf(file));
        integrity = null;
      } catch (e) {
        await showError(e);
      }
    }
  }

  /** 检查完整性：按规则 json 的 FolderList 识别头像包归属哪个标准（可能因用户改坏 json 报错）。 */
  async function checkIntegrity() {
    if (!snap?.currentGroup) return;
    // 先取出目录到局部变量：闭包里 TS 的判空收窄会失效，且 runTask 包的正是这条命令。
    const imgDir = snap.currentGroup;
    try {
      // runTask(任务名, 命令)：命令执行期间由底部状态栏接管进度条（后端 task:progress 事件实时驱动），
      // 传任务名让状态栏在第一条事件到达前就显示正确的名字，不残留上一个任务。
      const got = await runTask(t.setup.checkIntegrityBtn, () => api.checkIntegrity(imgDir));
      // 【为什么要比对 currentGroup】检查一个 28 目录的大包要好几秒，期间用户可能已经换成另一个
      // 头像包：那份结论属于旧包，据此点「创建缺失目录」会在新包里凭空建出旧包的目录，
      // 还会把新包报成「完整」。
      if (snap?.currentGroup !== imgDir) return;
      integrity = got;
    } catch (e) {
      if (snap?.currentGroup !== imgDir) return; // 已经换包：这条错误不再打扰用户
      await showError(e);
    }
  }

  /** 打开标准规则目录（data/rules/）：编辑 standard2/standard3.json，保存后下次替换即生效。 */
  async function openRulesDir() {
    try {
      await api.openRulesDir();
    } catch (e) {
      await showError(e);
    }
  }

  /** 读取快照：解析当前展示档的存档 XML 快照并缓存（快照预览的数据源）；
   *  成功后记录已读路径、切到结果区「快照预览」并触发其重载。浏览态自洽（读操作）。 */
  async function readSnapshot() {
    if (!snap?.configXml?.exists) return;
    const xmlPath = snap.configXml.path; // 取到局部变量：闭包里 TS 的判空收窄会失效
    try {
      await runTask(t.setup.viewConfigBtn, () => api.readSnapshot(xmlPath));
      appStore.snapshotReadPath = xmlPath;
      ui.verifyView = "snapshot"; // 就地看到刚读进来的快照，不出台
      appStore.snapshotVersion++;
    } catch (e) {
      await showError(e);
    }
  }

  /** 查看位置：打开存档 XML 快照所在目录。传当前展示档的快照路径，浏览态自洽。 */
  async function openSnapshot() {
    if (!snap?.configXml?.exists) return;
    try {
      await api.openSnapshotDir(snap.configXml.path);
    } catch (e) {
      await showError(e);
    }
  }

  /** 扫描头像：建图片索引缓存（供替换复用），结果已在后端写入日志面板。 */
  async function scanImages() {
    if (!snap?.currentGroup) return;
    try {
      await runTask(t.setup.scanImagesBtn, () => api.scanImages());
    } catch (e) {
      await showError(e);
    }
  }

  /** 扫描RTF：解析当前 RTF 并填充名单预览缓存；摘要存入全局态（预览页据此显示随机人筛选），rosterVersion++ 触发名单预览刷新。成功后切到名单预览 tab。 */
  async function scanRtf() {
    if (!snap?.currentRtf) return;
    try {
      appStore.lastRtfScan = await runTask(t.setup.scanRtfBtn, () => api.scanRtf());
      appStore.rosterReady = true; // 名单缓存已填充：名单预览据此渲染工具条
      appStore.rosterVersion++;
      ui.verifyView = 'table'; // 扫描成功：存档资料台的验证视图切到「名单预览」，就地看到扫描结果
    } catch (e) {
      await showError(e);
    }
  }

  /** 一键创建缺失目录（先弹确认框）；创建成功后重新检查，拿最新结论。 */
  async function fixMissing() {
    if (!snap?.currentGroup || !integrity || integrity.missing.length === 0)
      return;
    const ok = await ask(
      t.setup.missingDirsConfirm(
        integrity.standard,
        integrity.missing.length,
        integrity.missing.join(", "),
      ),
      { title: t.setup.missingDirsTitle },
    );
    if (!ok) return; // 用户取消
    // 取到局部变量：闭包里 TS 的判空收窄会失效（下面 runTask 包的正是一条闭包）。
    const imgDir = snap.currentGroup;
    try {
      await api.createMissingDirs(imgDir, integrity.missing);
      // 复检和「检查完整性」按钮是同一条命令，同样经 runTask 让状态栏接管进度：
      // 不包的话后端照样发 task:progress，进度条会动而状态文字还停在「就绪」。
      integrity = await runTask(t.setup.checkIntegrityBtn, () => api.checkIntegrity(imgDir));
    } catch (e) {
      await showError(e);
    }
  }
</script>

<!-- {#if snap}：快照未加载时不渲染整张卡片（避免读 snap.xxx 报错）。 -->
{#if snap}
  <div class="card">
    <div class="card-title">
      {t.setup.cardTitle} <span class="muted small">{snap.currentProfile}</span>
    </div>
    <!-- 头像包路径行：只读输入框 + [检查完整性 | 扫描头像] 按钮组。use:pinScrollEnd 把长路径钉到最右。 -->
    <div class="path-row">
      <label for="sc-group">{t.setup.imgDirLabel}</label>
      <div class="input-wrap">
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true">{@html icons.folder.body}</svg
        >
        <input
          id="sc-group"
          type="text"
          readonly
          value={snap.currentGroup ?? ""}
          placeholder={t.setup.imgDirPlaceholder}
          title={snap.currentGroup ?? ""}
          use:pinScrollEnd={snap.currentGroup ?? ""}
        />
      </div>
      <div class="path-actions">
        <!-- disabled 在忙碌或无选中组时置 true；title 根据情况给不同提示。 -->
        <button
          class="btn"
          onclick={checkIntegrity}
          disabled={busy || !snap.currentGroup}
          title={snap.currentGroup
            ? t.setup.checkIntegrityHint
            : t.setup.noGroupHint}>{t.setup.checkIntegrityBtn}</button
        >
        <button
          class="btn"
          onclick={scanImages}
          disabled={busy || !snap.currentGroup}
          title={t.setup.scanImagesHint}>{t.setup.scanImagesBtn}</button
        >
      </div>
    </div>
    <!-- RTF 名单路径行：只读输入框（前置文件图标）+ [选择RTF | 扫描RTF] 按钮组。 -->
    <div class="path-row">
      <label for="sc-rtf">{t.setup.rtfLabel}</label>
      <div class="input-wrap">
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true">{@html icons.file.body}</svg
        >
        <input
          id="sc-rtf"
          type="text"
          readonly
          value={snap.currentRtf || t.setup.rtfUnsetValue}
          title={snap.currentRtf}
          use:pinScrollEnd={snap.currentRtf}
        />
      </div>
      <div class="path-actions">
        <button
          class="btn"
          onclick={browseRtf}
          disabled={busy || browseMode || !snap.currentGroup}
          title={browseMode
            ? t.setup.selectRtfBrowseHint
            : snap.currentGroup
              ? t.setup.selectRtfHint
              : t.setup.noGroupHint}>{t.setup.selectRtfBtn}</button
        >
        <button
          class="btn"
          onclick={scanRtf}
          disabled={busy || !snap.currentRtf}
          title={t.setup.scanRtfHint}>{t.setup.scanRtfBtn}</button
        >
      </div>
    </div>
    <!-- config.xml 行：当前展示档的存档 XML 快照路径（前置文件图标）+ [读取快照 | 查看位置]（左读取右位置）。
         两者都依赖快照文件已生成（执行替换后才有），未生成时一并禁用。 -->
    <div class="path-row">
      <label for="sc-xml">{t.setup.xmlLabel}</label>
      <div class="input-wrap">
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true">{@html icons.file.body}</svg
        >
        <input
          id="sc-xml"
          type="text"
          readonly
          value={snap.configXml?.path ?? ""}
          placeholder={t.setup.xmlPlaceholder}
          title={snap.configXml?.path ?? ""}
          use:pinScrollEnd={snap.configXml?.path ?? ""}
        />
      </div>
      <div class="path-actions">
        <button
          class="btn"
          onclick={readSnapshot}
          disabled={busy || !snap.configXml?.exists}
          title={t.setup.viewConfigHint}>{t.setup.viewConfigBtn}</button
        >
        <button
          class="btn"
          onclick={openSnapshot}
          disabled={busy || !snap.configXml?.exists}
          title={t.setup.viewSnapshotHint}>{t.setup.viewSnapshotBtn}</button
        >
      </div>
    </div>
    <!-- 完整性状态行：无组→引导关联；完整→绿色胶囊（旁注多余目录）；
         缺失目录→琥珀胶囊 + 详情长句 + 一键创建按钮；无法识别→琥珀胶囊 + 说明长句。 -->
    {#if !snap.currentGroup}
      <p class="muted small statusline">{t.setup.noGroupGuide}</p>
    {:else if integrity !== null}
      {#if integrity.status === "complete"}
        <div class="statusline">
          <span class="pill ok"><span class="dot"></span>{completeLabel}</span>
          {#if integrity.extra.length > 0}
            <span class="note"
              >{t.setup
                .extraDirsNote(integrity.extra.length)
                .replace(/^；/, "")}</span
            >
          {/if}
        </div>
      {:else if integrity.status === "fixable"}
        <div class="statusline">
          <span class="pill warn"
            ><span class="dot"></span>
            {t.setup.pillMissingDirs(
              integrity.missing.length,
              integrity.found,
              integrity.expectedTotal,
            )}
          </span>
          <button class="btn primary" onclick={fixMissing} disabled={busy}
            >{t.setup.createMissingBtn}</button
          >
        </div>
        <p class="muted small" style="margin:6px 0 0">
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
          <span class="pill warn"
            ><span class="dot"></span>{t.setup.pillUnknown}</span
          >
        </div>
        <p class="muted small" style="margin:6px 0 0">
          {t.setup.integrityUnknown}
        </p>
      {/if}
    {/if}
    <!-- 标准规则入口：顶部分隔线另起一行（打开查看/修改/保存 standard2|standard3.json 的对话框）。 -->
    <div class="rule-row">
      <button
        class="btn"
        onclick={() => ruleEditor?.openEditor()}
        disabled={busy}
        title={t.setup.ruleConfigHint}>{t.setup.ruleConfigBtn}</button
      >
      <span class="note">{t.setup.ruleConfigNote}</span>
    </div>
  </div>
{/if}

<RuleEditor bind:this={ruleEditor} />
