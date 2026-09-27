<!-- AI 生图 Tab 内容：顶部「工作台/台账/队列」分段独立成行（任何宽度都完整可见），
     其下是「服务商/API Key/模型」条。工作台左栏：模板/提示词/参数/生成；右栏：结果区。
     复用全局类与 icons，不重写控件。 -->
<script lang="ts">
  import { onMount } from "svelte";
  import { confirm, message, save } from "@tauri-apps/plugin-dialog";
  import { api } from "../../services/api";
  import UsageLedger from "./UsageLedger.svelte";
  import QueuePanel from "./QueuePanel.svelte";
  import ProviderDialog from "./ProviderDialog.svelte";
  import TemplateDialog from "./TemplateDialog.svelte";
  import AvatarThumb from "../shared/AvatarThumb.svelte"; // 数据源条的头像缩略图 + 放大气泡（与名单预览共用）
  import { createAvatarBlobs } from "../../avatarBlobs.svelte"; // 缩略图 blob 加载器（卸载自动回收）
  import { appStore } from "../../stores/app.svelte"; // 气泡缓存失效键（currentGroup）与备份开关
  import { t } from "../../text";
  import { showError } from "../../ui/dialog";
  import { icons } from "../../ui/icons";
  import {
    VARIABLE_HINTS,
    appendVariable,
    applyTemplate,
    clearSourcePlayer,
    generate,
    imageaiStore,
    loadImageState,
    removeResult,
    saveAndReplace,
    selectModel,
    selectProvider,
    setProviderKey,
  } from "../../stores/imageai.svelte";

  let view = $state<"workbench" | "usage" | "queue">("workbench");
  let providerOpen = $state(false);
  let templateOpen = $state(false);
  let showNeg = $state(true);

  // ---- 顶栏 API Key 直达输入：掩码显示、不回显真值，成功即清空 ----
  let keyInput = $state("");

  // ---- 尺寸：预设 + 自定义混合。SIZE_CUSTOM 是下拉里「自定义…」的哨兵值，
  //      预设尺寸均为 WxH 形式（小写 x），不会与之相撞。 ----
  const SIZE_CUSTOM = "__custom__";
  // 「自定义…」是显式选择的模式：即便合成值恰好等于某预设也保持自定义态，
  // 直到改选预设、或切服务商重置了尺寸才退出（切模型不动尺寸，见 store 注释）。
  let sizeCustomMode = $state(false);
  function parseSize(s: string): [number, number] | null {
    const m = /^(\d+)x(\d+)$/.exec(s);
    return m ? [Number(m[1]), Number(m[2])] : null;
  }
  let sizeW = $state(parseSize(imageaiStore.size)?.[0] ?? 350);
  let sizeH = $state(parseSize(imageaiStore.size)?.[1] ?? 350);

  const sizePresets = $derived(
    imageaiStore.modelInfo?.capabilities.sizes ?? [],
  );
  const sizeSelectValue = $derived(
    sizeCustomMode || !sizePresets.includes(imageaiStore.size)
      ? SIZE_CUSTOM
      : imageaiStore.size,
  );

  // store.size 变化（初始挂载、切服务商重置、自定义输入写回）时把 WxH 回填进两个数字输入；
  // 解析失败（理论不会发生）保持现值。输入过程中的同值回填是幂等的，不会打断正在输入的数字。
  $effect(() => {
    const parsed = parseSize(imageaiStore.size);
    if (parsed) {
      sizeW = parsed[0];
      sizeH = parsed[1];
    }
  });

  // 切服务商会把 size 重置为新模型的首个预设 → 退出「自定义…」强制态；
  // 同时清空 Key 输入框（Key 跟随服务商，输入内容不跨服务商残留；已设置状态见 keysSet）。
  $effect(() => {
    void imageaiStore.providerId;
    sizeCustomMode = false;
    keyInput = "";
  });

  async function commitKey() {
    const key = keyInput.trim();
    if (!key) return; // 空值 = 未输入，不动已设置的 Key
    try {
      await setProviderKey(imageaiStore.providerId, key);
      keyInput = ""; // 成功即清空，绝不回显真值
    } catch (e) {
      await showError(e);
    }
  }

  function onSizeSelect(e: Event) {
    const v = (e.currentTarget as HTMLSelectElement).value;
    if (v === SIZE_CUSTOM) {
      sizeCustomMode = true;
      imageaiStore.size = `${sizeW}x${sizeH}`; // 以当前宽高起步，立即生成也是所见即所发
    } else {
      sizeCustomMode = false;
      imageaiStore.size = v;
    }
  }

  function onCustomSize() {
    // 宽高都合法（64–4096）才写回；清空/输入中间态不写，store 保留上一个合法值
    const ok = (v: number) => Number.isFinite(v) && v >= 64 && v <= 4096;
    if (ok(sizeW) && ok(sizeH)) imageaiStore.size = `${sizeW}x${sizeH}`;
  }

  // 首次进入懒加载配置；失败统一走错误弹窗。
  onMount(() => {
    if (!imageaiStore.config) void loadImageState().catch(showError);
  });

  // ---- 数据源条（P3）：头像缩略图（有映射时）+ 姓名 + UID + 清除按钮 ----
  // 头像路径按 UID 问后端 query_avatars（映射缓存 → 存档快照，与名单预览同源），blob 交给
  // 共用加载器（组件卸载自动回收）；取不到/取失败都静默降级为只显示姓名+UID（辅助展示不弹窗）。
  const srcBlobs = createAvatarBlobs();
  let srcAvatarPath = $state<string | undefined>(undefined);
  let srcAvatarToken = 0;
  $effect(() => {
    const sp = imageaiStore.sourcePlayer;
    const token = ++srcAvatarToken;
    srcAvatarPath = undefined;
    srcBlobs.clear();
    if (!sp) return;
    api
      .queryAvatars([sp.uid])
      .then((paths) => {
        if (token !== srcAvatarToken) return; // 期间已换数据源：过期响应作废
        const p = paths[sp.uid] ?? undefined;
        srcAvatarPath = p;
        if (p) void srcBlobs.load({ [sp.uid]: p });
      })
      .catch(() => {
        if (token === srcAvatarToken) srcAvatarPath = undefined;
      });
  });

  async function onGenerate() {
    try {
      await generate();
    } catch (e) {
      await showError(e);
    }
  }

  // ---- 结果卡「保存并替换」（P3）：确认后写入 _generated 并走单人换脸链路 ----
  async function onSaveReplace(imgId: string) {
    const sp = imageaiStore.sourcePlayer;
    if (!sp) return; // 无数据源时按钮已禁用，这里只是防御
    if (
      !(await confirm(t.gen.saveReplace.confirm(sp.name), {
        title: t.dialog.info,
      }))
    )
      return;
    try {
      await saveAndReplace(imgId);
      await message(t.gen.saveReplace.ok(sp.name), { title: t.dialog.info });
    } catch (e) {
      await showError(e); // 批量替换进行中被拒等后端错误走统一弹窗
    }
  }

  async function onSave(imgId: string) {
    const target = await save({
      defaultPath: `${imgId}.png`,
      filters: [{ name: "PNG", extensions: ["png"] }],
    });
    if (!target) return;
    try {
      await api.saveGeneratedImage(imgId, target);
      await message(t.gen.saved, { title: t.dialog.info });
    } catch (e) {
      await showError(e);
    }
  }
