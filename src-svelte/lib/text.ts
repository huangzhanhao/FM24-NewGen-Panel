/**
 * 全局文案常量：所有「显示给用户看的文字」统一在这里维护（默认中文）。
 * 组件里 `import { t } from '../text'`，模板中直接用 {t.xxx.yyy}；需拼变量的动态文案导出为函数。
 *
 * 约定：参与逻辑判断的「协议值」不放这里——如缺省档名 'No Profile'、替换模式 'Preserve'、
 * 日志级别 'INFO'、事件名、排序键等，它们的取值发往后端，改动会破坏前后端逻辑。
 * 好处：改文案只改这一处；将来多语言时替换为按语言分发的实现即可。
 */

export const t = {
  /** 应用品牌名（窗口/弹窗标题、侧栏品牌区共用；品牌名不翻译） */
  brand: "NewGAN Manager",

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

  /** 顶栏三工作台（由原五步旅程引导改造：chip 可点击切换，右侧状态点随就绪度变色） */
  workbench: {
    archive: "存档资料",
    replace: "批量/单人头像替换",
    ai: "AI 生成头像",
    /** 顶栏右端作用域回显：档名与组名之间的分隔符（组为空时只显示档名） */
    scopeSep: " · ",
    /** 无头像包组时的回显占位（侧栏还没关联目录） */
    scopeNoGroup: "未选头像包",
    /** chip 状态点的悬停说明（三档：绿 / 琥珀 / 灰） */
    stateOk: "条件已满足",
    stateWarn: "还差几项，点进去看提示",
    stateIdle: "尚未开始",
  },

  /** 侧栏底部主题切换 */
  theme: {
    dark: "深色",
    light: "浅色",
    switchTo: (mode: "dark" | "light") =>
      mode === "light" ? "切换到浅色主题" : "切换到深色主题",
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
    /** 队列运行中的切档/切组护栏（S10）：空闲切换不弹窗不打断，运行中才问这一句 */
    queueScopeTitle: "AI 头像队列正在运行",
    queueScopeConfirm:
      "切换存档或头像包后，未完成的生成任务将被取消。确定要切换吗？",
    /** 队列运行中执行删除时追加在删除确认文案后的那一段。删除本身已有确认框，
     *  这里并入同一段正文，避免「先问删除、再问队列」连弹两个框（一次操作只打扰一次）。 */
    queueScopeAppend: "\n\nAI 头像队列正在运行，继续将取消未完成的生成任务。",
    rtfSet: "RTF ✓",
    rtfUnset: "未设置 RTF",
    emptyGroups: "暂无头像包。点上方 ＋ 关联头像包目录。",
    linkTutorial: "教程",
    linkTroubleshooting: "故障排查",
    linkFaq: "FAQ",
    linkDiscord: "Discord",
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
      "直接查看、修改并保存标准规则文件（国籍→目录映射、FolderList 等），保存时自动校验 JSON 格式",
    ruleConfigNote:
      "抽图规则来自规则 json（可编辑），保存后下次替换/检查完整性即生效",
  },

  gen: {
    // 工作台顶部与面板
    provider: "服务商",
    model: "模型",
    providerSettings: "服务商设置",
    /** 服务配置条就绪状态灯（S6）：判据与顶栏 ③chip 同源（Key 已设置 + 模型已选），不冒充「已连通」 */
    statusReady: "已就绪",
    statusNoKey: "未设置 Key",
    statusNoModel: "未选模型",
    statusReadyTitle: "Key 已设置且模型已选（连通性以实际生成为准）",
    /** 就绪卡：Key 未设置时条件出现（「去设置」聚焦配置条 Key 输入框） */
    /** Key 输入框右侧状态胶囊的未设置态文案（警示胶囊的 title 用 keyMissingHint 讲清后果） */
    keyMissingHint:
      "生成前需先设置当前服务商的 API Key（仅本会话记忆，不落盘）——点此聚焦输入框",
    cardSource: "球员源 · 单人",
    cardSettings: "生成设置",
    resultsTitle: "本次结果",
    resultsCount: (n: number) => `${n} 张`,
    clearResults: "清空",
    clearResultsTitle: "清空本次会话的全部生成结果",
    clearResultsConfirm: (n: number) =>
      `清空本次会话的 ${n} 张生成结果？临时图片将被删除，未手动保存的无法恢复。`,
    keySubmit: "提交 Key",
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
    seed: "Seed",
    steps: "采样步数",
    stepsHint: "扩散采样步数，越大通常越慢、细节可能更充分",
    guidance: "引导系数",
    guidanceHint: "提示词引导强度，越大越贴合提示词，越小越多变",
    sizeError: {
      format: "尺寸格式不正确，应为 宽×高（如 1280×1280）",
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
    generateShortcut: "Ctrl + Enter 直接生成",
    generate: "生成图片",
    generating: "生成中…",
    genSec: (n: number) => `生成中 · ${n}s`,
    genErrorTitle: "生成失败",
    retry: "重试",
    closeAlert: "关闭错误提示",
    closePreview: "关闭预览",
    viewLarge: "查看大图",
    sourceEditableNote:
      "已选择数据源：提示词已按该球员数据组装，可直接在此修改；未修改时每次生成都会重新随机细节，批量生成按各球员分别组装（不使用此处文本）",
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
    /** 生成前渲染发现「既无默认值、又无数据源」的占位符时抛出（P1 无数据源，出现即需用户手工替换）。 */
    missingPlaceholders: (keys: string[]) => `占位符未填充：${keys.join("、")}`,
    // P3 数据源：名单/单人预览入口把球员设为生图数据源，模板变量按名单记录自动填充
    source: {
      clear: "清除",
      clearTitle: "清除数据源，恢复手动提示词",
      /** 档案卡（S7）：无数据源时的台内 UID 查询（球员源第三来源） */
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
    // P3 结果卡「保存并替换」：生成图写入 <头像包>/_generated/ 并走单人换脸链路
    saveReplace: {
      btn: "保存并替换",
      needPlayer:
        "需先选择球员：在「名单预览」行尾点「去抽图」或单人替换页点「送生图」",
      confirm: (name: string) =>
        `把这张生成图设为 ${name} 的头像？将写入头像包并替换其映射。`,
      ok: (name: string) => `已保存并替换 ${name} 的头像`,
      needSource: "请先选择数据源球员（名单预览行尾「去抽图」）",
    },
    // 服务商设置
    // 顶栏直达的 Key 掩码输入（唯一入口，与主界面配置条的 key-row 同源）
    keyBarLabel: "API Key",
    keyBarPlaceholder: "输入当前服务商的 Key，仅本会话记忆，不落盘",
    keyBarSetPlaceholder: "已设置（不回显），如需更换请重新输入",
    baseUrl: "接口地址 Base URL",
    /** 百炼基址的官方口径：公共域仍可用，业务空间专属域是官方建议迁移的新域名；
     *  地域必须与模型、API Key 一致，跨地域直接失败。 */
    baseUrlHintDashscope:
      "留空 = 华北2（北京）公共域 https://dashscope.aliyuncs.com/api/v1，官方明确仍可使用。" +
      "官方建议迁移到业务空间专属域 https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/api/v1" +
      "（性能与稳定性更好；WorkspaceId 在控制台的业务空间详情页）。" +
      "模型、接口地址、API Key 三者必须同地域，跨地域调用会直接失败。",
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
    /** 火山 Seedream output_format：Pro/Flash/Lite 可设 png/jpeg（适配器固定 png）；4.5/4.0 仅 jpeg */
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
      colPlayer: "球员",
      colDetail: "详情",
      count: (total: number) => `共 ${total} 条`,
      startConfirm: (n: number) =>
        `开始批量生成 ${n} 个排队任务？将逐条调用服务商接口并产生费用。`,
      cancelConfirm:
        "取消后未启动的条目将标记为已取消，正在生成的会跑完。确定取消？",
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
          polling: "排队中",
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
      selectAllPlayersAria: "全选或取消本页球员",
    },
  },

  /** 「替换执行」面板 */
  replace: {
    cardTitle: "替换执行",
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
    ] as const,
    startBtn: "替换头像",
    runningBtn: "替换中", // 按钮上会追加实时百分比，如「替换中 45%」
    cancelBtn: "取消",
    browseDisabledHint: "请先在左侧「激活」存档后再执行替换（当前为浏览预览）",
    browseNotice: (name: string) =>
      `正在浏览「${name}」（未激活）。替换只作用于已激活的存档，请先在左侧点「激活」。`,
    needProfile: "选择 Profile",
    needGroup: "选择头像包",
    needRtf: "设置 RTF 名单",
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
    saveBtn: "保存",
    closeBtn: "关闭",
    saveHint: "保存前会校验 JSON 格式，非法内容不会被写入",
    savedStatus: "已保存 ✓ 下次替换 / 检查完整性即生效",
    unsavedTitle: "未保存修改",
    unsavedSwitch: (label: string) =>
      `${label} 有未保存的修改，确定放弃并切换？`,
    unsavedClose: (label: string) => `${label} 有未保存的修改，确定关闭？`,
    editorAria: "标准规则内容",
  },
} as const;
