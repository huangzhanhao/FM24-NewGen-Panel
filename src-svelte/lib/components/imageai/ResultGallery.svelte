<!-- ResultGallery · 结果画廊（空态 / 骨架 / 网格三态互斥渲染 + 全屏灯箱）：
     结果数据的生命周期（没有→排队中→已产出→细看）自成一群；唯一外部触点是
     卡内「再生成」回调——onregenerate(meta) 交父级 onGenerate（Key 提交在父级包装）。 -->
<script lang="ts">
  import { message } from "@tauri-apps/plugin-dialog";
  import Icon from "../../ui/Icon.svelte";
  import { saveGeneratedImageAs } from "../../ui/generatedImageSave"; // 生成图另存为（与队列坞共用）
  import { showError } from "../../ui/dialog";
  import { tip } from "../../stores/app.svelte";
  import { t } from "../../text";
  import {
    imageaiStore,
    parseSize,
    previewSaveTarget,
    removeResult,
    saveAndReplace,
  } from "../../stores/imageai.svelte";
  import type { GeneratedImage } from "../../types";

  let { onregenerate }: { onregenerate: (meta: { size: string; seed: string }) => void } = $props();

  // ---- 结果大图灯箱 -------------------------------------------------------------------------------------------------
  let lightboxUrl = $state<string | null>(null);

  /** 结果卡真实比例样式（由生成时快照尺寸推导；解析不出回退正方形）。 */
  function ratioStyle(size?: string): string {
    const p = size ? parseSize(size) : null;
    return p ? `aspect-ratio:${p[0]} / ${p[1]}` : "aspect-ratio:1";
  }

  // ---- 结果卡「保存并替换」：先取落点预览，弹窗确认/修改后再写入 ----------------------------------------------------
  /** 待确认的落点：base 只读展示，rel 可编辑（相对 `_generated`，含池与文件名）。
   *  uid/name 在点开弹窗的瞬间从**这张图**上取定并冻结：弹窗期间用户可能改工作台选人，
   *  保存对象与成功文案都不能跟着变。 */
  let saveTarget = $state<{
    imgId: string;
    uid: string | null;
    name: string;
    base: string;
    rel: string;
  } | null>(null);

  /** 这张图属于哪位球员：图自带的优先（队列产出/持久记录都有），缺失再回落当前数据源。
   *  只用于文案与判禁用，不参与「保存给谁」的判定（那在 store 里同一套回落顺序）。 */
  function ownerName(img: GeneratedImage): string {
    return img.name || imageaiStore.sourcePlayer?.name || "";
  }

  /** 能否保存并替换：这张图自带球员，或工作台已选了数据源。两者都没有才禁用。 */
  function canSaveReplace(img: GeneratedImage): boolean {
    return !!img.uid || !!imageaiStore.sourcePlayer;
  }

  async function onSaveReplace(img: GeneratedImage) {
    if (!canSaveReplace(img)) return; // 按钮已禁用，这里只是防御
    try {
      const p = await previewSaveTarget(img.uid);
      saveTarget = { imgId: img.id, uid: img.uid ?? null, name: ownerName(img), base: p.base, rel: p.rel };
    } catch (e) {
      await showError(e); // 未关联头像包等后端错误走统一弹窗
    }
  }

  async function confirmSaveReplace() {
    const st = saveTarget;
    if (!st) return;
    try {
      const r = await saveAndReplace(st.imgId, st.rel, st.uid, st.name);
      saveTarget = null; // 成功才关：路径非法时报错弹窗关掉后，用户还能改着重试
      // 未命中＝图片已落盘，但该球员在当前档没有映射行；必须警告，不能报成功。
      if (r.replaced) {
        await message(t.gen.saveReplace.ok(st.name), { title: t.dialog.info });
      } else {
        await message(t.gen.saveReplace.notMapped(st.name), { title: t.dialog.warning });
      }
    } catch (e) {
      await showError(e); // 路径非法、批量替换进行中等后端错误走统一弹窗
    }
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key !== "Escape") return;
    // 落点弹窗在灯箱之上：先关它，再轮到灯箱
    if (saveTarget) saveTarget = null;
    else lightboxUrl = null;
  }}
/>

