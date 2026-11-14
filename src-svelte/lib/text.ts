/**
 * 全局文案常量（三语）：所有「显示给用户看的文字」按语言分三份维护，
 * 组件统一经 `t` 取当前语言字典（t 是按 ui.lang 分发的 getter，切换即全界面刷新）。
 * 组件里 `import { t } from '../text'`，模板中直接用 {t.xxx.yyy}；需拼变量的动态文案是函数。
 *
 * 约定：参与逻辑判断的「协议值」不放这里——如缺省档名 'No Profile'、替换模式 value
 * 'Preserve'/'Overwrite'/'Generate'、日志级别 'INFO'、事件名、排序键等，
 * 它们的取值发往后端，改动会破坏前后端逻辑。
 * 好处：改文案只改这一处；zh-TW / en 两份与 zh-CN 结构一致（TS 以 zh-CN 推导类型校验）。
 */

import { ui, type Lang } from './stores/ui.svelte';

/** 替换模式：value 是发往后端的协议值（字面量联合，与后端 Mode 一一对应）；
 *  label/desc 才是随语言切换的界面文案。 */
type ReplaceMode = {
  value: 'Preserve' | 'Overwrite' | 'Generate';
  label: string;
  desc: string;
};

/** 简体中文（基准语言：另两份的键集合与函数签名以此为准）。 */
const zhCN = {
  /** 应用品牌名（窗口/弹窗标题、侧栏品牌区共用；品牌名不翻译） */
  brand: "FM24 NewGen Panel",

  /** 通用系统弹窗标题 */
  dialog: {
    error: "错误",
    info: "提示",
    warning: "警告",
    /** 结构化错误里 userAction 的引导词（如「建议：检查 API Key」）。 */
    suggestion: "建议：",
  },

  /** 根组件：替换结束的系统提示 */
  app: {
    replaceFinished: "替换完成！",
    replaceFailed: "替换失败，请查看日志",
    brandVer: "v2.0",
  },

  /** 顶栏三工作台（chip 可点击切换，右侧状态点随就绪度变色） */
  workbench: {
    archive: "存档资料",
    replace: "批量/单人头像替换",
    ai: "AI 生成头像",
    /** 顶栏右端作用域回显：档名与组名之间的分隔符（组为空时只显示档名） */
    scopeSep: " · ",
    /** 无头像包组时的回显占位（侧栏还没关联目录） */
    scopeNoGroup: "未选头像包",
    /** chip 状态点的悬停说明（两档：绿=就绪 / 橙=未就绪） */
    stateOk: "已就绪",
    stateWarn: "未就绪，点进去查看缺少的条件",
  },

  /** 侧栏底部主题切换 */
  theme: {
    dark: "深色",
    light: "浅色",
    switchTo: (mode: "dark" | "light"): string =>
      mode === "light" ? "切换到浅色主题" : "切换到深色主题",
  },

  /** 侧栏底部「语言」菜单：按钮名 + 三个选项的自显示名（各语言写自己的名字） */
  lang: {
    label: "语言",
    zhCN: "简体中文",
    zhTW: "繁體中文",
    en: "English",
  },

  /** 内置文档面板：侧栏「教程 / FAQ」两个入口各打开同一面板的对应页签 */
  help: {
    title: "使用文档",
    subtitle: "内置教程与常见问题，离线可查",
    tutorialTab: "教程",
    faqTab: "FAQ",
    searchPlaceholder: "搜索章节或关键词…",
    searchHint: "Ctrl+F 聚焦搜索",
    resultCount: (shown: number, total: number) => `显示 ${shown} / 共 ${total} 条`,
    noResults: "没有匹配的条目，换个关键词试试。",
    clearSearch: "清空搜索",
    note: "提示",
    warning: "注意",
  },

  /** 侧边栏：存档（Profile）与头像包（组）管理 */
  sidebar: {
    profileSection: "存档资料",
    newProfileBtn: "新建 Profile",
    newProfilePlaceholder: "新 Profile 名称",
    createBtn: "创建",
    notActivated: "未激活",
    activated: "已激活",
    deleteCurrentProfile: "删除当前 Profile",
    noProfileNoActivate: "No Profile 为启动缺省档，未激活",
    noProfileNoDelete: "缺省档不可删",
    activeAlreadyHint: "当前已激活该存档",
    activateBtn: "激活",
    activateHint: (name: string) => `激活存档「${name}」并切换其组配置与映射`,
    deleteProfileConfirm: (name: string) =>
      `删除 Profile「${name}」及其全部组配置与映射备份？`,
    deleteProfileTitle: "删除 Profile",
    groupSection: "头像包（组）",
    linkNewHint: "关联新的头像包目录",
    linkBrowseHint: "请先点击「激活」后再关联头像包",
    linkDialogTitle: "关联头像包目录",
    selectGroupBrowseHint: "请先点击「激活」后再切换当前组（当前为浏览预览）",
    deleteGroupBrowseHint: "请先激活后再删除组",
    deleteGroupBtn: "删除组",
    deleteGroupConfirm: (dir: string) => `删除组及其映射备份？\n${dir}`,
    deleteGroupTitle: "删除组",
    /** 队列运行中的切档/切组护栏：空闲切换不弹窗不打断，运行中才问这一句 */
    queueScopeTitle: "AI 头像队列正在运行",
    queueScopeConfirm:
      "切换存档或头像包后，未完成的生成任务将被取消。确定要切换吗？",
    /** 队列运行中执行删除时追加在删除确认文案后的那一段。删除本身已有确认框，
     *  这里并入同一段正文，避免「先问删除、再问队列」连弹两个框（一次操作只打扰一次）。 */
    queueScopeAppend: "\n\nAI 头像队列正在运行，继续将取消未完成的生成任务。",
    rtfSet: "RTF ✓",
    rtfUnset: "未设置 RTF",
    fmSet: "FM ✓",
    fmUnset: "未设置 FM",
    emptyGroups: "暂无头像包。点上方 ＋ 关联头像包目录。",
    linkHome: "首页",
    linkTutorial: "教程",
    linkFaq: "FAQ",
    langMenuTitle: "切换界面语言",
  },

  /** 「当前任务」卡：路径展示、完整性检查、扫描、规则配置入口 */
  setup: {
    cardTitle: "当前任务",
    imgDirLabel: "头像包：",
    rtfLabel: "RTF 名单：",
    xmlLabel: "config.xml：",
    imgDirPlaceholder: "未关联头像包目录",
    rtfUnsetValue: "未设置",
    xmlPlaceholder: "未关联头像包（执行替换后生成）",
    noGroupHint: "请先关联头像包目录",
    checkIntegrityBtn: "检查完整性",
    checkIntegrityHint: "按规则 json 的 FolderList 识别头像包归属哪个标准",
    scanImagesBtn: "扫描头像",
    scanImagesHint: "扫描头像包并缓存图片索引（结果写入日志）",
    selectRtfBtn: "选择RTF",
    selectRtfDialogTitle: "选择 RTF 名单",
    selectRtfHint: "选择 FM 导出的 RTF 名单",
    selectRtfBrowseHint: "请先点击「激活」后再设置 RTF（当前为浏览预览）",
    scanRtfBtn: "扫描RTF",
    scanRtfHint: "解析 RTF 并填充名单预览（结果写入日志）",
    selectFmDialogTitle: "选择 FM 存档",
    scanFmFileBtn: "扫描.fm存档",
    scanFmFileHint: "选择 .fm 存档并立即扫描，结果刷新球员/职员缓存",
    selectFmBrowseHint: "请先点击「激活」后再扫描 .fm 存档（当前为浏览预览）",
    readMemoryBtn: "运行游戏读取",
    readMemoryHint: "扫描正在运行的 FM 进程内存，结果刷新球员/职员缓存（无需 .fm 存档）",
    viewConfigBtn: "读取快照",
    viewConfigHint: "解析存档 XML 快照，在结果区「快照预览」中查看映射列表",
    viewSnapshotBtn: "查看位置",
    viewSnapshotHint: "打开存档 XML 快照所在目录",
    noGroupGuide: "先在左侧「头像包（组）」关联一个头像包目录，或点击已有组。",
    tpl1Complete: "标准一：图片直铺，无种族子目录",
    tplComplete: (standard: number, total: number) =>
      `标准${standard}：${total} 个配置目录齐全`,
    integrityOk: (label: string) => `✓ 头像包结构完整（${label}）`,
    extraDirsNote: (n: number) =>
      n > 0 ? `另含 ${n} 个未配置目录（不参与抽图）` : "",
    integrityFixable: (
      standard: number,
      missing: number,
      found: number,
      total: number,
      list: string,
    ) =>
      `标准${standard}：缺少 ${missing} 个目录（已有 ${found}/${total}）：${list}`,
    createMissingBtn: "一键创建缺失目录",
    missingDirsTitle: "缺失目录",
    missingDirsConfirm: (standard: number, count: number, list: string) =>
      `标准${standard} 缺少 ${count} 个目录：\n${list}\n\n自动创建？`,
    integrityUnknown:
      "⚠ 无法识别头像包结构：子目录与任何标准的 FolderList 清单都不构成包含关系，替换时将把包内全部图片随机分配；也可在「规则配置」中调整标准目录清单。",
    // 完整性状态胶囊的短文案（详情仍用上方 integrityFixable / integrityUnknown 长句）
    pillMissingDirs: (missing: number, found: number, total: number) =>
      `缺少 ${missing} 个目录（已有 ${found}/${total}）`,
    pillUnknown: "无法识别头像包结构",
    ruleConfigBtn: "规则配置",
    ruleConfigHint:
      "结构化编辑目录清单、国籍映射、种族码修正与规则块；也可切到「源码」直接改 JSON，保存时自动校验",
    nationalityConfigBtn: "国籍对照表",
    nationalityConfigHint:
      "调整、新增或删除国籍三字码与各语言国籍全称的对照映射（保存后下次扫描/替换即生效）",
  },

  gen: {
    // 工作台顶部与面板
    provider: "服务商",
    model: "模型",
    providerSettings: "服务商设置",
    /** 服务配置条就绪状态灯：判据与顶栏 ③chip 同源（Key 已设置 + 模型已选），不冒充「已连通」 */
    statusReady: "已就绪",
    statusNoKey: "未设置 Key",
    statusNoModel: "未选模型",
    statusReadyTitle: "Key 已设置且模型已选（连通性以实际生成为准）",
    /** 就绪状态灯缺 Key 档的 title：讲清后果 + 指出去哪设置 */
    keyMissingHint:
      "生成前需先设置当前服务商的 API Key——点此打开服务商设置",
    cardSource: "球员源 · 单人",
    cardSettings: "生成设置",
    resultsTitle: "本次结果",
    resultsCount: (n: number) => `${n} 张`,
    clearResults: "清空",
    clearResultsTitle: "清空本次会话的全部生成结果",
    clearResultsConfirm: (n: number) =>
      `清空本次会话的 ${n} 张生成结果？临时图片将被删除，未手动保存的无法恢复。`,
    template: "提示词模板",
    manageTemplate: "管理模板",
    /** 模板管理弹窗：变量胶囊（点击在正文光标处插入占位符） */
    templateVariables: "可用变量（点击插入光标处）",
    prompt: "提示词（可手动编辑）",
    /** 负向词折叠条（默认收起；展开/收起同一行，随折叠图标切换） */
    negativeCollapsed: "负向提示词（可选）",
    negative: "负向提示词（模型不支持时仅记录不发送）",
    /** 画幅预览帧：尺寸不合规时的简短状态（详细原因仍显示在自定义宽高行下） */
    sizeInvalidShort: "尺寸不合规",
    regenTitle: "按该图生成参数再生成",
    resultAlt: "生成结果图",
    previewAlt: "生成图预览",
    // 连接卡下方的模型能力摘要（与参数控件显隐同一能力集口径）
    capNoModel: "未选择模型明细行：仅可填写接口参数，高级能力不可用",
    capPresetSizes: (n: number) => `${n} 个预设尺寸`,
    capFreeSize: "自由尺寸",
    capMaxN: (n: number) => `单次最多 ${n} 张`,
    capNAny: "张数不限",
    capQuality: "质量可选",
    capSeed: "支持 Seed",
    capNegative: "负向提示词",
    capPromptExtend: "智能改写",
    capWatermark: "水印可选",
    size: "尺寸",
    // 尺寸下拉末尾的「自定义…」项 + 宽高输入与提醒（选中后显示）
    sizeCustom: "自定义…",
    sizeWLabel: "宽",
    sizeHLabel: "高",
    sizeCustomHint:
      "非官方推荐尺寸可能被厂商拒绝，生成失败时请改选上方比例回填。",
    ratio: "比例",
    n: "张数",
    nSingle: "该模型每次仅生成 1 张",
    nLess: "减少",
    nMore: "增加",
    nCount: "数量",
    nMaxHint: (n: number) => `单次最多 ${n} 张`,
    quality: "质量",
    qualityAuto: "自动",
    qualityLow: "低",
    qualityMedium: "标准",
    qualityHigh: "高",
    promptExtend: "智能改写提示词",
    promptExtendHint:
      "由大模型优化提示词，出图更慢；z-image 开启后为思考模式，按更高单价计费；遇版权审核拦截可关闭重试",
    extendMode: "改写模式",
    extendModeDirect: "直接改写",
    extendModeAgent: "智能体改写",
    extendModeAgentHint: "智能体改写仅支持文生图，图生图场景会被厂商拒绝",
    thinking: "思考模式",
    thinkingHint: "提升出图质量但更慢；需先开启智能改写才生效",
    callMode: "调用方式",
    callModeSync: "同步",
    callModeAsync: "异步",
    watermark: "添加「AI生成」水印",
    watermarkHint: "水印位于图片右下角",
    /** 输出图片格式（火山 Seedream / 硅基流动）：选项值是协议字段值 png/jpeg，不翻译 */
    outputFormat: "输出格式",
    outputFormatHint: "服务端返回的图片编码格式；仅声明该参数的模型可选",
    seed: "Seed",
    steps: "采样步数",
    stepsHint: "扩散采样步数，越大通常越慢、细节可能更充分",
    guidance: "引导系数",
    guidanceHint: "提示词引导强度，越大越贴合提示词，越小越多变",
    sizeError: {
      format: "尺寸格式不正确，应为 宽×高 或 宽*高（如 1280×1280）",
      minSide: (v: number) => `宽/高不能小于 ${v} 像素`,
      maxSide: (v: number) => `宽/高不能超过 ${v} 像素`,
      totalMin: (v: number) => `总像素过小，建议约 ${v}×${v} 以上`,
      totalMax: (v: number) => `总像素过大，建议约 ${v}×${v} 以内`,
      ratio: (lo: number, hi: number) => `宽高比需在 ${lo}:1 至 ${hi}:1 之间`,
    },
    promptTooLong: (limit: number) =>
      `提示词超出该模型 ${limit} 字上限，超出部分不会生效`,
    seedPlaceholder: "留空=随机",
    seedOutOfRange: (max: number) => `Seed 超出官方取值范围 0–${max}，本次生成将按随机处理`,
    generate: "生成图片",
    generating: "生成中…",
    genSec: (n: number) => `生成中 · ${n}s`,
    genErrorTitle: "生成失败",
    retry: "重试",
    closeAlert: "关闭错误提示",
    closePreview: "关闭预览",
    viewLarge: "查看大图",
    rerollPrompt: "重掷",
    rerollPromptTitle: "按当前数据源重新随机细节，覆盖输入框内容",
    metaSeed: (s: string) => `Seed ${s}`,
    skeletonHint: "正在生成，通常 10–30 秒，智能改写开启时可能更久",
    empty:
      "填写提示词后点「生成图片」，结果会显示在这里；图片仅存于会话临时目录，手动保存才会落盘。",
    /** 持久画廊里图片已被会话清理的条目占位文案 */
    imageMissing: "图片已清理（仅存会话临时目录）",
    save: "保存",
    saved: "图片已保存",
    removeImageTitle: "移除",
    needModel: "请先在「AI 生成头像」工作台选择服务商与模型",
    /** 渲染时抛出：无默认值且无数据源的占位符无法自动解析，只能由用户手工替换，故直接报错并列出变量名。 */
    missingPlaceholders: (keys: string[]) => `占位符未填充：${keys.join("、")}`,
    // 数据源：名单/单人预览入口可把球员设为生图数据源，模板变量按名单记录自动填充
    source: {
      clear: "清除",
      clearTitle: "清除数据源，恢复手动提示词",
      /** 档案卡：无数据源时的台内 UID 查询（球员源第三来源） */
      queryHint: "输入 UID 查询球员并设为数据源",
      queryPlaceholder: "如 r-2000472008",
      queryBtn: "查询",
      queryMiss: (uid: string) => `未找到 UID ${uid} 的资料`,
      /** 名单未扫描时的就绪卡：直达存档资料台 */
      scanFirst: "名单尚未扫描：请先在「存档资料」台扫描 RTF，才能按球员生成",
      goArchive: "去存档资料台",
      /** 资料降级：uid 不在当前名单（可能来自上一次扫描），仅姓名/UID 可信 */
      noProfile: "未在当前名单中找到该球员的资料，生成时变量可能取不到值",
      /** 名单预览行尾按钮：把该行球员设为生图数据源并跳到 AI 生图 */
      sendBtn: "去抽图",
      sendTitle: (name: string) => `以 ${name} 为数据源生成头像`,
      sent: "已选",
      useForGen: "送生图",
      selected: "已是数据源",
    },
    // 结果卡「保存并替换」：生成图写入 <头像包>/_generated/<池>/ 并走单人换脸链路
    saveReplace: {
      btn: "保存并替换",
      needPlayer:
        "需先选择球员：在「名单预览」行尾点「去抽图」或单人替换页点「送生图」",
      pathTitle: "保存位置",
      // 落点弹窗里显式点名保存对象：图可能有自己的球员（队列产出/旧图），不一定是工作台当前数据源
      target: (name: string) => `保存为 ${name} 的头像`,
      pathHint:
        "路径相对于 _generated 目录，可用 / 分多级；文件名已按命名模板避让，同名会自动加序号。",
      cancelBtn: "取消",
      ok: (name: string) => `已保存并替换 ${name} 的头像`,
      notMapped: (name: string) =>
        `图片已保存到 _generated 下的目标目录，但没能写入映射：当前头像包的 config.xml 缺失或结构异常。请检查头像包后重试。`,
      needSource: "请先选择数据源球员（名单预览行尾「去抽图」）",
    },
    // 服务商设置：Key 跟着服务商走，这里是唯一编辑入口（工作台只显示状态）
    keyBarLabel: "API Key",
    keyBarPlaceholder: "输入该服务商的 Key，加密保存在本机",
    keyBarSetPlaceholder: "已设置（不回显）；输入新值即替换",
    keyStatusUnset: "未设置",
    keyClear: "清除 Key",
    keyClearUndo: "撤销清除",
    keyWillClear: "保存后将清除该服务商的 Key（点右侧可撤销）",
    baseUrl: "接口地址 Base URL",
    /** 百炼基址的官方口径：公共域仍可用，业务空间专属域是官方建议迁移的新域名；
     *  地域必须与模型、API Key 一致，跨地域直接失败。 */
    baseUrlHintDashscope:
      "留空 = 华北2（北京）公共域 https://dashscope.aliyuncs.com/api/v1，官方明确仍可使用。" +
      "官方建议迁移到业务空间专属域 https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/api/v1" +
      "（性能与稳定性更好；WorkspaceId 在控制台的业务空间详情页）。" +
      "模型、接口地址、API Key 三者必须同地域，跨地域调用会直接失败。",
    keyStorageHint:
      "API Key 由本机加密保存，仅当前系统账户可解；更换设备或重装系统后需重新填写。",
    kind: "类型",
    providerIdLabel: "ID",
    providerIdPlaceholder: "留空自动生成",
    providerIdFixedNote:
      "ID 是服务商的唯一标识，创建后不可修改；如需换 ID 请新建服务商。",
    providerNameLabel: "名称",
    providerModelLabel: "默认模型",
    /** 模型清单：一套协议可挂多个模型（OpenAI 协议下有大量兼容模型），在此增删改，展开可编辑能力 */
    modelListLabel: "模型",
    modelNamePlaceholder: "模型名（API 参数）",
    addModel: "添加模型",
    removeModelTitle: "移除该模型",
    modelCapToggle: "展开／收起该模型的能力参数",
    /** 能力编辑区：协议参数名（中文语义名）+ 官方字段名小字，另加各参数的附属项标签 */
    capProtoPrompt: "正向提示词",
    capProtoSize: "输出分辨率",
    capProtoN: "出图张数",
    capProtoNegative: "反向提示词",
    capProtoSeed: "随机种子",
    capProtoExtend: "智能改写",
    capProtoExtendMode: "改写模式",
    capProtoThinking: "思考模式",
    capProtoWatermark: "水印",
    capProtoCallMode: "调用方式",
    capProtoQuality: "质量档位",
    capProtoResponseFormat: "返回 base64",
    /** 火山 Seedream output_format：Pro/Flash/Lite 可设 png/jpeg（值随用户选择）；4.5/4.0 仅 jpeg */
    capProtoOutputFormat: "输出格式",
    capProtoSteps: "采样步数",
    capProtoGuidance: "引导系数",
    /** 枚举型参数的默认档（当前仅改写模式：官方默认 direct） */
    capFieldModeDefault: "默认档",
    /** 官方字段原名（英文，含大小写/下划线/嵌套层级）：只作悬停核验用，名称列一律显示上面的中文名 */
    capApi: {
      prompt: "input.messages[].content[].text",
      size: "parameters.size",
      n: "parameters.n",
      negative: "parameters.negative_prompt",
      seed: "parameters.seed",
      extend: "parameters.prompt_extend",
      extendMode: "parameters.prompt_extend_mode",
      thinking: "parameters.enable_thinking",
      watermark: "parameters.watermark",
      callMode: "X-DashScope-Async",
      quality: "quality",
      responseFormat: "response_format",
      outputFormat: "output_format",
      // steps/guidance 仅硅基流动/魔搭用，不在 DashScope 行集合内；这里给出平铺基名（实际由 capApiByKind 覆盖）。
      steps: "steps",
      guidance: "guidance",
    },
    /** 平铺协议（OpenAI 兼容 / 火山方舟）的官方字段名：字段同名，但没有 DashScope 的
     *  `input.messages[]` 与 `parameters.` 两级嵌套，全部摆在请求体顶层。
     *  小字是给人对着文档核验用的，位置写错就会照着发错。缺项回落 capApi。 */
    capApiFlat: {
      prompt: "prompt",
      size: "size",
      n: "n",
      negative: "negative_prompt",
      seed: "seed",
      extend: "prompt_extend",
      extendMode: "prompt_extend_mode",
      thinking: "enable_thinking",
      watermark: "watermark",
      quality: "quality",
      responseFormat: "response_format",
      outputFormat: "output_format",
      steps: "steps",
      guidance: "guidance",
    } as Record<string, string>,
    /** 按服务商覆盖的官方字段名（优先于 capApiFlat）：硅基流动的尺寸/张数/步数/引导系数
     *  字段名与 OpenAI 不同（image_size / batch_size / num_inference_steps / guidance_scale），
     *  魔搭虽与 Openai 同名但也在此显式列出，避免小字写成会发错的字段名。 */
    capApiByKind: {
      siliconflow: {
        size: "image_size",
        n: "batch_size",
        steps: "num_inference_steps",
        guidance: "guidance_scale",
      },
      modelscope: {
        size: "size",
        n: "n",
        steps: "steps",
        guidance: "guidance",
      },
      // 本地 ComfyUI：参数是工作流节点输入（非扁平 API 字段），小字标注「节点.输入」便于对照导出的 workflow。
      localcomfyui: {
        prompt: "text (CLIPTextEncode)",
        size: "width/height (EmptyLatentImage)",
        n: "batch_size",
        negative: "negative (KSampler)",
        seed: "seed (KSampler)",
        steps: "steps (KSampler)",
        guidance: "cfg (KSampler)",
      },
    } as Record<string, Record<string, string>>,
    capFieldSizes: "取值",
    capRuleSide: "单边 px",
    capRuleTotal: "总像素",
    capRuleRatio: "宽高比",
    capZeroHint: "0 = 该条不检查",
    capFieldMaxN: "张数上限",
    capFieldMaxNHint: "0 = 不限张数",
    capFieldStepsDefault: "默认步数",
    capFieldStepsMax: "步数上限",
    capFieldGuidanceDefault: "默认引导",
    capFieldGuidanceMax: "引导上限",
    capFieldMaxChars: "提示词上限",
    capFieldMaxCharsHint: "0 = 不限字数",
    capFieldQualities: "取值",
    capFieldSizesPlaceholder: "2048x2048, 2688x1536",
    capFieldQualitiesPlaceholder: "low, medium, high",
    addProvider: "新建服务商",
    providerSaved: "服务商已保存",
    deleteProviderConfirm: (name: string) => `删除服务商「${name}」？`,
    // 模板管理
    templateManageTitle: "提示词模板",
    templateNameLabel: "模板名称",
    templateBodyLabel: "模板正文（占位符写法 {{变量|默认值}}）",
    templateNegativeLabel: "负向提示词",
    addTemplate: "新建模板",
    deleteTemplateConfirm: (name: string) => `删除模板「${name}」？`,
    // 通用
    edit: "编辑",
    remove: "删除",
    back: "返回",
    done: "完成",
    colStatus: "状态",
    // 生成队列（批量）
    queue: {
      tabQueue: "生成队列",
      startBtn: "启动生成",
      cancelBtn: "取消生成",
      /** 已请求取消、在跑条目尚未收尾时的按钮文案 */
      cancellingBtn: "取消中…",
      /** 「查看单人」：把唯一勾选的队列条目转入「球员源·单人」卡（原「清空队列」职责由全选+移除覆盖） */
      viewSingle: "查看单人",
      viewSingleTitle: (name: string) => `把 ${name} 转入「球员源·单人」卡`,
      viewSingleHint: "勾选一条队列条目后，可将其球员转入「球员源·单人」卡",
      removeBtn: "移除选中",
      addBtn: (n: number) => `加入队列（${n}）`,
      statusQueued: "排队中",
      statusRunning: "生成中",
      statusSuccess: "成功",
      statusFailed: "失败",
      /** 用户取消（数据上是 failed + errorKind=cancelled），与「失败」不是一回事 */
      statusCancelled: "已取消",
      infoConfig: "配置",
      /** 队列运行中：单人卡与工作台被这条任务占用 */
      queueRunningTag: "队列生成中",
      infoPrompt: "提示词",
      infoNegative: "负向",
      colPlayer: "球员",
      colDetail: "详情",
      // ---- 启动方式（手动确认 / 全自动）与队列行的逐条确认 ----------------------------------------------------------
      /** 确认方式分段控件：左「手动确认」、右「全自动」（用户指定的顺序） */
      confirmModeManual: "手动确认",
      confirmModeAuto: "全自动",
      confirmModeHintManual: "启动前需逐条确认：点队列行，在右侧核对并修改该条参数",
      confirmModeAria: "确认方式",
      /** 单条确认按钮：把该行核对过的参数写回队列 */
      confirmBtn: "确认",
      /** 已确认态：按钮同位置转灰不可点 */
      confirmedBtn: "已确认",
      /** 启动被拦：还有 N 条没逐条确认 */
      needConfirm: (n: number) => `还有 ${n} 条未确认，请先逐条确认后再启动。`,
      /** 手动模式但未选中任何行时的引导（贴在原表单上方） */
      queueEditHint: "点左侧队列行，在这里核对并修改该条的生成参数。",
      /** 借用失败：该行入队后它的服务商/模型已不在配置里（能力位未知，不能在这里改参数） */
      borrowUnavailable: (name: string) => `「${name}」用的服务商或模型已不在配置里，无法核对这条的参数；可勾选后点「移除」，或从名单重新加入后再核对。`,
      count: (total: number) => `共 ${total} 条`,
      startConfirm: (n: number) =>
        `开始批量生成 ${n} 个排队任务？将逐条调用服务商接口并产生费用。`,
      /** 只跑勾选的排队条目（其余留在队列里） */
      startConfirmPicked: (n: number, rest: number) =>
        `开始批量生成已勾选的 ${n} 个任务？未勾选的 ${rest} 条会留在队列里，可稍后再启动。`,
      cancelConfirm:
        "取消后未提交的条目立即取消；已提交的会等待本次请求返回并保留结果（会产生费用）。确定取消？",
      cancellingHint:
        "正在取消：未提交的已停止；已提交的等待服务端返回后结束（会产生费用）。",
      removeConfirm: (n: number) => `移除选中的 ${n} 条条目？`,
      empty: "队列为空。在「名单预览」勾选球员后点「加入队列」，即可批量生成。",
      enqueued: (n: number) => `已加入 ${n} 条生成任务`,
      imagesDone: (n: number) => `生成 ${n} 张图`,
      progressLabel: "批量生成进度",
      /** 队列进度计数：首个数字是**已完成数**（终态条目累计），不是派发序号。 */
      progressCount: (done: number, total: number) => `已完成 ${done} / 总数 ${total}`,
      /** 运行中条目的阶段文案：与后端 GenStage 的 as_str 取值一一对应（submitting/polling/downloading）。 */
      stageLabel: (stage: string): string => {
        const labels: Record<string, string> = {
          submitting: "提交中",
          polling: "服务端处理中",
          downloading: "下载中",
        };
        return labels[stage] ?? "";
      },
      finishedSummary: (finished: number, failed: number, total: number) =>
        `批量生成结束：成功 ${finished} · 失败 ${failed} · 共 ${total} 条`,
      retryEntry: "重试本条",
      /** 执行期重试徽标：attempts 是「总尝试次数」（首发即 1），故实际重试次数 = attempts - 1。
       *  仅在 attempts > 1（即确实重试过）时展示。 */
      retriedBadge: (attempts: number) => `已重试 ${attempts - 1} 次`,
      /** 失败条目按错误类别给出的处置建议（对应后端 GenErrorKind）；无建议类别返回空串。 */
      errorAction: (kind: string): string => {
        const actions: Record<string, string> = {
          auth: "检查 API Key 是否正确或已过期",
          rate_limited: "稍后重试或降低并发",
          not_configured: "到服务商设置填写 Key/模型",
          invalid_param: "按提示调整参数",
          content_rejected: "修改提示词后重试",
          network: "检查网络后重试",
          provider: "稍后重试，持续失败请查看服务商状态",
          io: "检查磁盘空间与目录权限",
        };
        return actions[kind] ?? "";
      },
      viewImages: "查看图片",
      collapseImages: "收起图片",
      saveImage: "另存",
      // 复选框的读屏文案：队列条目（QueueDock）与名单表格（PlayersTable）各一套
      selectRowAria: (name: string) => `选择 ${name}`,
      selectAllAria: "全选或取消本页条目",
      selectPlayerAria: (name: string) => `选择球员 ${name}`,
      selectAllPlayersAria: "全选或取消全部筛选结果（跨页）",
    },
  },

  /** 「批量替换」面板 */
  replace: {
    cardTitle: "批量替换",
    modeLabel: "模式：",
    optionsLabel: "选项设置：",
    optAllowDuplicates: "允许复用图片",
    optFilterNewgan: "仅随机人",
    optSaveBackup: "写入前备份",
    /** 三个替换模式：value 发往后端（协议值，勿改）；label/desc 为界面显示文案（与后端日志用词一致） */
    modes: [
      {
        value: "Preserve",
        label: "保留",
        desc: "保留已换过的球员，只为新球员补图（推荐日常增量）",
      },
      {
        value: "Overwrite",
        label: "覆写",
        desc: "名单内球员全部重新抽图，名单外保留",
      },
      {
        value: "Generate",
        label: "生成",
        desc: "无视现有 config.xml，从零生成全部映射",
      },
    ] as ReplaceMode[],
    startBtn: "替换头像",
    runningBtn: "替换中", // 按钮上会追加实时百分比，如「替换中 45%」
    cancelBtn: "取消",
    browseDisabledHint: "请先在左侧「激活」存档后再执行替换（当前为浏览预览）",
    browseNotice: (name: string) =>
      `正在浏览「${name}」（未激活）。替换只作用于已激活的存档，请先在左侧点「激活」。`,
    needProfile: "选择 Profile",
    needGroup: "选择头像包",
    needRtf: "设置 RTF 名单",
    needRoster: "先生成球员/职员名单缓存（运行游戏读取或扫描 .fm 存档）",
    todoPrefix: "还需完成：",
    missingDirsTitle: "缺失目录",
    missingDirsConfirm: (standard: number, count: number) =>
      `标准${standard} 缺少 ${count} 个目录，自动创建并继续？`,
    unknownTitle: "无法识别的结构",
    unknownConfirm:
      "头像包结构无法识别，替换时将把包内所有图片（含子/孙目录）随机分配，仍要继续？",
    // 发起前预检（preflight_replace）：只有「生成」+ 本档档景非空时会走到这条确认。
    preflightTask: "检查既有映射",
    blankTitle: "将清空本档现有映射",
    blankConfirm: (existing: number) =>
      `「生成」模式不携带既有条目。若本次一条也没抽到（名单被筛空、UID 对不上、包里没有图），\n` +
      `会用空表覆盖本档现有的 ${existing} 条映射。\n\n确认仍要这样执行？`,
  },

  /** 结果区顶部 Tab（替换台读视图：名单/快照；单人替换已常驻替换台左栏，AI 生图已独立成台） */
  results: {
    tabTable: "名单预览",
    tabSnapshot: "快照预览",
  },

  /** 名单预览表（PlayersTable） */
  table: {
    title: "名单预览",
    recordCount: (total: number) => `${total} 条`,
    searchPlaceholder: "搜索 UID / 姓名 / 国籍 / 俱乐部…",
    prevPage: "‹ 上一页",
    nextPage: "下一页 ›",
    pageOf: (current: number, total: number) => `第 ${current} / ${total} 页`,
    /** 快速跳页：「前往 [输入框] 页 [跳转]」 */
    gotoPrefix: "前往",
    gotoSuffix: "页",
    gotoBtn: "跳转",
    gotoAria: "输入页码后回车快速跳转",
    colUid: "UID",
    colNat: "国籍",
    col2ndNat: "第二国籍",
    colName: "姓名",
    colHair: "发长",
    colColor: "发色",
    colEth: "种族",
    colSkin: "肤色",
    colAvatar: "头像",
    /** 批量取头像失败时挂在「头像」列表头提示上（逐图取不到仍是静默留白，那本来就是正常情况）。 */
    avatarFailHint: (why: string) =>
      `头像取数失败（${why}），本列留空；这不是「名单里没有脸」`,
    colNewgan: "随机人",
    colGender: "性别",
    colAge: "年龄",
    colHeight: "身高",
    colClub: "俱乐部",
    colBased: "所在地",
    colJob: "偏好工作",
    colCa: "能力",
    colPa: "潜力",
    colRca: "R.C.A",
    /** 头像列缩略图 + 悬停/点击气泡的状态文案。 */
    avatarBtn: "预览",
    avatarAria: "预览该球员的头像图片",
    avatarClose: "关闭头像预览",
    avatarLoading: "加载中…",
    avatarNotFound: "未找到头像图片",
    /** 随机人筛选（仅拓展名单显示）：标签 + 三个选项的文案。 */
    newganFilterLabel: "随机人",
    newganAny: "任意",
    newganYes: "是",
    newganNo: "否",
    /** 球员/职员视图切换（仅混合名单显示）：标签 + 三个选项 + 视图内无记录的空态文案。 */
    rosterViewLabel: "视图",
    rosterViewAll: "全部",
    rosterViewPlayer: "球员",
    rosterViewStaff: "职员",
    rosterViewEmpty: "当前视图没有匹配的记录。",
    /** 混合名单下第 15 列「全部」视图的表头：两种语义并列（三字典同值，不逐词翻译）。 */
    rosterViewMixedHeader: "所在地 / 偏好工作",
    emptyHint:
      "执行「替换头像」或「扫描RTF」后可在此预览名单（最近一次解析结果）。",
    /** 缓存里没有匹配当前筛选条件的记录（与快照预览的 noMatchHint 同款语义）。 */
    noMatchHint: "没有匹配的球员。",
    /** 替换台「拓展列」开关：补齐仅拓展名单才有的字段列 */
    extCols: "拓展列",
  },

  /** 单人预览/微调（PlayerViewer） */
  viewer: {
    title: "球员预览",
    /** 替换台左栏单人替换卡的标题（读写分栏后常驻左栏，不属于结果区读视图） */
    section: "单人替换",
    hint: "输入关键词后回车搜索",

    searchBtn: "搜索",
    /** 命中多个时的前后切换（到头即禁用） */
    prevMatch: "上一个",
    nextMatch: "下一个",
    colMapping: "当前映射",
    /** 拓展名单第 19 列起的用户自定义列（后端只保留原文，无表头名，按序号命名）。 */
    extraField: (n: number) => `自定义${n}`,
    noImage: "暂无图片",
    imgPlaceholder: "输入新图片路径…",
    imgPathAria: "替换图片路径",
    browseBtn: "浏览",
    replaceBtn: "指定替换",
    /** 重掷替换：按标准规则给该球员重新抽图（后端固定重抽语义） */
    resetBtn: "重掷替换",
    resetOk: (uid: string) => `已重掷 UID ${uid} 的头像（按规则重新抽图）`,
    imageDialogTitle: "选择图片文件",
    imageFilterName: "图片",
    missingInput: "请先搜索并选中一名球员，再选择有效图片路径",
    replacedOk: (uid: string) => `已替换 UID ${uid} 的头像`,
    mappingNotFound: (uid: string) =>
      `config.xml 中未找到 UID ${uid} 的现有映射，未做修改`,
  },

  /** 快照预览（SnapshotPreview）：展示「读取快照」解析出的存档快照映射。分页器与名单预览共用（TablePager，文案取 t.table.*） */
  snapshot: {
    searchPlaceholder: "按 UID 筛选…",
    recordCount: (total: number) => `${total} 条`,
    colUid: "UID",
    colPath: "头像相对路径",
    colImage: "头像图",
    noImage: "无图",
    // 未读取（或切档/切组后失效）时的引导文案
    readHint:
      "点击「当前任务」中的「读取快照」，解析存档 XML 快照后在此预览映射列表。",
    noMatchHint: "没有匹配的 UID。",
    emptyHint: "快照中没有映射记录。",
  },

  /** 底部状态栏 */
  status: {
    ready: "就绪",
    noGroup: "未选择头像包",
    log: "日志",
  },

  /** 日志抽屉 */
  log: {
    title: "运行日志",
    onlyLevel: "仅该级别",
    openFileBtn: "打开日志文件",
    clearBtn: "清空显示",
    closeAria: "关闭日志面板",
  },

  /** 标准规则编辑对话框 */
  ruleEditor: {
    std2Label: "标准二（14 种族目录）",
    std3Label: "标准三（细分目录）",
    title: "规则配置",
    kicker: (standard: number, dirs: number) => `STANDARD ${standard} · ${dirs} 个目录`,
    sourceBtn: "源码",
    structuredBtn: "结构化",
    sourceHint: "以 JSON 文本编辑；保存前由后端校验格式与目录引用",
    sourceTitle: "JSON 源码",
    sourceDesc: "高级模式：直接编辑整份规则文件。切回「结构化」时会先解析，格式错误会留在本页提示。",
    parseFailed: (detail: string) => `无法结构化解析：${detail}。请在源码中修复后再切回。`,
    loading: "正在载入规则文件…",
    saveBtn: "保存",
    closeBtn: "关闭",
    saveHint: "保存前会校验 JSON 格式，非法内容不会被写入",
    savedStatus: "已保存 ✓ 下次替换 / 检查完整性即生效",
    unsavedTitle: "未保存修改",
    unsavedSwitch: (label: string) =>
      `${label} 有未保存的修改，确定放弃并切换？`,
    unsavedClose: (label: string) => `${label} 有未保存的修改，确定关闭？`,
    editorAria: "规则配置内容",
    sections: {
      overview: "概览",
      folders: "目录清单",
      ethnics: "国籍映射",
      codeRules: "种族码修正",
      blocks: "规则块",
      settings: "全局设置",
    },
    statFolders: "目录",
    statNations: "国籍",
    statCodeRules: "码修正",
    statRaceCodes: "种族码",
    statBlocks: "规则块",
    statRuleEntries: "规则条目",
    common: {
      add: "添加",
      delete: "删除",
      notInList: "不在清单",
    },
    overview: {
      title: "配置概览",
      desc: "先体检再编辑；保存前后端会做最终校验，非法目录引用不会写入。",
      healthTitle: "配置体检",
      healthOk: "结构检查通过",
      healthWarn: (count: number) => `${count} 项需要处理`,
      healthOkDesc: "没有发现重复目录、失效引用或缺失默认块。",
      healthEmptyFolders: "目录清单为空，至少需要一个目录",
      healthEmptyEthnics: "还没有国籍映射，所有球员都会被跳过",
      healthDup: (count: number) => `目录清单有 ${count} 个重复项`,
      healthEthnics: (count: number) => `${count} 条国籍映射指向清单外目录`,
      healthRefs: (count: number) => `${count} 处规则引用了清单外目录`,
      healthMissingDefault: "缺少 _DEFAULT_ 全局默认块",
      healthVersion: (version: number) => `Version 为 ${version}，仅支持 4`,
      healthOrphans: (count: number) => `${count} 个 DefaultCodes 键没有对应国籍`,
      goFix: "去修复",
      flowTitle: "抽图链路",
      flow2: ["国籍映射", "种族码修正", "目录池抽图"],
      flow3: ["国家块", "族群块", "_DEFAULT_ 全局块"],
      flowHint: "规则块按顺序取该种族码的规则；块内缺码会继续穿透，目录不可用时走 Fallback。",
      fileLabel: "文件",
    },
    folders: {
      title: "目录清单",
      desc: "FolderList 是判定标准与抽图的目录白名单；不在这里的目录不会被扫描或使用。",
      searchPlaceholder: "搜索目录",
      addPlaceholder: "新增目录名",
      empty: "没有匹配的目录",
      duplicate: "该目录已在清单中",
      blank: "目录名不能为空",
      used: (count: number) => `${count} 处引用`,
      removeHint: "从清单中删除该目录",
      removeConfirm: (name: string, count: number) =>
        count > 0
          ? `目录「${name}」被 ${count} 处引用，删除后这些引用会失效，需手动重新指定。继续？`
          : `删除目录「${name}」？`,
      count: (n: number) => `${n} 个目录`,
    },
    ethnics: {
      title: "国籍映射",
      desc: "国籍三字码 → 本国基础池目录；名单里查不到映射的国籍会被跳过。",
      searchPlaceholder: "搜索国籍码或目录",
      addCodePlaceholder: "国籍码，如 CHN",
      codeHeader: "国籍码",
      folderHeader: "归属目录",
      exists: "该国籍码已存在",
      invalidCode: "国籍码应为 2–3 位大写字母",
      noFolders: "请先在「目录清单」中添加目录",
      empty: "没有匹配的国籍",
      removeHint: "删除该国籍映射",
      removeConfirm: (code: string) => `删除国籍 ${code} 及关联的默认种族码/国家规则块？`,
      count: (n: number) => `${n} 个国籍`,
    },
    codeRules: {
      title: "种族码修正",
      desc: "按 FM 种族码修正抽图目录：优先列表按顺序匹配主/第二国籍目录，都不命中时使用兜底目录。",
      codeLabel: (code: string) => `种族码 ${code}`,
      unset: "未配置——沿用国籍目录",
      addBtn: "添加修正",
      deleteHint: "删除该种族码修正",
      preferLabel: "优先目录（按顺序匹配）",
      fallbackLabel: "兜底目录",
      emptyPrefer: "未设置优先目录，直接使用兜底目录",
      addPlaceholder: "添加优先目录",
      addBtnShort: "添加",
      moveUp: "上移",
      moveDown: "下移",
      removeDir: "移除该目录",
    },
    blocks: {
      title: "规则块",
      desc: "先查国家块 → 再查族群块 → 最后落到 _DEFAULT_；块内缺码会继续穿透到下一级。",
      searchPlaceholder: "搜索规则块",
      addPlaceholder: "选择新增规则块的键",
      defaultKind: "全局",
      nationKind: "国籍",
      folderKind: "族群",
      unknownKind: "未知",
      count: (n: number, total: number) => `${n}/${total} 码`,
      empty: "没有匹配的规则块",
      noSelection: "从左侧选择一个规则块开始编辑",
      detailDesc: "块内某个码未配置时，该码会继续向下一个块穿透；这是合法配置。",
      codeMissing: "未配置（穿透下一级）",
      addRuleBtn: "添加规则",
      deleteRuleHint: "删除该码的规则",
      deleteBlockHint: "删除该规则块",
      deleteBlockConfirm: (key: string) => `删除规则块「${key}」？`,
      unknownRule: "规则结构无法识别，请到「源码」修复或重置",
      resetRuleBtn: "重置为默认",
      typeLabel: "映射方式",
      typeUnknown: "无法识别",
      typeDefault: "默认",
      typeDirectly: "直接",
      typeRandom: "随机",
      typeWeighted: "加权",
      typeDefaultHint: "使用球员国籍在 Ethnics 里的归属目录",
      typeDirectlyHint: "固定映射到一个目录",
      typeRandomHint: "从目录列表中等概率选一个",
      typeWeightedHint: "按权重随机选一个，权重按总和归一",
      directoryLabel: "目录",
      addDirPlaceholder: "添加目录",
      addDirBtn: "添加",
      weightLabel: "权重",
      addWeightBtn: "添加加权项",
      removeItem: "移除",
      atLeastOne: "至少保留一项",
      extraCodes: (count: number) => `另有 ${count} 个非 0–10 的码未在此编辑（保存时原样保留）`,
    },
    settings: {
      title: "全局设置",
      desc: "规则文件的版本与目录不可用时的兜底行为。",
      versionLabel: "Version",
      versionHint: "标准三规则格式版本，仅支持 4",
      fallbackLabel: "兜底策略",
      fallbackRandom: "全池随机",
      fallbackRandomHint: "规则目录不可用时，从包内递归全池随机抽图",
      fallbackSkip: "跳过",
      fallbackSkipHint: "规则目录不可用时跳过该球员",
      passthroughTitle: "透传数据",
      passthroughDesc:
        "DefaultCodes / FolderMapping* 由出厂配置维护，结构化界面不改写；保存时原样保留，可在「源码」中编辑。",
      passthroughEditBtn: "在源码中编辑",
      count: (n: number) => `${n} 条`,
    },
  },

  /** 国籍对照表编辑对话框 */
  nationalityEditor: {
    aria: "国籍对照表",
    title: "国籍对照表",
    subtitle:
      "三字码 ↔ 各语言国籍全称（含别名）；码与全称分开维护。保存后下一次扫描 RTF / 替换 / 提示词预览即生效，文件：data/nationalities.json",
    searchPlaceholder: "搜索三字码、语言或全称",
    countLabel: (shown: number, total: number) => `显示 ${shown} / 共 ${total} 条`,
    codeHeader: "三字码",
    languageHeader: "语言",
    nameHeader: "国籍全称/别名",
    languageLabel: "语言列表",
    addCodePlaceholder: "三字码，如 CHN",
    addNamePlaceholder: "该语言下的国籍全称（留空 = 只登记码）",
    addBtn: "新增",
    addLanguagePlaceholder: "语言名，如 Deutsch",
    addLanguageBtn: "添加语言",
    languageExists: "该语言已在列表中",
    duplicate: "这条映射已存在",
    codeOnly: "已登记该码（暂无全称）",
    codeExists: "该码已收录",
    incomplete: "请填写三字码（全称可留空）",
    bareLabel: (n: number) => `仅有码（暂无全称）${n} 个`,
    removeCodeHint: "移除该码：码退出码全集后不再算合法国籍值",
    noMatch: "没有匹配的映射",
    deleteBtn: "删除",
    deleteHint: "删除这条映射（点保存后生效）",
    emptyHint: "三字码为 2–3 位大写字母；同一语言下同一全称只能对应一个码；至少保留一个码或一条映射。编辑后点「保存」写盘。",
    saveBtn: "保存",
    closeBtn: "关闭",
    saveHint: "保存前会校验映射并原子写盘；非法内容不会写入",
    savedStatus: "已保存 ✓ 下次扫描 / 替换即生效",
    unsavedTitle: "未保存修改",
    unsavedClose: "国籍对照表有未保存的修改，确定关闭？",
  },
};

