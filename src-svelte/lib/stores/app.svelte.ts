/**
 * 全局应用状态与后端事件订阅：前端与后端交互的中枢。
 * listen 订阅后端 emit 的事件（与命令 invoke 相反：事件由后端主动推送）。
 *
 * 状态用 $state 声明；命令返回的快照整体替换 appStore.snapshot。
 * 事件订阅是进程级副作用，initListeners 由 App 挂载时调用一次（内部用 inited 防重复）。
 */
import { listen } from '@tauri-apps/api/event';
import { api } from '../api';
import type { AppSnapshot, ProgressPayload, RtfScanResult } from '../types';
import { appendLogBatch, clearDisplay, logStore } from './log.svelte';

/** 全局应用状态（runes，响应式）。snapshot 由命令返回值整体替换。 */
export const appStore = $state({
  snapshot: null as AppSnapshot | null, // 后端回传的全局快照（真实激活态；初始为 null，启动后填充）
  /** 浏览态预览快照：下拉选择存档但未点「激活」时的展示数据（null = 无浏览态）。
   *  展示类组件（侧栏/当前任务卡）优先用它；操作类组件（替换执行等）始终用 snapshot。 */
  profilePreview: null as AppSnapshot | null,
  /** 下拉选中的存档名（浏览态）。与 snapshot.currentProfile 不同 = 正在浏览未激活的存档。 */
  selectedProfile: 'No Profile',
  progress: { status: '', value: 0 },   // 当前替换进度（状态文字 + 百分比）
  lastResult: '' as 'finished' | 'cancelled' | 'failed' | '', // 最近一次替换结果
  rosterVersion: 0,                     // 名单预览版本号：名单缓存被填充/清空时 +1（「扫描RTF」成功、替换结束），触发 PlayersTable 重载
  /** 最近一次解析名单的摘要（null=手上没有名单）。两条来源：「扫描RTF」成功、替换成功
   *  （摘要随 replace:finished 回传，无需用户再点一次扫描）。名单预览/单人微调据此判断名单格式
   *  ——拓展格式才有「随机人」筛选，且 format 含「职员」时第 15 列是偏好工作。 */
  lastRtfScan: null as RtfScanResult | null,
  /** 名单缓存可读标记：后端名单缓存里有没有数据。对应快照预览的 snapshotReadPath。
   *  两处会填充后端缓存：「扫描RTF」成功 → true；替换结束 → 成功为 true、失败/取消为 false
   *  （后端失败/取消时会把缓存写回空表）。名单预览据此决定是否渲染工具条。 */
  rosterReady: false,
  /** 结果区当前 Tab：'table'=名单预览 / 'viewer'=单人微调 / 'snapshot'=快照预览。
   *  放全局态：「读取快照」成功后要跨组件（当前任务卡 → 结果区）切换到快照预览。 */
  resultsView: 'table' as 'table' | 'viewer' | 'snapshot',
  /** 最近一次「读取快照」的快照文件路径（null=尚未读取过）。
   *  快照预览只认这份：当前展示的快照路径与它一致才拉数据，切档/切组后自动回到引导空态。
   *  两处会置它：「读取快照」成功；替换成功（后端把新 config.xml 同步到该档快照位置并刷新缓存）。 */
  snapshotReadPath: null as string | null,
  /** 快照预览版本号：「读取快照」成功、或替换成功（后端同步快照）后 +1，
   *  触发 SnapshotPreview 重载（同路径重读也生效）。 */
  snapshotVersion: 0,
});

/** 主动拉一次最新快照（调后端 get_app_state）。 */
export async function refreshSnapshot() {
  appStore.snapshot = await api.getAppState();
}

/** 直接用命令返回的快照替换本地状态（避免多一次 get_app_state 往返）。 */
export async function applySnapshot(s: AppSnapshot) {
  appStore.snapshot = s;
}

// inited 是一个普通模块变量（非响应式），用于保证 initListeners 只真正执行一次。
let inited = false;
/** 从 Rust 环形缓冲回填当前过滤条件下的日志历史。 */
export async function reloadLogs() {
  const page = await api.queryLogs(logStore.level, logStore.only, 0, 10_000);
  clearDisplay();               // 先清空旧的显示
  logStore.lines.push(...page.items); // 再填入后端返回的历史日志
}

/** 订阅后端事件（App 挂载时调用一次）。onFinished 是替换结束时的回调。 */
export async function initListeners(onFinished: (r: string) => Promise<void>) {
  if (inited) return; // 防止重复订阅
  inited = true;
  await refreshSnapshot(); // 先拉一次全局快照，界面立即有数据可渲染
  // 启动阶段的 log:batch（包含 “Starting Application v2”）在 WebView 导航开始前就已 emit，
  // 前端无法收到，因此先回填历史再订阅增量（顺序反了会出现重复行）。
  await reloadLogs();
  // 订阅进度事件：后端每进入一个阶段就 emit，这里把载荷存进 appStore.progress。
  await listen<ProgressPayload>('replace:progress', (e) => {
    appStore.progress = e.payload;
  });
  // 订阅完成事件：记下结果、刷新快照（replacing 会变 false）、调用回调弹窗提示。
  await listen<{ result: string; roster: RtfScanResult | null }>(
    'replace:finished',
    async (e) => {
      appStore.lastResult = e.payload.result as typeof appStore.lastResult;
      // 名单摘要随事件回传（后端替换时已经解析过名单）：前端不必再点一次「扫描RTF」就知道
      // 名单格式；失败/取消时为 null——后端已把名单缓存写回空表，摘要一并清掉。
      appStore.lastRtfScan = e.payload.roster;
      // 替换流程结束也会写后端名单缓存（见 commands.rs 的 start_replace）：
      // 成功 = 填上这份名单 → 名单预览可读；失败/取消 = 写回空表 → 回到引导空态。
      // 这两个标记必须在 refreshSnapshot 之前置好：名单预览要立刻按新缓存重载一次。
      appStore.rosterReady = e.payload.result === 'finished';
      appStore.rosterVersion++; // 触发名单预览重载（替换前的查询结果已过期）
      await refreshSnapshot();
      // 快照预览同样有两条写缓存路径（「读取快照」按钮 / 替换收尾的自动同步）：
      // 替换成功时后端已把新 config.xml 同步到该档快照位置并刷新快照缓存，
      // 这里补上与 SetupCard::readSnapshot 相同的两个标记——记下已读路径 + 触发预览重载。
      // 失败/取消不改 config.xml（快照保持原样），因此不动这两个标记。
      if (e.payload.result === 'finished') {
        const xmlPath = appStore.snapshot?.configXml?.path;
        if (xmlPath) {
          appStore.snapshotReadPath = xmlPath;
          appStore.snapshotVersion++;
        }
      }
      await onFinished(e.payload.result);
    },
  );
  // 订阅日志批量事件：把新日志追写到前端日志列表。
  await listen<import('../types').LogLine[]>('log:batch', (e) => {
    appendLogBatch(e.payload);
  });
}
