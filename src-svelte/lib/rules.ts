/**
 * 规则文件（data/rules/standard2.json / standard3.json）的纯数据层：
 * 解析校验、类型收窄、目录引用统计与配置体检。
 *
 * 这里只放不依赖 Svelte/Tauri 的纯函数，供 RuleEditor 的交互层复用，也方便单测覆盖。
 * 保存仍走 api.saveRuleFile（后端按各标准 schema 做权威校验），前端不重复实现业务校验。
 */

export type RuleStandard = 'standard2' | 'standard3';

/** FM 种族码固定范围。 */
export const RACE_CODES = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '10'] as const;
export type RaceCode = (typeof RACE_CODES)[number];

type JsonRecord = Record<string, unknown>;

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function readStringArray(value: unknown, where: string): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.some((v) => typeof v !== 'string')) {
    throw new Error(`${where} 应为字符串数组`);
  }
  return value as string[];
}

function readStringRecord(value: unknown, where: string): Record<string, string> {
  if (value === undefined) return {};
  if (!isRecord(value) || Object.values(value).some((v) => typeof v !== 'string')) {
    throw new Error(`${where} 应为「键 → 字符串」对象`);
  }
  return value as Record<string, string>;
}

// ===================== 标准二 ========================================================================================

export interface S2CodeRule {
  prefer: string[];
  fallback: string;
}

export interface S2Doc {
  FolderList: string[];
  Ethnics: Record<string, string>;
  CodeRules: Record<string, S2CodeRule>;
  [key: string]: unknown;
}

function normalizeS2(raw: unknown): S2Doc {
  if (!isRecord(raw)) throw new Error('规则文件顶层应为 JSON 对象');
  const codeRulesRaw = raw.CodeRules;
  if (codeRulesRaw !== undefined && !isRecord(codeRulesRaw)) {
    throw new Error('CodeRules 应为「种族码 → 规则」对象');
  }
  const codeRules: Record<string, S2CodeRule> = {};
  for (const [code, value] of Object.entries(codeRulesRaw ?? {})) {
    if (!isRecord(value)) throw new Error(`CodeRules[${code}] 应为对象`);
    codeRules[code] = {
      prefer: readStringArray(value.prefer, `CodeRules[${code}].prefer`),
      fallback: typeof value.fallback === 'string' ? value.fallback : '',
    };
  }
  return {
    ...raw,
    FolderList: readStringArray(raw.FolderList, 'FolderList'),
    Ethnics: readStringRecord(raw.Ethnics, 'Ethnics'),
    CodeRules: codeRules,
  };
}

// ===================== 标准三 ========================================================================================

export type MappingKind = 'default' | 'directly' | 'random' | 'randomWithWeight';

export type MappingRule =
  | 'default'
  | { typeOfMapping: 'directly'; directory: string }
  | { typeOfMapping: 'random'; directory: string[] }
  | {
      typeOfMapping: 'randomWithWeight';
      directory: { ethnicities: string[]; weights: number[] };
    };

export interface S3Doc {
  FolderList: string[];
  Ethnics: Record<string, string>;
  DefaultCodes: Record<string, number[]>;
  EthnicityMappingRules: Record<string, Record<string, MappingRule>>;
  Version: number;
  Fallback?: 'random' | 'skip';
  FolderMappingFemale?: Record<string, string[]>;
  FolderMappingStaff?: Record<string, string[]>;
  FolderMappingStaffFemale?: Record<string, string>;
  [key: string]: unknown;
}

function normalizeS3(raw: unknown): S3Doc {
  if (!isRecord(raw)) throw new Error('规则文件顶层应为 JSON 对象');

  const defaultCodesRaw = raw.DefaultCodes;
  if (defaultCodesRaw !== undefined && !isRecord(defaultCodesRaw)) {
    throw new Error('DefaultCodes 应为「国籍码 → 种族码数组」对象');
  }
  const defaultCodes: Record<string, number[]> = {};
  for (const [code, value] of Object.entries(defaultCodesRaw ?? {})) {
    if (!Array.isArray(value) || value.some((v) => typeof v !== 'number')) {
      throw new Error(`DefaultCodes[${code}] 应为数字数组`);
    }
    defaultCodes[code] = value as number[];
  }

  const blocksRaw = raw.EthnicityMappingRules;
  if (blocksRaw !== undefined && !isRecord(blocksRaw)) {
    throw new Error('EthnicityMappingRules 应为「块键 → 码表」对象');
  }
  const blocks: Record<string, Record<string, MappingRule>> = {};
  for (const [block, value] of Object.entries(blocksRaw ?? {})) {
    if (!isRecord(value)) throw new Error(`EthnicityMappingRules[${block}] 应为对象`);
    blocks[block] = value as Record<string, MappingRule>;
  }

  if (raw.Version !== undefined && typeof raw.Version !== 'number') {
    throw new Error('Version 应为数字');
  }
  const fallback = raw.Fallback;
  if (fallback !== undefined && fallback !== 'random' && fallback !== 'skip') {
    throw new Error('Fallback 应为 "random" 或 "skip"');
  }
  return {
    ...raw,
    FolderList: readStringArray(raw.FolderList, 'FolderList'),
    Ethnics: readStringRecord(raw.Ethnics, 'Ethnics'),
    DefaultCodes: defaultCodes,
    EthnicityMappingRules: blocks,
    Version: typeof raw.Version === 'number' ? raw.Version : 4,
    ...(fallback === undefined ? {} : { Fallback: fallback }),
  };
}