/** 字典结构类型：以 zh-CN 为基准推导，zh-TW / en 须满足同结构（含各动态文案的函数签名）。 */
type Dict = typeof zhCN;

/** 繁體中文（台湾用词：OpenCC s2twp 转换后人工校对术语）。 */
const zhTW: Dict = {
  /** 应用品牌名（窗口/弹窗标题、侧栏品牌区共用；品牌名不翻译） */
  brand: "FM24 NewGen Panel",

  /** 通用系统弹窗标题 */
  dialog: {
    error: "錯誤",
    info: "提示",
    warning: "警告",
    /** 结构化错误里 userAction 的引导词（如「建议：检查 API Key」）。 */
    suggestion: "建議：",
  },

  /** 根组件：替换结束的系统提示 */
  app: {
    replaceFinished: "替換完成！",
    replaceFailed: "替換失敗，請檢視日誌",
    brandVer: "v2.0",
  },

  /** 顶栏三工作台（chip 可点击切换，右侧状态点随就绪度变色） */
  workbench: {
    archive: "存檔資料",
    replace: "批次/單人頭像替換",
    ai: "AI 生成頭像",
    /** 顶栏右端作用域回显：档名与组名之间的分隔符（组为空时只显示档名） */
    scopeSep: " · ",
    /** 无头像包组时的回显占位（侧栏还没关联目录） */
    scopeNoGroup: "未選頭像包",
    /** chip 状态点的悬停说明（兩檔：綠=就緒 / 橙=未就緒） */
    stateOk: "已就緒",
    stateWarn: "未就緒，點進去看缺少的條件",
  },

  /** 侧栏底部主题切换 */
  theme: {
    dark: "深色",
    light: "淺色",
    switchTo: (mode: "dark" | "light") =>
      mode === "light" ? "切換到淺色主題" : "切換到深色主題",
  },

  /** 侧栏底部「语言」菜单：按钮名 + 三个选项的自显示名（各语言写自己的名字） */
  lang: {
    label: "語言",
    zhCN: "简体中文",
    zhTW: "繁體中文",
    en: "English",
  },

  /** 内置文档面板：侧栏「教程 / FAQ」两个入口各打开同一面板的对应页签 */
  help: {
    title: "使用文件",
    subtitle: "內建教學與常見問題，離線可查",
    tutorialTab: "教學",
    faqTab: "FAQ",
    searchPlaceholder: "搜尋章節或關鍵字…",
    searchHint: "Ctrl+F 聚焦搜尋",
    resultCount: (shown: number, total: number) => `顯示 ${shown} / 共 ${total} 筆`,
    noResults: "沒有符合的項目，換個關鍵字試試。",
    clearSearch: "清除搜尋",
    note: "提示",
    warning: "注意",
  },

  /** 侧边栏：存档（Profile）与头像包（组）管理 */
  sidebar: {
    profileSection: "存檔資料",
    newProfileBtn: "新建 Profile",
    newProfilePlaceholder: "新 Profile 名稱",
    createBtn: "建立",
    notActivated: "未啟用",
    activated: "已啟用",
    deleteCurrentProfile: "刪除當前 Profile",
    noProfileNoActivate: "No Profile 為啟動預設檔，未啟用",
    noProfileNoDelete: "預設檔不可刪",
    activeAlreadyHint: "當前已啟用該存檔",
    activateBtn: "啟用",
    activateHint: (name: string) => `啟用存檔「${name}」並切換其組配置與對映`,
    deleteProfileConfirm: (name: string) =>
      `刪除 Profile「${name}」及其全部組配置與對映備份？`,
    deleteProfileTitle: "刪除 Profile",
    groupSection: "頭像包（組）",
    linkNewHint: "關聯新的頭像包目錄",
    linkBrowseHint: "請先點選「啟用」後再關聯頭像包",
    linkDialogTitle: "關聯頭像包目錄",
    selectGroupBrowseHint: "請先點選「啟用」後再切換當前組（當前為瀏覽預覽）",
    deleteGroupBrowseHint: "請先啟用後再刪除組",
    deleteGroupBtn: "刪除組",
    deleteGroupConfirm: (dir: string) => `刪除組及其對映備份？\n${dir}`,
    deleteGroupTitle: "刪除組",
    /** 队列运行中的切档/切组护栏：空闲切换不弹窗不打断，运行中才问这一句 */
    queueScopeTitle: "AI 頭像佇列正在執行",
    queueScopeConfirm:
      "切換存檔或頭像包後，未完成的生成任務將被取消。確定要切換嗎？",
    /** 队列运行中执行删除时追加在删除确认文案后的那一段。删除本身已有确认框，
     *  这里并入同一段正文，避免「先问删除、再问队列」连弹两个框（一次操作只打扰一次）。 */
    queueScopeAppend: "\n\nAI 頭像佇列正在執行，繼續將取消未完成的生成任務。",
    rtfSet: "RTF ✓",
    rtfUnset: "未設定 RTF",
    fmSet: "FM ✓",
    fmUnset: "未設定 FM",
    emptyGroups: "暫無頭像包。點上方 ＋ 關聯頭像包目錄。",
    linkHome: "首頁",
    linkTutorial: "教程",
    linkFaq: "FAQ",
    langMenuTitle: "切換介面語言",
  },

  /** 「当前任务」卡：路径展示、完整性检查、扫描、规则配置入口 */
  setup: {
    cardTitle: "當前任務",
    imgDirLabel: "頭像包：",
    rtfLabel: "RTF 名單：",
    xmlLabel: "config.xml：",
    imgDirPlaceholder: "未關聯頭像包目錄",
    rtfUnsetValue: "未設定",
    xmlPlaceholder: "未關聯頭像包（執行替換後生成）",
    noGroupHint: "請先關聯頭像包目錄",
    checkIntegrityBtn: "檢查完整性",
    checkIntegrityHint: "按規則 json 的 FolderList 識別頭像包歸屬哪個標準",
    scanImagesBtn: "掃描頭像",
    scanImagesHint: "掃描頭像包並快取圖片索引（結果寫入日誌）",
    selectRtfBtn: "選擇RTF",
    selectRtfDialogTitle: "選擇 RTF 名單",
    selectRtfHint: "選擇 FM 匯出的 RTF 名單",
    selectRtfBrowseHint: "請先點選「啟用」後再設定 RTF（當前為瀏覽預覽）",
    scanRtfBtn: "掃描RTF",
    scanRtfHint: "解析 RTF 並填充名單預覽（結果寫入日誌）",
    selectFmDialogTitle: "選擇 FM 存檔",
    scanFmFileBtn: "掃描.fm存檔",
    scanFmFileHint: "選擇 .fm 存檔並立即掃描，結果刷新球員/職員快取",
    selectFmBrowseHint: "請先點選「啟用」後再掃描 .fm 存檔（當前為瀏覽預覽）",
    readMemoryBtn: "執行遊戲讀取",
    readMemoryHint: "掃描正在執行的 FM 行程記憶體，結果刷新球員/職員快取（無需 .fm 存檔）",
    viewConfigBtn: "讀取快照",
    viewConfigHint: "解析存檔 XML 快照，在結果區「快照預覽」中檢視對映列表",
    viewSnapshotBtn: "檢視位置",
    viewSnapshotHint: "開啟存檔 XML 快照所在目錄",
    noGroupGuide: "先在左側「頭像包（組）」關聯一個頭像包目錄，或點選已有組。",
    tpl1Complete: "標準一：圖片直鋪，無種族子目錄",
    tplComplete: (standard: number, total: number) =>
      `標準${standard}：${total} 個配置目錄齊全`,
    integrityOk: (label: string) => `✓ 頭像包結構完整（${label}）`,
    extraDirsNote: (n: number) =>
      n > 0 ? `另含 ${n} 個未配置目錄（不參與抽圖）` : "",
    integrityFixable: (
      standard: number,
      missing: number,
      found: number,
      total: number,
      list: string,
    ) =>
      `標準${standard}：缺少 ${missing} 個目錄（已有 ${found}/${total}）：${list}`,
    createMissingBtn: "一鍵建立缺失目錄",
    missingDirsTitle: "缺失目錄",
    missingDirsConfirm: (standard: number, count: number, list: string) =>
      `標準${standard} 缺少 ${count} 個目錄：\n${list}\n\n自動建立？`,
    integrityUnknown:
      "⚠ 無法識別頭像包結構：子目錄與任何標準的 FolderList 清單都不構成包含關係，替換時將把包內全部圖片隨機分配；也可在「規則配置」中調整標準目錄清單。",
    // 完整性状态胶囊的短文案（详情仍用上方 integrityFixable / integrityUnknown 长句）
    pillMissingDirs: (missing: number, found: number, total: number) =>
      `缺少 ${missing} 個目錄（已有 ${found}/${total}）`,
    pillUnknown: "無法識別頭像包結構",
    ruleConfigBtn: "規則配置",
    ruleConfigHint:
      "結構化編輯目錄清單、國籍對映、種族碼修正與規則區塊；也可切到「原始碼」直接改 JSON，儲存時自動校驗",
    nationalityConfigBtn: "國籍對照表",
    nationalityConfigHint:
      "調整、新增或刪除國籍三字碼與各語言國籍全稱的對照映射（儲存後下次掃描/替換即生效）",
  },

  gen: {
    // 工作台顶部与面板
    provider: "服務商",
    model: "模型",
    providerSettings: "服務商設定",
    /** 服务配置条就绪状态灯：判据与顶栏 ③chip 同源（Key 已设置 + 模型已选），不冒充「已连通」 */
    statusReady: "已就緒",
    statusNoKey: "未設定 Key",
    statusNoModel: "未選模型",
    statusReadyTitle: "Key 已設定且模型已選（連通性以實際生成為準）",
    /** 就緒狀態燈缺 Key 檔的 title：講清後果 + 指出去哪設定 */
    keyMissingHint:
      "生成前需先設定當前服務商的 API Key——點此開啟服務商設定",
    cardSource: "球員源 · 單人",
    cardSettings: "生成設定",
    resultsTitle: "本次結果",
    resultsCount: (n: number) => `${n} 張`,
    clearResults: "清空",
    clearResultsTitle: "清空本次會話的全部生成結果",
    clearResultsConfirm: (n: number) =>
      `清空本次會話的 ${n} 張生成結果？臨時圖片將被刪除，未手動儲存的無法恢復。`,
    template: "提示詞模板",
    manageTemplate: "管理模板",
    /** 模板管理弹窗：变量胶囊（点击在正文光标处插入占位符） */
    templateVariables: "可用變數（點選插入游標處）",
    prompt: "提示詞（可手動編輯）",
    /** 负向词折叠条（默认收起；展开/收起同一行，随折叠图标切换） */
    negativeCollapsed: "負向提示詞（可選）",
    negative: "負向提示詞（模型不支援時僅記錄不傳送）",
    /** 画幅预览帧：尺寸不合规时的简短状态（详细原因仍显示在自定义宽高行下） */
    sizeInvalidShort: "尺寸不合規",
    regenTitle: "按該圖生成參數再生成",
    resultAlt: "生成結果圖",
    previewAlt: "生成圖預覽",
    // 连接卡下方的模型能力摘要（与参数控件显隐同一能力集口径）
    capNoModel: "未選擇模型明細行：僅可填寫介面參數，高階能力不可用",
    capPresetSizes: (n: number) => `${n} 個預設尺寸`,
    capFreeSize: "自由尺寸",
    capMaxN: (n: number) => `單次最多 ${n} 張`,
    capNAny: "張數不限",
    capQuality: "品質可選",
    capSeed: "支援 Seed",
    capNegative: "負向提示詞",
    capPromptExtend: "智慧改寫",
    capWatermark: "水印可選",
    size: "尺寸",
    // 尺寸下拉末尾的「自定义…」项 + 宽高输入与提醒（选中后显示）
    sizeCustom: "自定義…",
    sizeWLabel: "寬",
    sizeHLabel: "高",
    sizeCustomHint:
      "非官方推薦尺寸可能被廠商拒絕，生成失敗時請改選上方比例回填。",
    ratio: "比例",
    n: "張數",
    nSingle: "該模型每次僅生成 1 張",
    nLess: "減少",
    nMore: "增加",
    nCount: "數量",
    nMaxHint: (n: number) => `單次最多 ${n} 張`,
    quality: "品質",
    qualityAuto: "自動",
    qualityLow: "低",
    qualityMedium: "標準",
    qualityHigh: "高",
    promptExtend: "智慧改寫提示詞",
    promptExtendHint:
      "由大模型最佳化提示詞，出圖更慢；z-image 開啟後為思考模式，按更高單價計費；遇版權稽核攔截可關閉重試",
    extendMode: "改寫模式",
    extendModeDirect: "直接改寫",
    extendModeAgent: "智慧體改寫",
    extendModeAgentHint: "智慧體改寫僅支援文生圖，圖生圖場景會被廠商拒絕",
    thinking: "思考模式",
    thinkingHint: "提升出圖品質但更慢；需先開啟智慧改寫才生效",
    callMode: "呼叫方式",
    callModeSync: "同步",
    callModeAsync: "非同步",
    watermark: "新增「AI生成」水印",
    watermarkHint: "水印位於圖片右下角",
    /** 輸出圖片格式（火山 Seedream / 矽基流動）：選項值是協議欄位值 png/jpeg，不翻譯 */
    outputFormat: "輸出格式",
    outputFormatHint: "服務端回傳的圖片編碼格式；僅宣告該參數的模型可選",
    seed: "Seed",
    steps: "取樣步數",
    stepsHint: "擴散取樣步數，越大通常越慢、細節可能更充分",
    guidance: "引導係數",
    guidanceHint: "提示詞引導強度，越大越貼合提示詞，越小越多變",
    sizeError: {
      format: "尺寸格式不正確，應為 寬×高 或 寬*高（如 1280×1280）",
      minSide: (v: number) => `寬/高不能小於 ${v} 畫素`,
      maxSide: (v: number) => `寬/高不能超過 ${v} 畫素`,
      totalMin: (v: number) => `總畫素過小，建議約 ${v}×${v} 以上`,
      totalMax: (v: number) => `總畫素過大，建議約 ${v}×${v} 以內`,
      ratio: (lo: number, hi: number) => `寬高比需在 ${lo}:1 至 ${hi}:1 之間`,
    },
    promptTooLong: (limit: number) =>
      `提示詞超出該模型 ${limit} 字上限，超出部分不會生效`,
    seedPlaceholder: "留空=隨機",
    seedOutOfRange: (max: number) => `Seed 超出官方取值範圍 0–${max}，本次生成將按隨機處理`,
    generate: "生成圖片",
    generating: "生成中…",
    genSec: (n: number) => `生成中 · ${n}s`,
    genErrorTitle: "生成失敗",
    retry: "重試",
    closeAlert: "關閉錯誤提示",
    closePreview: "關閉預覽",
    viewLarge: "檢視大圖",
    rerollPrompt: "重擲",
    rerollPromptTitle: "按當前資料來源重新隨機細節，覆蓋輸入框內容",
    metaSeed: (s: string) => `Seed ${s}`,
    skeletonHint: "正在生成，通常 10–30 秒，智慧改寫開啟時可能更久",
    empty:
      "填寫提示詞後點「生成圖片」，結果會顯示在這裡；圖片僅存於會話臨時目錄，手動儲存才會落盤。",
    /** 持久画廊里图片已被会话清理的条目占位文案 */
    imageMissing: "圖片已清理（僅存會話臨時目錄）",
    save: "儲存",
    saved: "圖片已儲存",
    removeImageTitle: "移除",
    needModel: "請先在「AI 生成頭像」工作臺選擇服務商與模型",
    /** 渲染时抛出：无默认值且无数据源的占位符无法自动解析，只能由用户手工替换，故直接报错并列出变量名。 */
    missingPlaceholders: (keys: string[]) => `佔位符未填充：${keys.join("、")}`,
    // 数据源：名单/单人预览入口可把球员设为生图数据源，模板变量按名单记录自动填充
    source: {
      clear: "清除",
      clearTitle: "清除資料來源，恢復手動提示詞",
      /** 档案卡：无数据源时的台内 UID 查询（球员源第三来源） */
      queryHint: "輸入 UID 查詢球員並設為資料來源",
      queryPlaceholder: "如 r-2000472008",
      queryBtn: "查詢",
      queryMiss: (uid: string) => `未找到 UID ${uid} 的資料`,
      /** 名单未扫描时的就绪卡：直达存档资料台 */
      scanFirst: "名單尚未掃描：請先在「存檔資料」臺掃描 RTF，才能按球員生成",
      goArchive: "去存檔資料臺",
      /** 资料降级：uid 不在当前名单（可能来自上一次扫描），仅姓名/UID 可信 */
      noProfile: "未在當前名單中找到該球員的資料，生成時變數可能取不到值",
      /** 名单预览行尾按钮：把该行球员设为生图数据源并跳到 AI 生图 */
      sendBtn: "去抽圖",
      sendTitle: (name: string) => `以 ${name} 為資料來源生成頭像`,
      sent: "已選",
      useForGen: "送生圖",
      selected: "已是資料來源",
    },
    // 结果卡「保存并替换」：生成图写入 <头像包>/_generated/<池>/ 并走单人换脸链路
    saveReplace: {
      btn: "儲存並替換",
      needPlayer:
        "需先選擇球員：在「名單預覽」行尾點「去抽圖」或單人替換頁點「送生圖」",
      pathTitle: "儲存位置",
      // 落點彈窗裡顯式點名儲存對象：圖可能有自己的球員（佇列產出/舊圖），不一定是工作台目前資料來源
      target: (name: string) => `儲存為 ${name} 的頭像`,
      pathHint:
        "路徑相對於 _generated 目錄，可用 / 分多級；檔名已按命名模板避讓，同名會自動加序號。",
      cancelBtn: "取消",
      ok: (name: string) => `已儲存並替換 ${name} 的頭像`,
      notMapped: (name: string) =>
        `圖片已儲存到 _generated 下的目標目錄，但沒能寫入對映：當前頭像包的 config.xml 缺失或結構異常。請檢查頭像包後重試。`,
      needSource: "請先選擇資料來源球員（名單預覽行尾「去抽圖」）",
    },
    // 服務商設定：Key 跟著服務商走，這裡是唯一編輯入口（工作台只顯示狀態）
    keyBarLabel: "API Key",
    keyBarPlaceholder: "輸入該服務商的 Key，加密儲存在本機",
    keyBarSetPlaceholder: "已設定（不回顯）；輸入新值即替換",
    keyStatusUnset: "未設定",
    keyClear: "清除 Key",
    keyClearUndo: "撤銷清除",
    keyWillClear: "儲存後將清除該服務商的 Key（點右側可撤銷）",
    baseUrl: "介面位址 Base URL",
    /** 百炼基址的官方口径：公共域仍可用，业务空间专属域是官方建议迁移的新域名；
     *  地域必须与模型、API Key 一致，跨地域直接失败。 */
    baseUrlHintDashscope:
      "留空 = 華北2（北京）公共域 https://dashscope.aliyuncs.com/api/v1，官方明確仍可使用。" +
      "官方建議遷移到業務空間專屬域 https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/api/v1" +
      "（效能與穩定性更好；WorkspaceId 在控制檯的業務空間詳情頁）。" +
      "模型、介面位址、API Key 三者必須同地域，跨地域呼叫會直接失敗。",
    keyStorageHint:
      "API Key 由本機加密儲存，僅當前系統帳戶可解；更換裝置或重裝系統後需重新填寫。",
    kind: "型別",
    providerIdLabel: "ID",
    providerIdPlaceholder: "留空自動生成",
    providerIdFixedNote:
      "ID 是服務商的唯一標識，建立後不可修改；如需換 ID 請新增服務商。",
    providerNameLabel: "名稱",
    providerModelLabel: "預設模型",
    /** 模型清单：一套协议可挂多个模型（OpenAI 协议下有大量兼容模型），在此增删改，展开可编辑能力 */
    modelListLabel: "模型",
    modelNamePlaceholder: "模型名（API 參數）",
    addModel: "新增模型",
    removeModelTitle: "移除該模型",
    modelCapToggle: "展開／收起該模型的能力參數",
    /** 能力编辑区：协议参数名（中文语义名）+ 官方字段名小字，另加各参数的附属项标签 */
    capProtoPrompt: "正向提示詞",
    capProtoSize: "輸出解析度",
    capProtoN: "出圖張數",
    capProtoNegative: "反向提示詞",
    capProtoSeed: "隨機種子",
    capProtoExtend: "智慧改寫",
    capProtoExtendMode: "改寫模式",
    capProtoThinking: "思考模式",
    capProtoWatermark: "水印",
    capProtoCallMode: "呼叫方式",
    capProtoQuality: "品質檔位",
    capProtoResponseFormat: "返回 base64",
    /** 火山 Seedream output_format：Pro/Flash/Lite 可设 png/jpeg（值随用户选择）；4.5/4.0 仅 jpeg */
    capProtoOutputFormat: "輸出格式",
    capProtoSteps: "取樣步數",
    capProtoGuidance: "引導係數",
    /** 枚举型参数的默认档（当前仅改写模式：官方默认 direct） */
    capFieldModeDefault: "預設檔",
    /** 官方字段原名（英文，含大小写/下划线/嵌套层级）：只作悬停核验用，名称列一律显示上面的中文名 */
    capApi: {
      prompt: "input.messages[].content[].text",
      size: "parameters.size",
      n: "parameters.n",
      negative: "parameters.negative_prompt",
      seed: "parameters.seed",
      extend: "parameters.prompt_extend",
      extendMode: "parameters.prompt_extend_mode",
      thinking: "parameters.enable_thinking",
      watermark: "parameters.watermark",
      callMode: "X-DashScope-Async",
      quality: "quality",
      responseFormat: "response_format",
      outputFormat: "output_format",
      // steps/guidance 仅硅基流动/魔搭用，不在 DashScope 行集合内；这里给出平铺基名（实际由 capApiByKind 覆盖）。
      steps: "steps",
      guidance: "guidance",
    },
    /** 平铺协议（OpenAI 兼容 / 火山方舟）的官方字段名：字段同名，但没有 DashScope 的
     *  `input.messages[]` 与 `parameters.` 两级嵌套，全部摆在请求体顶层。
     *  小字是给人对着文档核验用的，位置写错就会照着发错。缺项回落 capApi。 */
    capApiFlat: {
      prompt: "prompt",
      size: "size",
      n: "n",
      negative: "negative_prompt",
      seed: "seed",
      extend: "prompt_extend",
      extendMode: "prompt_extend_mode",
      thinking: "enable_thinking",
      watermark: "watermark",
      quality: "quality",
      responseFormat: "response_format",
      outputFormat: "output_format",
      steps: "steps",
      guidance: "guidance",
    } as Record<string, string>,
    /** 按服务商覆盖的官方字段名（优先于 capApiFlat）：硅基流动的尺寸/张数/步数/引导系数
     *  字段名与 OpenAI 不同（image_size / batch_size / num_inference_steps / guidance_scale），
     *  魔搭虽与 Openai 同名但也在此显式列出，避免小字写成会发错的字段名。 */
    capApiByKind: {
      siliconflow: {
        size: "image_size",
        n: "batch_size",
        steps: "num_inference_steps",
        guidance: "guidance_scale",
      },
      modelscope: {
        size: "size",
        n: "n",
        steps: "steps",
        guidance: "guidance",
      },
      // 本地 ComfyUI：参数是工作流节点输入（非扁平 API 字段），小字标注「节点.输入」便于对照导出的 workflow。
      localcomfyui: {
        prompt: "text (CLIPTextEncode)",
        size: "width/height (EmptyLatentImage)",
        n: "batch_size",
        negative: "negative (KSampler)",
        seed: "seed (KSampler)",
        steps: "steps (KSampler)",
        guidance: "cfg (KSampler)",
      },
    } as Record<string, Record<string, string>>,
    capFieldSizes: "取值",
    capRuleSide: "單邊 px",
    capRuleTotal: "總畫素",
    capRuleRatio: "寬高比",
    capZeroHint: "0 = 該條不檢查",
    capFieldMaxN: "張數上限",
    capFieldMaxNHint: "0 = 不限張數",
    capFieldStepsDefault: "預設步數",
    capFieldStepsMax: "步數上限",
    capFieldGuidanceDefault: "預設引導",
    capFieldGuidanceMax: "引導上限",
    capFieldMaxChars: "提示詞上限",
    capFieldMaxCharsHint: "0 = 不限字數",
    capFieldQualities: "取值",
    capFieldSizesPlaceholder: "2048x2048, 2688x1536",
    capFieldQualitiesPlaceholder: "low, medium, high",
    addProvider: "新增服務商",
    providerSaved: "服務商已儲存",
    deleteProviderConfirm: (name: string) => `刪除服務商「${name}」？`,
    // 模板管理
    templateManageTitle: "提示詞模板",
    templateNameLabel: "模板名稱",
    templateBodyLabel: "模板正文（佔位符寫法 {{變數|預設值}}）",
    templateNegativeLabel: "負向提示詞",
    addTemplate: "新建模板",
    deleteTemplateConfirm: (name: string) => `刪除模板「${name}」？`,
    // 通用
    edit: "編輯",
    remove: "刪除",
    back: "返回",
    done: "完成",
    colStatus: "狀態",
    // 生成队列（批量）
    queue: {
      tabQueue: "生成佇列",
      startBtn: "啟動生成",
      cancelBtn: "取消生成",
      /** 已请求取消、在跑条目尚未收尾时的按钮文案 */
      cancellingBtn: "取消中…",
      /** 「查看单人」：把唯一勾选的队列条目转入「球员源·单人」卡（原「清空队列」职责由全选+移除覆盖） */
      viewSingle: "檢視單人",
      viewSingleTitle: (name: string) => `把 ${name} 轉入「球員源 · 單人」卡`,
      viewSingleHint: "勾選一條佇列條目後，可將其球員轉入「球員源 · 單人」卡",
      removeBtn: "移除選中",
      addBtn: (n: number) => `加入佇列（${n}）`,
      statusQueued: "排隊中",
      statusRunning: "生成中",
      statusSuccess: "成功",
      statusFailed: "失敗",
      /** 使用者取消（資料上是 failed + errorKind=cancelled），與「失敗」不是一回事 */
      statusCancelled: "已取消",
      infoConfig: "設定",
      /** 佇列執行中：單人卡與工作台被這條任務佔用 */
      queueRunningTag: "佇列生成中",
      infoPrompt: "提示詞",
      infoNegative: "負向",
      colPlayer: "球員",
      colDetail: "詳情",
      // ---- 啟動方式（手動確認 / 全自動）與佇列列的逐條確認 ----------------------------------------------------------
      /** 確認方式分段控件：左「手動確認」、右「全自動」（使用者指定的順序） */
      confirmModeManual: "手動確認",
      confirmModeAuto: "全自動",
      confirmModeHintManual: "啟動前需逐條確認：點佇列列，在右側核對並修改該條參數",
      confirmModeAria: "確認方式",
      /** 單條確認按鈕：把該列核對過的參數寫回佇列 */
      confirmBtn: "確認",
      /** 已確認態：按鈕同位置轉灰不可點 */
      confirmedBtn: "已確認",
      /** 啟動被攔：還有 N 條沒逐條確認 */
      needConfirm: (n: number) => `還有 ${n} 條未確認，請先逐條確認後再啟動。`,
      /** 手動模式但未選中任何列時的引導（貼在原表單上方） */
      queueEditHint: "點左側佇列列，在這裡核對並修改該條的生成參數。",
      /** 借用失敗：該列入列後它的服務商/模型已不在設定裡（能力位未知，不能在這裡改參數） */
      borrowUnavailable: (name: string) => `「${name}」用的服務商或模型已不在設定裡，無法核對這條的參數；可勾選後點「移除」，或從名單重新加入後再核對。`,
      count: (total: number) => `共 ${total} 條`,
      startConfirm: (n: number) =>
        `開始批次生成 ${n} 個排隊任務？將逐條呼叫服務商介面並產生費用。`,
      /** 只跑勾選的排隊條目（其餘留在佇列裡） */
      startConfirmPicked: (n: number, rest: number) =>
        `開始批次生成已勾選的 ${n} 個任務？未勾選的 ${rest} 條會留在佇列裡，可稍後再啟動。`,
      cancelConfirm:
        "取消後未提交的條目立即取消；已提交的會等待本次請求返回並保留結果（會產生費用）。確定取消？",
      cancellingHint:
        "正在取消：未提交的已停止；已提交的等待服務端返回後結束（會產生費用）。",
      removeConfirm: (n: number) => `移除選中的 ${n} 條條目？`,
      empty: "佇列為空。在「名單預覽」勾選球員後點「加入佇列」，即可批次生成。",
      enqueued: (n: number) => `已加入 ${n} 條生成任務`,
      imagesDone: (n: number) => `生成 ${n} 張圖`,
      progressLabel: "批次生成進度",
      /** 队列进度计数：首个数字是**已完成数**（终态条目累计），不是派发序号。 */
      progressCount: (done: number, total: number) => `已完成 ${done} / 總數 ${total}`,
      /** 运行中条目的阶段文案：与后端 GenStage 的 as_str 取值一一对应（submitting/polling/downloading）。 */
      stageLabel: (stage: string): string => {
        const labels: Record<string, string> = {
          submitting: "提交中",
          polling: "伺服器處理中",
          downloading: "下載中",
        };
        return labels[stage] ?? "";
      },
      finishedSummary: (finished: number, failed: number, total: number) =>
        `批次生成結束：成功 ${finished} · 失敗 ${failed} · 共 ${total} 條`,
      retryEntry: "重試本條",
      /** 执行期重试徽标：attempts 是「总尝试次数」（首发即 1），故实际重试次数 = attempts - 1。
       *  仅在 attempts > 1（即确实重试过）时展示。 */
      retriedBadge: (attempts: number) => `已重試 ${attempts - 1} 次`,
      /** 失败条目按错误类别给出的处置建议（对应后端 GenErrorKind）；无建议类别返回空串。 */
      errorAction: (kind: string): string => {
        const actions: Record<string, string> = {
          auth: "檢查 API Key 是否正確或已過期",
          rate_limited: "稍後重試或降低併發",
          not_configured: "到服務商設定填寫 Key/模型",
          invalid_param: "按提示調整參數",
          content_rejected: "修改提示詞後重試",
          network: "檢查網路後重試",
          provider: "稍後重試，持續失敗請檢視服務商狀態",
          io: "檢查磁碟空間與目錄許可權",
        };
        return actions[kind] ?? "";
      },
      viewImages: "檢視圖片",
      collapseImages: "收起圖片",
      saveImage: "另存",
      // 复选框的读屏文案：队列条目（QueueDock）与名单表格（PlayersTable）各一套
      selectRowAria: (name: string) => `選擇 ${name}`,
      selectAllAria: "全選或取消本頁條目",
      selectPlayerAria: (name: string) => `選擇球員 ${name}`,
      selectAllPlayersAria: "全選或取消全部篩選結果（跨頁）",
    },
  },

  /** 「替换执行」面板 */
  replace: {
    cardTitle: "批次替換",
    modeLabel: "模式：",
    optionsLabel: "選項設定：",
    optAllowDuplicates: "允許複用圖片",
    optFilterNewgan: "僅隨機人",
    optSaveBackup: "寫入前備份",
    /** 三个替换模式：value 发往后端（协议值，勿改）；label/desc 为界面显示文案（与后端日志用词一致） */
    modes: [
      {
        value: "Preserve",
        label: "保留",
        desc: "保留已換過的球員，只為新球員補圖（推薦日常增量）",
      },
      {
        value: "Overwrite",
        label: "覆寫",
        desc: "名單內球員全部重新抽圖，名單外保留",
      },
      {
        value: "Generate",
        label: "生成",
        desc: "無視現有 config.xml，從零生成全部對映",
      },
    ] as ReplaceMode[],
    startBtn: "替換頭像",
    runningBtn: "替換中", // 按钮上会追加实时百分比，如「替换中 45%」
    cancelBtn: "取消",
    browseDisabledHint: "請先在左側「啟用」存檔後再執行替換（當前為瀏覽預覽）",
    browseNotice: (name: string) =>
      `正在瀏覽「${name}」（未啟用）。替換只作用於已啟用的存檔，請先在左側點「啟用」。`,
    needProfile: "選擇 Profile",
    needGroup: "選擇頭像包",
    needRtf: "設定 RTF 名單",
    needRoster: "先產生球員/職員名單快取（執行遊戲讀取或掃描 .fm 存檔）",
    todoPrefix: "還需完成：",
    missingDirsTitle: "缺失目錄",
    missingDirsConfirm: (standard: number, count: number) =>
      `標準${standard} 缺少 ${count} 個目錄，自動建立並繼續？`,
    unknownTitle: "無法識別的結構",
    unknownConfirm:
      "頭像包結構無法識別，替換時將把包內所有圖片（含子/孫目錄）隨機分配，仍要繼續？",
    // 发起前预检（preflight_replace）：只有「生成」+ 本档档景非空时会走到这条确认。
    preflightTask: "檢查既有對映",
    blankTitle: "將清空本檔現有對映",
    blankConfirm: (existing: number) =>
      `「生成」模式不攜帶既有條目。若本次一條也沒抽到（名單被篩空、UID 對不上、包裡沒有圖），\n` +
      `會用空表覆蓋本檔現有的 ${existing} 條對映。\n\n確認仍要這樣執行？`,
  },

  /** 结果区顶部 Tab（替换台读视图：名单/快照；单人替换已常驻替换台左栏，AI 生图已独立成台） */
  results: {
    tabTable: "名單預覽",
    tabSnapshot: "快照預覽",
  },

  /** 名单预览表（PlayersTable） */
  table: {
    title: "名單預覽",
    recordCount: (total: number) => `${total} 條`,
    searchPlaceholder: "搜尋 UID / 姓名 / 國籍 / 俱樂部…",
    prevPage: "‹ 上一頁",
    nextPage: "下一頁 ›",
    pageOf: (current: number, total: number) => `第 ${current} / ${total} 頁`,
    /** 快速跳页：「前往 [输入框] 页 [跳转]」 */
    gotoPrefix: "前往",
    gotoSuffix: "頁",
    gotoBtn: "跳轉",
    gotoAria: "輸入頁碼後回車快速跳轉",
    colUid: "UID",
    colNat: "國籍",
    col2ndNat: "第二國籍",
    colName: "姓名",
    colHair: "髮長",
    colColor: "髮色",
    colEth: "種族",
    colSkin: "膚色",
    colAvatar: "頭像",
    /** 批量取头像失败时挂在「头像」列表头提示上（逐图取不到仍是静默留白，那本来就是正常情况）。 */
    avatarFailHint: (why: string) =>
      `頭像取數失敗（${why}），本列留空；這不是「名單裡沒有臉」`,
    colNewgan: "隨機人",
    colGender: "性別",
    colAge: "年齡",
    colHeight: "身高",
    colClub: "俱樂部",
    colBased: "所在地",
    colJob: "偏好工作",
    colCa: "能力",
    colPa: "潛力",
    colRca: "R.C.A",
    /** 头像列缩略图 + 悬停/点击气泡的状态文案。 */
    avatarBtn: "預覽",
    avatarAria: "預覽該球員的頭像圖片",
    avatarClose: "關閉頭像預覽",
    avatarLoading: "載入中…",
    avatarNotFound: "未找到頭像圖片",
    /** 随机人筛选（仅拓展名单显示）：标签 + 三个选项的文案。 */
    newganFilterLabel: "隨機人",
    newganAny: "任意",
    newganYes: "是",
    newganNo: "否",
    /** 球员/职员视图切换（仅混合名单显示）：标签 + 三个选项 + 视图内无记录的空态文案。 */
    rosterViewLabel: "檢視",
    rosterViewAll: "全部",
    rosterViewPlayer: "球員",
    rosterViewStaff: "職員",
    rosterViewEmpty: "目前檢視沒有匹配的記錄。",
    /** 混合名单下第 15 列「全部」视图的表头：两种语义并列（三字典同值，不逐词翻译）。 */
    rosterViewMixedHeader: "所在地 / 偏好工作",
    emptyHint:
      "執行「替換頭像」或「掃描RTF」後可在此預覽名單（最近一次解析結果）。",
    /** 缓存里没有匹配当前筛选条件的记录（与快照预览的 noMatchHint 同款语义）。 */
    noMatchHint: "沒有匹配的球員。",
    /** 替换台「拓展列」开关：补齐仅拓展名单才有的字段列 */
    extCols: "拓展列",
  },

  /** 单人预览/微调（PlayerViewer） */
  viewer: {
    title: "球員預覽",
    /** 替换台左栏单人替换卡的标题（读写分栏后常驻左栏，不属于结果区读视图） */
    section: "單人替換",
    hint: "輸入關鍵詞後回車搜尋",

    searchBtn: "搜尋",
    /** 命中多个时的前后切换（到头即禁用） */
    prevMatch: "上一個",
    nextMatch: "下一個",
    colMapping: "當前對映",
    /** 拓展名单第 19 列起的用户自定义列（后端只保留原文，无表头名，按序号命名）。 */
    extraField: (n: number) => `自定義${n}`,
    noImage: "暫無圖片",
    imgPlaceholder: "輸入新圖片路徑…",
    imgPathAria: "替換圖片路徑",
    browseBtn: "瀏覽",
    replaceBtn: "指定替換",
    /** 重掷替换：按标准规则给该球员重新抽图（后端固定重抽语义） */
    resetBtn: "重擲替換",
    resetOk: (uid: string) => `已重擲 UID ${uid} 的頭像（按規則重新抽圖）`,
    imageDialogTitle: "選擇圖片檔案",
    imageFilterName: "圖片",
    missingInput: "請先搜尋並選中一名球員，再選擇有效圖片路徑",
    replacedOk: (uid: string) => `已替換 UID ${uid} 的頭像`,
    mappingNotFound: (uid: string) =>
      `config.xml 中未找到 UID ${uid} 的現有對映，未做修改`,
  },

  /** 快照预览（SnapshotPreview）：展示「读取快照」解析出的存档快照映射。分页器与名单预览共用（TablePager，文案取 t.table.*） */
  snapshot: {
    searchPlaceholder: "按 UID 篩選…",
    recordCount: (total: number) => `${total} 條`,
    colUid: "UID",
    colPath: "頭像相對路徑",
    colImage: "頭像圖",
    noImage: "無圖",
    // 未读取（或切档/切组后失效）时的引导文案
    readHint:
      "點選「當前任務」中的「讀取快照」，解析存檔 XML 快照後在此預覽對映列表。",
    noMatchHint: "沒有匹配的 UID。",
    emptyHint: "快照中沒有對映記錄。",
  },

  /** 底部状态栏 */
  status: {
    ready: "就緒",
    noGroup: "未選擇頭像包",
    log: "日誌",
  },

  /** 日志抽屉 */
  log: {
    title: "執行日誌",
    onlyLevel: "僅該級別",
    openFileBtn: "開啟日誌檔案",
    clearBtn: "清空顯示",
    closeAria: "關閉日誌面板",
  },

  /** 标准规则编辑对话框 */
  ruleEditor: {
    std2Label: "標準二（14 種族目錄）",
    std3Label: "標準三（細分目錄）",
    title: "規則配置",
    kicker: (standard: number, dirs: number) => `STANDARD ${standard} · ${dirs} 個目錄`,
    sourceBtn: "原始碼",
    structuredBtn: "結構化",
    sourceHint: "以 JSON 文字編輯；儲存前由後端校驗格式與目錄參照",
    sourceTitle: "JSON 原始碼",
    sourceDesc: "進階模式：直接編輯整份規則檔案。切回「結構化」時會先解析，格式錯誤會留在本頁提示。",
    parseFailed: (detail: string) => `無法結構化解析：${detail}。請在原始碼中修復後再切回。`,
    loading: "正在載入規則檔案…",
    saveBtn: "儲存",
    closeBtn: "關閉",
    saveHint: "儲存前會校驗 JSON 格式，非法內容不會被寫入",
    savedStatus: "已儲存 ✓ 下次替換 / 檢查完整性即生效",
    unsavedTitle: "未儲存修改",
    unsavedSwitch: (label: string) =>
      `${label} 有未儲存的修改，確定放棄並切換？`,
    unsavedClose: (label: string) => `${label} 有未儲存的修改，確定關閉？`,
    editorAria: "規則配置內容",
    sections: {
      overview: "概覽",
      folders: "目錄清單",
      ethnics: "國籍對映",
      codeRules: "種族碼修正",
      blocks: "規則區塊",
      settings: "全域設定",
    },
    statFolders: "目錄",
    statNations: "國籍",
    statCodeRules: "碼修正",
    statRaceCodes: "種族碼",
    statBlocks: "規則區塊",
    statRuleEntries: "規則條目",
    common: {
      add: "新增",
      delete: "刪除",
      notInList: "不在清單",
    },
    overview: {
      title: "設定概覽",
      desc: "先檢查再編輯；儲存前後端會做最終校驗，非法目錄參照不會寫入。",
      healthTitle: "設定檢查",
      healthOk: "結構檢查通過",
      healthWarn: (count: number) => `${count} 項需要處理`,
      healthOkDesc: "沒有發現重複目錄、失效參照或缺少預設區塊。",
      healthEmptyFolders: "目錄清單為空，至少需要一個目錄",
      healthEmptyEthnics: "還沒有國籍對映，所有球員都會被跳過",
      healthDup: (count: number) => `目錄清單有 ${count} 個重複項目`,
      healthEthnics: (count: number) => `${count} 條國籍對映指向清單外目錄`,
      healthRefs: (count: number) => `${count} 處規則參照了清單外目錄`,
      healthMissingDefault: "缺少 _DEFAULT_ 全域預設區塊",
      healthVersion: (version: number) => `Version 為 ${version}，僅支援 4`,
      healthOrphans: (count: number) => `${count} 個 DefaultCodes 鍵沒有對應國籍`,
      goFix: "去修復",
      flowTitle: "抽圖鏈路",
      flow2: ["國籍對映", "種族碼修正", "目錄池抽圖"],
      flow3: ["國家區塊", "族群區塊", "_DEFAULT_ 全域區塊"],
      flowHint: "規則區塊依序取該種族碼的規則；區塊內缺碼會繼續穿透，目錄不可用時走 Fallback。",
      fileLabel: "檔案",
    },
    folders: {
      title: "目錄清單",
      desc: "FolderList 是判定標準與抽圖的目錄白名單；不在這裡的目錄不會被掃描或使用。",
      searchPlaceholder: "搜尋目錄",
      addPlaceholder: "新增目錄名",
      empty: "沒有符合的目錄",
      duplicate: "該目錄已在清單中",
      blank: "目錄名不能為空",
      used: (count: number) => `${count} 處參照`,
      removeHint: "從清單中刪除該目錄",
      removeConfirm: (name: string, count: number) =>
        count > 0
          ? `目錄「${name}」被 ${count} 處參照，刪除後這些參照會失效，需手動重新指定。繼續？`
          : `刪除目錄「${name}」？`,
      count: (n: number) => `${n} 個目錄`,
    },
    ethnics: {
      title: "國籍對映",
      desc: "國籍三字碼 → 本國基礎池目錄；名單裡查不到對映的國籍會被跳過。",
      searchPlaceholder: "搜尋國籍碼或目錄",
      addCodePlaceholder: "國籍碼，如 CHN",
      codeHeader: "國籍碼",
      folderHeader: "歸屬目錄",
      exists: "該國籍碼已存在",
      invalidCode: "國籍碼應為 2–3 位大寫字母",
      noFolders: "請先在「目錄清單」中新增目錄",
      empty: "沒有符合的國籍",
      removeHint: "刪除該國籍對映",
      removeConfirm: (code: string) => `刪除國籍 ${code} 及關聯的預設種族碼/國家規則區塊？`,
      count: (n: number) => `${n} 個國籍`,
    },
    codeRules: {
      title: "種族碼修正",
      desc: "依 FM 種族碼修正抽圖目錄：優先清單依序匹配主/第二國籍目錄，都不命中時使用兜底目錄。",
      codeLabel: (code: string) => `種族碼 ${code}`,
      unset: "未設定——沿用國籍目錄",
      addBtn: "新增修正",
      deleteHint: "刪除該種族碼修正",
      preferLabel: "優先目錄（依序匹配）",
      fallbackLabel: "兜底目錄",
      emptyPrefer: "未設定優先目錄，直接使用兜底目錄",
      addPlaceholder: "新增優先目錄",
      addBtnShort: "新增",
      moveUp: "上移",
      moveDown: "下移",
      removeDir: "移除該目錄",
    },
    blocks: {
      title: "規則區塊",
      desc: "先查國家區塊 → 再查族群區塊 → 最後落到 _DEFAULT_；區塊內缺碼會繼續穿透到下一級。",
      searchPlaceholder: "搜尋規則區塊",
      addPlaceholder: "選擇新增規則區塊的鍵",
      defaultKind: "全域",
      nationKind: "國籍",
      folderKind: "族群",
      unknownKind: "未知",
      count: (n: number, total: number) => `${n}/${total} 碼`,
      empty: "沒有符合的規則區塊",
      noSelection: "從左側選擇一個規則區塊開始編輯",
      detailDesc: "區塊內某個碼未設定時，該碼會繼續向下一個區塊穿透；這是合法設定。",
      codeMissing: "未設定（穿透下一級）",
      addRuleBtn: "新增規則",
      deleteRuleHint: "刪除該碼的規則",
      deleteBlockHint: "刪除該規則區塊",
      deleteBlockConfirm: (key: string) => `刪除規則區塊「${key}」？`,
      unknownRule: "規則結構無法識別，請到「原始碼」修復或重設",
      resetRuleBtn: "重設為預設",
      typeLabel: "對映方式",
      typeUnknown: "無法識別",
      typeDefault: "預設",
      typeDirectly: "直接",
      typeRandom: "隨機",
      typeWeighted: "加權",
      typeDefaultHint: "使用球員國籍在 Ethnics 裡的歸屬目錄",
      typeDirectlyHint: "固定對映到一個目錄",
      typeRandomHint: "從目錄清單中等機率選一個",
      typeWeightedHint: "依權重隨機選一個，權重按總和歸一",
      directoryLabel: "目錄",
      addDirPlaceholder: "新增目錄",
      addDirBtn: "新增",
      weightLabel: "權重",
      addWeightBtn: "新增加權項",
      removeItem: "移除",
      atLeastOne: "至少保留一項",
      extraCodes: (count: number) => `另有 ${count} 個非 0–10 的碼未在此編輯（儲存時原樣保留）`,
    },
    settings: {
      title: "全域設定",
      desc: "規則檔案的版本與目錄不可用時的兜底行為。",
      versionLabel: "Version",
      versionHint: "標準三規則格式版本，僅支援 4",
      fallbackLabel: "兜底策略",
      fallbackRandom: "全池隨機",
      fallbackRandomHint: "規則目錄不可用時，從包內遞迴全池隨機抽圖",
      fallbackSkip: "跳過",
      fallbackSkipHint: "規則目錄不可用時跳過該球員",
      passthroughTitle: "透傳資料",
      passthroughDesc:
        "DefaultCodes / FolderMapping* 由出廠設定維護，結構化介面不改寫；儲存時原樣保留，可在「原始碼」中編輯。",
      passthroughEditBtn: "在原始碼中編輯",
      count: (n: number) => `${n} 條`,
    },
  },

  /** 國籍對照表編輯對話框 */
  nationalityEditor: {
    aria: "國籍對照表",
    title: "國籍對照表",
    subtitle:
      "三字碼 ↔ 各語言國籍全稱（含別名）；碼與全稱分開維護。儲存後下一次掃描 RTF / 替換 / 提示詞預覽即生效，檔案：data/nationalities.json",
    searchPlaceholder: "搜尋三字碼、語言或全稱",
    countLabel: (shown: number, total: number) => `顯示 ${shown} / 共 ${total} 條`,
    codeHeader: "三字碼",
    languageHeader: "語言",
    nameHeader: "國籍全稱/別名",
    languageLabel: "語言列表",
    addCodePlaceholder: "三字碼，如 CHN",
    addNamePlaceholder: "該語言下的國籍全稱（留空 = 只登記碼）",
    addBtn: "新增",
    addLanguagePlaceholder: "語言名，如 Deutsch",
    addLanguageBtn: "新增語言",
    languageExists: "該語言已在列表中",
    duplicate: "這條映射已存在",
    codeOnly: "已登記該碼（暫無全稱）",
    codeExists: "該碼已收錄",
    incomplete: "請填寫三字碼（全稱可留空）",
    bareLabel: (n: number) => `僅有碼（暫無全稱）${n} 個`,
    removeCodeHint: "移除該碼：碼退出碼全集後不再算合法國籍值",
    noMatch: "沒有符合的映射",
    deleteBtn: "刪除",
    deleteHint: "刪除這條映射（點儲存後生效）",
    emptyHint: "三字碼為 2–3 位大寫字母；同一語言下同一全稱只能對應一個碼；至少保留一個碼或一條映射。編輯後點「儲存」寫入檔案。",
    saveBtn: "儲存",
    closeBtn: "關閉",
    saveHint: "儲存前會校驗映射並原子寫入；非法內容不會寫入",
    savedStatus: "已儲存 ✓ 下次掃描 / 替換即生效",
    unsavedTitle: "未儲存修改",
    unsavedClose: "國籍對照表有未儲存的修改，確定關閉？",
  },
};

