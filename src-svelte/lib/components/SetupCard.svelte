<!-- 【新手导读】“当前任务”卡片：展示当前头像包与 RTF 路径，提供完整性检查与一键修复。
     重点演示 $effect（副作用）和 use:action（上一文件介绍过的 pinScrollEnd）。 -->
<script lang="ts">
  import { open, ask } from '@tauri-apps/plugin-dialog'; // open=文件/目录选择框；ask=确认框
  import { api } from '../api';
  import { showError } from '../dialog';
  import { t } from '../text'; // 全局文案常量
  import { pinScrollEnd } from '../actions'; // 把长路径钉到最右的 action
  import { appStore, applySnapshot } from '../stores/app.svelte';
  import type { IntegrityResult } from '../types';
  import RuleEditor from './RuleEditor.svelte';

  let integrity = $state<IntegrityResult | null>(null); // null = 未检查
  let ruleEditor = $state<{ openEditor: () => void } | null>(null);   // 规则编辑对话框实例

  const snap = $derived(appStore.profilePreview ?? appStore.snapshot); // 展示层：浏览态优先
  const busy = $derived(!!snap?.replacing); // !! 把值强转为布尔（替换中时禁用按钮）
  // 浏览态：下拉选中的存档 ≠ 后端当前激活档——展示只读预览；
  // 写「后端当前档」状态的操作（选择 RTF 等）禁用，读操作（检查完整性/扫描）自洽可用。
  const browseMode = $derived(
    !!snap && appStore.selectedProfile !== (appStore.snapshot?.currentProfile ?? ''),
  );
  // 完整时按命中的标准号给出中文标注（目录总数来自标准 FolderList，用户可配置）。
  const completeLabel = $derived(
    integrity?.standard === 1 ? t.setup.tpl1Complete
    : integrity && integrity.standard >= 2
      ? t.setup.tplComplete(integrity.standard, integrity.expectedTotal)
    : '',
  );

  // 切档 / 换组后清掉上一次检查结论，不把旧组的结论显示到新组上。
  // $effect(函数)：在组件挂载后运行，并在它“读取过的响应式值”变化时重新运行。
  // 这里读了 snap?.currentGroup，所以当前组一变就会重新执行。
  let lastGroup = '';
  $effect(() => {
    const g = snap?.currentGroup ?? ''; // ?? 是空值合并：左边为 null/undefined 时取右边
    if (g !== lastGroup) { lastGroup = g; integrity = null; }
  });

  /** 浏览并选择 RTF 文件。open 返回选中路径（字符串）或 null（取消）。 */
  async function browseRtf() {
    const file = await open({ filters: [{ name: 'RTF', extensions: ['rtf'] }], title: t.setup.selectRtfDialogTitle });
    if (typeof file === 'string') {
      // 调后端设置 RTF，并用返回的快照刷新界面；成功后清掉检查结论。
      try { await applySnapshot(await api.setCurrentRtf(file)); integrity = null; }
      catch (e) { await showError(e); }
    }
  }

  /** 检查完整性：按规则 json 的 FolderList 识别头像包归属哪个标准（可能因用户改坏 json 报错）。 */
  async function checkIntegrity() {
    if (!snap?.currentGroup) return;
    try { integrity = await api.checkIntegrity(snap.currentGroup); } catch (e) { await showError(e); }
  }

  /** 打开标准规则目录（data/rules/）：编辑 standard2/standard3.json，保存后下次替换即生效。 */
  async function openRulesDir() {
    try { await api.openRulesDir(); } catch (e) { await showError(e); }
  }

  /** 查看config：打开头像包目录（config.xml 就在包根目录）。传当前展示组的目录，浏览态自洽。 */
  async function openPackConfig() {
    if (!snap?.currentGroup) return;
    try { await api.openPackConfigDir(snap.currentGroup); } catch (e) { await showError(e); }
  }

  /** 查看快照：打开存档 XML 快照所在目录。传当前展示档的快照路径，浏览态自洽。 */
  async function openSnapshot() {
    if (!snap?.configXml?.exists) return;
    try { await api.openSnapshotDir(snap.configXml.path); } catch (e) { await showError(e); }
  }

  /** 扫描头像：建图片索引缓存（供替换复用），结果已在后端写入日志面板。 */
  async function scanImages() {
    if (!snap?.currentGroup) return;
    try { await api.scanImages(); } catch (e) { await showError(e); }
  }

  /** 扫描RTF：解析当前 RTF 并填充名单预览缓存；rosterVersion++ 触发名单预览刷新。 */
  async function scanRtf() {
    if (!snap?.currentRtf) return;
    try { await api.scanRtf(); appStore.rosterVersion++; } catch (e) { await showError(e); }
  }

  /** 一键创建缺失目录（先弹确认框）；创建成功后重新检查，拿最新结论。 */
  async function fixMissing() {
    if (!snap?.currentGroup || !integrity || integrity.missing.length === 0) return;
    const ok = await ask(
      t.setup.missingDirsConfirm(integrity.standard, integrity.missing.length, integrity.missing.join(', ')),
      { title: t.setup.missingDirsTitle },
    );
    if (!ok) return; // 用户取消
    try {
      await api.createMissingDirs(snap.currentGroup, integrity.missing);
      integrity = await api.checkIntegrity(snap.currentGroup); // 复检，得到权威结论
    } catch (e) { await showError(e); }
  }