/** 按标准解析原始 JSON 文本；失败抛带中文定位信息的 Error（由调用方决定提示或进源码模式）。 */
export function parseRuleFile(name: RuleStandard, text: string): S2Doc | S3Doc {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch (e) {
    throw new Error(`JSON 格式错误：${e instanceof Error ? e.message : String(e)}`);
  }
  return name === 'standard2' ? normalizeS2(raw) : normalizeS3(raw);
}

/** 结构化文档 → 落盘 JSON 文本（两空格缩进，与后端 json_store::save 的 pretty 格式一致）。 */
export function formatDoc(doc: S2Doc | S3Doc): string {
  return JSON.stringify(doc, null, 2);
}

// ===================== 规则形态 ======================================================================================

/** 判定一条标准三规则的形态；无法识别的结构返回 null（UI 提示改源码）。 */
export function mappingKind(rule: unknown): MappingKind | null {
  if (rule === 'default') return 'default';
  if (!isRecord(rule) || typeof rule.typeOfMapping !== 'string') return null;
  const dir = rule.directory;
  switch (rule.typeOfMapping) {
    case 'directly':
      return typeof dir === 'string' ? 'directly' : null;
    case 'random':
      return Array.isArray(dir) && dir.every((d) => typeof d === 'string') ? 'random' : null;
    case 'randomWithWeight':
      return isRecord(dir) &&
        Array.isArray(dir.ethnicities) &&
        dir.ethnicities.every((d) => typeof d === 'string') &&
        Array.isArray(dir.weights) &&
        dir.weights.every((w) => typeof w === 'number') &&
        dir.ethnicities.length === dir.weights.length
        ? 'randomWithWeight'
        : null;
    default:
      return null;
  }
}

/** 一条规则引用的全部目录（default 不引用具体目录）。 */
export function ruleDirs(rule: unknown): string[] {
  if (mappingKind(rule) === 'directly') return [directDir(rule)];
  if (mappingKind(rule) === 'random') return randomDirs(rule);
  if (mappingKind(rule) === 'randomWithWeight') return weightedList(rule)?.ethnicities ?? [];
  return [];
}

/** directly 形态的目录（非该形态返回空串）。返回的是原数组里的对象，可直接改。 */
export function directDir(rule: unknown): string {
  return mappingKind(rule) === 'directly' ? (rule as { directory: string }).directory : '';
}

/** random 形态的目录数组（非该形态返回空数组）。 */
export function randomDirs(rule: unknown): string[] {
  return mappingKind(rule) === 'random' ? (rule as { directory: string[] }).directory : [];
}

/** randomWithWeight 形态的目录/权重（非该形态返回 null）。 */
export function weightedList(rule: unknown): { ethnicities: string[]; weights: number[] } | null {
  return mappingKind(rule) === 'randomWithWeight'
    ? (rule as { directory: { ethnicities: string[]; weights: number[] } }).directory
    : null;
}

/** 块键类别：_DEFAULT_ / 国籍码块 / 族群目录块 / 未知（保存时后端会拦）。 */
export function blockKind(
  doc: S3Doc,
  key: string,
): 'default' | 'nation' | 'folder' | 'unknown' {
  if (key === '_DEFAULT_') return 'default';
  if (Object.hasOwn(doc.Ethnics, key)) return 'nation';
  if (doc.FolderList.includes(key)) return 'folder';
  return 'unknown';
}

/**
 * 目录引用统计：目录名 → 被引用次数。
 * 口径覆盖会被后端校验的引用点（Ethnics / CodeRules / 规则块键 / 规则目录 / 类别映射），
 * 用于目录标签上的「N 处引用」与删除前确认；不追求与运行时逐一等价，只做破坏性提示。
 */
