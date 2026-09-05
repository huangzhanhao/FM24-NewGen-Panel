export interface DetailedInfo {
  skinCode: string; faceId: string; club: string;
  age: string; height: string; weight: string; isNewgan: string;
}

export interface PlayerRecord {
  uid: string; primaryNat: string; secNat: string; name: string;
  hairLength: string; hairColor: string; ethnicityCode: string;
  detailed?: DetailedInfo;
}

export interface MappingEntry { uid: string; ethnicity: string; image: string; }

export interface GroupInfo { imgDir: string; rtf: string; }

export interface AppSnapshot {
  profiles: string[]; currentProfile: string; groups: GroupInfo[];
  currentGroup: string | null; currentRtf: string;
  replacing: boolean; logPath: string;
}

export interface ProgressPayload { status: string; value: number; }

export interface ReplaceArgs {
  mode: 'Preserve' | 'Overwrite' | 'Generate';
  filterNewgan: boolean; allowDuplicates: boolean; saveBackup: boolean;
}

export interface PlayerQuery {
  offset: number; limit: number; search: string; sortKey: string; sortDesc: boolean;
}

export interface PageResult<T> { total: number; items: T[]; }

export interface PlayerPreview {
  record: PlayerRecord | null; mapping: MappingEntry | null; imagePath: string | null;
}

export interface LogLine { ts: string; level: string; module: string; message: string; }
