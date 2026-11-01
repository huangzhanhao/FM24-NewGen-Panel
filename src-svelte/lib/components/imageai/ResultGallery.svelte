<!-- ResultGallery · 结果画廊（空态 / 骨架 / 网格三态互斥渲染 + 全屏灯箱）：
     结果数据的生命周期（没有→排队中→已产出→细看）自成一群；唯一外部触点是
     卡内「再生成」回调——onregenerate(meta) 交父级 onGenerate（Key 提交在父级包装）。 -->
<script lang="ts">
  import { confirm, message } from "@tauri-apps/plugin-dialog";
  import Icon from "../../ui/Icon.svelte";
  import { saveGeneratedImageAs } from "../../ui/generatedImageSave"; // 生成图另存为（与队列坞共用）
  import { showError } from "../../ui/dialog";
  import { tip } from "../../stores/app.svelte";
  import { t } from "../../text";
  import {
    imageaiStore,
    parseSize,
    removeResult,
    saveAndReplace,
  } from "../../stores/imageai.svelte";

  let { onregenerate }: { onregenerate: (meta: { size: string; seed: string }) => void } = $props();

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
      const r = await saveAndReplace(imgId);
      // 未命中＝图片进了 _generated，但该球员在当前档没有映射行；必须警告，不能报成功。
      if (r.replaced) {
        await message(t.gen.saveReplace.ok(sp.name), { title: t.dialog.info });
      } else {
        await message(t.gen.saveReplace.notMapped(sp.name), { title: t.dialog.warning });
      }
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
    <!-- 画廊 = 本会话结果 ∪ 持久记录（store.gallery 已按 id 去重）；持久记录图被会话清理时走下方占位分支 -->
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
            <!-- 保存并替换：需数据源球员，无数据源禁用并提示入口 -->
            <button
              class="btn sm"
              disabled={!imageaiStore.sourcePlayer}
              use:tip={imageaiStore.sourcePlayer ? t.gen.saveReplace.btn : t.gen.saveReplace.needPlayer}
              onclick={() => onSaveReplace(img.id)}
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

<style>
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
