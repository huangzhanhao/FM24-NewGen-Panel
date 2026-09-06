<!-- 【新手导读】“当前任务”卡片：展示当前头像包与 RTF 路径，提供完整性检查与一键修复。
     重点演示 $effect（副作用）和 use:action（上一文件介绍过的 pinScrollEnd）。 -->
<script lang="ts">
  import { open, ask } from '@tauri-apps/plugin-dialog'; // open=文件/目录选择框；ask=确认框
  import { api } from '../api';
  import { showError } from '../dialog';
  import { pinScrollEnd } from '../actions'; // 把长路径钉到最右的 action
  import { appStore, applySnapshot } from '../stores/app.svelte';
  import type { IntegrityResult } from '../types';

  let integrity = $state<IntegrityResult | null>(null); // null = 未检查

  const snap = $derived(appStore.snapshot);
  const busy = $derived(!!snap?.replacing); // !! 把值强转为布尔（替换中时禁用按钮）
  // 完整时按命中的标准给出中文标注（标准一/二/三）。
  const completeLabel = $derived(
    integrity?.standard === 1 ? '标准一：图片直铺，无种族子目录'
    : integrity?.standard === 2 ? '标准二：14 个种族目录齐全'
    : integrity?.standard === 3 ? '标准三：65 个种族目录齐全'
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

  /** 检查完整性：让后端识别头像包属于三套标准中的哪一套，并返回缺失目录。 */
  async function checkIntegrity() {
    if (!snap?.currentGroup) return;
    integrity = await api.checkIntegrity(snap.currentGroup);
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
    const stdName = integrity.standard === 3 ? '标准三(65)' : '标准二(14)';
    const ok = await ask(
      `${stdName} 缺少 ${integrity.missing.length} 个目录：\n${integrity.missing.join(', ')}\n\n自动创建？`,
      { title: 'Missing Directories' },
    );
    if (!ok) return; // 用户取消
    try {
      await api.createMissingDirs(snap.currentGroup, integrity.missing);
      integrity = await api.checkIntegrity(snap.currentGroup); // 复校，得到权威结论
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
                title={snap.currentGroup ? '识别并校验三套标准：图片直铺 / 14 种族 / 65 细分' : '请先关联头像包目录'}>检查完整性</button>
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
    <!-- 底部提示区：无组→引导关联；已检查且完整→绿色✓；已检查且缺失→黄色警告 + 一键创建按钮。 -->
    {#if !snap.currentGroup}
      <p class="muted small">先在左侧「头像包（组）」关联一个头像包目录，或点击已有组。</p>
    {:else if integrity !== null}
      {#if integrity.status === 'complete'}
        <p class="ok-text small">✓ 头像包结构完整（{completeLabel}）</p>
      {:else if integrity.status === 'fixable'}
        <p class="warn-text small">标准{integrity.standard === 3 ? '三(65)' : '二(14)'}：缺少 {integrity.missing.length} 个目录（已有 {integrity.found}/{integrity.expectedTotal}）：{integrity.missing.join(', ')}</p>
        <button class="btn primary" onclick={fixMissing} disabled={busy}>一键创建缺失目录</button>
      {:else}
        <p class="warn-text small">⚠ 无法识别头像包结构：子目录既不属于标准二(14 种族)，也不属于标准三(65 细分)，请检查目录命名后重试。</p>
      {/if}
    {/if}
  </div>
{/if}
