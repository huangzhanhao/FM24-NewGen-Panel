<script module lang="ts">
  /** 单个头像的解析结果：url=null 表示取不到（err 为给用户看的原因，如「未找到头像图片」）。 */
  export interface AvatarEntry {
    url: string | null;
    err: string;
  }
  /** 气泡实例方法集（父组件经 bind:this 获取，接到行内触发按钮的鼠标/键盘事件上）。 */
  export interface AvatarPopoverApi {
    open: (key: string, el: HTMLElement, pin: boolean) => Promise<void>;
    scheduleClose: () => void;
    close: () => void;
  }
</script>

<!-- 头像预览气泡（名单预览/快照预览共用）：
     悬停/聚焦触发按钮 → 按钮上方弹 186×186 气泡（带「加载中…」），移到气泡上不关、移出延迟 160ms 关；
     点击 → 钉住（右上角 ×，点气泡外任意处 / Esc / 再点同一按钮关闭）。
     解析结果按 key 缓存（反复悬停不重复读盘）；groupKey 变化（切组）整体失效并释放 blob。
     两处取图途径不同（名单预览问后端要路径、快照预览行内自带路径），resolve 由父组件注入；
     气泡状态机/缓存/定位/样式在本组件单点维护。 -->
<script lang="ts">
  import { t } from "../text"; // 全局文案常量

  let {
    resolve,
    groupKey,
  }: {
    resolve: (key: string) => Promise<AvatarEntry>;
    groupKey: unknown; // 缓存失效键：变化（切组）即释放全部 blob 并关气泡
  } = $props();

  // 气泡状态：null=关闭；pin=true=点击钉住（鼠标移出不关闭，靠 ×/Esc/点外部关闭）。
  let pop = $state<{
    key: string;
    url: string | null;
    err: string;
    loading: boolean;
    x: number;
    y: number;
    pin: boolean;
  } | null>(null);
  // 头像标识 → 解析结果缓存（同一次浏览反复悬停不重复走 IPC/读图）。
  const cache = new Map<string, AvatarEntry>();
  let popToken = 0; // 每次打开自增，快速来回悬停时丢弃过期的异步回填
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;

  // groupKey 变化（切组）后旧头像包的解析结果全部失效：释放 blob、清缓存、关气泡。
  // 这里直接置空 pop 而不经 close()——close 会读 pop，把「气泡开关」变成 effect 依赖，
  // 开气泡就会立刻被本 effect 关掉（读依赖 → 重跑 → 关闭的循环）。
  $effect(() => {
    void groupKey;
    for (const e of cache.values()) {
      if (e.url) URL.revokeObjectURL(e.url);
    }
    cache.clear();
    clearTimeout(hoverTimer);
    pop = null;
    return () => clearTimeout(hoverTimer);
  });

  /** 气泡固定定位：优先按钮上方居中显示，上方空间不足翻下方，横向夹到视口内。 */
  function popPos(el: HTMLElement) {
    const W = 204,
      H = 232; // 气泡宽/高（图 186×186 + 内边距 + 图注）
    const r = el.getBoundingClientRect();
    const x = Math.min(
      Math.max(8, r.left + r.width / 2 - W / 2),
      Math.max(8, window.innerWidth - W - 8),
    );
    const y =
      r.top - H - 8 >= 8
        ? r.top - H - 8
        : Math.min(r.bottom + 8, Math.max(8, window.innerHeight - H - 8));
    return { x, y };
  }

  /** 取单头像解析结果：缓存命中直接回；未命中调注入的 resolve 并入缓存（即使气泡已移走也缓存）。 */
  async function loadEntry(key: string): Promise<AvatarEntry> {
    const cached = cache.get(key);
    if (cached) return cached;
    let entry: AvatarEntry;
    try {
      entry = await resolve(key);
    } catch (e) {
      entry = { url: null, err: String(e) }; // 如「请先选择头像包目录」
    }
    cache.set(key, entry);
    return entry;
  }

  /** 打开气泡：pin=false=悬停（移出即关），pin=true=点击钉住；点击已打开的同一气泡则取消钉住/关闭。 */
  export async function open(key: string, el: HTMLElement, pin: boolean) {
    clearTimeout(hoverTimer);
    if (pop?.key === key) {
      if (pin) {
        if (pop.pin) pop = null;
        else pop.pin = true;
      }
      return;
    }
    const token = ++popToken;
    const cached = cache.get(key);
    pop = {
      key,
      url: cached?.url ?? null,
      err: cached?.err ?? "",
      loading: !cached,
      x: popPos(el).x,
      y: popPos(el).y,
      pin,
    };
    if (cached) return;
    const entry = await loadEntry(key);
    // 异步回来时用户已移到别的按钮/关闭：不回填旧气泡（blob 已入缓存，下次直接命中）。
    if (token !== popToken || pop?.key !== key) return;
    pop.url = entry.url;
    pop.err = entry.err;
    pop.loading = false;
  }

  /** 离开按钮/气泡：钉住态保留，未钉住延迟 160ms 关闭，给鼠标移进气泡留裕量。 */
  export function scheduleClose() {
    if (pop?.pin) return;
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      if (!pop?.pin) pop = null;
    }, 160);
  }

  /** 立即关闭气泡（点击外部 / × / Esc）。 */
  export function close() {
    clearTimeout(hoverTimer);
    pop = null;
  }

  // 钉住态下注册窗口 Esc 关闭（仅钉住时挂载监听）。
  $effect(() => {
    if (!pop?.pin) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
</script>

{#if pop}
  {#if pop.pin}
    <button
      type="button"
      class="pop-backdrop"
      aria-label="关闭"
      onclick={close}
    ></button>
  {/if}
  <div
    class="avatar-pop"
    class:pin={pop.pin}
    style:left="{pop.x}px"
    style:top="{pop.y}px"
    role="dialog"
    tabindex="-1"
    aria-label={t.table.avatarAria}
    onmouseenter={() => clearTimeout(hoverTimer)}
    onmouseleave={scheduleClose}
  >
    <div class="pop-body">
      {#if pop.loading}
        <span class="pop-status">{t.table.avatarLoading}</span>
      {:else if pop.url}
        <img src={pop.url} alt={pop.key} />
      {:else}
        <span class="pop-status err" title={pop.err}>{pop.err}</span>
      {/if}
    </div>
    <div class="pop-cap mono" title={pop.key}>{pop.key}</div>
    {#if pop.pin}
      <button type="button" class="pop-x" aria-label="关闭" onclick={close}
        >×</button
      >
    {/if}
  </div>
{/if}

<style>
  /* 钉住态的透明捕获层：点气泡外任意处关闭（浮在所有内容之上、气泡之下） */
  .pop-backdrop { position: fixed; inset: 0; z-index: 1000; margin: 0; padding: 0; border: 0; background: transparent; cursor: default; }
  /* 头像气泡：固定定位脱离滚动容器；入场动画 140ms / 4px，符合动效约束 */
  .avatar-pop { position: fixed; z-index: 1001; width: 204px; padding: 8px; border: 1px solid var(--hairline); border-radius: var(--radius-md); background: var(--bg-surface); box-shadow: var(--shadow-pop); animation: pop-in 140ms ease; }
  @keyframes pop-in {
    from { opacity: 0; transform: translateY(4px); }
    to { opacity: 1; transform: none; }
  }
  .pop-body { width: 186px; height: 186px; display: grid; place-items: center; background: var(--bg-inset); border-radius: calc(var(--radius-md) - 2px); overflow: hidden; }
  .pop-body img { display: block; width: 100%; height: 100%; object-fit: contain; }
  .pop-status { padding: 8px; font-size: 12px; line-height: 1.6; color: var(--text-3); text-align: center; }
  .pop-status.err { color: var(--danger); }
  /* 气泡底部的头像标识原文：超长省略，悬停看 title */
  .pop-cap { margin-top: 6px; font-size: 11px; color: var(--text-3); text-align: center; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .pop-x { position: absolute; top: 4px; right: 4px; width: 20px; height: 20px; display: grid; place-items: center; border: 0; border-radius: 6px; background: rgba(0, 0, 0, 0.28); color: #fff; font-size: 13px; line-height: 1; cursor: pointer; }
  .pop-x:hover { background: rgba(0, 0, 0, 0.5); }
</style>
