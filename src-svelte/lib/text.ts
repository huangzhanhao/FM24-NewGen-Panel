// 【新手导读】全局文案常量模块：所有「显示给用户看的文字」统一在这里维护（默认中文）。
// 【用法】组件里 `import { t } from '../text'`，模板中直接 {t.xxx.yyy}；
//         需要拼变量的动态文案导出为函数，例如 t.sidebar.deleteProfileConfirm(name)。
// 【约定】参与逻辑判断的「协议值」不放这里：如缺省档名 'No Profile'、替换模式 'Preserve'
//         等发往后端的取值、日志级别 'INFO'、事件名、排序键——它们不是文案，改动会破坏逻辑。
// 【好处】改文案只改这一个文件；将来若要多语言，把本模块替换为按语言分发的实现即可。

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
    noProfileNoDelete: 'No Profile 为缺省档，无法删除',
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
    viewConfigBtn: '查看config',
    viewConfigHint: '打开头像包目录（config.xml 所在目录）',
    viewSnapshotBtn: '查看快照',
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
    runningBtn: '替换中…',
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
  },

  /** 结果区顶部 Tab */
  results: {
    tabTable: '名单预览',
    tabViewer: '单人微调',
  },

  /** 名单预览表（PlayersTable） */
  table: {
    title: '名单预览',
    recordCount: (total: number) => `${total} 条`,
    searchPlaceholder: '搜索 UID / 姓名 / 国籍…',
    prevPage: '‹ 上一页',
    nextPage: '下一页 ›',
    pageOf: (current: number, total: number) => `第 ${current} / ${total} 页`,
    colUid: 'UID',
    colNat: '国籍',
    col2ndNat: '第二国籍',
    colName: '姓名',
    colHair: '发长',
    colColor: '发色',
    colEth: '种族',
    colSkin: '肤色',
    colClub: '俱乐部',
    colAge: '年龄',
    colHeight: '身高',
    colNewgan: '随机人',
    emptyHint: '执行「替换头像」或「扫描RTF」后可在此预览名单（最近一次解析结果）。',
  },

  /** 单人预览/微调（PlayerViewer） */
  viewer: {
    title: '球员预览',
    hint: '输入 UID 回车预览',
    colMapping: '当前映射',
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
    labels: {
      uid: 'UID:',
      nat: '国籍:',
      nat2: '第二国籍:',
      name: '姓名:',
      hair: '发长:',
      color: '发色:',
      eth: '种族:',
      skin: '肤色:',
      newgan: '随机人:',
      club: '俱乐部:',
      age: '年龄:',
      height: '身高:',
    },
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
