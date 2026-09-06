<!-- 【新手导读】“当前任务”卡片：展示当前头像包与 RTF 路径，提供完整性检查与一键修复。
     重点演示 $effect（副作用）和 use:action（上一文件介绍过的 pinScrollEnd）。 -->
<script lang="ts">
  import { open, ask } from '@tauri-apps/plugin-dialog'; // open=文件/目录选择框；ask=确认框
  import { api } from '../api';
  import { showError } from '../dialog';
  import { pinScrollEnd } from '../actions'; // 把长路径钉到最右的 action
  import { appStore, applySnapshot } from '../stores/app.svelte';
  import type { IntegrityResult } from '../types';
  import TextViewDialog from './TextViewDialog.svelte';
  import RuleEditor from './RuleEditor.svelte';

  let integrity = $state<IntegrityResult | null>(null); // null = 未检查
  let xmlView = $state<{ title: string; text: string } | null>(null); // config.xml 查看对话框
  let ruleEditor = $state<{ openEditor: () => void } | null>(null);   // 规则编辑对话框实例

  const snap = $derived(appStore.snapshot);
  const busy = $derived(!!snap?.replacing); // !! 把值强转为布尔（替换中时禁用按钮）
  // 完整时按命中的模板号给出中文标注（目录总数来自模板 FolderList，用户可配置）。
  const completeLabel = $derived(
    integrity?.standard === 1 ? '模板一：图片直铺，无种族子目录'
    : integrity && integrity.standard >= 2
      ? `模板${integrity.standard}：${integrity.expectedTotal} 个配置目录齐全`
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
    const file = await open({ filters: [{ name: 'RTF', extensions: ['rtf'] }], title: '选择 RTF 名单' });
    if (typeof file === 'string') {
      // 调后端设置 RTF，并用返回的快照刷新界面；成功后清掉检查结论。
      try { await applySnapshot(await api.setCurrentRtf(file)); integrity = null; }
      catch (e) { await showError(e); }
    }
  }

  /** 检查完整性：按规则 json 的 FolderList 识别头像包归属哪个模板（可能因用户改坏 json 报错）。 */
  async function checkIntegrity() {
    if (!snap?.currentGroup) return;
    try { integrity = await api.checkIntegrity(snap.currentGroup); } catch (e) { await showError(e); }
  }

  /** 打开模板规则目录（data/rules/）：编辑 template2/template3.json，保存后下次替换即生效。 */
  async function openRulesDir() {
    try { await api.openRulesDir(); } catch (e) { await showError(e); }
  }

  /** 查看 config.xml 内容（对话框只读展示）。 */
  async function viewConfigXml() {
    if (!snap?.configXml?.exists) return;
    try {
      xmlView = { title: `config.xml（${snap.configXml.path}）`, text: await api.readConfigXml() };
    } catch (e) { await showError(e); }
  }

  /** 在资源管理器中定位 config.xml。 */
  async function revealConfigXml() {
    try { await api.revealConfigXml(); } catch (e) { await showError(e); }
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
      `模板${integrity.standard} 缺少 ${integrity.missing.length} 个目录：\n${integrity.missing.join(', ')}\n\n自动创建？`,
      { title: 'Missing Directories' },
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
    <div class="card-title">当前任务 <span class="muted small">{snap.currentProfile}</span></div>
    <!-- 头像包路径行：只读输入框 + [检查完整性 | 扫描头像] 按钮组。use:pinScrollEnd 把长路径钉到最右。 -->
    <div class="path-row">
      <label for="sc-group">头像包：</label>
      <input id="sc-group" type="text" readonly value={snap.currentGroup ?? ''}
             placeholder="未关联头像包目录" title={snap.currentGroup ?? ''}
             use:pinScrollEnd={snap.currentGroup ?? ''} />
      <div class="path-actions">
        <!-- disabled 在忙碌或无选中组时置 true；title 根据情况给不同提示。 -->
        <button class="btn" onclick={checkIntegrity} disabled={busy || !snap.currentGroup}
                title={snap.currentGroup ? '按规则 json 的 FolderList 识别头像包归属哪个模板' : '请先关联头像包目录'}>检查完整性</button>
        <button class="btn" onclick={scanImages} disabled={busy || !snap.currentGroup}
                title="扫描头像包并缓存图片索引（结果写入日志）">扫描头像</button>
      </div>
    </div>
    <!-- RTF 名单路径行：只读输入框 + [选择RTF | 扫描RTF] 按钮组。 -->
    <div class="path-row">
      <label for="sc-rtf">RTF 名单：</label>
      <input id="sc-rtf" type="text" readonly value={snap.currentRtf || '未设置'} title={snap.currentRtf}
             use:pinScrollEnd={snap.currentRtf} />
      <div class="path-actions">
        <button class="btn" onclick={browseRtf} disabled={busy || !snap.currentGroup}
                title={snap.currentGroup ? '选择 FM 导出的 RTF 名单' : '请先关联头像包目录'}>选择RTF</button>
        <button class="btn" onclick={scanRtf} disabled={busy || !snap.currentRtf}
                title="解析 RTF 并填充名单预览（结果写入日志）">扫描RTF</button>
      </div>
    </div>
    <!-- config.xml 行：当前组的映射配置文件路径 + [查看XML | 所在位置]。 -->
    <div class="path-row">
      <label for="sc-xml">config.xml：</label>
      <input id="sc-xml" type="text" readonly value={snap.configXml?.path ?? ''}
             placeholder="未关联头像包（执行替换后生成）" title={snap.configXml?.path ?? ''}
             use:pinScrollEnd={snap.configXml?.path ?? ''} />
      <div class="path-actions">
        <button class="btn" onclick={viewConfigXml} disabled={busy || !snap.configXml?.exists}
                title={snap.configXml?.exists ? '查看 config.xml 内容' : 'config.xml 尚未生成（请先执行一次替换）'}>查看XML</button>
        <button class="btn" onclick={revealConfigXml} disabled={busy || !snap.configXml?.exists}
                title={snap.configXml?.exists ? '在资源管理器中定位 config.xml' : 'config.xml 尚未生成（请先执行一次替换）'}>所在位置</button>
      </div>
    </div>
    <!-- 底部提示区：无组→引导关联；已检查且完整→绿色✓（含多余目录提示）；已检查且缺失→黄色警告 + 一键创建按钮。 -->
    {#if !snap.currentGroup}
      <p class="muted small">先在左侧「头像包（组）」关联一个头像包目录，或点击已有组。</p>
    {:else if integrity !== null}
      {#if integrity.status === 'complete'}
        <p class="ok-text small">✓ 头像包结构完整（{completeLabel}）{integrity.extra.length > 0 ? `；另含 ${integrity.extra.length} 个未配置目录（不参与抽图）` : ''}</p>
      {:else if integrity.status === 'fixable'}
        <p class="warn-text small">模板{integrity.standard}：缺少 {integrity.missing.length} 个目录（已有 {integrity.found}/{integrity.expectedTotal}）：{integrity.missing.join(', ')}</p>
        <button class="btn primary" onclick={fixMissing} disabled={busy}>一键创建缺失目录</button>
      {:else}
        <p class="warn-text small">⚠ 无法识别头像包结构：子目录与任何模板的 FolderList 清单都不构成包含关系，替换时将把包内全部图片随机分配；也可在「规则配置」中调整模板目录清单。</p>
      {/if}
    {/if}
    <!-- 模板规则入口：打开规则编辑对话框（查看/修改/保存 template2|template3.json）。 -->
    <div class="row">
      <button class="btn" onclick={() => ruleEditor?.openEditor()} disabled={busy}
              title="直接查看、修改并保存模板规则文件（国籍→目录映射、FolderList 等），保存时自动校验 JSON 格式">规则配置</button>
      <span class="muted small">抽图规则来自规则 json（可编辑），保存后下次替换/检查完整性即生效</span>
    </div>
  </div>
{/if}

{#if xmlView}
  <TextViewDialog title={xmlView.title} text={xmlView.text} onclose={() => (xmlView = null)} />
{/if}
<RuleEditor bind:this={ruleEditor} />
