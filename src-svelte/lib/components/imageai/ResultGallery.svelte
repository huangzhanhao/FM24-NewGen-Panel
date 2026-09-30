<!-- ResultGallery · 结果画廊（空态 / 骨架 / 网格三态互斥渲染 + 全屏灯箱）：
     结果数据的生命周期（没有→排队中→已产出→细看）自成一群；唯一外部触点是
     卡内「再生成」回调——onregenerate(meta) 交父级 onGenerate（Key 提交在父级包装）。 -->
<script lang="ts">
  import { confirm, message } from "@tauri-apps/plugin-dialog";
  import Icon from "../../ui/Icon.svelte";
  import { saveGeneratedImageAs } from "../../ui/generatedImageSave"; // 生成图另存为（与队列坞共用）
  import { showError } from "../../ui/dialog";
  import { t } from "../../text";
  import {
    imageaiStore,
    parseSize,
    removeResult,
    saveAndReplace,
  } from "../../stores/imageai.svelte";

  let { onregenerate }: { onregenerate: (meta: { size: string; seed: string }) => void } =
    $props();

  // ---- 结果大图灯箱 ----
  let lightboxUrl = $state<string | null>(null);

  /** 结果卡真实比例样式（由生成时快照尺寸推导；解析不出回退正方形）。 */
  function ratioStyle(size?: string): string {
    const p = size ? parseSize(size) : null;
    return p ? `aspect-ratio:${p[0]} / ${p[1]}` : "aspect-ratio:1";
  }

  // ---- 结果卡「保存并替换」：确认后写入 _generated 并走单人换脸链路 ----
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
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape") lightboxUrl = null;
  }}
/>

{#if imageaiStore.results.length === 0 && !imageaiStore.generating}
  <div class="empty res-empty">
    <Icon name="image" stroke={1.6} />
    <p>{t.gen.empty}</p>
  </div>
{:else}
  <div class="gallery">
    {#if imageaiStore.generating}
      {#each Array(Math.max(1, imageaiStore.n)) as _, i (i)}
        <div class="img-card skel-card">
          <div
            class="skel-img"
            aria-hidden="true"
            style={ratioStyle(imageaiStore.size)}
          ></div>
          <div class="skel-meta">
            <span class="skel-line"></span>
            {#if i === 0}<span class="skel-hint"
                >{t.gen.skeletonHint}</span
              >{/if}
          </div>
        </div>
      {/each}
    {/if}
    {#each imageaiStore.results as img (img.id)}
      <div class="img-card">
        <button
          type="button"
          class="img-frame"
          style={ratioStyle(img.size)}
          title={t.gen.viewLarge}
          onclick={() => (lightboxUrl = img.url)}
        >
          <img src={img.url} alt={t.gen.resultAlt} loading="lazy" decoding="async" />
        </button>
        <div class="img-meta mono">
          <span>{img.size ? img.size.replace("*", "×") : ""}</span>
          {#if img.seed}<span>{t.gen.metaSeed(img.seed)}</span>{/if}
        </div>
        <div class="actions">
          <button
            class="btn sm"
            onclick={() => saveGeneratedImageAs(img.id)}
            title={t.gen.save}
          >
            <Icon name="download" class="ic" />
            {t.gen.save}
          </button>
          <!-- 保存并替换：需数据源球员，无数据源禁用并提示入口 -->
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
          <button
            class="btn sm"
            title={t.gen.regenTitle}
            onclick={() => onregenerate({ size: img.size ?? '', seed: img.seed ?? '' })}
          >
            <Icon name="refresh" class="ic" />
          </button>
          <button
            class="btn sm icon-as-btn"
            title={t.gen.viewLarge}
            onclick={() => (lightboxUrl = img.url)}
          >
            <Icon name="zoom" class="ic" />
          </button>
          <button
            class="icon-btn"
            title={t.gen.removeImageTitle}
            onclick={() => removeResult(img.id)}
          >
            <Icon name="x" size={13} />
          </button>
        </div>
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
    <div
      class="lb-dialog"
      role="dialog"
      aria-modal="true"
      aria-label={t.gen.viewLarge}
    >
      <img src={lightboxUrl} alt={t.gen.previewAlt} />
      <button
        class="icon-btn lb-close"
        title={t.gen.closePreview}
        onclick={() => (lightboxUrl = null)}
      >
        <Icon name="x" size={16} />
      </button>
    </div>
  </div>
{/if}

<style>
  .res-empty {
    flex: 1;
  }
  .gallery {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 10px;
    align-content: start;
    overflow-y: auto;
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
    padding: 8px;
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
  .img-meta {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-3);
    white-space: nowrap;
    overflow: hidden;
  }
  .actions {
    display: flex;
    gap: 6px;
    padding: 6px 8px 8px;
    align-items: center;
  }
  .actions .icon-btn {
    margin-left: auto;
  }
  .icon-as-btn {
    width: 30px;
    padding-left: 0;
    padding-right: 0;
  }

  /* 骨架卡 */
  .skel-card {
    border-style: dashed;
  }
  .skel-img {
    box-sizing: border-box;
    width: calc(100% - 16px);
    min-height: 90px;
    border-radius: var(--radius-sm);
    margin: 8px;
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
    gap: 4px;
    padding: 0 8px 8px;
  }
  .skel-line {
    height: 10px;
    border-radius: 5px;
    background: var(--bg-raised);
    width: 55%;
  }
  .skel-hint {
    font-size: 11px;
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
    background: rgba(0, 0, 0, 0.72);
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
    border-color: rgba(255, 255, 255, 0.4);
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
  }
  .lb-close:hover:not(:disabled) {
    background: rgba(0, 0, 0, 0.8);
    color: #fff;
    border-color: #fff;
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
