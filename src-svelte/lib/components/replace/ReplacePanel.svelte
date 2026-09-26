<!-- “替换执行”面板：选择模式与选项、发起/取消替换、显示进度条。 -->
<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api } from "../../services/api";
  import { showError } from "../../ui/dialog";
  import { t } from "../../text"; // 全局文案常量
  import { appStore, refreshSnapshot, runTask } from "../../stores/app.svelte";
  import { icons } from "../../ui/icons";

  // 三个模式的取值/标签/说明统一在 text.ts 的 t.replace.modes 维护；
  // value 发往后端（协议值），label 显示在模式按钮上，desc 显示在选中模式旁。
  const modeDesc = $derived(
    t.replace.modes.find((m) => m.value === mode)?.desc ?? "",
  );

  // 三个表单选项（与复选框双向绑定）；mode 的取值是发往后端的协议值。
  // saveBackup 用 appStore 共享：单人微调页读同一个开关，两条路径行为一致。
  let mode = $state<"Preserve" | "Overwrite" | "Generate">("Preserve");
  let allowDuplicates = $state(true);
  let filterNewgan = $state(true);

  // 替换只作用于后端当前激活档（浏览态禁用）；展示态统一取自 store getter。
  const snap = $derived(appStore.snapshot);
  const busy = $derived(appStore.busy);
  const browseMode = $derived(appStore.browseMode);
  const ready = $derived(appStore.replaceReady);

  /** 发起替换：兜底校验目录完整性 → 既有映射预检（必要时确认）→ 调后端 start_replace。 */
  async function startReplace() {
    if (!snap?.currentGroup) return;
    // 取到局部变量：闭包里 TS 的判空收窄会失效（runTask 包的正是一条闭包）。
    const imgDir = snap.currentGroup;
    try {
      // 兜底校验：用户可能跳过配置卡的完整性检查，这里再查一次。
      // 标准归属由后端按规则 json 的 FolderList 判定；complete（含超集）→ 静默继续。
      // 经 runTask 让状态栏显示「检查完整性」进度：这条命令同样会发 task:progress，
      // 且平铺大包的顶层条目很多，用户点完「替换头像」正是最需要反馈的时刻。
      const res = await runTask(t.setup.checkIntegrityBtn, () =>
        api.checkIntegrity(imgDir),
      );
      if (res.status === "fixable") {
        const ok = await ask(
          t.replace.missingDirsConfirm(res.standard, res.missing.length),
          { title: t.replace.missingDirsTitle },
        );
        if (!ok) return;
        await api.createMissingDirs(imgDir, res.missing);
      } else if (res.status === "unknown") {
        const ok = await ask(t.replace.unknownConfirm, {
          title: t.replace.unknownTitle,
        });
        if (!ok) return;
      } // complete（标准目录齐全，含多出未配置目录）→ 静默继续
      // 发起前预检：本次会不会把本档既有映射清空（只有「生成」+ 档景非空才要求确认）。
      // 点过确认的这一轮带 allowBlank 发起，流水线里那条写前拦截才会放行而不是中止——
      // 「就是要清空」由此有了应用内通路，不必再去手工删档景文件绕开守卫。
      const pre = await runTask(t.replace.preflightTask, () =>
        api.preflightReplace(mode),
      );
      let allowBlank = false;
      if (pre.needsConfirm) {
        const ok = await ask(t.replace.blankConfirm(pre.existing), {
          title: t.replace.blankTitle,
        });
        if (!ok) return;
        allowBlank = true;
      }
      // 调后端启动替换（异步后台线程），随后刷新快照：replacing 变 true → 全局禁用。
      await api.startReplace({
        mode,
        filterNewgan,
        allowDuplicates,
        saveBackup: appStore.saveBackup,
        allowBlank,
      });
      await refreshSnapshot(); // 不可丢：替换期间取消按钮可用、全局禁用生效
    } catch (e) {
      await showError(e);
    }
  }

  /** 请求取消替换。原来是不 await、不 catch 的浮空 promise：30k 替换中按取消若失败，
      界面毫无反馈，用户只会以为按钮坏了。 */
  let cancelling = $state(false);
  async function cancelReplace() {
    if (cancelling) return; // 一次请求在途就不重发（失败弹一次窗就够，别叠一层）
    cancelling = true;
    try {
      await api.cancelReplace();
    } catch (e) {
      await showError(e);
    } finally {
      cancelling = false;
    }
  }
