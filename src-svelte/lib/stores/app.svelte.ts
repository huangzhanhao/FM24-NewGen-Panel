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
  rosterVersion: 0,                     // 名单预览版本号：「扫描RTF」成功后 +1，触发 PlayersTable 重载
  /** 最近一次「扫描RTF」的摘要（null=尚未扫描过）。名单预览据此判断名单是否为拓展格式。 */
  lastRtfScan: null as RtfScanResult | null,
  /** 结果区当前 Tab：'table'=名单预览 / 'viewer'=单人微调 / 'snapshot'=快照预览。
   *  放全局态：「读取快照」成功后要跨组件（当前任务卡 → 结果区）切换到快照预览。 */
  resultsView: 'table' as 'table' | 'viewer' | 'snapshot',
  /** 最近一次「读取快照」的快照文件路径（null=尚未读取过）。
   *  快照预览只认这份：当前展示的快照路径与它一致才拉数据，切档/切组后自动回到引导空态。 */
  snapshotReadPath: null as string | null,
  /** 快照预览版本号：「读取快照」成功后 +1，触发 SnapshotPreview 重载（同路径重读也生效）。 */
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
  await listen<{ result: string }>('replace:finished', async (e) => {
    appStore.lastResult = e.payload.result as typeof appStore.lastResult;
    await refreshSnapshot();
    await onFinished(e.payload.result);
  });
  // 订阅日志批量事件：把新日志追写到前端日志列表。
  await listen<import('../types').LogLine[]>('log:batch', (e) => {
    appendLogBatch(e.payload);
  });
}
