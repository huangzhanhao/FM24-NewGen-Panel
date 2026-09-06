<!-- 【新手导读】“替换执行”面板：选择模式与选项、发起/取消替换、显示进度条。
     重点演示 bind:checked（双向绑定）、{#each} 循环渲染、以及“未就绪引导”的派生计算。 -->
<script lang="ts">
  import { ask } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { showError } from '../dialog';
  import { appStore, refreshSnapshot } from '../stores/app.svelte';

  // 三个模式的中文说明（用于在选中模式旁显示提示）。Record<string,string> 是“字符串→字符串”字典类型。
  const MODE_INFO: Record<string, string> = {
    Preserve: '保留已换过的球员，只为新球员补图（推荐日常增量）',
    Overwrite: '名单内球员全部重新抽图，名单外保留',
    Generate: '无视现有 config.xml，从零生成全部映射',
  };

  // 下面四个是表单选项，用 $state 声明；通过 bind:checked 与复选框双向绑定。
  let mode = $state<'Preserve' | 'Overwrite' | 'Generate'>('Preserve');
  let allowDuplicates = $state(true);
  let filterNewgan = $state(true);
  let saveBackup = $state(true);

  const snap = $derived(appStore.snapshot);
  const busy = $derived(!!snap?.replacing);
  // ready：只有“已选 Profile + 已选头像包 + 已设 RTF”都满足才允许开始替换。
  const ready = $derived(
    !!snap && snap.currentProfile !== 'No Profile' && !!snap.currentGroup && !!snap.currentRtf,
  );
  // 未就绪原因（引导用户补全，对等 v1 按钮三级禁用逻辑）。
  // 把三个可能缺失的项组成数组，filter(Boolean) 去掉空字符串，剩下就是待完成项。
  const missing = $derived([
    !snap || snap.currentProfile === 'No Profile' ? '选择 Profile' : '',
    snap && !snap.currentGroup ? '选择头像包' : '',
    snap && !snap.currentRtf ? '设置 RTF 名单' : '',
  ].filter(Boolean));

  /** 发起替换：先兜底校验目录完整性（识别三套标准），再调后端 start_replace。 */
  async function startReplace() {
    if (!snap?.currentGroup) return;
    try {
      // 兜底校验：用户可能跳过配置卡的完整性检查，这里再查一次。
      // 标准一（平铺）complete → 静默继续，不再被旧口径误建 14 个空目录。
      const res = await api.checkIntegrity(snap.currentGroup);
      if (res.standard === 3) {
        await showError('标准三（65 目录）替换暂未实现，当前仅支持标准一/标准二');
        return;
      }
      if (res.status === 'fixable') {
        const ok = await ask(
          `标准${res.standard === 3 ? '三(65)' : '二(14)'} 缺少 ${res.missing.length} 个目录，自动创建并继续？`,
          { title: 'Missing Directories' },
        );
        if (!ok) return;
        await api.createMissingDirs(snap.currentGroup, res.missing);
      } else if (res.status === 'unknown') {
        const ok = await ask('头像包结构无法识别，替换时将把包内所有图片（含子/孙目录）随机分配，仍要继续？', { title: 'Unrecognized Layout' });
        if (!ok) return;
      } // complete（标准一/二齐全）→ 静默继续
      // 调后端启动替换（异步后台线程），随后刷新快照：replacing 变 true → 全局禁用。
      await api.startReplace({ mode, filterNewgan, allowDuplicates, saveBackup });
      await refreshSnapshot(); // 不可丢：替换期间取消按钮可用、全局禁用生效
    } catch (e) { await showError(e); }
  }
</script>

<div class="card">
  <div class="card-title">替换执行</div>
  <!-- 模式选择：{#each} 遍历 MODE_INFO 的键生成三个按钮，选中的加 active 类。 -->
  <div class="row">
    <span class="row-label">模式：</span>
    {#each Object.keys(MODE_INFO) as m}
      <button
        class="mode-chip"
        class:active={mode === m}
        onclick={() => (mode = m as typeof mode)}
        disabled={busy}
      >{m}</button>
    {/each}
    <!-- 当前选中模式的说明文字（flex-1 占据剩余宽度）。 -->
    <span class="muted small flex-1">{MODE_INFO[mode]}</span>
  </div>
  <!-- 选项复选框：bind:checked 实现双向绑定（勾选→变量 true；变量变→勾选状态变）。 -->
  <div class="row checks">
    <span class="row-label">选项设置：</span>
    <label><input type="checkbox" bind:checked={allowDuplicates} disabled={busy} /> 允许复用图片</label>
    <label><input type="checkbox" bind:checked={filterNewgan} disabled={busy} /> 仅随机人</label>
    <label><input type="checkbox" bind:checked={saveBackup} disabled={busy} /> 写入前备份</label>
  </div>
  <!-- 开始/取消按钮：未就绪或忙碌时禁用开始；非忙碌时禁用取消。 -->
  <div class="row">
    <button class="btn primary big flex-1" onclick={startReplace} disabled={busy || !ready}>
      {busy ? '替换中…' : 'Replace Faces'}
    </button>
    <button class="btn" onclick={() => api.cancelReplace()} disabled={!busy}>取消</button>
  </div>
  <!-- 进度条：忙碌或有进度时显示；fill 宽度直接绑定百分比。 -->
  {#if busy || appStore.progress.value > 0}
    <div class="row">
      <div class="progress-track"><div class="progress-fill" style="width:{appStore.progress.value}%"></div></div>
      <span class="muted" style="min-width:180px">{appStore.progress.status}</span>
    </div>
  {/if}
  <!-- 未就绪时把待完成项列出来引导用户。 -->
  {#if !ready && missing.length > 0}
    <p class="muted small">还需完成：{missing.join(' · ')}</p>
  {/if}
</div>
