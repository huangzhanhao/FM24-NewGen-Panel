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

/** 名单格式协议码：Rust 枚举 RtfFormat 的 serde 序列化值（camelCase 变体名）。
 *  它是枚举变体的身份、不是可改的文案——Rust 侧重命名变体属于显式协议变更（match 强制穷举），
 *  而 format 中文串只是展示/日志文案，可随意改词。 */
export type RtfFormatCode = 'basic' | 'extPlayer' | 'extStaff';

/** 「扫描RTF」返回摘要（对应 Rust 的 RtfScanResult）。 */
export interface RtfScanResult {
  total: number; newgan: number; detailed: boolean; format: string;
  formatCode: RtfFormatCode; language: string;
}

/** 当前组 config.xml 的路径与存在性（对应 Rust 的 ConfigXmlInfo）。 */
export interface ConfigXmlInfo { path: string; exists: boolean; }

/** 应用全局快照（对应 Rust 的 AppSnapshot）；前端大部分界面基于它渲染。 */
export interface AppSnapshot {
  profiles: string[]; currentProfile: string; groups: GroupInfo[];
  currentGroup: string | null; currentRtf: string;
  replacing: boolean; configXml: ConfigXmlInfo | null; logPath: string;
}

/** 进度事件载荷（对应 Rust 的 ProgressPayload）：替换流水线的进度。
 *  current/total 是阶段内的「第 n 个 / 共 total 个」（如逐球员抽图 3500/30000）；
 *  纯阶段锚点或该阶段无可数单位时为 0/0，状态栏据此只显示百分比。 */
export interface ProgressPayload { status: string; value: number; current: number; total: number; }

/** 轻量任务的进度载荷（对应 Rust 的 TaskProgressPayload）：「检查完整性 / 扫描头像 / 扫描RTF /
 *  读取快照」四个按钮的进度。value 由 current/total 折算而来，状态栏另显示「n/total」原始口径。 */
export interface TaskProgressPayload { status: string; value: number; current: number; total: number; }

/** 发起替换的参数（对应 Rust 的 ReplaceArgs）；mode 用字面量联合限定三个合法值（协议值）。
 *  allowBlank：本轮已在预检里确认过「可能把本档既有映射清空」，由前端在确认框点「是」后置 true，
 *  界面上没有对应的勾选项——它是确认的凭据，不是给用户关的守卫。 */
export interface ReplaceArgs {
  mode: 'Preserve' | 'Overwrite' | 'Generate';
  filterNewgan: boolean; allowDuplicates: boolean; saveBackup: boolean; allowBlank: boolean;
}

/** 「发起前预检」返回（对应 Rust 的 PreflightResult）：是否需要先弹确认，以及本档档景现有条数。 */
export interface PreflightResult { needsConfirm: boolean; existing: number; }

/** 通用分页结果（对应 Rust 的 PageResult<T>）；日志查询仍在用。 */
export interface PageResult<T> { total: number; items: T[]; }

/** 单人预览（对应 Rust 的 PlayerPreview）；三个字段都可能为 null（后端 Option 序列化为 null）。 */
export interface PlayerPreview {
  record: PlayerRecord | null; mapping: MappingEntry | null; imagePath: string | null;
}

/** 「读取快照」返回摘要（对应 Rust 的 SnapshotInfo）。 */
export interface SnapshotInfo { total: number; }

/** 快照预览的一行（对应 Rust 的 SnapshotEntry）；relPath 是映射原文，imagePath 由前端按页批量解析。 */
export interface SnapshotEntry { uid: string; relPath: string; imagePath: string | null; }

/** 一行日志（对应 Rust 的 LogLine）。 */
export interface LogLine { ts: string; level: string; module: string; message: string; }

/** 决定适配器（实为「协议方言」轴）。OpenAI 兼容服务与官方同为一套 SDK，合并为 openai 一档（改 baseUrl 即可）。 */
export type ProviderKindCode =
  | 'openai'
  | 'volcano'
  | 'dashscope'
  | 'siliconflow'
  | 'modelscope'
  | 'localcomfyui';

/** 自由尺寸规则（万相 2.5/2.6 等）；0/缺省 = 该条约束不检查。字段可部分给出。 */
export interface SizeRules {
  minSide?: number;
  maxSide?: number;
  totalMin?: number;
  totalMax?: number;
  ratioMin?: number;
  ratioMax?: number;
}

