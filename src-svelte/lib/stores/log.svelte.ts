/**
 * 日志的前端显示状态与过滤工具。
 * 后端日志器另有一份环形缓冲（供 query_logs 回查）；这里维护的是「用于显示」的列表 + 过滤条件。
 */
import type { LogLine } from '../types';

const MAX = 10_000; // 前端显示上限（超出则从头部丢弃，与后端环形缓冲一致）
// 级别口径对齐 Rust log 门面（Trace/Debug/Info/Warn/Error，显示串沿用 v1 拼写 WARNING）。
// as const 把数组锁定为字面量类型（而不是宽泛的 string[]），便于类型推导。
export const LEVELS = ['TRACE', 'DEBUG', 'INFO', 'WARNING', 'ERROR'] as const;

/** 日志环形缓冲 + 过滤条件（对等旧版 LogTab 的 log_store + _filter_log）。
 * $state 让这个对象响应式：lines/level/only 变化时，日志面板会自动刷新。 */
export const logStore = $state({
  lines: [] as LogLine[],   // 当前显示的日志行
  level: 'INFO' as string,  // 过滤级别阈值
  only: false,              // true=仅该级别；false=该级别及以上
});

/** 追写一批日志（收到后端 log:batch 事件时调），并保证不超 MAX。 */
export function appendLogBatch(batch: LogLine[]) {
  logStore.lines.push(...batch);
  if (logStore.lines.length > MAX) {
    // 从头部删掉多出的条数，保留最新 MAX 条（与后端环形缓冲同语义）。
    logStore.lines.splice(0, logStore.lines.length - MAX);
  }
}

/** 清空显示（切换过滤条件重新回填前调）。 */
export function clearDisplay() {
  logStore.lines = [];
}

/** 按级别阈值或「仅该级别」过滤（与后端 logger.query 的过滤逻辑一致）。
 * 思路：把级别转为在 LEVELS 里的序号，序号越大级别越高；按 only 决定“等于”还是“大于等于”。 */
export function filterLines(lines: LogLine[], level: string, only: boolean): LogLine[] {
  const want = LEVELS.indexOf(level as (typeof LEVELS)[number]);
  return lines.filter((l) => {
    const cur = LEVELS.indexOf(l.level as (typeof LEVELS)[number]);
    return only ? cur === want : cur >= want;
  });
}