export function folderUsageMap(name: RuleStandard, doc: S2Doc | S3Doc): Map<string, number> {
  const uses = new Map<string, number>();
  const hit = (dir: string) => {
    if (!dir) return;
    uses.set(dir, (uses.get(dir) ?? 0) + 1);
  };

  if (name === 'standard2') {
    const d = doc as S2Doc;
    for (const dir of Object.values(d.Ethnics)) hit(dir);
    for (const rule of Object.values(d.CodeRules)) {
      for (const dir of rule.prefer) hit(dir);
      hit(rule.fallback);
    }
    return uses;
  }

  const d = doc as S3Doc;
  for (const dir of Object.values(d.Ethnics)) hit(dir);
  for (const [key, block] of Object.entries(d.EthnicityMappingRules)) {
    hit(key); // 族群块键本身也是目录
    for (const rule of Object.values(block)) for (const dir of ruleDirs(rule)) hit(dir);
  }
  for (const dirs of Object.values(d.FolderMappingFemale ?? {})) dirs.forEach(hit);
  for (const regions of Object.values(d.FolderMappingStaff ?? {})) {
    for (const region of regions) {
      hit(`${region}/male`);
      hit(`${region}/female`);
    }
  }
  for (const region of Object.values(d.FolderMappingStaffFemale ?? {})) {
    hit(`${region}/male`);
    hit(`${region}/female`);
  }
  return uses;
}

// ===================== 配置体检 ======================================================================================

export type HealthIssue =
  | { kind: 'emptyFolders' }
  | { kind: 'emptyEthnics' }
  | { kind: 'dupFolders'; count: number }
  | { kind: 'badEthnics'; count: number }
  | { kind: 'badRefs'; count: number }
  | { kind: 'missingDefault' }
  | { kind: 'badVersion'; version: number }
  | { kind: 'orphanDefaultCodes'; count: number };

/** 结构性体检（与后端 validate 的关键不变量对齐，用于概览页跳转修复）。 */
export function healthIssues(name: RuleStandard, doc: S2Doc | S3Doc): HealthIssue[] {
  const issues: HealthIssue[] = [];
  const folderSet = new Set(doc.FolderList);
  if (doc.FolderList.length === 0) issues.push({ kind: 'emptyFolders' });
  if (Object.keys(doc.Ethnics).length === 0) issues.push({ kind: 'emptyEthnics' });
  const dupCount = doc.FolderList.length - folderSet.size;
  if (dupCount > 0) issues.push({ kind: 'dupFolders', count: dupCount });

  const badEthnics = Object.values(doc.Ethnics).filter((dir) => !folderSet.has(dir)).length;
  if (badEthnics > 0) issues.push({ kind: 'badEthnics', count: badEthnics });

  let badRefs = 0;
  if (name === 'standard2') {
    for (const rule of Object.values((doc as S2Doc).CodeRules)) {
      badRefs += rule.prefer.filter((d) => !folderSet.has(d)).length;
      if (rule.fallback && !folderSet.has(rule.fallback)) badRefs += 1;
    }
  } else {
    const d = doc as S3Doc;
    if (d.Version !== 4) issues.push({ kind: 'badVersion', version: d.Version });
    if (!Object.hasOwn(d.EthnicityMappingRules, '_DEFAULT_')) {
      issues.push({ kind: 'missingDefault' });
    }
    for (const [key, block] of Object.entries(d.EthnicityMappingRules)) {
      if (blockKind(d, key) === 'unknown') badRefs += 1;
      for (const rule of Object.values(block)) {
        badRefs += ruleDirs(rule).filter((dir) => !folderSet.has(dir)).length;
      }
    }
    // 类别映射是结构化界面不写的透传数据，但后端同样会校验：一并纳入体检，
    // 避免「概览说没问题、保存时才被目录引用拦下」。
    for (const dirs of Object.values(d.FolderMappingFemale ?? {})) {
      for (const dir of dirs) {
        if (!folderSet.has(dir) || !dir.startsWith('WF/')) badRefs += 1;
      }
    }
    for (const regions of Object.values(d.FolderMappingStaff ?? {})) {
      for (const region of regions) {
        if (!folderSet.has(`${region}/male`)) badRefs += 1;
      }
    }
    const staffRegions = new Set(Object.values(d.FolderMappingStaff ?? {}).flat());
    for (const [src, dst] of Object.entries(d.FolderMappingStaffFemale ?? {})) {
      if (!staffRegions.has(src) || !staffRegions.has(dst)) badRefs += 1;
    }
    const orphan = Object.keys(d.DefaultCodes).filter((code) => !Object.hasOwn(d.Ethnics, code)).length;
    if (orphan > 0) issues.push({ kind: 'orphanDefaultCodes', count: orphan });
  }
  if (badRefs > 0) issues.push({ kind: 'badRefs', count: badRefs });
  return issues;
}

/** 按字母序排列的键列表（对象键顺序在编辑/删除后会漂移，展示时统一排序）。 */
export function sortedKeys(record: Record<string, unknown>): string[] {
  return Object.keys(record).sort((a, b) => a.localeCompare(b, 'en'));
}