</script>

<!-- {#if snap}：快照未加载时不渲染整张卡片（避免读 snap.xxx 报错）。 -->
{#if snap}
  <div class="card">
    <div class="card-title">{t.setup.cardTitle} <span class="muted small">{snap.currentProfile}</span></div>
    <!-- 头像包路径行：只读输入框 + [检查完整性 | 扫描头像] 按钮组。use:pinScrollEnd 把长路径钉到最右。 -->
    <div class="path-row">
      <label for="sc-group">{t.setup.imgDirLabel}</label>
      <input id="sc-group" type="text" readonly value={snap.currentGroup ?? ''}
             placeholder={t.setup.imgDirPlaceholder} title={snap.currentGroup ?? ''}
             use:pinScrollEnd={snap.currentGroup ?? ''} />
      <div class="path-actions">
        <!-- disabled 在忙碌或无选中组时置 true；title 根据情况给不同提示。 -->
        <button class="btn" onclick={checkIntegrity} disabled={busy || !snap.currentGroup}
                title={snap.currentGroup ? t.setup.checkIntegrityHint : t.setup.noGroupHint}>{t.setup.checkIntegrityBtn}</button>
        <button class="btn" onclick={scanImages} disabled={busy || !snap.currentGroup}
                title={t.setup.scanImagesHint}>{t.setup.scanImagesBtn}</button>
      </div>
    </div>
    <!-- RTF 名单路径行：只读输入框 + [选择RTF | 扫描RTF] 按钮组。 -->
    <div class="path-row">
      <label for="sc-rtf">{t.setup.rtfLabel}</label>
      <input id="sc-rtf" type="text" readonly value={snap.currentRtf || t.setup.rtfUnsetValue} title={snap.currentRtf}
             use:pinScrollEnd={snap.currentRtf} />
      <div class="path-actions">
        <button class="btn" onclick={browseRtf} disabled={busy || browseMode || !snap.currentGroup}
                title={browseMode ? t.setup.selectRtfBrowseHint
                      : snap.currentGroup ? t.setup.selectRtfHint : t.setup.noGroupHint}>{t.setup.selectRtfBtn}</button>
        <button class="btn" onclick={scanRtf} disabled={busy || !snap.currentRtf}
                title={t.setup.scanRtfHint}>{t.setup.scanRtfBtn}</button>
      </div>
    </div>
    <!-- config.xml 行：当前组的映射配置文件路径 + [查看config | 查看快照]（左 config 右快照）。 -->
    <div class="path-row">
      <label for="sc-xml">{t.setup.xmlLabel}</label>
      <input id="sc-xml" type="text" readonly value={snap.configXml?.path ?? ''}
             placeholder={t.setup.xmlPlaceholder} title={snap.configXml?.path ?? ''}
             use:pinScrollEnd={snap.configXml?.path ?? ''} />
      <div class="path-actions">
        <button class="btn" onclick={openPackConfig} disabled={busy || !snap.currentGroup}
                title={snap.currentGroup ? t.setup.viewConfigHint : t.setup.noGroupHint}>{t.setup.viewConfigBtn}</button>
        <button class="btn" onclick={openSnapshot} disabled={busy || !snap.configXml?.exists}
                title={t.setup.viewSnapshotHint}>{t.setup.viewSnapshotBtn}</button>
      </div>
    </div>
    <!-- 底部提示区：无组→引导关联；已检查且完整→绿色✓（含多余目录提示）；已检查且缺失→黄色警告 + 一键创建按钮。 -->
    {#if !snap.currentGroup}
      <p class="muted small">{t.setup.noGroupGuide}</p>
    {:else if integrity !== null}
      {#if integrity.status === 'complete'}
        <p class="ok-text small">{t.setup.integrityOk(completeLabel)}{t.setup.extraDirsNote(integrity.extra.length)}</p>
      {:else if integrity.status === 'fixable'}
        <p class="warn-text small">{t.setup.integrityFixable(integrity.standard, integrity.missing.length, integrity.found, integrity.expectedTotal, integrity.missing.join(', '))}</p>
        <button class="btn primary" onclick={fixMissing} disabled={busy}>{t.setup.createMissingBtn}</button>
      {:else}
        <p class="warn-text small">{t.setup.integrityUnknown}</p>
      {/if}
    {/if}
    <!-- 标准规则入口：打开规则编辑对话框（查看/修改/保存 standard2|standard3.json）。 -->
    <div class="row">
      <button class="btn" onclick={() => ruleEditor?.openEditor()} disabled={busy}
              title={t.setup.ruleConfigHint}>{t.setup.ruleConfigBtn}</button>
      <span class="muted small">{t.setup.ruleConfigNote}</span>
    </div>
  </div>
{/if}

<RuleEditor bind:this={ruleEditor} />