export interface Capabilities {
  sizes: string[];
  maxN: number;
  supportsQuality: boolean;
  supportsSeed: boolean;
  supportsSteps: boolean;
  /** 采样步数上限（0 = 不校验；配合 supportsSteps）。硅基流动 num_inference_steps / 魔搭 steps。 */
  stepsMax?: number;
  /** 采样步数默认值（切模型时据此归位）。 */
  stepsDefault?: number;
  /** 引导系数（Classifier-Free Guidance）：硅基流动 guidance_scale、魔搭 guidance。 */
  supportsGuidance?: boolean;
  /** 引导系数默认值（硅基流动 Kolors 7.5 / 魔搭 3.5）。 */
  guidanceDefault?: number;
  /** 引导系数上限（0 = 不校验）。 */
  guidanceMax?: number;
  supportsNegativePrompt: boolean;
  supportsResponseFormat: boolean;
  /** 火山 Seedream output_format（png/jpeg）：5.0 Pro/Flash/Lite 可设；4.5/4.0 仅回 jpeg、不可设。
   *  其余协议不用。适配器固定要 png，本字段只决定参数是否发送。 */
  supportsOutputFormat?: boolean;
  qualities: string[];
  /** 以下字段可缺省，消费侧按「缺省=false/0」处理。 */
  supportsPromptExtend?: boolean;
  supportsWatermark?: boolean;
  maxPromptChars?: number;
  /** null/缺省 = 不吃自由尺寸约束（有预设尺寸的模型）。 */
  sizeRules?: SizeRules | null;
  /** 请求体是否携带 n（z-image 无此参数，固定出 1 张；缺省按 true 处理）。 */
  supportsN?: boolean;
  /** 智能改写开关的官方默认值：千问/万相 true，z-image false（开启即思考模式、单价翻倍）。 */
  promptExtendDefault?: boolean;
  /** `watermark` 的默认值（官方 false；按模型存放，便于日后分化）。 */
  watermarkDefault?: boolean;
  /** `prompt_extend_mode`：智能改写模式（direct=DPE / agent=APE，agent 仅文生图）。3.0 独有。 */
  supportsPromptExtendMode?: boolean;
  /** `prompt_extend_mode` 的默认档（官方 direct）。 */
  promptExtendModeDefault?: string;
  /** `enable_thinking`：思考模式（官方默认 true，仅 prompt_extend=true 时生效）。3.0 独有。 */
  supportsEnableThinking?: boolean;
  /** `enable_thinking` 的默认值（官方 true）。 */
  enableThinkingDefault?: boolean;
  /** 是否支持异步调用（提交任务 → 轮询取结果）。协议级能力位，按模型逐条声明。 */
  supportsAsync?: boolean;
}

export interface ModelInfo {
  model: string;
  capabilities: Capabilities;
}

export interface ProviderConfig {
  id: string; kind: ProviderKindCode; name: string;
  baseUrl: string; defaultModel: string; models: ModelInfo[];
}

/** 生图设置。concurrency/namingPattern 在 P1 只随文件读写、不参与逻辑（分别 P2/P3 生效）；
 *  historyLimit 与 galleryRecordLimit 是两条独立淘汰闸门的上限（图片数 / 记录行数），互不换算。 */
export interface GenSettings {
  defaultProviderId: string; historyLimit: number; galleryRecordLimit: number;
  concurrency: number; namingPattern: string;
}

export interface PromptTemplate { id: string; name: string; body: string; negative: string; }

export interface ImageConfig {
  providers: ProviderConfig[]; settings: GenSettings;
  templates: PromptTemplate[];
  dictionary: Record<string, Record<string, string>>;
}

export interface GenRequest {
  providerId: string; model: string; prompt: string; negativePrompt: string;
  size: string; n: number; quality: string | null; seed: number | null;
  /** P2 生效（steps 依赖支持该参数的适配器）；硅基流动/魔搭用；null/缺省 = 不发送（用服务端默认）。 */
  steps?: number | null;
  /** 引导系数（硅基流动 guidance_scale / 魔搭 guidance）；null/缺省 = 不发送。 */
  guidance?: number | null;
  /** P3 生效（uid 依赖名单数据源）；字段先落下，P1 恒不传。 */
  uid?: string | null;
  /** 球员名（随请求带入，仅用于画廊记录展示；无数据源时缺省 null）。 */
  name?: string | null;
  /** 提示词模板 id（随请求带入写入画廊记录；未选模板时缺省 null）。 */
  templateId?: string | null;
  /** 调用方式：'async' = 想走异步（后端仍按模型 supportsAsync 再门控一遍）；null/缺省 = 同步。 */
  callMode?: string | null;
  /** 万相智能改写开关（null/缺省 = 按官方默认 true）。 */
  promptExtend?: boolean | null;
  /** 万相「AI生成」水印开关（null/缺省 = 按官方默认 false）。 */
  watermark?: boolean | null;
  /** 百炼 `prompt_extend_mode`（null/缺省 = 按该模型默认档 direct）。 */
  promptExtendMode?: string | null;
  /** 百炼 `enable_thinking`（null/缺省 = 按该模型默认值 true）。 */
  enableThinking?: boolean | null;
  /** 内部字段（serde 缺省为空）：后端 generate 解析出服务商后回填 base_url，前端提交（单张/批量）
   *  时不传；仅队列条目经 IPC 快照回传时会带值（见 image_ai_queue::QueueEntry 内嵌本类型）。
   *  能力位不在此列：后端整体快照走 `caps`（前端亦不传）。 */
  baseUrl?: string;
}