/** English（协议值与专有名词保持原文：Profile / UID / RTF / Seed / API Key / Base URL 等）。 */
const en: Dict = {
  /** 应用品牌名（窗口/弹窗标题、侧栏品牌区共用；品牌名不翻译） */
  brand: "FM24 NewGen Panel",

  /** 通用系统弹窗标题 */
  dialog: {
    error: "Error",
    info: "Info",
    warning: "Warning",
    /** 结构化错误里 userAction 的引导词（如「建议：检查 API Key」）。 */
    suggestion: "Suggestion: ",
  },

  /** 根组件：替换结束的系统提示 */
  app: {
    replaceFinished: "Replacement complete!",
    replaceFailed: "Replacement failed — check the logs",
    brandVer: "v2.0",
  },

  /** 顶栏三工作台（chip 可点击切换，右侧状态点随就绪度变色） */
  workbench: {
    archive: "Profiles",
    replace: "Face replacement",
    ai: "AI avatars",
    /** 顶栏右端作用域回显：档名与组名之间的分隔符（组为空时只显示档名） */
    scopeSep: " · ",
    /** 无头像包组时的回显占位（侧栏还没关联目录） */
    scopeNoGroup: "No facepack",
    /** chip status tooltip (two states: green = ready / orange = not ready) */
    stateOk: "Ready",
    stateWarn: "Not ready — click to see what is missing",
  },

  /** 侧栏底部主题切换 */
  theme: {
    dark: "Dark",
    light: "Light",
    switchTo: (mode: "dark" | "light") =>
      mode === "light" ? "Switch to light theme" : "Switch to dark theme",
  },

  /** 侧栏底部「语言」菜单：按钮名 + 三个选项的自显示名（各语言写自己的名字） */
  lang: {
    label: "Language",
    zhCN: "简体中文",
    zhTW: "繁體中文",
    en: "English",
  },

  /** 内置文档面板：侧栏「教程 / FAQ」两个入口各打开同一面板的对应页签 */
  help: {
    title: "Documentation",
    subtitle: "Built-in tutorial and FAQ, available offline",
    tutorialTab: "Tutorial",
    faqTab: "FAQ",
    searchPlaceholder: "Search sections or keywords…",
    searchHint: "Ctrl+F to focus search",
    resultCount: (shown: number, total: number) => `${shown} of ${total} entries shown`,
    noResults: "No matching entries. Try another keyword.",
    clearSearch: "Clear search",
    note: "Note",
    warning: "Warning",
  },

  /** 侧边栏：存档（Profile）与头像包（组）管理 */
  sidebar: {
    profileSection: "Profiles",
    newProfileBtn: "New Profile",
    newProfilePlaceholder: "New profile name",
    createBtn: "Create",
    notActivated: "Not activated",
    activated: "Activated",
    deleteCurrentProfile: "Delete profile",
    noProfileNoActivate: "No Profile is the startup default and is not activated",
    noProfileNoDelete: "The default profile cannot be deleted",
    activeAlreadyHint: "This profile is already active",
    activateBtn: "Activate",
    activateHint: (name: string) =>
      `Activate profile "${name}" and switch to its group config and mapping`,
    deleteProfileConfirm: (name: string) =>
      `Delete profile "${name}" and all its group configs and mapping backups?`,
    deleteProfileTitle: "Delete profile",
    groupSection: "Facepacks (groups)",
    linkNewHint: "Link a new facepack directory",
    linkBrowseHint: 'Click "Activate" first before linking a facepack',
    linkDialogTitle: "Link facepack directory",
    selectGroupBrowseHint:
      'Click "Activate" first before switching the current group (currently previewing)',
    deleteGroupBrowseHint: "Activate the profile first before deleting a group",
    deleteGroupBtn: "Delete group",
    deleteGroupConfirm: (dir: string) =>
      `Delete the group and its mapping backup?\n${dir}`,
    deleteGroupTitle: "Delete group",
    /** 队列运行中的切档/切组护栏：空闲切换不弹窗不打断，运行中才问这一句 */
    queueScopeTitle: "The AI avatar queue is running",
    queueScopeConfirm:
      "Switching the profile or facepack cancels unfinished generation tasks. Continue?",
    /** 队列运行中执行删除时追加在删除确认文案后的那一段。删除本身已有确认框，
     *  这里并入同一段正文，避免「先问删除、再问队列」连弹两个框（一次操作只打扰一次）。 */
    queueScopeAppend:
      "\n\nThe AI avatar queue is running; continuing cancels unfinished generation tasks.",
    rtfSet: "RTF ✓",
    rtfUnset: "No RTF",
    fmSet: "FM ✓",
    fmUnset: "No FM",
    emptyGroups: "No facepacks yet. Click ＋ above to link a facepack directory.",
    linkHome: "Home",
    linkTutorial: "Tutorial",
    linkFaq: "FAQ",
    langMenuTitle: "Switch interface language",
  },

  /** 「当前任务」卡：路径展示、完整性检查、扫描、规则配置入口 */
  setup: {
    cardTitle: "Current task",
    imgDirLabel: "Facepack:",
    rtfLabel: "RTF roster:",
    xmlLabel: "config.xml:",
    imgDirPlaceholder: "No facepack directory linked",
    rtfUnsetValue: "Not set",
    xmlPlaceholder: "No facepack linked (generated by replacement)",
    noGroupHint: "Link a facepack directory first",
    checkIntegrityBtn: "Check integrity",
    checkIntegrityHint:
      "Identify which standard the facepack belongs to via the FolderList in the rules JSON",
    scanImagesBtn: "Scan faces",
    scanImagesHint:
      "Scan the facepack and cache the image index (result written to the log)",
    selectRtfBtn: "Select RTF",
    selectRtfDialogTitle: "Select RTF roster",
    selectRtfHint: "Select an RTF roster exported from FM",
    selectRtfBrowseHint:
      'Click "Activate" first before setting the RTF (currently previewing)',
    scanRtfBtn: "Scan RTF",
    scanRtfHint:
      "Parse the RTF and fill the roster preview (result written to the log)",
    selectFmDialogTitle: "Select FM save",
    scanFmFileBtn: "Scan .fm save",
    scanFmFileHint:
      "Pick a .fm save and scan it immediately; the result refreshes the player/staff cache",
    selectFmBrowseHint:
      'Click "Activate" first before scanning a .fm save (currently previewing)',
    readMemoryBtn: "Read running game",
    readMemoryHint:
      "Scan the memory of the running FM process; the result refreshes the player/staff cache (no .fm save needed)",
    viewConfigBtn: "Read snapshot",
    viewConfigHint:
      "Parse the savegame XML snapshot and view the mapping list under Snapshot preview",
    viewSnapshotBtn: "Open location",
    viewSnapshotHint: "Open the directory containing the savegame XML snapshot",
    noGroupGuide:
      'Link a facepack directory under "Facepacks (groups)" on the left, or click an existing group.',
    tpl1Complete: "Standard 1: images placed flat, no ethnicity subdirectories",
    tplComplete: (standard: number, total: number) =>
      `Standard ${standard}: all ${total} config directories present`,
    integrityOk: (label: string) => `✓ Facepack structure intact (${label})`,
    extraDirsNote: (n: number) =>
      n > 0 ? `Also includes ${n} unconfigured directories (excluded from drawing)` : "",
    integrityFixable: (
      standard: number,
      missing: number,
      found: number,
      total: number,
      list: string,
    ) =>
      `Standard ${standard}: ${missing} directories missing (${found}/${total} present): ${list}`,
    createMissingBtn: "Create missing directories",
    missingDirsTitle: "Missing directories",
    missingDirsConfirm: (standard: number, count: number, list: string) =>
      `Standard ${standard} is missing ${count} directories:\n${list}\n\nCreate them automatically?`,
    integrityUnknown:
      "⚠ Cannot identify the facepack structure: the subdirectories do not form a containment relationship with any standard's FolderList. Replacement will distribute all images in the pack at random; you can also adjust the standard directory list in Rule settings.",
    // 完整性状态胶囊的短文案（详情仍用上方 integrityFixable / integrityUnknown 长句）
    pillMissingDirs: (missing: number, found: number, total: number) =>
      `${missing} directories missing (${found}/${total} present)`,
    pillUnknown: "Cannot identify the facepack structure",
    ruleConfigBtn: "Rule settings",
    ruleConfigHint:
      "Edit directories, nationality mappings, race-code fixes and rule blocks structurally; switch to Source for raw JSON. Format is validated on save",
    nationalityConfigBtn: "Nationality table",
    nationalityConfigHint:
      "Adjust, add or delete mappings between three-letter nationality codes and full nationality names in various languages (takes effect on the next scan / replacement after saving)",
  },

  gen: {
    // 工作台顶部与面板
    provider: "Provider",
    model: "Model",
    providerSettings: "Provider settings",
    /** 服务配置条就绪状态灯：判据与顶栏 ③chip 同源（Key 已设置 + 模型已选），不冒充「已连通」 */
    statusReady: "Ready",
    statusNoKey: "No key",
    statusNoModel: "No model",
    statusReadyTitle:
      "Key is set and model is selected (connectivity is proven by actual generation)",
    /** Title of the missing-key state: state the consequence and point at the settings dialog */
    keyMissingHint:
      "An API key for the current provider must be set before generating — click to open provider settings",
    cardSource: "Player source · Single",
    cardSettings: "Generation settings",
    resultsTitle: "Results",
    resultsCount: (n: number) => `${n} image${n === 1 ? "" : "s"}`,
    clearResults: "Clear",
    clearResultsTitle: "Clear all generation results of this session",
    clearResultsConfirm: (n: number) =>
      `Clear the ${n} generation results of this session? Temporary images will be deleted; any not saved manually cannot be recovered.`,
    template: "Prompt template",
    manageTemplate: "Manage templates",
    /** 模板管理弹窗：变量胶囊（点击在正文光标处插入占位符） */
    templateVariables: "Available variables (click to insert at the cursor)",
    prompt: "Prompt (editable)",
    /** 负向词折叠条（默认收起；展开/收起同一行，随折叠图标切换） */
    negativeCollapsed: "Negative prompt (optional)",
    negative:
      "Negative prompt (logged only, not sent, when the model does not support it)",
    /** 画幅预览帧：尺寸不合规时的简短状态（详细原因仍显示在自定义宽高行下） */
    sizeInvalidShort: "Invalid size",
    regenTitle: "Regenerate with this image's parameters",
    resultAlt: "Generated image",
    previewAlt: "Generated image preview",
    // 连接卡下方的模型能力摘要（与参数控件显隐同一能力集口径）
    capNoModel:
      "No model detail row selected: only interface parameters can be filled in; advanced capabilities are unavailable",
    capPresetSizes: (n: number) => `${n} preset sizes`,
    capFreeSize: "Free size",
    capMaxN: (n: number) => `Up to ${n} images per call`,
    capNAny: "No image limit",
    capQuality: "Quality options",
    capSeed: "Seed supported",
    capNegative: "Negative prompt",
    capPromptExtend: "Prompt rewriting",
    capWatermark: "Optional watermark",
    size: "Size",
    // 尺寸下拉末尾的「自定义…」项 + 宽高输入与提醒（选中后显示）
    sizeCustom: "Custom…",
    sizeWLabel: "Width",
    sizeHLabel: "Height",
    sizeCustomHint:
      "Non-recommended sizes may be rejected by the provider; if generation fails, pick a ratio above to fill back in.",
    ratio: "Ratio",
    n: "Images",
    nSingle: "This model generates only 1 image per call",
    nLess: "Fewer",
    nMore: "More",
    nCount: "Count",
    nMaxHint: (n: number) => `Up to ${n} images per call`,
    quality: "Quality",
    qualityAuto: "Auto",
    qualityLow: "Low",
    qualityMedium: "Standard",
    qualityHigh: "High",
    promptExtend: "Rewrite prompt",
    promptExtendHint:
      "An LLM optimizes the prompt; generation is slower. For z-image this enables thinking mode at a higher unit price; if a copyright review blocks it, turn it off and retry.",
    extendMode: "Rewrite mode",
    extendModeDirect: "Direct",
    extendModeAgent: "Agent",
    extendModeAgentHint:
      "Agent rewriting supports text-to-image only; image-to-image requests are rejected by the provider",
    thinking: "Thinking mode",
    thinkingHint:
      "Improves output quality but is slower; only takes effect when prompt rewriting is enabled",
    callMode: "Call mode",
    callModeSync: "Sync",
    callModeAsync: "Async",
    watermark: 'Add "AI generated" watermark',
    watermarkHint: "The watermark sits in the bottom-right corner of the image",
    /** Output image format (Volcano Seedream / SiliconFlow): option values are protocol values, not translated */
    outputFormat: "Output format",
    outputFormatHint: "Encoding of the returned image; only models declaring this parameter offer it",
    seed: "Seed",
    steps: "Steps",
    stepsHint:
      "Diffusion sampling steps; higher is usually slower with possibly richer detail",
    guidance: "Guidance",
    guidanceHint:
      "Prompt guidance strength; higher sticks closer to the prompt, lower is more varied",
    sizeError: {
      format: "Invalid size format, expected width×height or width*height (e.g. 1280×1280)",
      minSide: (v: number) => `Width/height must be at least ${v} px`,
      maxSide: (v: number) => `Width/height must not exceed ${v} px`,
      totalMin: (v: number) => `Total pixels too small; about ${v}×${v} or more is recommended`,
      totalMax: (v: number) => `Total pixels too large; about ${v}×${v} or less is recommended`,
      ratio: (lo: number, hi: number) => `Aspect ratio must be between ${lo}:1 and ${hi}:1`,
    },
    promptTooLong: (limit: number) =>
      `Prompt exceeds this model's ${limit}-character limit; the excess has no effect`,
    seedPlaceholder: "Empty = random",
    seedOutOfRange: (max: number) =>
      `Seed is outside the official range 0–${max}; this generation will be randomized`,
    generate: "Generate",
    generating: "Generating…",
    genSec: (n: number) => `Generating · ${n}s`,
    genErrorTitle: "Generation failed",
    retry: "Retry",
    closeAlert: "Dismiss error",
    closePreview: "Close preview",
    viewLarge: "View full size",
    rerollPrompt: "Re-roll",
    rerollPromptTitle:
      "Re-randomize details from the current data source, overwriting the input",
    metaSeed: (s: string) => `Seed ${s}`,
    skeletonHint:
      "Generating — usually 10–30 seconds, possibly longer with prompt rewriting enabled",
    empty:
      "Fill in a prompt and click Generate; results show up here. Images live only in the session temp directory and hit the disk when you save them manually.",
    /** 持久画廊里图片已被会话清理的条目占位文案 */
    imageMissing: "Image cleaned up (session temp directory only)",
    save: "Save",
    saved: "Image saved",
    removeImageTitle: "Remove",
    needModel: "Select a provider and model in the AI avatars workbench first",
    /** 渲染时抛出：无默认值且无数据源的占位符无法自动解析，只能由用户手工替换，故直接报错并列出变量名。 */
    missingPlaceholders: (keys: string[]) =>
      `Unfilled placeholders: ${keys.join(", ")}`,
    // 数据源：名单/单人预览入口可把球员设为生图数据源，模板变量按名单记录自动填充
    source: {
      clear: "Clear",
      clearTitle: "Clear the data source and restore the manual prompt",
      /** 档案卡：无数据源时的台内 UID 查询（球员源第三来源） */
      queryHint: "Enter a UID to look up a player and set them as the data source",
      queryPlaceholder: "e.g. r-2000472008",
      queryBtn: "Query",
      queryMiss: (uid: string) => `No player found for UID ${uid}`,
      /** 名单未扫描时的就绪卡：直达存档资料台 */
      scanFirst:
        "The roster has not been scanned yet: scan the RTF in the Profiles workbench first to generate by player",
      goArchive: "Go to Profiles",
      /** 资料降级：uid 不在当前名单（可能来自上一次扫描），仅姓名/UID 可信 */
      noProfile:
        "This player was not found in the current roster; variables may not resolve during generation",
      /** 名单预览行尾按钮：把该行球员设为生图数据源并跳到 AI 生图 */
      sendBtn: "Generate",
      sendTitle: (name: string) => `Generate an avatar with ${name} as the source`,
      sent: "Selected",
      useForGen: "Send to generation",
      selected: "Already the source",
    },
    // 结果卡「保存并替换」：生成图写入 <头像包>/_generated/<池>/ 并走单人换脸链路
    saveReplace: {
      btn: "Save & replace",
      needPlayer:
        'Select a player first: click "Generate" at the end of a roster preview row, or "Send to generation" on the single replacement page',
      pathTitle: "Save location",
      // Name the save target explicitly: the image may belong to its own player (queue output / older
      // image), not necessarily the workbench's current source player.
      target: (name: string) => `Save as ${name}'s avatar`,
      pathHint:
        "Path is relative to the _generated folder; use / for nested pools. The file name is de-duplicated automatically.",
      cancelBtn: "Cancel",
      ok: (name: string) => `Saved and replaced ${name}'s avatar`,
      notMapped: (name: string) =>
        `The image was saved under _generated, but the mapping could not be written: the current facepack's config.xml is missing or malformed. Check the facepack and retry.`,
      needSource:
        'Select a source player first ("Generate" at the end of a roster preview row)',
    },
    // Provider settings: the key belongs to the provider; this is the only place to edit it
    keyBarLabel: "API Key",
    keyBarPlaceholder:
      "Enter this provider's key; stored encrypted on this machine",
    keyBarSetPlaceholder: "Set (not shown again); enter a new value to replace it",
    keyStatusUnset: "Not set",
    keyClear: "Clear key",
    keyClearUndo: "Undo clear",
    keyWillClear: "The key will be cleared after saving (click again to undo)",
    baseUrl: "Base URL",
    /** 百炼基址的官方口径：公共域仍可用，业务空间专属域是官方建议迁移的新域名；
     *  地域必须与模型、API Key 一致，跨地域直接失败。 */
    baseUrlHintDashscope:
      "Leave empty for the China North 2 (Beijing) public domain https://dashscope.aliyuncs.com/api/v1, which the vendor confirms remains available." +
      "Migration to the workspace-specific domain https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/api/v1 is officially recommended" +
      " (better performance and stability; find WorkspaceId on the workspace details page of the console)." +
      " Model, base URL and API key must share the same region; cross-region calls fail outright.",
    keyStorageHint:
      "The API key is stored encrypted on this machine and can only be decrypted by the current OS account; re-enter it after switching devices or reinstalling the system.",
    kind: "Kind",
    providerIdLabel: "ID",
    providerIdPlaceholder: "Leave empty to auto-generate",
    providerIdFixedNote:
      "The ID uniquely identifies the provider and cannot be changed after creation; create a new provider to use a different ID.",
    providerNameLabel: "Name",
    providerModelLabel: "Default model",
    /** 模型清单：一套协议可挂多个模型（OpenAI 协议下有大量兼容模型），在此增删改，展开可编辑能力 */
    modelListLabel: "Models",
    modelNamePlaceholder: "Model name (API parameter)",
    addModel: "Add model",
    removeModelTitle: "Remove this model",
    modelCapToggle: "Expand / collapse this model's capability parameters",
    /** 能力编辑区：协议参数名（中文语义名）+ 官方字段名小字，另加各参数的附属项标签 */
    capProtoPrompt: "Prompt",
    capProtoSize: "Output resolution",
    capProtoN: "Image count",
    capProtoNegative: "Negative prompt",
    capProtoSeed: "Random seed",
    capProtoExtend: "Prompt rewriting",
    capProtoExtendMode: "Rewrite mode",
    capProtoThinking: "Thinking mode",
    capProtoWatermark: "Watermark",
    capProtoCallMode: "Call mode",
    capProtoQuality: "Quality tier",
    capProtoResponseFormat: "Return base64",
    /** 火山 Seedream output_format：Pro/Flash/Lite 可设 png/jpeg（值随用户选择）；4.5/4.0 仅 jpeg */
    capProtoOutputFormat: "Output format",
    capProtoSteps: "Steps",
    capProtoGuidance: "Guidance",
    /** 枚举型参数的默认档（当前仅改写模式：官方默认 direct） */
    capFieldModeDefault: "Default",
    /** 官方字段原名（英文，含大小写/下划线/嵌套层级）：只作悬停核验用，名称列一律显示上面的中文名 */
    capApi: {
      prompt: "input.messages[].content[].text",
      size: "parameters.size",
      n: "parameters.n",
      negative: "parameters.negative_prompt",
      seed: "parameters.seed",
      extend: "parameters.prompt_extend",
      extendMode: "parameters.prompt_extend_mode",
      thinking: "parameters.enable_thinking",
      watermark: "parameters.watermark",
      callMode: "X-DashScope-Async",
      quality: "quality",
      responseFormat: "response_format",
      outputFormat: "output_format",
      // steps/guidance 仅硅基流动/魔搭用，不在 DashScope 行集合内；这里给出平铺基名（实际由 capApiByKind 覆盖）。
      steps: "steps",
      guidance: "guidance",
    },
    /** 平铺协议（OpenAI 兼容 / 火山方舟）的官方字段名：字段同名，但没有 DashScope 的
     *  `input.messages[]` 与 `parameters.` 两级嵌套，全部摆在请求体顶层。
     *  小字是给人对着文档核验用的，位置写错就会照着发错。缺项回落 capApi。 */
    capApiFlat: {
      prompt: "prompt",
      size: "size",
      n: "n",
      negative: "negative_prompt",
      seed: "seed",
      extend: "prompt_extend",
      extendMode: "prompt_extend_mode",
      thinking: "enable_thinking",
      watermark: "watermark",
      quality: "quality",
      responseFormat: "response_format",
      outputFormat: "output_format",
      steps: "steps",
      guidance: "guidance",
    } as Record<string, string>,
    /** 按服务商覆盖的官方字段名（优先于 capApiFlat）：硅基流动的尺寸/张数/步数/引导系数
     *  字段名与 OpenAI 不同（image_size / batch_size / num_inference_steps / guidance_scale），
     *  魔搭虽与 Openai 同名但也在此显式列出，避免小字写成会发错的字段名。 */
    capApiByKind: {
      siliconflow: {
        size: "image_size",
        n: "batch_size",
        steps: "num_inference_steps",
        guidance: "guidance_scale",
      },
      modelscope: {
        size: "size",
        n: "n",
        steps: "steps",
        guidance: "guidance",
      },
      // 本地 ComfyUI：参数是工作流节点输入（非扁平 API 字段），小字标注「节点.输入」便于对照导出的 workflow。
      localcomfyui: {
        prompt: "text (CLIPTextEncode)",
        size: "width/height (EmptyLatentImage)",
        n: "batch_size",
        negative: "negative (KSampler)",
        seed: "seed (KSampler)",
        steps: "steps (KSampler)",
        guidance: "cfg (KSampler)",
      },
    } as Record<string, Record<string, string>>,
    capFieldSizes: "Values",
    capRuleSide: "Per side px",
    capRuleTotal: "Total pixels",
    capRuleRatio: "Aspect ratio",
    capZeroHint: "0 = skip this rule",
    capFieldMaxN: "Max images",
    capFieldMaxNHint: "0 = unlimited",
    capFieldStepsDefault: "Default steps",
    capFieldStepsMax: "Max steps",
    capFieldGuidanceDefault: "Default guidance",
    capFieldGuidanceMax: "Max guidance",
    capFieldMaxChars: "Prompt limit",
    capFieldMaxCharsHint: "0 = no character limit",
    capFieldQualities: "Values",
    capFieldSizesPlaceholder: "2048x2048, 2688x1536",
    capFieldQualitiesPlaceholder: "low, medium, high",
    addProvider: "New provider",
    providerSaved: "Provider saved",
    deleteProviderConfirm: (name: string) => `Delete provider "${name}"?`,
    // 模板管理
    templateManageTitle: "Prompt templates",
    templateNameLabel: "Template name",
    templateBodyLabel: "Template body (placeholder syntax {{var|default}})",
    templateNegativeLabel: "Negative prompt",
    addTemplate: "New template",
    deleteTemplateConfirm: (name: string) => `Delete template "${name}"?`,
    // 通用
    edit: "Edit",
    remove: "Remove",
    back: "Back",
    done: "Done",
    colStatus: "Status",
    // 生成队列（批量）
    queue: {
      tabQueue: "Generation queue",
      startBtn: "Start generation",
      cancelBtn: "Cancel generation",
      /** Button label while a cancel request is pending (the running entry is still finishing). */
      cancellingBtn: "Cancelling…",
      /** 「查看单人」：把唯一勾选的队列条目转入「球员源·单人」卡（原「清空队列」职责由全选+移除覆盖） */
      viewSingle: "View single",
      viewSingleTitle: (name: string) =>
        `Move ${name} to the Player source · Single card`,
      viewSingleHint:
        "With one queue entry checked, its player can be moved to the Player source · Single card",
      removeBtn: "Remove selected",
      addBtn: (n: number) => `Add to queue (${n})`,
      statusQueued: "Queued",
      statusRunning: "Generating",
      statusSuccess: "Success",
      statusFailed: "Failed",
      /** User-cancelled (failed + errorKind=cancelled); not a failure. */
      statusCancelled: "Cancelled",
      infoConfig: "Config",
      /** While the queue runs, the single card and workbench are borrowed by this task. */
      queueRunningTag: "Queue running",
      infoPrompt: "Prompt",
      infoNegative: "Negative",
      colPlayer: "Player",
      colDetail: "Detail",
      // ---- Start mode (manual confirm / fully automatic) and per-entry confirmation ---------------------------------
      /** Segmented control: manual on the left, fully automatic on the right. */
      confirmModeManual: "Confirm manually",
      confirmModeAuto: "Fully automatic",
      confirmModeHintManual:
        "Each entry must be confirmed before starting: click a queue row to review and edit it on the right",
      confirmModeAria: "Confirmation mode",
      /** Per-entry confirm button: writes the reviewed parameters back to the queue. */
      confirmBtn: "Confirm",
      /** Confirmed state: same slot, greyed out and disabled. */
      confirmedBtn: "Confirmed",
      /** Start blocked: N entries are still unconfirmed. */
      needConfirm: (n: number) => `${n} entries are still unconfirmed. Confirm each one before starting.`,
      /** Guidance shown above the form when manual mode has no selected row. */
      queueEditHint: "Click a queue row on the left to review and edit its generation parameters here.",
      /** Borrow refused: the row's provider/model is no longer configured, so its capabilities are unknown. */
      borrowUnavailable: (name: string) =>
        `The provider or model used by "${name}" is no longer configured, so its parameters can't be reviewed here. Pick the row and remove it, or re-add the player from the roster.`,
      count: (total: number) => `${total} total`,
      startConfirm: (n: number) =>
        `Start batch generation of ${n} queued tasks? Each will call the provider API and incur charges.`,
      /** Only the checked queue entries run; the rest stay queued. */
      startConfirmPicked: (n: number, rest: number) =>
        `Start batch generation of the ${n} checked tasks? The other ${rest} stay queued for later.`,
      cancelConfirm:
        "Unsubmitted entries are cancelled immediately; already-submitted ones wait for this request to return and keep their results (charges apply). Cancel anyway?",
      cancellingHint:
        "Cancelling: unsubmitted entries are stopped; submitted ones finish once the provider responds (charges apply).",
      removeConfirm: (n: number) => `Remove the ${n} selected entries?`,
      empty:
        "Queue is empty. Check players in the roster preview and click Add to queue for batch generation.",
      enqueued: (n: number) => `Added ${n} generation tasks`,
      imagesDone: (n: number) => `${n} images generated`,
      progressLabel: "Batch generation progress",
      /** 队列进度计数：首个数字是**已完成数**（终态条目累计），不是派发序号。 */
      progressCount: (done: number, total: number) => `${done} done / ${total} total`,
      /** 运行中条目的阶段文案：与后端 GenStage 的 as_str 取值一一对应（submitting/polling/downloading）。 */
      stageLabel: (stage: string): string => {
        const labels: Record<string, string> = {
          submitting: "Submitting",
          polling: "Waiting for provider",
          downloading: "Downloading",
        };
        return labels[stage] ?? "";
      },
      finishedSummary: (finished: number, failed: number, total: number) =>
        `Batch generation finished: ${finished} succeeded · ${failed} failed · ${total} total`,
      retryEntry: "Retry this entry",
      /** 执行期重试徽标：attempts 是「总尝试次数」（首发即 1），故实际重试次数 = attempts - 1。
       *  仅在 attempts > 1（即确实重试过）时展示。 */
      retriedBadge: (attempts: number) => `Retried ${attempts - 1}×`,
      /** 失败条目按错误类别给出的处置建议（对应后端 GenErrorKind）；无建议类别返回空串。 */
      errorAction: (kind: string): string => {
        const actions: Record<string, string> = {
          auth: "Check whether the API key is correct or expired",
          rate_limited: "Retry later or reduce concurrency",
          not_configured: "Fill in the key/model in provider settings",
          invalid_param: "Adjust parameters as prompted",
          content_rejected: "Edit the prompt and retry",
          network: "Check your network and retry",
          provider: "Retry later; check the provider status if it keeps failing",
          io: "Check disk space and directory permissions",
        };
        return actions[kind] ?? "";
      },
      viewImages: "View images",
      collapseImages: "Hide images",
      saveImage: "Save as",
      // 复选框的读屏文案：队列条目（QueueDock）与名单表格（PlayersTable）各一套
      selectRowAria: (name: string) => `Select ${name}`,
      selectAllAria: "Select or deselect all entries on this page",
      selectPlayerAria: (name: string) => `Select player ${name}`,
      selectAllPlayersAria: "Select or deselect all filtered rows (across pages)",
    },
  },

  /** 「替换执行」面板 */
  replace: {
    cardTitle: "Batch replacement",
    modeLabel: "Mode:",
    optionsLabel: "Options:",
    optAllowDuplicates: "Allow image reuse",
    optFilterNewgan: "NewGAN only",
    optSaveBackup: "Backup before writing",
    /** 三个替换模式：value 发往后端（协议值，勿改）；label/desc 为界面显示文案（与后端日志用词一致） */
    modes: [
      {
        value: "Preserve",
        label: "Preserve",
        desc: "Keep already-replaced players; only new players get faces (recommended for daily increments)",
      },
      {
        value: "Overwrite",
        label: "Overwrite",
        desc: "Redraw all players in the roster; players outside the roster are kept",
      },
      {
        value: "Generate",
        label: "Generate",
        desc: "Ignore the existing config.xml and generate all mappings from scratch",
      },
    ] as ReplaceMode[],
    startBtn: "Replace faces",
    runningBtn: "Replacing", // 按钮上会追加实时百分比，如「替换中 45%」
    cancelBtn: "Cancel",
    browseDisabledHint:
      'Click "Activate" on the left before replacing (currently previewing)',
    browseNotice: (name: string) =>
      `Browsing "${name}" (not activated). Replacement only affects the active profile; click Activate on the left first.`,
    needProfile: "Select a profile",
    needGroup: "Select a facepack",
    needRtf: "Set the RTF roster",
    needRoster:
      "Build the player/staff roster cache first (read the running game or scan a .fm save)",
    todoPrefix: "Still needed:",
    missingDirsTitle: "Missing directories",
    missingDirsConfirm: (standard: number, count: number) =>
      `Standard ${standard} is missing ${count} directories. Create them and continue?`,
    unknownTitle: "Unrecognized structure",
    unknownConfirm:
      "The facepack structure is unrecognized; replacement will distribute all images (including sub/sub-sub directories) at random. Continue?",
    // 发起前预检（preflight_replace）：只有「生成」+ 本档档景非空时会走到这条确认。
    preflightTask: "Check existing mapping",
    blankTitle: "This will clear the profile's existing mappings",
    blankConfirm: (existing: number) =>
      `Generate mode carries no existing entries. If this run draws nothing (roster filtered empty, UIDs unmatched, no images in the pack),\n` +
      `the profile's existing ${existing} mappings will be overwritten with an empty table.\n\nProceed anyway?`,
  },

  /** 结果区顶部 Tab（替换台读视图：名单/快照；单人替换已常驻替换台左栏，AI 生图已独立成台） */
  results: {
    tabTable: "Roster preview",
    tabSnapshot: "Snapshot preview",
  },

  /** 名单预览表（PlayersTable） */
  table: {
    title: "Roster preview",
    recordCount: (total: number) => `${total} total`,
    searchPlaceholder: "Search UID / name / nationality / club…",
    prevPage: "‹ Prev",
    nextPage: "Next ›",
    pageOf: (current: number, total: number) => `Page ${current} / ${total}`,
    /** 快速跳页：「前往 [输入框] 页 [跳转]」 */
    gotoPrefix: "Go to",
    gotoSuffix: "",
    gotoBtn: "Go",
    gotoAria: "Enter a page number and press Enter to jump",
    colUid: "UID",
    colNat: "Nationality",
    col2ndNat: "2nd nationality",
    colName: "Name",
    colHair: "Hair length",
    colColor: "Hair color",
    colEth: "Ethnicity",
    colSkin: "Skin tone",
    colAvatar: "Avatar",
    /** 批量取头像失败时挂在「头像」列表头提示上（逐图取不到仍是静默留白，那本来就是正常情况）。 */
    avatarFailHint: (why: string) =>
      `Failed to load avatars (${why}); this column stays empty — it does not mean the roster has no faces`,
    colNewgan: "NewGAN",
    colGender: "Gender",
    colAge: "Age",
    colHeight: "Height",
    colClub: "Club",
    colBased: "Based",
    colJob: "Preferred job",
    colCa: "CA",
    colPa: "PA",
    colRca: "R.C.A",
    /** 头像列缩略图 + 悬停/点击气泡的状态文案。 */
    avatarBtn: "Preview",
    avatarAria: "Preview this player's avatar image",
    avatarClose: "Close avatar preview",
    avatarLoading: "Loading…",
    avatarNotFound: "Avatar image not found",
    /** 随机人筛选（仅拓展名单显示）：标签 + 三个选项的文案。 */
    newganFilterLabel: "NewGAN",
    newganAny: "Any",
    newganYes: "Yes",
    newganNo: "No",
    /** 球员/职员视图切换（仅混合名单显示）：标签 + 三个选项 + 视图内无记录的空态文案。 */
    rosterViewLabel: "View",
    rosterViewAll: "All",
    rosterViewPlayer: "Players",
    rosterViewStaff: "Staff",
    rosterViewEmpty: "No records in the current view.",
    /** 混合名单下第 15 列「全部」视图的表头：两种语义并列（三字典同值，不逐词翻译）。 */
    rosterViewMixedHeader: "所在地 / 偏好工作",
    emptyHint:
      'Run "Replace faces" or "Scan RTF" to preview the roster here (most recent parse).',
    /** 缓存里没有匹配当前筛选条件的记录（与快照预览的 noMatchHint 同款语义）。 */
    noMatchHint: "No matching players.",
    /** 替换台「拓展列」开关：补齐仅拓展名单才有的字段列 */
    extCols: "Extra columns",
  },

  /** 单人预览/微调（PlayerViewer） */
  viewer: {
    title: "Player preview",
    /** 替换台左栏单人替换卡的标题（读写分栏后常驻左栏，不属于结果区读视图） */
    section: "Single replacement",
    hint: "Enter a keyword and press Enter to search",

    searchBtn: "Search",
    /** 命中多个时的前后切换（到头即禁用） */
    prevMatch: "Previous",
    nextMatch: "Next",
    colMapping: "Current mapping",
    /** 拓展名单第 19 列起的用户自定义列（后端只保留原文，无表头名，按序号命名）。 */
    extraField: (n: number) => `Custom ${n}`,
    noImage: "No image",
    imgPlaceholder: "Enter a new image path…",
    imgPathAria: "Replacement image path",
    browseBtn: "Browse",
    replaceBtn: "Replace",
    /** 重掷替换：按标准规则给该球员重新抽图（后端固定重抽语义） */
    resetBtn: "Re-roll",
    resetOk: (uid: string) =>
      `Re-rolled the avatar for UID ${uid} (redrawn per the rules)`,
    imageDialogTitle: "Select image file",
    imageFilterName: "Images",
    missingInput: "Search and select a player first, then choose a valid image path",
    replacedOk: (uid: string) => `Replaced the avatar for UID ${uid}`,
    mappingNotFound: (uid: string) =>
      `No existing mapping for UID ${uid} in config.xml; nothing changed`,
  },

  /** 快照预览（SnapshotPreview）：展示「读取快照」解析出的存档快照映射。分页器与名单预览共用（TablePager，文案取 t.table.*） */
  snapshot: {
    searchPlaceholder: "Filter by UID…",
    recordCount: (total: number) => `${total} total`,
    colUid: "UID",
    colPath: "Avatar relative path",
    colImage: "Avatar",
    noImage: "None",
    // 未读取（或切档/切组后失效）时的引导文案
    readHint:
      'Click "Read snapshot" under "Current task" to parse the savegame XML snapshot and preview the mapping list here.',
    noMatchHint: "No matching UIDs.",
    emptyHint: "The snapshot has no mapping records.",
  },

  /** 底部状态栏 */
  status: {
    ready: "Ready",
    noGroup: "No facepack selected",
    log: "Logs",
  },

  /** 日志抽屉 */
  log: {
    title: "Run log",
    onlyLevel: "This level only",
    openFileBtn: "Open log file",
    clearBtn: "Clear view",
    closeAria: "Close log panel",
  },

  /** 标准规则编辑对话框 */
  ruleEditor: {
    std2Label: "Standard 2 (14 ethnicity directories)",
    std3Label: "Standard 3 (fine-grained directories)",
    title: "Rule configuration",
    kicker: (standard: number, dirs: number) => `STANDARD ${standard} · ${dirs} DIRECTORIES`,
    sourceBtn: "Source",
    structuredBtn: "Structured",
    sourceHint: "Edit the raw JSON; format and directory references are validated on save",
    sourceTitle: "JSON source",
    sourceDesc:
      "Advanced mode: edit the whole rules file directly. Switching back to Structured parses it first and keeps errors on this page.",
    parseFailed: (detail: string) =>
      `Could not parse as structured rules: ${detail}. Fix it in Source, then switch back.`,
    loading: "Loading rules file…",
    saveBtn: "Save",
    closeBtn: "Close",
    saveHint: "JSON format is validated before saving; invalid content is not written",
    savedStatus: "Saved ✓ — takes effect on the next replacement / integrity check",
    unsavedTitle: "Unsaved changes",
    unsavedSwitch: (label: string) =>
      `${label} has unsaved changes. Discard them and switch?`,
    unsavedClose: (label: string) =>
      `${label} has unsaved changes. Discard them and close?`,
    editorAria: "Rule configuration content",
    sections: {
      overview: "Overview",
      folders: "Directories",
      ethnics: "Nationalities",
      codeRules: "Race-code fixes",
      blocks: "Rule blocks",
      settings: "Global settings",
    },
    statFolders: "Directories",
    statNations: "Nationalities",
    statCodeRules: "Code fixes",
    statRaceCodes: "Race codes",
    statBlocks: "Blocks",
    statRuleEntries: "Rule entries",
    common: {
      add: "Add",
      delete: "Delete",
      notInList: "not in list",
    },
    overview: {
      title: "Configuration overview",
      desc: "Check first, then edit; the backend still performs the final validation on save.",
      healthTitle: "Configuration check",
      healthOk: "Structure check passed",
      healthWarn: (count: number) => `${count} item(s) to fix`,
      healthOkDesc: "No duplicate directories, broken references or missing default block found.",
      healthEmptyFolders: "FolderList is empty; at least one directory is required",
      healthEmptyEthnics: "No nationality mappings yet; every player will be skipped",
      healthDup: (count: number) => `${count} duplicate director${count === 1 ? "y" : "ies"} in FolderList`,
      healthEthnics: (count: number) => `${count} nationality mapping(s) point outside FolderList`,
      healthRefs: (count: number) => `${count} rule reference(s) point outside FolderList`,
      healthMissingDefault: "The _DEFAULT_ global block is missing",
      healthVersion: (version: number) => `Version is ${version}; only 4 is supported`,
      healthOrphans: (count: number) => `${count} DefaultCodes key(s) have no matching nationality`,
      goFix: "Fix",
      flowTitle: "Picking pipeline",
      flow2: ["Nationality map", "Race-code fix", "Random pick"],
      flow3: ["Country block", "Group block", "_DEFAULT_ block"],
      flowHint:
        "Blocks are searched in order for the player's race code; a missing code falls through, and an unavailable directory uses Fallback.",
      fileLabel: "File",
    },
    folders: {
      title: "Directories",
      desc: "FolderList is the whitelist for standard detection and image picking; directories outside it are never scanned or used.",
      searchPlaceholder: "Search directories",
      addPlaceholder: "New directory name",
      empty: "No matching directories",
      duplicate: "That directory is already in the list",
      blank: "Directory name cannot be empty",
      used: (count: number) => `${count} reference(s)`,
      removeHint: "Remove this directory from the list",
      removeConfirm: (name: string, count: number) =>
        count > 0
          ? `Directory "${name}" has ${count} reference(s); removing it will break them and require manual fixes. Continue?`
          : `Remove directory "${name}"?`,
      count: (n: number) => `${n} directories`,
    },
    ethnics: {
      title: "Nationalities",
      desc: "Three-letter nationality code → home pool directory. Players whose nationality has no mapping are skipped.",
      searchPlaceholder: "Search code or directory",
      addCodePlaceholder: "Code, e.g. CHN",
      codeHeader: "Code",
      folderHeader: "Home directory",
      exists: "That nationality code already exists",
      invalidCode: "The code must be 2–3 uppercase letters",
      noFolders: "Add a directory in Directories first",
      empty: "No matching nationalities",
      removeHint: "Delete this nationality mapping",
      removeConfirm: (code: string) =>
        `Delete nationality ${code} together with its DefaultCodes entry and country rule block?`,
      count: (n: number) => `${n} nationalities`,
    },
    codeRules: {
      title: "Race-code fixes",
      desc: "Remap the picking directory by FM race code: prefer entries are matched in order against the primary/second nationality directory, then the fallback is used.",
      codeLabel: (code: string) => `Race code ${code}`,
      unset: "Not configured — use the nationality directory",
      addBtn: "Add fix",
      deleteHint: "Delete this race-code fix",
      preferLabel: "Preferred directories (in order)",
      fallbackLabel: "Fallback directory",
      emptyPrefer: "No preferred directory; the fallback is used directly",
      addPlaceholder: "Add a preferred directory",
      addBtnShort: "Add",
      moveUp: "Move up",
      moveDown: "Move down",
      removeDir: "Remove this directory",
    },
    blocks: {
      title: "Rule blocks",
      desc: "Country block → group block → _DEFAULT_; a missing code inside a block falls through to the next level.",
      searchPlaceholder: "Search rule blocks",
      addPlaceholder: "Choose a key for the new block",
      defaultKind: "Global",
      nationKind: "Country",
      folderKind: "Group",
      unknownKind: "Unknown",
      count: (n: number, total: number) => `${n}/${total} codes`,
      empty: "No matching rule blocks",
      noSelection: "Select a rule block on the left to start editing",
      detailDesc: "A missing code in this block falls through to the next block; that is valid configuration.",
      codeMissing: "Not configured (falls through)",
      addRuleBtn: "Add rule",
      deleteRuleHint: "Delete the rule for this code",
      deleteBlockHint: "Delete this rule block",
      deleteBlockConfirm: (key: string) => `Delete rule block "${key}"?`,
      unknownRule: "The rule structure cannot be recognised; fix it in Source or reset it",
      resetRuleBtn: "Reset to default",
      typeLabel: "Mapping type",
      typeUnknown: "Unrecognised",
      typeDefault: "Default",
      typeDirectly: "Direct",
      typeRandom: "Random",
      typeWeighted: "Weighted",
      typeDefaultHint: "Use the player's nationality home directory from Ethnics",
      typeDirectlyHint: "Map to one fixed directory",
      typeRandomHint: "Pick one directory from the list with equal probability",
      typeWeightedHint: "Pick one directory by weight; weights are normalised by their sum",
      directoryLabel: "Directory",
      addDirPlaceholder: "Add a directory",
      addDirBtn: "Add",
      weightLabel: "Weight",
      addWeightBtn: "Add weighted entry",
      removeItem: "Remove",
      atLeastOne: "Keep at least one entry",
      extraCodes: (count: number) =>
        `${count} other code(s) outside 0–10 are kept as-is and not edited here`,
    },
    settings: {
      title: "Global settings",
      desc: "Rules-file version and the fallback behaviour when a directory is unavailable.",
      versionLabel: "Version",
      versionHint: "Standard 3 rules format version; only 4 is supported",
      fallbackLabel: "Fallback",
      fallbackRandom: "Random from full pool",
      fallbackRandomHint: "When rule directories are unavailable, pick recursively from the whole pack",
      fallbackSkip: "Skip",
      fallbackSkipHint: "Skip the player when rule directories are unavailable",
      passthroughTitle: "Passthrough data",
      passthroughDesc:
        "DefaultCodes / FolderMapping* are maintained by the factory configuration and are not rewritten here; they are preserved on save and can be edited in Source.",
      passthroughEditBtn: "Edit in Source",
      count: (n: number) => `${n} entries`,
    },
  },

  /** Nationality table editor */
  nationalityEditor: {
    aria: "Nationality table",
    title: "Nationality table",
    subtitle:
      "Three-letter code ↔ full nationality name in each language (aliases allowed); codes and names are maintained separately, so you can register a code without a full name yet. Changes take effect on the next RTF scan / replacement / prompt preview after saving. File: data/nationalities.json",
    searchPlaceholder: "Search code, language or name",
    countLabel: (shown: number, total: number) => `Showing ${shown} / ${total}`,
    codeHeader: "Code",
    languageHeader: "Language",
    nameHeader: "Full name/alias",
    languageLabel: "Languages",
    addCodePlaceholder: "Code, e.g. CHN",
    addNamePlaceholder: "Full name in this language (empty = register the code only)",
    addBtn: "Add",
    addLanguagePlaceholder: "Language, e.g. Deutsch",
    addLanguageBtn: "Add language",
    languageExists: "That language is already in the list",
    duplicate: "This mapping already exists",
    codeOnly: "Code registered (no full name yet)",
    codeExists: "That code is already registered",
    incomplete: "Fill in the code (the name may be left empty)",
    bareLabel: (n: number) => `Codes without names: ${n}`,
    removeCodeHint: "Remove this code: it will no longer count as a valid nationality value",
    noMatch: "No matching mappings",
    deleteBtn: "Delete",
    deleteHint: "Delete this mapping (takes effect after saving)",
    emptyHint:
      "Codes are 2–3 uppercase letters; a name can map to only one code within a language; keep at least one code or one mapping. Click Save to write the file.",
    saveBtn: "Save",
    closeBtn: "Close",
    saveHint: "Mappings are validated and written atomically; invalid content is not saved",
    savedStatus: "Saved ✓ — takes effect on the next scan / replacement",
    unsavedTitle: "Unsaved changes",
    unsavedClose: "The nationality table has unsaved changes. Discard them and close?",
  },
};

