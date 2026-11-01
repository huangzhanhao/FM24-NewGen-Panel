<!-- AvatarThumb · 头像缩略图：名单预览 / 快照预览 / 生图台各处共用的缩略图 + 悬停/点击放大气泡。
     单人替换是单张大图预览（PlayerViewer 的球员卡），不用本组件，只共用 blob 加载器。

     分工：
     - 缩略图的 blob 由父组件用 createAvatarBlobs 统一加载后经 url 传入（一批图同一套回收账）；
     - 气泡里的原图由本组件自己读（blob 谁创建谁释放：把行内那个 URL 交给气泡缓存的话，
       气泡失效时的 revoke 会把行内缩略图一起弄坏）；
     - path（图片绝对路径）为空 = 没找到图，渲染占位符
     - path 变化（同一 UID 换了图）时用 {#key} 重建气泡：否则气泡缓存里会留着旧图 -->
<script lang="ts">
  import { loadImageBlobUrl } from "../../services/images";
  import { tip } from "../../stores/app.svelte";
  import { t } from "../../text"; // 全局文案常量
  import AvatarPopover, { type AvatarEntry, type AvatarPopoverApi } from "./AvatarPopover.svelte"; // 放大气泡本体：缓存/钉住/延迟关闭都在其中

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

  /** 气泡取头像：行内已有绝对路径，直接读字节包成 blob URL（读失败由气泡统一兜成文案） */
  async function resolve(): Promise<AvatarEntry> {
    if (!path) return { url: null, err: t.table.avatarNotFound };
    return { url: await loadImageBlobUrl(path), err: "" };
  }

  // 气泡实例（bind:this）：缩略图按钮的鼠标/键盘事件接到它的 open/scheduleClose/close
  let pop = $state<AvatarPopoverApi | null>(null);
</script>

{#if url && path}
  <!-- 缩略图本身即触发器：悬停/聚焦弹大气泡、点击钉住 -->
  <button
    type="button"
    class="thumb-btn"
    use:tip={itemKey}
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
  <!-- 无图占位也占满 30px 固定盒：与有图行同高（纯文本会矮 2px，导致快照表按行高实测
       自适应时图装载前后行数抖动）；宽度随文案（≤60px 与缩略图 max-width 齐） -->
  <span class="thumb-ph" use:tip={itemKey}>{placeholder}</span>
{/if}

{#key path}
  <AvatarPopover bind:this={pop} {resolve} {groupKey} />
{/key}

<style>
  /* 头像缩略图：高度取标准控件高（30px），父表格行高由 --table-row-h 固化（40px），
     图不会把行撑高；无图占位同高，故行高与图片是否就绪无关。 */
  .thumb {
    height: var(--control-h);
    width: auto;
    max-width: 60px;
    object-fit: cover;
    border-radius: 3px; /* 装饰小圆角（令牌表刻意保留的 2/3/5px 档） */
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
  /* 无图占位盒：几何对齐缩略图（高 30、最小宽 30、最大宽 60），文案居中弱化 */
  .thumb-ph {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: var(--control-h);
    min-width: var(--control-h);
    max-width: 60px;
    padding: 0 var(--sp-1);
    color: var(--text-3);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
</style>
