// 【新手导读】全局应用状态 + 后端事件订阅。这是前端与后端交互的“中枢”。
// listen 用于订阅 Tauri 后端 emit 发出的事件（与命令 invoke 不同：事件是后端主动推送）。
import { listen } from '@tauri-apps/api/event';
import { api } from '../api';
import type { AppSnapshot, ProgressPayload } from '../types';
import { appendLogBatch, clearDisplay, logStore } from './log.svelte';

/** 全局应用状态（runes）。snapshot 由命令返回值整体替换。 */
// $state 创建响应式对象：任何组件读 appStore.snapshot，当它变化时那些组件会自动重渲染。
export const appStore = $state({
  snapshot: null as AppSnapshot | null, // 后端回传的全局快照（初始为 null，启动后填充）
  progress: { status: '', value: 0 },   // 当前替换进度（状态文字 + 百分比）
  lastResult: '' as 'finished' | 'cancelled' | 'failed' | '', // 最近一次替换结果
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
