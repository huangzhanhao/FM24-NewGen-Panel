/**
 * 前端调用后端的统一入口：把 Tauri 的 invoke 封装成一组语义化方法，组件里只需写 api.createProfile(name)，不必重复写 invoke 字符串，也方便类型提示。
 *
 * invoke("命令名", 参数对象) 异步调用 Rust 后端带 #[tauri::command] 的同名函数：
 * 返回一个 Promise：后端 Ok → resolve（拿到返回值）；Err(字符串) → reject（抛异常，用 try/catch 接）。
 * 命令名与参数名必须与 Rust 端一致：Rust 蛇形参数在 invoke 里写驼峰，Tauri 自动映射。
 */
import { invoke } from '@tauri-apps/api/core';
import type {
  AppSnapshot, GalleryItem, GenImage, GenRequest, SaveReplaceResult, SaveTargetPreview, GenResult, GeneratedImage, ImageConfig, ImageScanResult, IntegrityResult, LogLine,
  NationalityDoc, PageResult, PlayerChunk, PlayerPreview, PlayerRecord, PreflightResult, PromptTemplate, ProviderConfig,
  QueueEntry, RenderOutcome, ReplaceArgs, RtfScanResult, SnapshotEntry, SnapshotInfo,
} from '../types'; // 仅导入类型，编译后完全抹去（不产生运行时代码）

// 名单分块传输的块大小（与 UI 每页行数无关；后端夹 1..=65536）。默认 32768（§7.1）；
// 后端分页启用与否以 docs/fm存档名单读取-T04G实测记录.md 的签字结论为准。
// 导出给测试引用，避免换档时测试断言跟着硬编码值漂移。
export const PLAYERS_CHUNK_LIMIT = 32768;

