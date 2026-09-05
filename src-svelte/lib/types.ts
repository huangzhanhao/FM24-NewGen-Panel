// 【新手导读】TypeScript 类型定义（interface）——前后端的「数据契约」。
// 这里的每个接口都对应 Rust commands.rs 里的一个结构体（字段名用驼峰，与 Rust 的 camelCase 序列化一致）。
// interface 只在编译期做类型检查，不会产生运行时代码；作用是让编辑器和编译器帮你提前发现字段名写错。

/// 详细名单才有的额外信息（对应 Rust 的 DetailedInfo）。
export interface DetailedInfo {
  skinCode: string; faceId: string; club: string;
  age: string; height: string; weight: string; isNewgan: string;
}

/// 一个球员（对应 Rust 的 PlayerRecord）。detailed? 的 ? 表示可选（可能为 undefined）。
export interface PlayerRecord {
  uid: string; primaryNat: string; secNat: string; name: string;
  hairLength: string; hairColor: string; ethnicityCode: string;
  detailed?: DetailedInfo;
}

/// 一条头像映射（对应 Rust 的 MappingEntry）。
export interface MappingEntry { uid: string; ethnicity: string; image: string; }

/// 一个头像包（组）的摘要（对应 Rust 的 GroupInfo）。
export interface GroupInfo { imgDir: string; rtf: string; }

/// 应用全局快照（对应 Rust 的 AppSnapshot）。前端大部分界面都基于它渲染。
export interface AppSnapshot {
  profiles: string[]; currentProfile: string; groups: GroupInfo[];
  currentGroup: string | null; currentRtf: string; // string | null 联合类型：可能是字符串或 null
  replacing: boolean; logPath: string;
}

/// 进度事件载荷（对应 Rust 的 ProgressPayload）。
export interface ProgressPayload { status: string; value: number; }

/// 发起替换的参数（对应 Rust 的 ReplaceArgs）。mode 用字面量联合限定三个合法值。
export interface ReplaceArgs {
  mode: 'Preserve' | 'Overwrite' | 'Generate';
  filterNewgan: boolean; allowDuplicates: boolean; saveBackup: boolean;
}

/// 名单分页查询参数（对应 Rust 的 PlayerQuery）。
export interface PlayerQuery {
  offset: number; limit: number; search: string; sortKey: string; sortDesc: boolean;
}

/// 通用分页结果（对应 Rust 的 PageResult<T>）。<T> 是泛型，使用时传入具体条目类型。
export interface PageResult<T> { total: number; items: T[]; }

/// 单人预览（对应 Rust 的 PlayerPreview）。三个字段都可能为 null（后端 Option 序列化为 null）。
export interface PlayerPreview {
  record: PlayerRecord | null; mapping: MappingEntry | null; imagePath: string | null;
}

/// 一行日志（对应 Rust 的 LogLine）。
export interface LogLine { ts: string; level: string; module: string; message: string; }
