/**
 * 前端调用后端的统一入口：把 Tauri 的 invoke 封装成一组语义化方法，组件里只需写 api.createProfile(name)，不必重复写 invoke 字符串，也方便类型提示。
 *
 * invoke("命令名", 参数对象) 异步调用 Rust 后端带 #[tauri::command] 的同名函数：
 * 返回一个 Promise：后端 Ok → resolve（拿到返回值）；Err(字符串) → reject（抛异常，用 try/catch 接）。
 * 命令名与参数名必须与 Rust 端一致：Rust 蛇形参数在 invoke 里写驼峰，Tauri 自动映射。
 */
import { invoke } from '@tauri-apps/api/core';
import type {
  AppSnapshot, ImageScanResult, IntegrityResult, LogLine, PageResult, PlayerPreview,
  PlayerQuery, PlayerRecord, PreflightResult, ReplaceArgs, RtfScanResult, SnapshotEntry, SnapshotInfo, SnapshotQuery,
} from '../types'; // 仅导入类型，编译后完全抹去（不产生运行时代码）

/** 全部后端命令的前端封装；每个方法的泛型参数是该命令 resolve 出的返回类型。 */
export const api = {
  getAppState: () => invoke<AppSnapshot>('get_app_state'),
  createProfile: (name: string) => invoke<AppSnapshot>('create_profile', { name }),
  deleteProfile: (name: string) => invoke<AppSnapshot>('delete_profile', { name }),
  switchProfile: (name: string) => invoke<AppSnapshot>('switch_profile', { name }),
  selectImageDirectory: (path: string) => invoke<AppSnapshot>('select_image_directory', { path }),
  setCurrentRtf: (path: string) => invoke<AppSnapshot>('set_current_rtf', { path }),
  // 参数写 imgDir（驼峰），对应 Rust 端 img_dir（蛇形），Tauri 自动映射。
  setCurrentGroup: (imgDir: string) => invoke<AppSnapshot>('set_current_group', { imgDir }),
  deleteGroup: (imgDir: string) => invoke<AppSnapshot>('delete_group', { imgDir }),
  validateImageDirs: (imgDir: string) => invoke<string[]>('validate_image_dirs', { imgDir }),
  // 检查完整性：识别三套头像包标准并给出可补全的缺失目录（「检查完整性」按钮用）。
  checkIntegrity: (imgDir: string) => invoke<IntegrityResult>('check_integrity', { imgDir }),
  // 扫描头像：建图片索引缓存（供替换复用），结果写入后端日志。
  scanImages: () => invoke<ImageScanResult>('scan_images'),
  // 扫描RTF：解析当前 RTF 并填充名单预览缓存，结果写入后端日志。
  scanRtf: () => invoke<RtfScanResult>('scan_rtf'),
  // 打开标准规则目录（data/rules/）：编辑 standard2/standard3.json 后下次替换即生效。
  openRulesDir: () => invoke<void>('open_rules_dir'),
  // 预览指定存档（下拉浏览展示，不切换后端当前档/激活状态）。
  previewProfile: (name: string) => invoke<AppSnapshot>('preview_profile', { name }),
  // 用资源管理器打开当前展示档的存档 XML 快照所在目录（查看位置）。
  openSnapshotDir: (path: string) => invoke<void>('open_snapshot_dir', { path }),
  // 读取快照：解析当前展示档的存档 XML 快照并缓存（「读取快照」按钮 → 快照预览的数据源）。
  readSnapshot: (xmlPath: string) => invoke<SnapshotInfo>('read_snapshot', { xmlPath }),
  // 快照预览分页查询：UID 筛选 + 只对当前页解析图片路径（imgDir 为当前展示组的头像包目录）。
  querySnapshot: (xmlPath: string, imgDir: string, query: SnapshotQuery) =>
    invoke<PageResult<SnapshotEntry>>('query_snapshot', { xmlPath, imgDir, query }),
  // 读取/保存标准规则文件（name ∈ standard2|standard3；保存时后端先按该标准 schema 校验）。
  readRuleFile: (name: 'standard2' | 'standard3') => invoke<string>('read_rule_file', { name }),
  saveRuleFile: (name: 'standard2' | 'standard3', content: string) =>
    invoke<void>('save_rule_file', { name, content }),
  createMissingDirs: (imgDir: string, names: string[]) =>
    invoke<void>('create_missing_dirs', { imgDir, names }), // void 表示不关心返回值
  // startReplace 的参数包在 { args: params } 里，对应 Rust 端函数参数名 args: ReplaceArgs。
  // 发起前先问一次预检：只有后端判定 needsConfirm（「生成」+ 本档档景非空）时才弹确认，
  // 确认过的这一轮把 allowBlank 带上，流水线的写前拦截才会放行。
  preflightReplace: (mode: ReplaceArgs['mode']) => invoke<PreflightResult>('preflight_replace', { mode }),
  startReplace: (params: ReplaceArgs) => invoke<void>('start_replace', { args: params }),
  cancelReplace: () => invoke<void>('cancel_replace'),
  queryPlayers: (query: PlayerQuery) => invoke<PageResult<PlayerRecord>>('query_players', { query }),
  previewPlayer: (uid: string) => invoke<PlayerPreview>('preview_player', { uid }),
  // queryAvatars：名单预览头像列按 UID 批量取头像绝对路径（一次拉一页；null = 无映射或图片不在包内）。
  queryAvatars: (uids: string[]) =>
    invoke<Record<string, string | null>>('query_avatars', { uids }),
  replaceSingleFace: (uid: string, imagePath: string, saveBackup: boolean) =>
    invoke<boolean>('replace_single_face', { uid, imagePath, saveBackup }),
  // readImage 返回 ArrayBuffer（二进制图片字节），前端再转成 blob URL 显示。
  readImage: (path: string) => invoke<ArrayBuffer>('read_image', { path }),
  // resolveAvatar：把名单「头像」列原文解析成当前头像包内的图片绝对路径（找不到为 null）。
  resolveAvatar: (avatar: string) => invoke<string | null>('resolve_avatar', { avatar }),
  queryLogs: (level: string, only: boolean, offset: number, limit: number) =>
    invoke<PageResult<LogLine>>('query_logs', { level, only, offset, limit }),
};