{#if imageaiStore.gallery.length === 0 && !imageaiStore.generating}
  <div class="empty res-empty">
    <Icon name="image" stroke={1.6} />
    <p>{t.gen.empty}</p>
  </div>
{:else}
  <div class="gallery">
    {#if imageaiStore.generating}
      {#each Array(Math.max(1, imageaiStore.n)) as _, i (i)}
        <div class="img-card skel-card">
          <div class="skel-img" aria-hidden="true" style={ratioStyle(imageaiStore.size)}></div>
          <div class="skel-meta">
            <span class="skel-line"></span>
            {#if i === 0}<span class="skel-hint">{t.gen.skeletonHint}</span>{/if}
          </div>
        </div>
      {/each}
    {/if}
    <!-- 画廊 = 本会话结果 ∪ 持久记录（store.gallery 已按 id 去重）；读不出字节的记录在载入时就被滤掉，
         下方占位分支只兜底「url 事后失效」这一极端情况 -->
    {#each imageaiStore.gallery as img (img.id)}
      {@const hasImg = img.imageAvailable !== false && !!img.url}
      <div class="img-card">
        {#if hasImg}
          <button
            type="button"
            class="img-frame"
            style={ratioStyle(img.size)}
            use:tip={t.gen.viewLarge}
            onclick={() => (lightboxUrl = img.url)}
          >
            <img src={img.url} alt={t.gen.resultAlt} loading="lazy" decoding="async" />
          </button>
          <div class="img-meta mono">
            <span>{img.size ? img.size.replace("*", "×") : ""}</span>
            {#if img.seed}<span>{t.gen.metaSeed(img.seed)}</span>{/if}
          </div>
          <div class="actions">
            <button class="btn sm" onclick={() => saveGeneratedImageAs(img.id)} use:tip={t.gen.save}>
              <Icon name="download" class="ic" />
              {t.gen.save}
            </button>
            <!-- 保存并替换：图自带球员（队列产出/持久记录）或工作台选了数据源都可用，两者都没有才禁用 -->
            <button
              class="btn sm"
              disabled={!canSaveReplace(img)}
              use:tip={canSaveReplace(img) ? t.gen.saveReplace.btn : t.gen.saveReplace.needPlayer}
              onclick={() => onSaveReplace(img)}
            >
              {t.gen.saveReplace.btn}
            </button>
            <button
              class="btn sm"
              use:tip={t.gen.regenTitle}
              onclick={() => onregenerate({ size: img.size ?? "", seed: img.seed ?? "" })}
            >
              <Icon name="refresh" class="ic" />
            </button>
            <button
              class="btn sm icon-as-btn"
              use:tip={t.gen.viewLarge}
              onclick={() => (lightboxUrl = img.url)}
            >
              <Icon name="zoom" class="ic" />
            </button>
            <!-- 移除只对本会话结果有效；持久记录没有对应命令，故不渲染该钮（removable=false） -->
            {#if img.removable !== false}
              <button
                class="icon-btn"
                use:tip={t.gen.removeImageTitle}
                onclick={() => removeResult(img.id)}
              >
                <Icon name="x" size={13} />
              </button>
            {/if}
          </div>
        {:else}
          <!-- 图片已不在（会话临时目录被清理）：复用 img-frame 几何的中置占位，不新造组件 -->
          <div class="img-frame ph-frame" style={ratioStyle(img.size)} use:tip={t.gen.imageMissing}>
            <Icon name="image" class="ph-ic" stroke={1.6} />
            <span class="ph-text">{t.gen.imageMissing}</span>
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<!-- 结果大图灯箱：点遮罩空白处/关闭钮/Esc 退出（遮罩写法对齐项目内其他模态） -->
{#if lightboxUrl}
  <div
    class="lightbox"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) lightboxUrl = null;
    }}
  >
    <div class="lb-dialog" role="dialog" aria-modal="true" aria-label={t.gen.viewLarge}>
      <img src={lightboxUrl} alt={t.gen.previewAlt} />
      <button
        class="icon-btn lb-close"
        use:tip={t.gen.closePreview}
        onclick={() => (lightboxUrl = null)}
      >
        <Icon name="x" size={16} />
      </button>
    </div>
  </div>
{/if}

<!-- 落点确认：base 只读，rel 可编辑（相对 _generated，含池与文件名）；确认后才写盘 -->
{#if saveTarget}
  <div
    class="modal-overlay"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) saveTarget = null;
    }}
  >
    <div class="modal narrow" role="dialog" aria-modal="true" aria-label={t.gen.saveReplace.pathTitle}>
      <div class="modal-title">{t.gen.saveReplace.pathTitle}</div>
      <div class="modal-body">
        <!-- 点名保存对象：图可能属于它自己的球员（队列产出/旧图），与工作台当前数据源未必同一人 -->
        {#if saveTarget.name || saveTarget.uid}
          <p class="save-target">{t.gen.saveReplace.target(saveTarget.name || saveTarget.uid || "")}</p>
        {/if}
        <p class="path-line mono">{saveTarget.base}/{saveTarget.rel}</p>
        <input type="text" class="path-input" bind:value={saveTarget.rel} spellcheck="false" />
        <p class="path-hint">{t.gen.saveReplace.pathHint}</p>
      </div>
      <div class="modal-foot">
        <button class="btn" onclick={() => (saveTarget = null)}>{t.gen.saveReplace.cancelBtn}</button>
        <button class="btn primary" onclick={() => void confirmSaveReplace()}>
          {t.gen.saveReplace.btn}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* 落点弹窗只需一行输入：覆写全局 .modal 的固定高宽，其余 modal 原语照用 */
  .modal.narrow {
    width: min(560px, 92vw);
    height: auto;
    max-height: 86vh;
  }
  .path-line {
    margin: 0 0 var(--sp-3);
    font-size: var(--fs-sm);
    color: var(--text-2);
    word-break: break-all;
  }
  /* 保存对象一行：这行说的是「本次真正会被改头像的人」，用正文色强调；
     字号与路径行同量级，不新造层级 */
  .save-target {
    margin: 0 0 var(--sp-2);
    font-size: var(--fs-sm);
    color: var(--text-1);
    font-weight: var(--fw-semibold);
    word-break: break-all;
  }
  .path-input {
    width: 100%;
    font-size: var(--fs-sm);
  }
  .path-hint {
    margin: var(--sp-3) 0 0;
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  .res-empty {
    flex: 1;
  }
  .gallery {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--sp-4);
    align-content: start;
    overflow-y: auto;
  }
  /* ≤1024 单栏堆叠：结果卡随内容自然高，画廊不再内部滚动（整栅格单点滚动），
     否则 flex:1 在自然高父级里会把画廊折叠为 0 */
  @media (max-width: 1024px) {
    .gallery {
      flex: none;
      overflow: visible;
    }
  }
  .img-card {
    border: 1px solid var(--hairline-2);
    border-radius: var(--radius-md);
    overflow: hidden;
    background: var(--bg-surface);
    display: flex;
    flex-direction: column;
  }
  /* 真实比例框：图片 contain 不裁剪；inset 底避免透明 PNG 看不清边界 */
  .img-frame {
    display: block;
    width: 100%;
    padding: var(--sp-3);
    border: none;
    background: var(--bg-inset);
    cursor: zoom-in;
  }
  .img-frame img {
    width: 100%;
    height: auto;
    object-fit: contain;
    display: block;
    border-radius: var(--radius-sm);
  }
  /* 图片已不在的占位：沿用 .img-frame 的几何与 inset 底，内部中置图标+说明；
     覆盖 .img-frame 的 zoom-in 光标（此处不可点看大图） */
  .ph-frame {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    color: var(--text-3);
    cursor: default;
  }
  :global(.ph-ic) {
    width: 22px;
    height: 22px;
    opacity: 0.75;
  }
  .ph-text {
    font-size: var(--fs-xs);
  }
  .img-meta {
    display: flex;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: 0 var(--sp-3);
    font-size: var(--fs-xs);
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
  }
  .actions {
    display: flex;
    gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3) var(--sp-3);
    align-items: center;
  }
  .actions .icon-btn {
    margin-left: auto;
  }
  .icon-as-btn {
    width: var(--control-h);
    padding-left: 0;
    padding-right: 0;
  }

  /* 骨架卡 */
  .skel-card {
    border-style: dashed;
  }
  .skel-img {
    box-sizing: border-box;
    width: calc(100% - var(--sp-6));
    min-height: 90px;
    border-radius: var(--radius-sm);
    margin: var(--sp-3);
    background: linear-gradient(
      100deg,
      var(--bg-inset) 40%,
      var(--bg-raised) 50%,
      var(--bg-inset) 60%
    );
    background-size: 200% 100%;
    animation: skel-shimmer 1.3s linear infinite;
  }
  .skel-meta {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: 0 var(--sp-3) var(--sp-3);
  }
  .skel-line {
    height: 10px;
    border-radius: 5px;
    background: var(--bg-raised);
    width: 55%;
  }
  .skel-hint {
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
  @keyframes skel-shimmer {
    from {
      background-position: 200% 0;
    }
    to {
      background-position: -200% 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .skel-img {
      animation: none;
    }
  }

  /* 灯箱 */
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: var(--z-modal);
    background: var(--scrim);
    display: grid;
    place-items: center;
    padding: 5vh 8vw;
    animation: lb-fade var(--dur-fast) var(--ease-out);
  }
  .lb-dialog {
    position: relative;
    display: grid;
    place-items: center;
    max-width: 100%;
    max-height: 100%;
  }
  .lb-dialog img {
    max-width: 84vw;
    max-height: 88vh;
    object-fit: contain;
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-pop);
  }
  .lb-close {
    position: absolute;
    top: -12px;
    right: -12px;
    width: 32px;
    height: 32px;
    border-color: var(--on-media-hairline);
    background: var(--on-media-surface);
    color: var(--on-media-strong);
  }
  .lb-close:hover:not(:disabled) {
    background: var(--on-media-surface-hover);
    color: var(--on-media-strong);
    border-color: var(--on-media-strong);
  }
  @keyframes lb-fade {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .lightbox {
      animation: none;
    }
  }
</style>
