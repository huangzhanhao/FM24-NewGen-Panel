/**
 * 一组头像图的 blob URL 加载器 —— 「名单预览 / 快照预览 / 单人替换」三个标签页共用。
 *
 * 【为什么需要它】WebView 不能直接用磁盘路径当 <img> 源，图片必须由后端读成字节（read_image）
 * 再包成 blob URL；而 blob URL 不会随组件消失自动回收，必须显式 revoke。
 * 这套「读字节 → 建 URL → 回收」的账原先在三个组件里各写一遍（两个预览表格还各自维护了
 * 令牌 + 在用清单 + 卸载清理），此处单点维护：
 *  - load() 先回收上一批 URL：翻页/换人立即换代，不留旧图；
 *  - 每次 load 自增令牌，在途的旧响应回来直接丢弃（慢的那批不会覆盖新的一批）；
 *  - 单张读取失败只让该 key 保持无图，不阻塞整批；
 *  - 组件卸载时自动回收全部 URL（调用方不必记得释放）。
 *
 * 【须在组件初始化期调用】内部使用 $state / $effect，只能由组件（或其它 effect 根）调用，
 * 且一个实例对应一处「同时只展示一批图」的位点（如一个表格当前页）。
 */
import { api } from './api';
import { bytesToBlobUrl } from './images';

/** 加载器对外接口（语义见 createAvatarBlobs）。 */
export interface AvatarBlobs {
  /** key → 已就绪的 blob URL（响应式；未就绪或加载失败的 key 不存在）。 */
  readonly urls: Record<string, string>;
  /** 用「key → 图片绝对路径」覆盖式加载一批（先回收旧批，再并发读图，就绪一张出现一张）。 */
  load(paths: Record<string, string>): Promise<void>;
  /** 立即回收全部 URL 并清空（切名单/翻页失效时用；组件卸载时自动调用一次）。 */
  clear(): void;
}

/** 创建一个头像 blob 加载器（用法与生命周期见文件头注释）。 */
export function createAvatarBlobs(): AvatarBlobs {
  let urls = $state<Record<string, string>>({});
  let live: string[] = []; // 在用 URL 清单（回收用；刻意不用 $state，避免被 effect 追踪）
  let token = 0; // 加载令牌：新一批开始后，在途的旧响应作废

  /** 回收全部 URL 并清空（换批与卸载都走这里）。 */
  function clear() {
    for (const u of live) URL.revokeObjectURL(u);
    live = [];
    urls = {};
  }

  /** 覆盖式加载一批：先回收旧批，再并发读图（失败的单张留空）。 */
  async function load(paths: Record<string, string>) {
    const t = ++token;
    clear();
    await Promise.all(
      Object.entries(paths).map(async ([key, path]) => {
        try {
          const bytes = await api.readImage(path);
          if (t !== token) return; // 期间已换批/卸载：过期响应不建 URL
          const url = bytesToBlobUrl(path, bytes);
          live.push(url); // 先登记再露出，保证之后的 clear 一定回收得到它
          urls[key] = url;
        } catch {
          // 单张读取失败不阻塞整批：该 key 保持无图，由调用方渲染占位
        }
      }),
    );
  }

  // 组件卸载时自动回收：blob URL 不随组件消失而释放，漏掉就是长会话的内存泄漏。
  $effect(() => () => clear());

  return {
    get urls() {
      return urls;
    },
    load,
    clear,
  };
}