/** render_prompt 的返回：渲染后文本 + 无值且无默认值的占位符键名。 */
export interface RenderOutcome { text: string; missing: string[]; }

export interface GenImage { id: string; path: string; bytes: number; }

export interface GenResult {
  images: GenImage[];
}

/** 会话内生成结果（视图模型）：id/path/url 来自后端，size/seed 是生成时快照，供结果卡展示。
 *  imageAvailable/removable 仅画廊合并视图使用：持久记录 imageAvailable 由后端查询期现算，
 *  图已被会话清理时为 false；removable=false 表示该条来自持久记录（无移除入口）。 */
export interface GeneratedImage {
  id: string;
  path: string;
  url: string;
  size?: string;
  seed?: string | null;
  imageAvailable?: boolean;
  removable?: boolean;
}

/** 一条持久画廊记录（对应 Rust 的 GalleryRecord）：与 `.imgtmp` 的图片字节是两条独立生命周期。
 *  除 id/ts 外的字段在 Rust 侧可缺省，故前端按可选处理。 */
export interface GalleryRecord {
  id: string; ts: string;
  uid?: string | null; name?: string | null;
  model: string; prompt: string; negativePrompt: string; templateId: string;
}

/** get_gallery 的返回项（对应 Rust 的 GalleryItem）：记录字段平铺 + 查询期现算的图片存在性。
 *  path 为 `.imgtmp` 实际路径（图片已被会话清理时为 null）；imageAvailable 只在查询期计算，绝不写回。 */
export interface GalleryItem extends GalleryRecord {
  imageAvailable: boolean;
  path: string | null;
}

// ---------------- 批量生成队列（对应 image_ai_queue.rs，serde 均为 camelCase） ----------------

/** 队列条目状态：对应 Rust QueueStatus（serde rename_all = "lowercase"）。 */
export type QueueStatus = 'queued' | 'running' | 'success' | 'failed';

/** 生成链错误类别（对应 Rust GenErrorKind，snake_case 序列化）。
 *  前端据此判断失败条目是否值得重试、并给出处置建议。 */
export type GenErrorKind =
  | 'not_configured'
  | 'auth'
  | 'rate_limited'
  | 'invalid_param'
  | 'content_rejected'
  | 'provider'
  | 'network'
  | 'io'
  | 'cancelled'
  | 'internal';

/** 生成链结构化错误（对应 Rust ImageGenError）：单张生成的命令 Err 即此对象。
 *  userAction 为可执行建议，无建议类别（cancelled/internal）为 null。 */
export interface GenError {
  kind: GenErrorKind;
  message: string;
  /** 后端「是否自动重试」的判据（队列执行期据它决定是否对同一请求自动再试）。
   * 【与前端 isRetryableErrorKind 分工】后者决定是否给用户「重试本条」手动入口，二者判据不同、不可互替。 */
  retryable: boolean;
  userAction: string | null;
}

/** 一条批量生成任务（对应 Rust 的 QueueEntry）。
 *  images/error 在 Rust 侧带 skip_serializing_if：成功前/无错误时 JSON 里不出现这两个字段，
 *  消费侧用 ?? 兜底，不要直接 .length/.error 裸取；errorKind 同为可缺省。
 *  attempts 为已尝试次数（执行前 +1，入队未跑为 0）；仅 > 1 表示发生过重试，据此显示「已重试 N 次」。 */
export interface QueueEntry {
  id: string; uid: string; name: string;
  req: GenRequest;
  status: QueueStatus;
  images: GenImage[];
  error: string;
  errorKind?: GenErrorKind | null;
  attempts?: number;
}

/** 进度事件 gen:queue:progress 的载荷；current 是**已完成数**（终态条目累计，非派发序号）。
 *  stage 仅运行中的阶段事件带值（submitting/polling/downloading），其余事件为 undefined。
 *  entryId 是条目唯一 id：同一球员重复入队时 uid 会重复，阶段定位须用 entryId。 */
export interface QueueProgress { status: 'running' | 'success' | 'failed'; current: number; total: number; uid: string; entryId?: string | null; stage?: string | null; }

/** 收尾事件 gen:queue:finished 的载荷；failed 含「已取消」条目。 */
export interface QueueFinished { finished: number; failed: number; total: number; }
