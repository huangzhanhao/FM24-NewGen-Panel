/**
 * TypeScript 类型定义 —— 前后端的「数据契约」。
 * 每个接口对应 Rust 侧的一个结构体（字段名为驼峰，与 Rust 的 camelCase 序列化一致）。
 * interface 只在编译期做类型检查、不产生运行时代码，作用是字段名写错时能提前发现。
 */

/** 拓展名单才有的额外信息（对应 Rust DetailedInfo），承载第 8-18 列及用户自定义列全部原文。 */
export interface DetailedInfo {
  skinCode: string; avatar: string; isNewgan: string; club: string;
  age: string; height: string;
  based: string;        // 所在地/俱乐部赛事级别（仅拓展球员）
  preferredJob: string; // 偏好工作（仅拓展职员）
  ca: string; pa: string;
  rca: string;          // 推荐能力 R.C.A（第 18 列，球员/职员都有；职员表头作 NP R.C.A）
  gender: string;       // 性别原文（第 11 列，球员/职员都有）
  extraFields: string[]; // 用户自定义列原文（第 19 列起）
}

/** 一个球员（对应 Rust 的 PlayerRecord）。detailed 可选，仅拓展名单有值。 */
export interface PlayerRecord {
  uid: string; primaryNat: string; secNat: string; name: string;
  hairLength: string; hairColor: string; ethnicityCode: string;
  detailed?: DetailedInfo;
}

/** 一条头像映射（对应 Rust 的 MappingEntry）。 */
export interface MappingEntry { uid: string; ethnicity: string; image: string; }

/** 一个头像包（组）的摘要（对应 Rust 的 GroupInfo）。 */
export interface GroupInfo { imgDir: string; rtf: string; }

/** 「检查完整性」结果（对应 Rust 的 IntegrityResult）。
 * standard：命中的标准号（2=14 种族 / 3=细分…），1=图片直铺，0=无法识别。 */
export interface IntegrityResult {
  standard: number;
  status: 'complete' | 'fixable' | 'unknown';
  missing: string[];
  extra: string[];   // 包内多出、未在标准 FolderList 配置的目录（不参与抽图）
  found: number;
  expectedTotal: number;
}

/** 「扫描头像」返回摘要（对应 Rust 的 ImageScanResult）。 */
export interface ImageScanResult { total: number; standard: number; }

/** 「扫描RTF」返回摘要（对应 Rust 的 RtfScanResult）。 */
export interface RtfScanResult { total: number; newgan: number; detailed: boolean; format: string; language: string; }

/** 当前组 config.xml 的路径与存在性（对应 Rust 的 ConfigXmlInfo）。 */
export interface ConfigXmlInfo { path: string; exists: boolean; }

/** 应用全局快照（对应 Rust 的 AppSnapshot）；前端大部分界面基于它渲染。 */
export interface AppSnapshot {
  profiles: string[]; currentProfile: string; groups: GroupInfo[];
  currentGroup: string | null; currentRtf: string;
  replacing: boolean; configXml: ConfigXmlInfo | null; logPath: string;
}

/** 进度事件载荷（对应 Rust 的 ProgressPayload）。 */
export interface ProgressPayload { status: string; value: number; }

/** 发起替换的参数（对应 Rust 的 ReplaceArgs）；mode 用字面量联合限定三个合法值（协议值）。 */
export interface ReplaceArgs {
  mode: 'Preserve' | 'Overwrite' | 'Generate';
  filterNewgan: boolean; allowDuplicates: boolean; saveBackup: boolean;
}

/** 名单分页查询参数（对应 Rust 的 PlayerQuery）。
 * newgan：随机人筛选（null=任意，true=仅随机人，false=仅真实球员）。 */
export interface PlayerQuery {
  offset: number; limit: number; search: string; sortKey: string; sortDesc: boolean;
  newgan: boolean | null;
}

/** 通用分页结果（对应 Rust 的 PageResult<T>）；<T> 是泛型，使用时传入具体条目类型。 */
export interface PageResult<T> { total: number; items: T[]; }

/** 单人预览（对应 Rust 的 PlayerPreview）；三个字段都可能为 null（后端 Option 序列化为 null）。 */
export interface PlayerPreview {
  record: PlayerRecord | null; mapping: MappingEntry | null; imagePath: string | null;
}

/** 「读取快照」返回摘要（对应 Rust 的 SnapshotInfo）。 */
export interface SnapshotInfo { total: number; }

/** 快照预览分页查询参数（对应 Rust 的 SnapshotQuery）；search 按 UID 子串筛选，空串=不筛选。 */
export interface SnapshotQuery { offset: number; limit: number; search: string; }

/** 快照预览的一行（对应 Rust 的 SnapshotEntry）；imagePath 为 null 表示头像包内找不到对应图片。 */
export interface SnapshotEntry { uid: string; relPath: string; imagePath: string | null; }

/** 一行日志（对应 Rust 的 LogLine）。 */
export interface LogLine { ts: string; level: string; module: string; message: string; }
