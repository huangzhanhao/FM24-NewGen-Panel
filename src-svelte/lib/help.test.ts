import { describe, expect, it } from 'vitest';
import { HELP, helpEntries, helpSearchText } from './help';
import type { Lang } from './stores/ui.svelte';

const LANGS: Lang[] = ['zh-CN', 'zh-TW', 'en'];

describe('docs content', () => {
  it('三语条目 id 一一对应，避免切语言后导航错位', () => {
    for (const tab of ['tutorial', 'faq'] as const) {
      const base = HELP['zh-CN'][tab].map((e) => e.id);
      for (const lang of LANGS) expect(HELP[lang][tab].map((e) => e.id)).toEqual(base);
    }
  });

  it('每个页签的 id 唯一且内容非空', () => {
    for (const lang of LANGS) {
      for (const tab of ['tutorial', 'faq'] as const) {
        const ids = HELP[lang][tab].map((e) => e.id);
        expect(new Set(ids).size).toBe(ids.length);
        for (const entry of HELP[lang][tab]) {
          expect(entry.title.trim()).not.toBe('');
          expect(entry.blocks.length).toBeGreaterThan(0);
          expect(helpSearchText(entry).trim()).not.toBe('');
        }
      }
    }
  });

  it('教程覆盖当前主流程与新增工作台', () => {
    const ids = helpEntries('zh-CN', 'tutorial').map((e) => e.id);
    expect(ids).toEqual([
      'install',
      'fm-prep',
      'export-rtf',
      'sidebar-scope',
      'archive-workbench',
      'replace',
      'single',
      'ai',
      'fm-reload',
      'logs-data',
      'cache',
    ]);
  });

  it('FAQ 覆盖缓存、队列、备份与 AI 生图等高频问题', () => {
    const text = helpEntries('zh-CN', 'faq')
      .map(helpSearchText)
      .join('\n');
    for (const key of ['RTF', '缓存', '队列', '备份', 'AI', 'ComfyUI', 'No Profile', '权限']) {
      expect(text).toContain(key.toLowerCase());
    }
  });
});
