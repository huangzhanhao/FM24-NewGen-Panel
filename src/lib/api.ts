import { invoke } from '@tauri-apps/api/core';
import type {
  AppSnapshot, LogLine, PageResult, PlayerPreview,
  PlayerQuery, PlayerRecord, ReplaceArgs,
} from './types';

export const api = {
  getAppState: () => invoke<AppSnapshot>('get_app_state'),
  createProfile: (name: string) => invoke<AppSnapshot>('create_profile', { name }),
  deleteProfile: (name: string) => invoke<AppSnapshot>('delete_profile', { name }),
  switchProfile: (name: string) => invoke<AppSnapshot>('switch_profile', { name }),
  selectImageDirectory: (path: string) => invoke<AppSnapshot>('select_image_directory', { path }),
  setCurrentRtf: (path: string) => invoke<AppSnapshot>('set_current_rtf', { path }),
  setCurrentGroup: (imgDir: string) => invoke<AppSnapshot>('set_current_group', { imgDir }),
  deleteGroup: (imgDir: string) => invoke<AppSnapshot>('delete_group', { imgDir }),
  validateImageDirs: (imgDir: string) => invoke<string[]>('validate_image_dirs', { imgDir }),
  createMissingDirs: (imgDir: string, names: string[]) =>
    invoke<void>('create_missing_dirs', { imgDir, names }),
  startReplace: (params: ReplaceArgs) => invoke<void>('start_replace', { args: params }),
  cancelReplace: () => invoke<void>('cancel_replace'),
  queryPlayers: (query: PlayerQuery) => invoke<PageResult<PlayerRecord>>('query_players', { query }),
  previewPlayer: (uid: string) => invoke<PlayerPreview>('preview_player', { uid }),
  replaceSingleFace: (uid: string, imagePath: string, saveBackup: boolean) =>
    invoke<boolean>('replace_single_face', { uid, imagePath, saveBackup }),
  readImage: (path: string) => invoke<ArrayBuffer>('read_image', { path }),
  queryLogs: (level: string, only: boolean, offset: number, limit: number) =>
    invoke<{ total: number; items: LogLine[] }>('query_logs', { level, only, offset, limit }),
};
