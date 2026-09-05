import type { LogLine } from '../types';

const MAX = 10_000;
export const LEVELS = ['DEBUG', 'INFO', 'WARNING', 'ERROR', 'CRITICAL'] as const;

/** 日志环形缓冲 + 过滤条件（对等 LogTab 的 log_store + _filter_log） */
export const logStore = $state({
  lines: [] as LogLine[],
  level: 'INFO' as string,
  only: false,
});

export function appendLogBatch(batch: LogLine[]) {
  logStore.lines.push(...batch);
  if (logStore.lines.length > MAX) {
    logStore.lines.splice(0, logStore.lines.length - MAX);
  }
}

export function clearDisplay() {
  logStore.lines = [];
}

/** 按级别阈值或「仅该级别」过滤 */
export function filterLines(lines: LogLine[], level: string, only: boolean): LogLine[] {
  const want = LEVELS.indexOf(level as (typeof LEVELS)[number]);
  return lines.filter((l) => {
    const cur = LEVELS.indexOf(l.level as (typeof LEVELS)[number]);
    return only ? cur === want : cur >= want;
  });
}