</script>

<div class="card">
  <div class="card-title">{t.replace.cardTitle}</div>
  <!-- 模式选择：遍历 t.replace.modes 生成三个按钮，按钮显示 label，点选后把 value（协议值）存进 mode。 -->
  <div class="path-row two">
    <span class="row-label">{t.replace.modeLabel}</span>
    <div class="mode-line">
      <div class="seg3">
        {#each t.replace.modes as m}
          <button
            class:active={mode === m.value}
            onclick={() => (mode = m.value)}
            disabled={busy}>{m.label}</button
          >
        {/each}
      </div>
      <!-- 当前选中模式的说明文字。 -->
      <span class="muted small">{modeDesc}</span>
    </div>
  </div>
  <!-- 选项开关：底层仍是 input checkbox + bind:checked（键盘/读屏语义不变），外观由 .switch/.track 皮肤化。 -->
  <div class="path-row two">
    <span class="row-label">{t.replace.optionsLabel}</span>
    <div class="opt-line">
      <label class="switch">
        <input type="checkbox" bind:checked={allowDuplicates} disabled={busy} />
        <span class="track"></span>{t.replace.optAllowDuplicates}
      </label>
      <label class="switch">
        <input type="checkbox" bind:checked={filterNewgan} disabled={busy} />
        <span class="track"></span>{t.replace.optFilterNewgan}
      </label>
      <label class="switch">
        <input
          type="checkbox"
          bind:checked={appStore.saveBackup}
          disabled={busy}
        />
        <span class="track"></span>{t.replace.optSaveBackup}
      </label>
    </div>
  </div>
  <!-- 开始/取消按钮：未就绪或忙碌时禁用开始；非忙碌时禁用取消。
       实时进度直接画在按钮里：底是禁用态中性灰，绿色填充层按百分比从左往右铺，100% 即整颗全绿；
       阶段文案与计数不再在此重复，底部状态栏是唯一出处。 -->
  <div class="cta-row">
    <button
      class="btn primary"
      class:running={busy}
      onclick={startReplace}
      disabled={busy || browseMode || !ready}
      title={browseMode ? t.replace.browseDisabledHint : ""}
    >
      <!-- 填充层：宽度绑定进度百分比；aria-hidden 避免被读屏当作内容 -->
      {#if busy}
        <span
          class="btn-fill"
          style="width:{appStore.progress.value}%"
          aria-hidden="true"
        ></span>
      {/if}
      <span class="btn-label">
        {#if !busy}
          <svg
            width="15"
            height="15"
            viewBox="0 0 24 24"
            fill="currentColor"
            aria-hidden="true">{@html icons.play.body}</svg
          >
        {/if}
        {busy ? `${t.replace.runningBtn} ${appStore.progress.value}%` : t.replace.startBtn}
      </span>
    </button>
    <button
      class="btn cancel"
      onclick={cancelReplace}
      disabled={!busy || cancelling}>{t.replace.cancelBtn}</button
    >
  </div>
</div>

<style>
  /* 模式行：分段控件定宽 360，右侧说明文字占用余宽（窄屏自动收缩） */
  .mode-line {
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 0;
  }
  .mode-line .seg3 {
    flex: 0 1 320px;
  }
  /* 仅收窄本面板分段按钮的纵向内边距（全局为 7px），不影响 RuleEditor 复用的 .seg3 */
  .mode-line .seg3 button {
    padding: 4px 10px;
  }
  /* 选项行：三个开关 22px 间距换行 */
  .opt-line {
    display: flex;
    gap: 22px;
    flex-wrap: wrap;
  }

  /* 窄屏（窗口 ≤900px）：模式行标签与分段控件纵向堆叠，分段占满整行，说明文字自然落在下方 */
  @media (max-width: 900px) {
    .path-row.two {
      grid-template-columns: 1fr;
      gap: 7px;
    }
    .path-row.two .row-label {
      text-align: left;
    }
    .mode-line {
      flex-direction: column;
      align-items: stretch;
      gap: 7px;
    }
    .mode-line .seg3 {
      flex: 1 1 auto;
    }
  }
</style>
