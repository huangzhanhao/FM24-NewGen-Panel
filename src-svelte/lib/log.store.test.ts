import { describe, expect, it } from 'vitest';
import { appendLogBatch, filterLines, logStore } from './stores/log.svelte';

describe('logStore', () => {
  it('超出上限时保留最新一万条', () => {
    const batch = Array.from({ length: 6000 }, (_, i) => ({
      ts: '', level: 'INFO', module: '', message: String(i),
    }));
    appendLogBatch(batch);
    appendLogBatch(batch);
    expect(logStore.lines.length).toBe(10_000);
    expect(logStore.lines[0].message).toBe('2000');
  });

  it('过滤：阈值与仅该级别', () => {
    const lines = [
      { ts: '', level: 'DEBUG', module: '', message: '' },
      { ts: '', level: 'INFO', module: '', message: '' },
      { ts: '', level: 'ERROR', module: '', message: '' },
    ];
    expect(filterLines(lines, 'INFO', false).length).toBe(2);
    expect(filterLines(lines, 'ERROR', true).length).toBe(1);
  });
});
