/**
 * 头像缩略图的两层数据管线 —— 「名单预览 / 快照预览 / 单人替换」等位点共用。
 * - createAvatarBlobs：底层「读字节 → 建 URL → 回收」的加载器；
 * - createAvatarSource：上层「按 uid 批量查路径 + 装载缩略图」的数据源（名单头像列、
 *   队列坞条目行、生图球员源卡三处共用，见文件尾注释）。
 *
 * 【为什么需要加载器】WebView 不能直接用磁盘路径当 <img> 源，图片必须由后端读成字节（read_image）
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
import { api } from './services/api';
import { bytesToBlobUrl } from './services/images';

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

// ---------------- 按 uid 批量展示头像的数据源（路径查询 + 缩略图装载） ----------------

/** 一处「按 uid 批量展示头像」位点的数据源（名单预览页 / 队列坞条目行 / 生图球员源卡共用）。
 *
 *  【为什么抽它】「query_avatars 批量查路径 → 滤 null → 交给 blob 加载器 → 令牌作废在途响应」
 *  这套骨架原先在三个组件里各写一遍（令牌、paths 记账、失败降级的细节各有出入），
 *  是「三处修一处漏两处」的典型面，此处单点维护。
 *  【为什么按 UID 查】RTF 名单「头像」列原文导出时恒为空（见 rtf_scanner 的 EXT_AVATAR_COL），
 *  头像只能按球员 UID 反查映射；解析（映射缓存 → 存档快照 → 快照文件）由后端 query_avatars
 *  完成，出处与单人预览同源。
 *
 *  语义：
 *   - load(uids)：令牌自增后批量查路径；成功整份写 paths（响应式）并把非空路径交给内置
 *     blob 加载器覆盖式装载（上一批 URL 自动回收）；在途旧响应回来直接丢弃。批量失败清空
 *     paths 与 blob（不留上一批旧图）并把原因记入 error——头像列是辅助展示，不弹窗，
 *     调用方自行决定展示（名单预览挂在列头）或忽略（队列坞/球员源卡静默降级）。
 *   - clear()：立即作废在途查询并清空全部状态（切名单/换数据源等需要就地清白的时机由调用方定）。
 *
 *  【须在组件初始化期调用】内部有 $state/$effect（卸载自动回收），一个实例对应一处头像位点。 */
export interface AvatarSource {
  /** uid → 头像绝对路径（null = 无映射或图片不在包内；新一批响应到达前保留上一批）。 */
  readonly paths: Record<string, string | null>;
  /** uid → 已就绪的缩略图 blob URL（未就绪/无图的 uid 不存在）。 */
  readonly urls: Record<string, string>;
  /** 最近一次批量查询失败的原因（'' = 无错）。 */
  readonly error: string;
  /** 批量查一组 uid 的头像路径并装载缩略图（同实例的在途旧响应自相作废）。 */
  load(uids: string[]): Promise<void>;
  /** 作废在途查询并清空 paths/URL/error。 */
  clear(): void;
}

/** 创建一个头像数据源（用法与生命周期见 AvatarSource 注释）。 */
export function createAvatarSource(): AvatarSource {
  let token = 0; // 查询令牌：新一批开始/清空后，在途的旧响应作废
  const blobs = createAvatarBlobs();
  let paths = $state<Record<string, string | null>>({});
  let error = $state('');

  return {
    get paths() {
      return paths;
    },
    get urls() {
      return blobs.urls;
    },
    get error() {
      return error;
    },
    clear() {
      token++;
      paths = {};
      error = '';
      blobs.clear();
    },
    async load(uids: string[]) {
      const mine = ++token;
      let got: Record<string, string | null>;
      try {
        got = await api.queryAvatars(uids);
      } catch (e) {
        if (mine !== token) return; // 已被更新的查询/清空取代：过期失败不落状态
        paths = {};
        error = String(e);
        blobs.clear();
        return;
      }
      if (mine !== token) return; // 期间已换批/清空：过期结果不写不装
      error = '';
      paths = got;
      // 只有确实找到图的条目才交给加载器；无映射/图片不在包内的留在占位。
      const wanted: Record<string, string> = {};
      for (const [uid, p] of Object.entries(got)) if (p) wanted[uid] = p;
      void blobs.load(wanted); // 覆盖式装载：空表也会回收上一批（load({}) 先 clear）
    },
  };
}
