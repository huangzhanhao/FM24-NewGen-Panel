import { listen } from '@tauri-apps/api/event';
import { api } from '../api';
import type { AppSnapshot, ProgressPayload } from '../types';
import { appendLogBatch } from './log.svelte';

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
/** 订阅后端事件（App 挂载时调用一次） */
export async function initListeners(onFinished: (r: string) => Promise<void>) {
  if (inited) return;
  inited = true;
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
  await refreshSnapshot();
}