const DICT: Record<Lang, Dict> = {
  "zh-CN": zhCN,
  "zh-TW": zhTW,
  en,
};

/** 按当前语言分发文案：getter 内读 ui.lang（$state），切换语言时模板自动重算。 */
export const t = {
  get brand() {
    return DICT[ui.lang].brand;
  },
  get dialog() {
    return DICT[ui.lang].dialog;
  },
  get app() {
    return DICT[ui.lang].app;
  },
  get workbench() {
    return DICT[ui.lang].workbench;
  },
  get theme() {
    return DICT[ui.lang].theme;
  },
  get lang() {
    return DICT[ui.lang].lang;
  },
  get help() {
    return DICT[ui.lang].help;
  },
  get sidebar() {
    return DICT[ui.lang].sidebar;
  },
  get setup() {
    return DICT[ui.lang].setup;
  },
  get gen() {
    return DICT[ui.lang].gen;
  },
  get replace() {
    return DICT[ui.lang].replace;
  },
  get results() {
    return DICT[ui.lang].results;
  },
  get table() {
    return DICT[ui.lang].table;
  },
  get viewer() {
    return DICT[ui.lang].viewer;
  },
  get snapshot() {
    return DICT[ui.lang].snapshot;
  },
  get status() {
    return DICT[ui.lang].status;
  },
  get log() {
    return DICT[ui.lang].log;
  },
  get ruleEditor() {
    return DICT[ui.lang].ruleEditor;
  },
  get nationalityEditor() {
    return DICT[ui.lang].nationalityEditor;
  },
};
