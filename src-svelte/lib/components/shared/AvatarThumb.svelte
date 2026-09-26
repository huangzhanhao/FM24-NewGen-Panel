<!-- 头像缩略图（+ 悬停/点击放大气泡）：「名单预览 / 快照预览」共用。
     单人替换是单张大图预览（PlayerViewer 的球员卡），不用本组件，只共用 blob 加载器。

     分工：
     - 缩略图的 blob 由父组件用 createAvatarBlobs 统一加载后经 url 传入（一批图同一套回收账）；
     - 气泡里的原图由本组件自己读（blob 谁创建谁释放：若把行内那个 URL 交给气泡缓存，
       气泡失效时的 revoke 会把行内缩略图一起弄坏）；
     - path（图片绝对路径）为空 = 没找到图，渲染占位符。
     - path 变化（同一 UID 换了图）时用 {#key} 重建气泡：否则气泡缓存里会留着旧图。 -->
<script lang="ts">
  import { loadImageBlobUrl } from "../../services/images";
  import { t } from "../../text"; // 全局文案常量
  import AvatarPopover, {
    type AvatarEntry,
    type AvatarPopoverApi,
  } from "./AvatarPopover.svelte"; // 放大气泡本体（缓存/钉住/延迟关闭都在其中）

  let {
    itemKey, // 气泡缓存键 + 图片 alt（一般传 UID）
    url, // 已就绪的缩略图 blob URL（由父组件的 avatarBlobs 加载器提供）
    path, // 图片磁盘绝对路径（气泡放大时读原图用）
    groupKey, // 气泡缓存失效键：变化（切组）即释放气泡里的 blob 并关气泡
    placeholder = "—", // 无图时的占位文案
  }: {
    itemKey: string;
    url?: string;
    path?: string;
    groupKey: unknown;
    placeholder?: string;
  } = $props();

  /** 气泡取头像：行内已确认路径存在，直接读字节包成 blob URL（读失败由气泡统一兜成文案）。 */
  async function resolve(): Promise<AvatarEntry> {
    if (!path) return { url: null, err: t.table.avatarNotFound };
    return { url: await loadImageBlobUrl(path), err: "" };
  }

  // 气泡实例（bind:this）：缩略图按钮把鼠标/键盘事件接到它的 open/scheduleClose/close 上。
  let pop = $state<AvatarPopoverApi | null>(null);
</script>

{#if url && path}
  <!-- 缩略图本身即触发器：悬停/聚焦弹大气泡、点击钉住。 -->
  <button
    type="button"
    class="thumb-btn"
    title={itemKey}
    aria-label={t.table.avatarAria}
    onmouseenter={(e) => pop?.open(itemKey, e.currentTarget, false)}
    onmouseleave={pop?.scheduleClose}
    onfocus={(e) => pop?.open(itemKey, e.currentTarget, false)}
    onblur={pop?.scheduleClose}
    onclick={(e) => pop?.open(itemKey, e.currentTarget, true)}
  >
    <img src={url} alt={itemKey} class="thumb" />
  </button>
{:else}
  {placeholder}
{/if}

{#key path}
  <AvatarPopover bind:this={pop} resolve={resolve} {groupKey} />
{/key}

<style>
  /* 头像缩略图：配父表格的 td.imgcell（去掉图片格上下内边距），30px 的图不超过文本格的
     盒子（约 32px），行高因此始终与其余文本行一致，不会因图片被撑高。 */
  .thumb {
    height: 30px;
    width: auto;
    max-width: 60px;
    object-fit: cover;
    border-radius: 4px;
    display: block;
  }
  /* 缩略图的触发按钮外壳：去掉按钮默认样式，让图贴齐行内基线 */
  .thumb-btn {
    appearance: none;
    border: 0;
    padding: 0;
    background: none;
    line-height: 0;
    cursor: zoom-in;
  }
</style>
