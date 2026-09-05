// 【新手导读】前端调用后端的「统一入口」。把 Tauri 的 invoke 封装成一组语义化方法。
//
// 【关键概念：invoke】invoke("命令名", 参数对象) 会异步调用 Rust 后端带 #[tauri::command] 的同名函数，
// 返回一个 Promise：后端 Ok → resolve（拿到返回值）；后端 Err(字符串) → reject（抛异常，用 try/catch 接）。
// 命令名和参数名必须与 Rust 端一致（Rust 蛇形参数在 invoke 里写驼峰，Tauri 会自动对应）。
//
// 【为何集中封装】组件里只需写 api.createProfile(name)，不必重复写 invoke 字符串，也方便类型提示。
import { invoke } from '@tauri-apps/api/core';
import type {
  AppSnapshot, IntegrityResult, LogLine, PageResult, PlayerPreview,
  PlayerQuery, PlayerRecord, ReplaceArgs,
} from './types'; // import type 仅导入类型，编译后会被完全抹去（不产生运行时代码）

// invoke<返回类型>('命令名', { 参数 }) —— 尖括号里的是告诉 TS 这个 Promise resolve 出什么类型。
export const api = {
  getAppState: () => invoke<AppSnapshot>('get_app_state'),
  createProfile: (name: string) => invoke<AppSnapshot>('create_profile', { name }),
  deleteProfile: (name: string) => invoke<AppSnapshot>('delete_profile', { name }),
  switchProfile: (name: string) => invoke<AppSnapshot>('switch_profile', { name }),
  selectImageDirectory: (path: string) => invoke<AppSnapshot>('select_image_directory', { path }),
  setCurrentRtf: (path: string) => invoke<AppSnapshot>('set_current_rtf', { path }),
  // 注意：参数对象里写 imgDir（驼峰），Rust 端对应 img_dir（蛇形），Tauri 自动映射。
  setCurrentGroup: (imgDir: string) => invoke<AppSnapshot>('set_current_group', { imgDir }),
  deleteGroup: (imgDir: string) => invoke<AppSnapshot>('delete_group', { imgDir }),
  validateImageDirs: (imgDir: string) => invoke<string[]>('validate_image_dirs', { imgDir }),
  // 检查完整性：识别三套头像包标准并给出可补全的缺失目录（仅“检查完整性”按钮使用）。
  checkIntegrity: (imgDir: string) => invoke<IntegrityResult>('check_integrity', { imgDir }),
  createMissingDirs: (imgDir: string, names: string[]) =>
    invoke<void>('create_missing_dirs', { imgDir, names }), // void 表示不关心返回值
  // startReplace 的参数包在 { args: params } 里，对应 Rust 端函数参数名 args: ReplaceArgs。
  startReplace: (params: ReplaceArgs) => invoke<void>('start_replace', { args: params }),
  cancelReplace: () => invoke<void>('cancel_replace'),
  queryPlayers: (query: PlayerQuery) => invoke<PageResult<PlayerRecord>>('query_players', { query }),
  previewPlayer: (uid: string) => invoke<PlayerPreview>('preview_player', { uid }),
  replaceSingleFace: (uid: string, imagePath: string, saveBackup: boolean) =>
    invoke<boolean>('replace_single_face', { uid, imagePath, saveBackup }),
  // readImage 返回 ArrayBuffer（二进制图片字节），前端再转成 blob URL 显示。
  readImage: (path: string) => invoke<ArrayBuffer>('read_image', { path }),
  queryLogs: (level: string, only: boolean, offset: number, limit: number) =>
    invoke<{ total: number; items: LogLine[] }>('query_logs', { level, only, offset, limit }),
};
