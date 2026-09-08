<!-- 【新手导读】“替换执行”面板：选择模式与选项、发起/取消替换、显示进度条。
     重点演示 bind:checked（双向绑定）、{#each} 循环渲染、以及“未就绪引导”的派生计算。 -->
<script lang="ts">
  import { ask } from '@tauri-apps/plugin-dialog';
  import { api } from '../api';
  import { showError } from '../dialog';
  import { t } from '../text'; // 全局文案常量
  import { appStore, refreshSnapshot } from '../stores/app.svelte';

  // 三个模式的取值/标签/说明统一在 text.ts 的 t.replace.modes 维护；
  // value 发往后端（协议值），label 显示在模式按钮上，desc 显示在选中模式旁。
  const modeDesc = $derived(t.replace.modes.find((m) => m.value === mode)?.desc ?? '');

  // 下面四个是表单选项，用 $state 声明；通过 bind:checked 与复选框双向绑定。
  let mode = $state<'Preserve' | 'Overwrite' | 'Generate'>('Preserve');
  let allowDuplicates = $state(true);
  let filterNewgan = $state(true);
  let saveBackup = $state(true);

  const snap = $derived(appStore.snapshot); // 替换只作用于后端当前激活档（浏览态禁用）
  const busy = $derived(!!snap?.replacing);
  // 浏览态：下拉选中的存档 ≠ 当前激活档——替换会作用于激活档，为避免误操作禁用替换。
  const browseMode = $derived(
    !!snap && appStore.selectedProfile !== snap.currentProfile,
  );
  // ready：只有“已选 Profile + 已选头像包 + 已设 RTF”都满足才允许开始替换。
  const ready = $derived(
    !!snap && snap.currentProfile !== 'No Profile' && !!snap.currentGroup && !!snap.currentRtf,
  );
  // 未就绪原因（引导用户补全，对等 v1 按钮三级禁用逻辑）。
  // 把三个可能缺失的项组成数组，filter(Boolean) 去掉空字符串，剩下就是待完成项。
  const missing = $derived([
    !snap || snap.currentProfile === 'No Profile' ? t.replace.needProfile : '',
    snap && !snap.currentGroup ? t.replace.needGroup : '',
    snap && !snap.currentRtf ? t.replace.needRtf : '',
  ].filter(Boolean));

  /** 发起替换：先兜底校验目录完整性（识别三套标准），再调后端 start_replace。 */
  async function startReplace() {
    if (!snap?.currentGroup) return;
    try {
      // 兜底校验：用户可能跳过配置卡的完整性检查，这里再查一次。
      // 模板归属由后端按规则 json 的 FolderList 判定；complete（含超集）→ 静默继续。
      const res = await api.checkIntegrity(snap.currentGroup);
      if (res.status === 'fixable') {
        const ok = await ask(
          t.replace.missingDirsConfirm(res.standard, res.missing.length),
          { title: t.replace.missingDirsTitle },
        );
        if (!ok) return;
        await api.createMissingDirs(snap.currentGroup, res.missing);
      } else if (res.status === 'unknown') {
        const ok = await ask(t.replace.unknownConfirm, { title: t.replace.unknownTitle });
        if (!ok) return;
      } // complete（模板目录齐全，含多出未配置目录）→ 静默继续
      // 调后端启动替换（异步后台线程），随后刷新快照：replacing 变 true → 全局禁用。
      await api.startReplace({ mode, filterNewgan, allowDuplicates, saveBackup });
      await refreshSnapshot(); // 不可丢：替换期间取消按钮可用、全局禁用生效
    } catch (e) { await showError(e); }
  }
</script>

<div class="card">
  <div class="card-title">{t.replace.cardTitle}</div>
  <!-- 模式选择：{#each} 遍历 t.replace.modes 生成三个按钮，选中的加 active 类。
       按钮显示 label（中文），点选后把 value（协议值）存进 mode。 -->
  <div class="row">
    <span class="row-label">{t.replace.modeLabel}</span>
    {#each t.replace.modes as m}
      <button
        class="mode-chip"
        class:active={mode === m.value}
        onclick={() => (mode = m.value)}
        disabled={busy}
      >{m.label}</button>
    {/each}
    <!-- 当前选中模式的说明文字（flex-1 占据剩余宽度）。 -->
    <span class="muted small flex-1">{modeDesc}</span>
  </div>
  <!-- 选项复选框：bind:checked 实现双向绑定（勾选→变量 true；变量变→勾选状态变）。 -->
  <div class="row checks">
    <span class="row-label">{t.replace.optionsLabel}</span>
    <label><input type="checkbox" bind:checked={allowDuplicates} disabled={busy} /> {t.replace.optAllowDuplicates}</label>
    <label><input type="checkbox" bind:checked={filterNewgan} disabled={busy} /> {t.replace.optFilterNewgan}</label>
    <label><input type="checkbox" bind:checked={saveBackup} disabled={busy} /> {t.replace.optSaveBackup}</label>
  </div>
  <!-- 开始/取消按钮：未就绪或忙碌时禁用开始；非忙碌时禁用取消。 -->
  <div class="row">
    <button class="btn primary big flex-1" onclick={startReplace}
            disabled={busy || browseMode || !ready}
            title={browseMode ? t.replace.browseDisabledHint : ''}>
      {busy ? t.replace.runningBtn : t.replace.startBtn}
    </button>
    <button class="btn" onclick={() => api.cancelReplace()} disabled={!busy}>{t.replace.cancelBtn}</button>
  </div>
  {#if browseMode}
    <p class="muted small">{t.replace.browseNotice(appStore.selectedProfile)}</p>
  {/if}
  <!-- 进度条：忙碌或有进度时显示；fill 宽度直接绑定百分比。 -->
  {#if busy || appStore.progress.value > 0}
    <div class="row">
      <div class="progress-track"><div class="progress-fill" style="width:{appStore.progress.value}%"></div></div>
      <span class="muted" style="min-width:180px">{appStore.progress.status}</span>
    </div>
  {/if}
  <!-- 未就绪时把待完成项列出来引导用户。 -->
  {#if !ready && missing.length > 0}
    <p class="muted small">{t.replace.todoPrefix}{missing.join(' · ')}</p>
  {/if}
</div>