</script>

<div class="panel">
  <!-- 分段独立成行（验收反馈）：分段是「页面切换」，放最上面；任何宽度下三个入口都完整可见 -->
  <div class="seg-tabs">
    <button
      class:active={view === "workbench"}
      onclick={() => (view = "workbench")}>{t.gen.tabWorkbench}</button
    >
    <button class:active={view === "usage"} onclick={() => (view = "usage")}
      >{t.gen.tabUsage}</button
    >
    <button class:active={view === "queue"} onclick={() => (view = "queue")}
      >{t.gen.queue.tabQueue}</button
    >
  </div>

  <!-- 数据源条（P3）：显示当前数据源球员（头像缩略图 + 姓名 + UID + 清除），选中态描边高亮；
       无数据源时灰字提示「变量走默认值」。 -->
  <div class="src-bar" class:on={!!imageaiStore.sourcePlayer}>
    {#if imageaiStore.sourcePlayer}
      {@const sp = imageaiStore.sourcePlayer}
      <AvatarThumb
        itemKey={sp.uid}
        url={srcBlobs.urls[sp.uid]}
        path={srcAvatarPath}
        groupKey={appStore.snapshot?.currentGroup}
      />
      <span class="src-name" title={sp.name}>{sp.name}</span>
      <span class="src-uid mono">{sp.uid}</span>
      <button
        class="btn sm"
        title={t.gen.source.clearTitle}
        onclick={clearSourcePlayer}>{t.gen.source.clear}</button
      >
    {:else}
      <span class="src-none">{t.gen.source.none}</span>
    {/if}
  </div>

  <div class="bar">
    <span class="bar-label">{t.gen.provider}</span>
    <select
      value={imageaiStore.providerId}
      onchange={(e) => selectProvider(e.currentTarget.value)}
    >
      {#each imageaiStore.config?.providers ?? [] as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
    <!-- Key 直达输入（验收反馈）：掩码输入绑定当前服务商，change/回车提交；已设置只出徽标不回显 -->
    <label class="bar-label" for="ig-key">{t.gen.keyBarLabel}</label>
    <input
      id="ig-key"
      class="key-input"
      type="password"
      autocomplete="off"
      placeholder={imageaiStore.keysSet[imageaiStore.providerId]
        ? t.gen.keyBarSetPlaceholder
        : t.gen.keyBarPlaceholder}
      bind:value={keyInput}
      onchange={commitKey}
      onkeydown={(e) => {
        if (e.key === "Enter") void commitKey();
      }}
    />
    {#if imageaiStore.keysSet[imageaiStore.providerId]}
      <span class="key-ok">{t.gen.keySetBadge}</span>
    {/if}
    <span class="bar-label">{t.gen.model}</span>
    <select
      value={imageaiStore.model}
      onchange={(e) => selectModel(e.currentTarget.value)}
    >
      <!-- 默认模型不在明细行里时也要能选中（custom 服务商常无明细行，见 store.modelInfo 兜底） -->
      {#if imageaiStore.provider?.defaultModel && !(imageaiStore.provider?.models ?? []).some((m) => m.model === imageaiStore.provider?.defaultModel)}
        <option value={imageaiStore.provider.defaultModel}>{imageaiStore.provider.defaultModel}</option>
      {/if}
      {#each imageaiStore.provider?.models ?? [] as m}
        <option value={m.model}>{m.label}</option>
      {/each}
    </select>
    <button class="btn" onclick={() => (providerOpen = true)}
      >{t.gen.providerSettings}</button
    >
  </div>

  {#if view === "workbench"}
    <div class="wb-grid">
      <div class="wb-left">
        <!-- 模板 -->
        <div class="block">
          <div class="block-head">
            <span>{t.gen.template}</span>
            <button class="btn sm" onclick={() => (templateOpen = true)}
              >{t.gen.manageTemplate}</button
            >
          </div>
          <select
            value={imageaiStore.templateId}
            onchange={(e) =>
              void applyTemplate(e.currentTarget.value).catch(showError)}
          >
            {#each imageaiStore.config?.templates ?? [] as tpl}
              <option value={tpl.id}>{tpl.name}</option>
            {/each}
          </select>
          <p class="hint">{t.gen.variables}</p>
          <div class="chips">
            {#each VARIABLE_HINTS as [key, label] (key)}
              <button class="chip" onclick={() => appendVariable(key)}
                >{label}</button
              >
            {/each}
          </div>
        </div>

        <!-- 提示词 -->
        <div class="block">
          <label class="field-label" for="ig-prompt">{t.gen.prompt}</label>
          <textarea id="ig-prompt" rows="7" bind:value={imageaiStore.prompt}
          ></textarea>
          <button class="neg-toggle" onclick={() => (showNeg = !showNeg)}>
            <svg
              class="ic"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
              >{@html (showNeg ? icons.chevronUp : icons.chevronDown).body}</svg
            >
            {t.gen.toggleNegative}
          </button>
          {#if showNeg}
            <textarea
              rows="2"
              placeholder={t.gen.negative}
              bind:value={imageaiStore.negative}
            ></textarea>
          {/if}
        </div>

        <!-- 参数：能力集驱动 -->
        <div class="block">
          <div class="frow">
            <label for="ig-size">{t.gen.size}</label>
            <select
              id="ig-size"
              value={sizeSelectValue}
              onchange={onSizeSelect}
            >
              {#each sizePresets as s (s)}
                <option value={s}>{s}</option>
              {/each}
              <option value={SIZE_CUSTOM}>{t.gen.sizeCustom}</option>
            </select>
          </div>
          {#if sizeSelectValue === SIZE_CUSTOM}
            <div class="frow size-custom">
              <label for="ig-size-w">{t.gen.sizeWLabel}</label>
              <input
                id="ig-size-w"
                type="number"
                min="64"
                max="4096"
                step="1"
                bind:value={sizeW}
                oninput={onCustomSize}
              />
              <span aria-hidden="true">×</span>
              <label for="ig-size-h">{t.gen.sizeHLabel}</label>
              <input
                id="ig-size-h"
                type="number"
                min="64"
                max="4096"
                step="1"
                bind:value={sizeH}
                oninput={onCustomSize}
              />
            </div>
            <p class="hint">{t.gen.sizeCustomHint}</p>
          {/if}
          <div class="frow">
            <label for="ig-n">{t.gen.n}</label>
            <input
              id="ig-n"
              type="number"
              min="1"
              max={imageaiStore.modelInfo?.capabilities.maxN ?? 1}
              bind:value={imageaiStore.n}
            />
          </div>
          {#if imageaiStore.modelInfo?.capabilities.supportsQuality}
            <div class="frow">
              <label for="ig-quality">{t.gen.quality}</label>
              <select id="ig-quality" bind:value={imageaiStore.quality}>
                {#each ["auto", ...imageaiStore.modelInfo?.capabilities.qualities.filter((q) => q !== "auto")] as q}
                  <option value={q}>{q}</option>
                {/each}
              </select>
            </div>
          {/if}
          {#if imageaiStore.modelInfo?.capabilities.supportsSeed}
            <div class="frow">
              <label for="ig-seed">{t.gen.seed}</label>
              <input
                id="ig-seed"
                type="number"
                placeholder={t.gen.seedPlaceholder}
                bind:value={imageaiStore.seed}
              />
            </div>
          {/if}
        </div>

        <button
          class="btn primary"
          disabled={imageaiStore.generating}
          onclick={onGenerate}
        >
          {imageaiStore.generating ? t.gen.generating : t.gen.generate}
        </button>
      </div>

      <div class="wb-right">
        {#if imageaiStore.results.length === 0}
          <div class="empty">
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              aria-hidden="true">{@html icons.image.body}</svg
            >
            <p>{t.gen.empty}</p>
          </div>
        {:else}
          <div class="gallery">
            {#each imageaiStore.results as img (img.id)}
              <div class="img-card">
                <img src={img.url} alt="generated" />
                <div class="actions">
                  <button class="btn sm" onclick={() => onSave(img.id)}>
                    <svg
                      class="ic"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      aria-hidden="true">{@html icons.download.body}</svg
                    >
                    {t.gen.save}
                  </button>
                  <!-- 保存并替换（P3）：需数据源球员，无数据源禁用并提示入口 -->
                  <button
                    class="btn sm"
                    disabled={!imageaiStore.sourcePlayer}
                    title={imageaiStore.sourcePlayer
                      ? t.gen.saveReplace.btn
                      : t.gen.saveReplace.needPlayer}
                    onclick={() => onSaveReplace(img.id)}
                  >
                    {t.gen.saveReplace.btn}
                  </button>
                  <button class="btn sm" onclick={onGenerate}>
                    <svg
                      class="ic"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      aria-hidden="true">{@html icons.refresh.body}</svg
                    >
                  </button>
                  <button
                    class="icon-btn"
                    title={t.gen.removeImageTitle}
                    onclick={() => removeResult(img.id)}
                  >
                    <svg
                      viewBox="0 0 24 24"
                      width="13"
                      height="13"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      aria-hidden="true">{@html icons.x.body}</svg
                    >
                  </button>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {:else if view === "usage"}
    <UsageLedger />
  {:else}
    <QueuePanel />
  {/if}
</div>

<ProviderDialog bind:open={providerOpen} />
<TemplateDialog bind:open={templateOpen} />

<style>
  /* 父级 .tab-pane.active 是 display:block（app.css L284），故这里不能用 flex:1 撑高，改 height:100%。 */
  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    gap: 10px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .bar-label {
    color: var(--text-2);
    font-size: 12.5px;
  }
  /* 分段已是独立的一行：回到全局 .seg-tabs 版式（保留绿色标条），只去掉自带下边距（间距交给 .panel 的 gap） */
  .seg-tabs {
    margin: 0;
  }
  /* 数据源条：默认灰描边弱提示；有数据源时主题色描边高亮（两主题各自取色） */
  .src-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border: 1px solid var(--hairline);
    border-radius: var(--radius-md);
    background: var(--bg-inset);
    font-size: 12.5px;
  }
  .src-bar.on {
    border-color: var(--accent);
  }
  .src-name {
    font-weight: 600;
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .src-uid {
    color: var(--text-3);
  }
  .src-none {
    color: var(--text-3);
  }
  .key-input {
    width: 170px;
  }
  /* 「已设置」徽标：小圆点 + 文字，取主题成功色令牌（两套主题各自定义） */
  .key-ok {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--ok);
    font-size: 12px;
  }
  .key-ok::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }
  .wb-grid {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(300px, 360px) minmax(0, 1fr);
    gap: 14px;
  }
  .wb-left {
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow-y: auto;
    padding-right: 2px;
  }
  .wb-right {
    min-height: 0;
    display: flex;
  }
  /* .empty 是全局类（app.css L765），但没有独立 .grow；这里让它填满右栏。 */
  .wb-right .empty {
    flex: 1;
  }
  .block-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 6px;
    font-size: 12.5px;
    color: var(--text-2);
  }
  .hint {
    margin: 8px 0 6px;
    color: var(--text-3);
    font-size: 12px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .field-label {
    display: block;
    font-size: 12.5px;
    color: var(--text-2);
    margin-bottom: 6px;
  }
  textarea {
    width: 100%;
    resize: vertical;
  }
  .neg-toggle {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin: 6px 0;
    background: none;
    border: none;
    color: var(--text-3);
    font-size: 12px;
    cursor: pointer;
    padding: 0;
  }
  .neg-toggle:hover {
    color: var(--accent);
  }
  .frow {
    margin: 6px 0;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .frow label {
    width: 48px;
    color: var(--text-2);
    font-size: 12.5px;
    text-align: right;
  }
  .frow input {
    width: 110px;
  }
  /* 自定义尺寸行：宽/高两个小数字输入，中间 × 分隔 */
  .size-custom input {
    width: 72px;
  }
  .size-custom label {
    width: auto;
  }
  .wb-left .btn.primary {
    padding: 10px;
    font-size: 14px;
  }
  .gallery {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 10px;
    align-content: start;
    overflow-y: auto;
  }
  .img-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--bg-surface);
    display: flex;
    flex-direction: column;
  }
  .img-card img {
    width: 100%;
    aspect-ratio: 1;
    object-fit: cover;
    display: block;
  }
  .actions {
    display: flex;
    gap: 6px;
    padding: 6px;
    align-items: center;
  }
  .actions .icon-btn {
    margin-left: auto;
  }
  .ic {
    width: 13px;
    height: 13px;
  }
</style>
