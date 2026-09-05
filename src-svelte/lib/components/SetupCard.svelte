<!-- 【新手导读】“当前任务”卡片：展示当前头像包与 RTF 路径，提供完整性检查与一键修复。
     重点演示 $effect（副作用）和 use:action（上一文件介绍过的 pinScrollEnd）。 -->
<script lang="ts">
  import { open, ask } from '@tauri-apps/plugin-dialog'; // open=文件/目录选择框；ask=确认框
  import { api } from '../api';
  import { showError } from '../dialog';
  import { pinScrollEnd } from '../actions'; // 把长路径钉到最右的 action
  import { appStore, applySnapshot } from '../stores/app.svelte';

  let missingDirs = $state<string[] | null>(null); // null = 未检查；[] = 完整；非空数组 = 缺失的目录

  const snap = $derived(appStore.snapshot);
  const busy = $derived(!!snap?.replacing); // !! 把值强转为布尔（替换中时禁用按钮）

  // 切档 / 换组后清掉上一次检查结论，不把旧组的结论显示到新组上。
  // $effect(函数)：在组件挂载后运行，并在它“读取过的响应式值”变化时重新运行。
  // 这里读了 snap?.currentGroup，所以当前组一变就会重新执行。
  let lastGroup = '';
  $effect(() => {
    const g = snap?.currentGroup ?? ''; // ?? 是空值合并：左边为 null/undefined 时取右边
    if (g !== lastGroup) { lastGroup = g; missingDirs = null; }
  });

  /** 浏览并选择 RTF 文件。open 返回选中路径（字符串）或 null（取消）。 */
  async function browseRtf() {
    const file = await open({ filters: [{ name: 'RTF', extensions: ['rtf'] }], title: '选择 RTF 名单' });
    if (typeof file === 'string') {
      // 调后端设置 RTF，并用返回的快照刷新界面；成功后清掉检查结论。
      try { await applySnapshot(await api.setCurrentRtf(file)); missingDirs = null; }
      catch (e) { await showError(e); }
    }
  }

  /** 检查完整性：调后端校验当前组缺哪些种族目录。 */
  async function checkIntegrity() {
    if (!snap?.currentGroup) return;
    missingDirs = await api.validateImageDirs(snap.currentGroup);
  }

  /** 一键创建缺失目录（先弹确认框）。 */
  async function fixMissing() {
    if (!snap?.currentGroup || !missingDirs) return;
    const ok = await ask(
      `缺少 ${missingDirs.length} 个种族目录：\n${missingDirs.join(', ')}\n\n自动创建？`,
      { title: 'Missing Directories' },
    );
    if (!ok) return; // 用户取消
    // 创建成功后把 missingDirs 置为空数组（表示已完整）。
    try { await api.createMissingDirs(snap.currentGroup, missingDirs); missingDirs = []; }
    catch (e) { await showError(e); }
  }
</script>

<!-- {#if snap}：快照未加载时不渲染整张卡片（避免读 snap.xxx 报错）。 -->
{#if snap}
  <div class="card">
    <div class="card-title">当前任务 <span class="muted small">{snap.currentProfile}</span></div>
    <!-- 头像包路径行：只读输入框 + 完整性检查按钮。use:pinScrollEnd 把长路径钉到最右。 -->
    <div class="path-row">
      <label for="sc-group">头像包：</label>
      <input id="sc-group" type="text" readonly value={snap.currentGroup ?? ''}
             placeholder="未关联头像包目录" title={snap.currentGroup ?? ''}
             use:pinScrollEnd={snap.currentGroup ?? ''} />
      <!-- disabled 在忙碌或无选中组时置 true；title 根据情况给不同提示。 -->
      <button class="btn" onclick={checkIntegrity} disabled={busy || !snap.currentGroup}
              title={snap.currentGroup ? '校验 14 个种族目录是否齐全' : '请先关联头像包目录'}>检查完整性</button>
    </div>
    <!-- RTF 名单路径行：只读输入框 + 选择文件按钮。 -->
    <div class="path-row">
      <label for="sc-rtf">RTF 名单：</label>
      <input id="sc-rtf" type="text" readonly value={snap.currentRtf || '未设置'} title={snap.currentRtf}
             use:pinScrollEnd={snap.currentRtf} />
      <button class="btn" onclick={browseRtf} disabled={busy || !snap.currentGroup}
              title={snap.currentGroup ? '选择 FM 导出的 RTF 名单' : '请先关联头像包目录'}>选择文件</button>
    </div>
    <!-- 底部提示区：无组→引导关联；已检查且完整→绿色✓；已检查且缺失→黄色警告 + 一键创建按钮。 -->
    {#if !snap.currentGroup}
      <p class="muted small">先在左侧「头像包（组）」关联一个头像包目录，或点击已有组。</p>
    {:else if missingDirs !== null}
      {#if missingDirs.length === 0}
        <p class="ok-text small">✓ 头像包目录完整（14 个种族目录齐全）</p>
      {:else}
        <p class="warn-text small">缺少 {missingDirs.length} 个种族目录：{missingDirs.join(', ')}</p>
        <button class="btn primary" onclick={fixMissing} disabled={busy}>一键创建缺失目录</button>
      {/if}
    {/if}
  </div>
{/if}
