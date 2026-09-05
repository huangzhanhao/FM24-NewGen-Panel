import { listen } from '@tauri-apps/api/event';
import { api } from '../api';
import type { AppSnapshot, ProgressPayload } from '../types';
import { appendLogBatch, clearDisplay, logStore } from './log.svelte';

/** 全局应用状态（runes）。snapshot 由命令返回值整体替换。 */
export const appStore = $state({
  snapshot: null as AppSnapshot | null,
  progress: { status: '', value: 0 },
  lastResult: '' as 'finished' | 'cancelled' | 'failed' | '',
});

export async function refreshSnapshot() {
  appStore.snapshot = await api.getAppState();
}

export async function applySnapshot(s: AppSnapshot) {
  appStore.snapshot = s;
}

let inited = false;
/** 从 Rust 环形缓冲回填当前过滤条件下的日志历史 */
export async function reloadLogs() {
  const page = await api.queryLogs(logStore.level, logStore.only, 0, 10_000);
  clearDisplay();
  logStore.lines.push(...page.items);
}

/** 订阅后端事件（App 挂载时调用一次） */
export async function initListeners(onFinished: (r: string) => Promise<void>) {
  if (inited) return;
  inited = true;
  await refreshSnapshot();
  // 启动阶段的 log:batch（包含 “Starting Application v2”）在 WebView 导航开始前就已 emit，
  // 前端无法收到，因此先回填历史再订阅增量（顺序反了会出现重复行）
  await reloadLogs();
  await listen<ProgressPayload>('replace:progress', (e) => {
    appStore.progress = e.payload;
  });
  await listen<{ result: string }>('replace:finished', async (e) => {
    appStore.lastResult = e.payload.result as typeof appStore.lastResult;
    await refreshSnapshot();
    await onFinished(e.payload.result);
  });
  await listen<import('../types').LogLine[]>('log:batch', (e) => {
    appendLogBatch(e.payload);
  });
}
