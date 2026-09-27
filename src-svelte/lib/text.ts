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
  brand: 'NewGAN Manager',

  /** 通用系统弹窗标题 */
  dialog: {
    error: '错误',
    info: '提示',
    warning: '警告',
  },

  /** 根组件：替换结束的系统提示 */
  app: {
    replaceFinished: '替换完成！',
    replaceFailed: '替换失败，请查看日志',
    brandVer: 'v2.0',
  },

  /** 顶部五步旅程引导（纯展示，不可点击，随快照状态点亮） */
  stepper: {
    steps: ['选档与头像包', '校验与名单', '选模式', '执行替换', '查看与微调'],
  },

  /** 侧栏底部主题切换 */
  theme: {
    dark: '深色',
    light: '浅色',
    switchTo: (mode: 'dark' | 'light') => (mode === 'light' ? '切换到浅色主题' : '切换到深色主题'),
  },

  /** 侧边栏：存档（Profile）与头像包（组）管理 */
  sidebar: {
    profileSection: '存档资料',
    newProfileBtn: '新建 Profile',
    newProfilePlaceholder: '新 Profile 名称',
    createBtn: '创建',
    notActivated: '未激活',
    activated: '已激活',
    deleteCurrentProfile: '删除当前 Profile',
    noProfileNoActivate: 'No Profile 为启动缺省档，未激活',
    noProfileNoDelete: '缺省档不可删',
    activeAlreadyHint: '当前已激活该存档',
    activateBtn: '激活',
    activateHint: (name: string) => `激活存档「${name}」并切换其组配置与映射`,
    deleteProfileConfirm: (name: string) => `删除 Profile「${name}」及其全部组配置与映射备份？`,
    deleteProfileTitle: '删除 Profile',
    groupSection: '头像包（组）',
    linkNewHint: '关联新的头像包目录',
    linkBrowseHint: '请先点击「激活」后再关联头像包',
    linkDialogTitle: '关联头像包目录',
    selectGroupBrowseHint: '请先点击「激活」后再切换当前组（当前为浏览预览）',
    deleteGroupBrowseHint: '请先激活后再删除组',
    deleteGroupBtn: '删除组',
    deleteGroupConfirm: (dir: string) => `删除组及其映射备份？\n${dir}`,
    deleteGroupTitle: '删除组',
    rtfSet: 'RTF ✓',
    rtfUnset: '未设置 RTF',
    emptyGroups: '暂无头像包。点上方 ＋ 关联头像包目录。',
    linkTutorial: '教程',
    linkTroubleshooting: '故障排查',
    linkFaq: 'FAQ',
    linkDiscord: 'Discord',
  },

  /** 「当前任务」卡：路径展示、完整性检查、扫描、规则配置入口 */
  setup: {
    cardTitle: '当前任务',
    imgDirLabel: '头像包：',
    rtfLabel: 'RTF 名单：',
    xmlLabel: 'config.xml：',
    imgDirPlaceholder: '未关联头像包目录',
    rtfUnsetValue: '未设置',
    xmlPlaceholder: '未关联头像包（执行替换后生成）',
    noGroupHint: '请先关联头像包目录',
    checkIntegrityBtn: '检查完整性',
    checkIntegrityHint: '按规则 json 的 FolderList 识别头像包归属哪个标准',
    scanImagesBtn: '扫描头像',
    scanImagesHint: '扫描头像包并缓存图片索引（结果写入日志）',
    selectRtfBtn: '选择RTF',
    selectRtfDialogTitle: '选择 RTF 名单',
    selectRtfHint: '选择 FM 导出的 RTF 名单',
    selectRtfBrowseHint: '请先点击「激活」后再设置 RTF（当前为浏览预览）',
    scanRtfBtn: '扫描RTF',
    scanRtfHint: '解析 RTF 并填充名单预览（结果写入日志）',
    viewConfigBtn: '读取快照',
    viewConfigHint: '解析存档 XML 快照，在结果区「快照预览」中查看映射列表',
    viewSnapshotBtn: '查看位置',
    viewSnapshotHint: '打开存档 XML 快照所在目录',
    noGroupGuide: '先在左侧「头像包（组）」关联一个头像包目录，或点击已有组。',
    tpl1Complete: '标准一：图片直铺，无种族子目录',
    tplComplete: (standard: number, total: number) => `标准${standard}：${total} 个配置目录齐全`,
    integrityOk: (label: string) => `✓ 头像包结构完整（${label}）`,
    extraDirsNote: (n: number) => (n > 0 ? `；另含 ${n} 个未配置目录（不参与抽图）` : ''),
    integrityFixable: (standard: number, missing: number, found: number, total: number, list: string) =>
      `标准${standard}：缺少 ${missing} 个目录（已有 ${found}/${total}）：${list}`,
    createMissingBtn: '一键创建缺失目录',
    missingDirsTitle: '缺失目录',
    missingDirsConfirm: (standard: number, count: number, list: string) =>
      `标准${standard} 缺少 ${count} 个目录：\n${list}\n\n自动创建？`,
    integrityUnknown: '⚠ 无法识别头像包结构：子目录与任何标准的 FolderList 清单都不构成包含关系，替换时将把包内全部图片随机分配；也可在「规则配置」中调整标准目录清单。',
    // 完整性状态胶囊的短文案（详情仍用上方 integrityFixable / integrityUnknown 长句）
    pillMissingDirs: (missing: number, found: number, total: number) =>
      `缺少 ${missing} 个目录（已有 ${found}/${total}）`,
    pillUnknown: '无法识别头像包结构',
    ruleConfigBtn: '规则配置',
    ruleConfigHint: '直接查看、修改并保存标准规则文件（国籍→目录映射、FolderList 等），保存时自动校验 JSON 格式',
    ruleConfigNote: '抽图规则来自规则 json（可编辑），保存后下次替换/检查完整性即生效',
  },

  gen: {
    // 结果区第 4 个 Tab
    tabGenerate: 'AI 生图',
    // 工作台顶部与面板
    provider: '服务商',
    model: '模型',
    providerSettings: '服务商设置',
    tabWorkbench: '生成工作台',
    tabUsage: '用量台账',
    template: '提示词模板',
    manageTemplate: '管理模板',
    variables: '可用变量（点击追加占位符）',
    prompt: '提示词（可手动编辑）',
    toggleNegative: '负向提示词',
    negative: '负向提示词（模型不支持时仅记录不发送）',
    size: '尺寸',
    n: '张数',
    quality: '质量',
    seed: 'Seed',
    seedPlaceholder: '留空=随机',
    generate: '生成图片',
    generating: '生成中…',
    empty: '填写提示词后点「生成图片」，结果会显示在这里；图片仅存于会话临时目录，手动保存才会落盘。',
    save: '保存',
    saved: '图片已保存',
    removeImageTitle: '移除',
    needModel: '请先选择模型',
    /** 生成前渲染发现「既无默认值、又无数据源」的占位符时抛出（P1 无数据源，出现即需用户手工替换）。 */
    missingPlaceholders: (keys: string[]) => `占位符未填充：${keys.join('、')}`,
    // 服务商设置
    keyLabel: 'API Key（仅本会话记忆，不落盘）',
    keySetHint: '已输入，如需更换请重新填写',
    baseUrl: '接口地址 Base URL',
    kind: '类型',
    providerIdLabel: 'ID',
    providerIdPlaceholder: '留空自动生成',
    providerIdFixedNote: 'ID 是服务商的唯一标识，创建后不可修改；如需换 ID 请新建服务商。',
    providerNameLabel: '名称',
    providerModelLabel: '默认模型',
    modelRowsNote: '模型明细（尺寸/价格/能力）请在 data/image_ai.json 中编辑。',
    addProvider: '新建服务商',
    providerSaved: '服务商已保存',
    deleteProviderConfirm: (name: string) => `删除服务商「${name}」？`,
    // 模板管理
    templateManageTitle: '提示词模板',
    templateNameLabel: '模板名称',
    templateBodyLabel: '模板正文（占位符写法 {{变量|默认值}}）',
    templateNegativeLabel: '负向提示词',
    addTemplate: '新建模板',
    deleteTemplateConfirm: (name: string) => `删除模板「${name}」？`,
    // 通用
    edit: '编辑',
    remove: '删除',
    back: '返回',
    done: '完成',
    // 用量台账
    filterAllProviders: '全部服务商',
    filterAllStatus: '全部状态',
    filterModelPlaceholder: '按模型筛选…',
    filterDatePlaceholder: '按日期筛选（YYYY-MM-DD）…',
    statusSuccess: '成功',
    statusFailed: '失败',
    colTs: '时间',
    colModel: '模型',
    colSize: '尺寸',
    colN: '张数',
    colSeed: 'Seed',
    colPrompt: '提示词 / 错误',
    colCost: '估算费用',
    colStatus: '状态',
    ledgerTotal: '本页估算合计',
    count: (total: number) => `共 ${total} 条`,
  },

  /** 「替换执行」面板 */
  replace: {
    cardTitle: '替换执行',
    modeLabel: '模式：',
    optionsLabel: '选项设置：',
    optAllowDuplicates: '允许复用图片',
    optFilterNewgan: '仅随机人',
    optSaveBackup: '写入前备份',
    /** 三个替换模式：value 发往后端（协议值，勿改）；label/desc 为界面显示文案（与后端日志用词一致） */
    modes: [
      { value: 'Preserve', label: '保留', desc: '保留已换过的球员，只为新球员补图（推荐日常增量）' },
      { value: 'Overwrite', label: '覆写', desc: '名单内球员全部重新抽图，名单外保留' },
      { value: 'Generate', label: '生成', desc: '无视现有 config.xml，从零生成全部映射' },
    ] as const,
    startBtn: '替换头像',
    runningBtn: '替换中', // 按钮上会追加实时百分比，如「替换中 45%」
    cancelBtn: '取消',
    browseDisabledHint: '请先在左侧「激活」存档后再执行替换（当前为浏览预览）',
    browseNotice: (name: string) =>
      `正在浏览「${name}」（未激活）。替换只作用于已激活的存档，请先在左侧点「激活」。`,
    needProfile: '选择 Profile',
    needGroup: '选择头像包',
    needRtf: '设置 RTF 名单',
    todoPrefix: '还需完成：',
    missingDirsTitle: '缺失目录',
    missingDirsConfirm: (standard: number, count: number) =>
      `标准${standard} 缺少 ${count} 个目录，自动创建并继续？`,
    unknownTitle: '无法识别的结构',
    unknownConfirm: '头像包结构无法识别，替换时将把包内所有图片（含子/孙目录）随机分配，仍要继续？',
    // 发起前预检（preflight_replace）：只有「生成」+ 本档档景非空时会走到这条确认。
    preflightTask: '检查既有映射',
    blankTitle: '将清空本档现有映射',
    blankConfirm: (existing: number) =>
      `「生成」模式不携带既有条目。若本次一条也没抽到（名单被筛空、UID 对不上、包里没有图），\n` +
      `会用空表覆盖本档现有的 ${existing} 条映射。\n\n确认仍要这样执行？`,
  },

  /** 结果区顶部 Tab */
  results: {
    tabTable: '名单预览',
    tabViewer: '单人替换',
    tabSnapshot: '快照预览',
  },

  /** 名单预览表（PlayersTable） */
  table: {
    title: '名单预览',
    recordCount: (total: number) => `${total} 条`,
    searchPlaceholder: '搜索 UID / 姓名 / 国籍…',
    prevPage: '‹ 上一页',
    nextPage: '下一页 ›',
    pageOf: (current: number, total: number) => `第 ${current} / ${total} 页`,
    /** 快速跳页：「前往 [输入框] 页 [跳转]」 */
    gotoPrefix: '前往',
    gotoSuffix: '页',
    gotoBtn: '跳转',
    gotoAria: '输入页码后回车快速跳转',
    colUid: 'UID',
    colNat: '国籍',
    col2ndNat: '第二国籍',
    colName: '姓名',
    colHair: '发长',
    colColor: '发色',
    colEth: '种族',
    colSkin: '肤色',
    colAvatar: '头像',
    /** 批量取头像失败时挂在「头像」列表头提示上（逐图取不到仍是静默留白，那本来就是正常情况）。 */
    avatarFailHint: (why: string) => `头像取数失败（${why}），本列留空；这不是「名单里没有脸」`,
    colNewgan: '随机人',
    colGender: '性别',
    colAge: '年龄',
    colHeight: '身高',
    colClub: '俱乐部',
    colBased: '所在地',
    colJob: '偏好工作',
    colCa: '能力',
    colPa: '潜力',
    colRca: 'R.C.A',
    /** 头像列缩略图 + 悬停/点击气泡的状态文案。 */
    avatarBtn: '预览',
    avatarAria: '预览该球员的头像图片',
    avatarLoading: '加载中…',
    avatarNotFound: '未找到头像图片',
    /** 随机人筛选（仅拓展名单显示）：标签 + 三个选项的文案。 */
    newganFilterLabel: '随机人',
    newganAny: '任意',
    newganYes: '是',
    newganNo: '否',
    emptyHint: '执行「替换头像」或「扫描RTF」后可在此预览名单（最近一次解析结果）。',
    /** 缓存里没有匹配当前筛选条件的记录（与快照预览的 noMatchHint 同款语义）。 */
    noMatchHint: '没有匹配的球员。',
  },

  /** 单人预览/微调（PlayerViewer） */
  viewer: {
    title: '球员预览',
    hint: '输入 UID 回车预览',
    /** UID 输入框 placeholder（示例教格式，全称说明在右侧 uidTip） */
    uidPlaceholder: '如 r-2000472008',
    searchBtn: '搜索',
    /** UID 输入框右侧的文本提示：回车等价点搜索 + r- 前缀可省略（短文案，保证头部单行放得下） */
    uidTip: '回车搜索 · 可省略 r- 前缀',
    colMapping: '当前映射',
    /** 拓展名单第 19 列起的用户自定义列（后端只保留原文，无表头名，按序号命名）。 */
    extraField: (n: number) => `自定义${n}`,
    noImage: '暂无图片',
    noMappingPlaceholder: '...\\...',
    imgPlaceholder: '输入新图片路径…',
    imgPathAria: '替换图片路径',
    browseBtn: '浏览',
    replaceBtn: '替换',
    imageDialogTitle: '选择图片文件',
    imageFilterName: '图片',
    missingInput: '请填写 UID 并选择有效图片路径',
    replacedOk: (uid: string) => `已替换 UID ${uid} 的头像`,
    mappingNotFound: (uid: string) => `config.xml 中未找到 UID ${uid} 的现有映射，未做修改`,
  },

  /** 快照预览（SnapshotPreview）：展示「读取快照」解析出的存档快照映射。分页器与名单预览共用（TablePager，文案取 t.table.*） */
  snapshot: {
    searchPlaceholder: '按 UID 筛选…',
    recordCount: (total: number) => `${total} 条`,
    colUid: 'UID',
    colPath: '头像相对路径',
    colImage: '头像图',
    noImage: '无图',
    // 未读取（或切档/切组后失效）时的引导文案
    readHint: '点击「当前任务」中的「读取快照」，解析存档 XML 快照后在此预览映射列表。',
    noMatchHint: '没有匹配的 UID。',
    emptyHint: '快照中没有映射记录。',
  },

  /** 底部状态栏 */
  status: {
    ready: '就绪',
    noGroup: '未选择头像包',
  },

  /** 日志抽屉 */
  log: {
    title: '运行日志',
    onlyLevel: '仅该级别',
    openFileBtn: '打开日志文件',
    clearBtn: '清空显示',
    closeAria: '关闭日志面板',
  },

  /** 标准规则编辑对话框 */
  ruleEditor: {
    std2Label: '标准二（14 种族目录）',
    std3Label: '标准三（细分目录）',
    saveBtn: '保存',
    closeBtn: '关闭',
    saveHint: '保存前会校验 JSON 格式，非法内容不会被写入',
    savedStatus: '已保存 ✓ 下次替换 / 检查完整性即生效',
    unsavedTitle: '未保存修改',
    unsavedSwitch: (label: string) => `${label} 有未保存的修改，确定放弃并切换？`,
    unsavedClose: (label: string) => `${label} 有未保存的修改，确定关闭？`,
    editorAria: '标准规则内容',
  },
} as const;