/** 全部后端命令的前端封装；每个方法的泛型参数是该命令 resolve 出的返回类型。 */
export const api = {
  // 取应用全量状态快照（当前档/头像包组/RTF 等），前端各视图的首屏数据源。
  getAppState: () => invoke<AppSnapshot>('get_app_state'),
  // 新建存档，返回刷新后的状态快照。
  createProfile: (name: string) => invoke<AppSnapshot>('create_profile', { name }),
  // 删除指定存档，返回删除后的状态快照。
  deleteProfile: (name: string) => invoke<AppSnapshot>('delete_profile', { name }),
  // 切换后端当前档（决定后续替换写回哪个存档），返回切换后的状态快照。
  switchProfile: (name: string) => invoke<AppSnapshot>('switch_profile', { name }),
  // 选定头像包目录，返回更新后的状态快照。
  selectImageDirectory: (path: string) => invoke<AppSnapshot>('select_image_directory', { path }),
  // 设定当前 RTF 名单文件（名单预览与替换的取数来源），返回状态快照。
  setCurrentRtf: (path: string) => invoke<AppSnapshot>('set_current_rtf', { path }),
  // 设定当前 .fm 存档（与 RTF 并立的名单来源），返回状态快照。
  setCurrentFm: (path: string) => invoke<AppSnapshot>('set_current_fm', { path }),
  // 参数写 imgDir（驼峰），对应 Rust 端 img_dir（蛇形），Tauri 自动映射。
  setCurrentGroup: (imgDir: string) => invoke<AppSnapshot>('set_current_group', { imgDir }),
  // 移除指定头像包组记录，返回删除后的状态快照。
  deleteGroup: (imgDir: string) => invoke<AppSnapshot>('delete_group', { imgDir }),
  // 校验头像包目录结构，返回识别结果或错误说明列表，供前端提前提示用户。
  validateImageDirs: (imgDir: string) => invoke<string[]>('validate_image_dirs', { imgDir }),
  // 检查完整性：识别三套头像包标准并给出可补全的缺失目录（「检查完整性」按钮用）。
  checkIntegrity: (imgDir: string) => invoke<IntegrityResult>('check_integrity', { imgDir }),
  // 扫描头像：建图片索引缓存（供替换复用），结果写入后端日志。
  scanImages: () => invoke<ImageScanResult>('scan_images'),
  // 扫描RTF：解析当前 RTF 并填充名单预览缓存，结果写入后端日志。
  scanRtf: () => invoke<RtfScanResult>('scan_rtf'),
  // 扫描.fm：解析当前 .fm 存档并填充名单预览缓存（与 scanRtf 同一缓存与摘要口径）。
  scanFm: () => invoke<RtfScanResult>('scan_fm'),
  // 打开标准规则目录（data/rules/）：编辑 standard2/standard3.json 后下次替换即生效。
  openRulesDir: () => invoke<void>('open_rules_dir'),
  // 预览指定存档（下拉浏览展示，不切换后端当前档/激活状态）。
  previewProfile: (name: string) => invoke<AppSnapshot>('preview_profile', { name }),
  // 用资源管理器打开当前展示档的存档 XML 快照所在目录（查看位置）。
  openSnapshotDir: (path: string) => invoke<void>('open_snapshot_dir', { path }),
  // 读取快照：解析当前展示档的存档 XML 快照并缓存（「读取快照」按钮 → 快照预览的数据源）。
  readSnapshot: (xmlPath: string) => invoke<SnapshotInfo>('read_snapshot', { xmlPath }),
  // 快照预览分页查询：UID 筛选 + 只对当前页解析图片路径（imgDir 为当前展示组的头像包目录）。
  querySnapshot: (xmlPath: string) => invoke<SnapshotEntry[]>('query_snapshot', { xmlPath }),
  // 读取标准规则文件（name ∈ standard2|standard3）。
  readRuleFile: (name: 'standard2' | 'standard3') => invoke<string>('read_rule_file', { name }),
  // 保存标准规则文件（保存时后端先按该标准 schema 校验）。
  saveRuleFile: (name: 'standard2' | 'standard3', content: string) => invoke<void>('save_rule_file', { name, content }),
  // 读取国籍对照表（data/nationalities.json 的用户副本；缺失时返回出厂默认）。
  getNationalities: () => invoke<NationalityDoc>('get_nationalities'),
  // 保存国籍对照表：后端校验（码格式/语言归属/同语言全称唯一）后原子写盘并立即生效。
  saveNationalities: (doc: NationalityDoc) => invoke<NationalityDoc>('save_nationalities', { doc }),
  // 按给定的缺失目录名在头像包内批量补齐目录结构。
  createMissingDirs: (imgDir: string, names: string[]) => invoke<void>('create_missing_dirs', { imgDir, names }),
  // startReplace 的参数包在 { args: params } 里，对应 Rust 端函数参数名 args: ReplaceArgs。
  // 发起前先问一次预检：只有后端判定 needsConfirm（「生成」+ 本档档景非空）时才弹确认，
  // 确认过的这一轮把 allowBlank 带上，流水线的写前拦截才会放行。
  preflightReplace: (mode: ReplaceArgs['mode']) => invoke<PreflightResult>('preflight_replace', { mode }),
  startReplace: (params: ReplaceArgs) => invoke<void>('start_replace', { args: params }),
// 请求取消正在进行的替换流水线。
  cancelReplace: () => invoke<void>('cancel_replace'),
  // 名单预览的流式取数：分块拉取，每块成功即经 onBatch 交付，调用方不必等整份读完即可先渲染。
  // total 是缓存全量条数，块间必须一致——不一致说明读取期间缓存被清/重填（切档/切组/重扫），
  // 此时先 onBatch(null) 声明已交付批次作废，再从头重取一次；二次仍不一致抛错，由上层 onError 弹窗，
  // 绝不把两代缓存的块拼在一起当完整名单。
  queryPlayersStream: async (onBatch: (items: PlayerRecord[] | null) => void): Promise<void> => {
    const limit = PLAYERS_CHUNK_LIMIT;
    let offset = 0;
    let expectedTotal: number | null = null;
    let retried = false;
    for (;;) {
      const chunk = await invoke<PlayerChunk>('query_players_chunk', { offset, limit });
      if (expectedTotal === null) {
        expectedTotal = chunk.total;
      } else if (chunk.total !== expectedTotal) {
        if (!retried) {
          retried = true;
          offset = 0;
          expectedTotal = null;
          onBatch(null); // 已交付的批次属于旧缓存：先作废，再从第 0 块重取
          continue;
        }
        throw new Error('名单在分块读取期间发生变化');
      }
      onBatch(chunk.items);
      offset += chunk.items.length;
      if (chunk.items.length === 0 || offset >= expectedTotal) break;
    }
  },
  // queryPlayersByUids：按 uid 批量取球员记录（队列坞条目行身份信息：年龄/所在地帮认人）；
  // 只读后端 rtf_cache（与 query_players 同源），未命中的 uid 不在返回之列。
  queryPlayersByUids: (uids: string[]) => invoke<PlayerRecord[]>('query_players_by_uids', { uids }),
  // 取单个球员的预览信息（球员详情查看用）。
  previewPlayer: (uid: string) => invoke<PlayerPreview>('preview_player', { uid }),
  // querySnapshotImages：快照预览当前页批量解析头像实际路径（与 relPaths 顺序一一对应；null = 找不到）。
  querySnapshotImages: (imgDir: string, relPaths: string[]) => invoke<(string | null)[]>('query_snapshot_images', { imgDir, relPaths }),
  // queryAvatars：名单预览头像列按 UID 批量取头像绝对路径（一次拉一页；null = 无映射或图片不在包内）。
  queryAvatars: (uids: string[]) => invoke<Record<string, string | null>>('query_avatars', { uids }),
  // 单人替换：把指定球员头像替换为给定图片，返回是否成功（saveBackup 控制是否留备份）。
  replaceSingleFace: (uid: string, imagePath: string, saveBackup: boolean) => invoke<boolean>('replace_single_face', { uid, imagePath, saveBackup }),
  // resetSingleFace：单人重掷——按标准规则给该球员重新抽图（固定重抽语义，后端单点注释）。
  resetSingleFace: (uid: string, saveBackup: boolean) => invoke<boolean>('reset_single_face', { uid, saveBackup }),
  // readImage 返回 ArrayBuffer（二进制图片字节），前端再转成 blob URL 显示。
  readImage: (path: string) => invoke<ArrayBuffer>('read_image', { path }),
  // resolveAvatar：把名单「头像」列原文解析成当前头像包内的图片绝对路径（找不到为 null）。
  resolveAvatar: (avatar: string) => invoke<string | null>('resolve_avatar', { avatar }),
  // 分页查询后端日志（按 level 过滤 + only 只看该级别，offset/limit 翻页）。
  queryLogs: (level: string, only: boolean, offset: number, limit: number) => invoke<PageResult<LogLine>>('query_logs', { level, only, offset, limit }),
  // 取 AI 生图全量配置（Provider 列表、提示词模板、密钥状态等），AI 设置页数据源。
  getImageAiState: () => invoke<ImageConfig>('get_image_ai_state'),
  // 画廊记录：持久清单 + 查询期现算的图片存在性（imageAvailable；绝不写回后端文件）。
  getGallery: () => invoke<GalleryItem[]>('get_gallery'),
  // 新增/更新一个 Provider，返回更新后的完整 AI 配置。
  saveProvider: (provider: ProviderConfig) => invoke<ImageConfig>('save_provider', { provider }),
  // 删除指定 Provider，返回更新后的完整 AI 配置。
  deleteProvider: (id: string) => invoke<ImageConfig>('delete_provider', { id }),
  // 单独保存 Provider 的 API Key（密钥不回传前端，故与 Provider 配置分通道）。
  setProviderKey: (id: string, key: string) => invoke<void>('set_provider_key', { id, key }),
  // 查询已配置密钥的 Provider id 列表（供前端显示「已配置」标记）。
  getProviderKeyState: () => invoke<string[]>('get_provider_key_state'),
  // 新增/更新提示词模板，返回更新后的完整 AI 配置。
  savePromptTemplate: (template: PromptTemplate) => invoke<ImageConfig>('save_prompt_template', { template }),
  // 删除指定提示词模板，返回更新后的完整 AI 配置。
  deletePromptTemplate: (id: string) => invoke<ImageConfig>('delete_prompt_template', { id }),
  // 用模板体 + 变量即时渲染提示词（不落库），返回渲染结果或错误（模板编辑器实时预览用）。
  renderPrompt: (body: string, vars?: Record<string, string>) => invoke<RenderOutcome>('render_prompt', { body, vars: vars ?? null }),
  // 调用所选 Provider 生成图片，返回生成结果（含生成图 id；AI 生图主流程）。
  generateImage: (req: GenRequest) => invoke<GenResult>('generate_image', { req }),
  // 把生成的图片另存到指定路径，返回实际写入的路径。
  saveGeneratedImage: (imageId: string, targetPath: string) => invoke<string>('save_generated_image', { imageId, targetPath }),
  // 按球员 UID + 模板渲染提示词（不发起生成），供生成前确认。
  previewPrompt: (uid: string, templateId: string) => invoke<RenderOutcome>('preview_prompt', { uid, templateId }),
  // 保存前取落点预览（`_generated` 基址 + 池/文件名），供弹窗确认或修改。
  previewSaveTarget: (uid: string) => invoke<SaveTargetPreview>('preview_save_target', { uid }),
  // 保存生成图并替换该球员头像；relTarget 为空 → 后端按球员所属池自动落点。
  // replaced 区分「真改了映射」与「只落盘」。
  // relTarget 显式转 null：invoke 会丢掉 undefined 字段，后端 Option<String> 收不到键会报参数缺失。
  saveAndReplace: (imageId: string, uid: string, saveBackup: boolean, relTarget?: string) =>
    invoke<SaveReplaceResult>('save_and_replace', { imageId, uid, saveBackup, relTarget: relTarget ?? null }),
  // ---- 批量生成队列（命令返回均为全量快照 Vec<QueueEntry>，条目 id 由后端统一补发） ---------------------------------
  // 追加条目到队列（id 由后端补发），返回全量队列。
  addToQueue: (entries: QueueEntry[]) => invoke<QueueEntry[]>('add_to_queue', { entries }),
  // 按 id 移除队列条目，返回全量队列。
  removeFromQueue: (ids: string[]) => invoke<QueueEntry[]>('remove_from_queue', { ids }),
  // 启动批量生成队列。
  // ids 为「本次要跑的条目」；不传 = 全部排队条目（null 走后端 None 分支）。
  startQueue: (ids?: string[]) => invoke<void>('start_queue', { ids: ids ?? null }),
  // 手动确认后写回条目参数（后端只改排队中的条目，非排队/不存在的 id 原样跳过），返回全量队列。
  // 参数非法时后端整批拒绝并回 Err（队列不变），由调用方内联展示。
  updateQueueEntries: (entries: { id: string; req: GenRequest }[]) => invoke<QueueEntry[]>('update_queue_entries', { entries }),
  // 取消批量生成队列。
  // 取消会把「未提交」的条目当场标成已取消，并回传快照（前端据此立即刷新，不等事件）。
  cancelQueue: () => invoke<QueueEntry[]>('cancel_queue'),
  // 查询当前队列全量快照。
  queryQueue: () => invoke<QueueEntry[]>('query_queue'),
};
