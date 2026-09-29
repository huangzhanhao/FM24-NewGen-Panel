/** 第 15 列口径的单点断言：isStaffRoster / basedLabel / basedValue。
 *  extStaff（拓展职员）名单的「所在地」列实为「偏好工作」——判据按后端结构化协议码
 *  formatCode，不依赖中文展示文案；四个消费点（名单表/单人替换/队列坞/球员源卡）共用。
 *  改动 appStore 这组 getter 或 basedValue 前，先看这里锁住的三种口径。 */
import { afterEach, describe, expect, it, vi } from 'vitest';

// 与 app.store.test.ts 同款：app.svelte.ts 顶层 import 了 tauri 的 listen/invoke（模块副作用），mock 掉即可。
const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen }));

import { appStore, basedValue } from './app.svelte';
import { t } from '../text';
import type { PlayerRecord, RtfScanResult } from '../types';

/** 构造扫描摘要：测试只关心 formatCode，其余字段以 as 断言放宽。 */
const roster = (formatCode: string): RtfScanResult =>
  ({ total: 3, newgan: 0, detailed: formatCode !== 'basic', format: 'rtf', formatCode, language: 'en' } as RtfScanResult);
/** 构造 detailed：测试只关心 based / preferredJob 两个字段的取舍。 */
const detailed = (over: { preferredJob?: string; based?: string } = {}): PlayerRecord['detailed'] =>
  ({ preferredJob: '', based: '', ...over } as PlayerRecord['detailed']);

describe('第 15 列口径（isStaffRoster/basedLabel/basedValue 单点）', () => {
  afterEach(() => {
    appStore.lastRtfScan = null;
  });

  it('extStaff：职员口径——表头偏好工作、取值读 preferredJob', () => {
    appStore.lastRtfScan = roster('extStaff');
    expect(appStore.isStaffRoster).toBe(true);
    expect(appStore.basedLabel).toBe(t.table.colJob);
    expect(basedValue(detailed({ preferredJob: '前台', based: '伦敦' }))).toBe('前台');
  });

  it('球员名单：表头所在地、取值读 based', () => {
    appStore.lastRtfScan = roster('extPlayer');
    expect(appStore.isStaffRoster).toBe(false);
    expect(appStore.basedLabel).toBe(t.table.colBased);
    expect(basedValue(detailed({ preferredJob: '前台', based: '伦敦' }))).toBe('伦敦');
  });

  it('无扫描摘要：默认球员口径；detailed 缺失时取空串', () => {
    appStore.lastRtfScan = null;
    expect(appStore.isStaffRoster).toBe(false);
    expect(appStore.basedLabel).toBe(t.table.colBased);
    expect(basedValue(null)).toBe('');
  });
});
