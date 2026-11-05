import type { Lang } from './stores/ui.svelte';

/** 文档内容块：面板只做结构化渲染，内容全部来自这里，便于随版本统一维护 */
export type HelpBlock =
  | { kind: 'p'; text: string }
  | { kind: 'list'; items: string[]; ordered?: boolean }
  | { kind: 'note'; text: string; tone?: 'info' | 'warn' }
  | { kind: 'table'; head: string[]; rows: string[][] }
  | { kind: 'code'; text: string };

export type HelpEntry = {
  id: string;
  title: string;
  summary?: string;
  blocks: HelpBlock[];
};

export type HelpTab = 'tutorial' | 'faq';

type HelpContent = Record<HelpTab, HelpEntry[]>;

const zhCN: HelpContent = {
  tutorial: [
    {
      id: 'install',
      title: '安装与首次启动',
      summary: '下载、运行环境、数据位置与安全提示。',
      blocks: [
        { kind: 'p', text: '到 Releases 下载对应平台的安装包。Windows 使用 NSIS 安装到当前用户目录，无需管理员权限；应用基于 Tauri 2，不依赖 Python 或 .NET 运行时。' },
        {
          kind: 'list',
          items: [
            'Windows 10 1809+ / Windows 11 一般自带 WebView2；若缺失，安装程序会联网引导安装。',
            '应用未购买代码签名证书，SmartScreen 或杀毒软件可能提示「Windows 已保护你的电脑」：点「更多信息」→「仍要运行」，也可把安装目录加入杀软信任区。源码公开，可自行审计或构建。',
            'macOS / Linux 使用 CI 构建产物。',
          ],
        },
        { kind: 'note', text: '应用数据全部便携化：开发调试写在项目根 data\\，正式安装写在安装目录 data\\，不会写到 C 盘 AppData。' },
        { kind: 'p', text: '从 v1 升级时，首次启动会把 v1 的 .user 目录复制到 v2 数据目录（只复制、不移动，v1 安装不受影响）。启动固定停在缺省档 No Profile，不会自动恢复上次激活的存档。' },
      ],
    },
    {
      id: 'fm-prep',
      title: '准备 FM 视图与过滤器',
      summary: '把 views/ 与 filters/ 安装到 FM 用户目录，并导出正确列结构。',
      blocks: [
        { kind: 'p', text: '把仓库里的 views/、filters/ 两个文件夹复制到 FM 用户目录：' },
        { kind: 'code', text: 'Windows：文档\\Sports Interactive\\Football Manager 20XX\\\nmacOS：~/Library/Application Support/Sports Interactive/Football Manager 20XX/' },
        { kind: 'p', text: '在 FM 里使用 SCRIPT FACES 的 player search / squad / shortlist / staff 视图导出名单；配合 filters/ 里的 is newgen 过滤器可以只看随机人。' },
        { kind: 'note', text: '视图与过滤器决定 RTF 的列结构。请使用本仓库最新 views/ 导出；旧版 14 列详细名单、以及性别在第 3 列的旧职员布局已不再支持。' },
      ],
    },
    {
      id: 'export-rtf',
      title: '在 FM 中导出 RTF',
      summary: '三种列结构、表头识别与中英文数据判定。',
      blocks: [
        { kind: 'p', text: '把列表另存为 RTF。扫描以表头为识别依据：先看列数，再核对前 4 列「编号 / 国籍 / 第二国籍 / 姓名」，并抽查前 20 行数据。' },
        {
          kind: 'table',
          head: ['格式', '列数', '列序（从 1 起）'],
          rows: [
            ['基础名单（球员 / 职员通用）', '7', 'UID、国籍、二国籍、姓名、发长码、发色码、种族码'],
            ['拓展球员名单', '≥18', '前 7 列同上；第 8 肤色码、第 9 头像、第 10 Newgen、第 11 性别、第 12 年龄、第 13 身高、第 14 俱乐部、第 15 所在地、第 16 CA、第 17 PA、第 18 R.C.A；第 19 列起为用户自定义列'],
            ['拓展职员名单', '≥18', '与拓展球员相同，仅第 15 列为「偏好工作」'],
          ],
        },
        { kind: 'p', text: '数据语言不看表头，而是拿数据行的国籍值查翻译表，因此中文、英文导出的名单都能识别。基础名单全员按随机人处理（UID 加 r- 前缀）；拓展名单按第 10 列 Newgen（是 / 否、Yes / No）判定。' },
        { kind: 'note', tone: 'warn', text: '国籍查不到的行会整行丢弃并写入警告日志，逐行跳过日志最多 50 条。整份文件不合法时会弹出「RTF 格式不合法」，并定位到表头列或数据行字段。' },
      ],
    },
    {
      id: 'sidebar-scope',
      title: '侧栏：存档与头像包',
      summary: 'Profile 的浏览、激活、新建、删除，以及头像包组的关联与切换。',
      blocks: [
        { kind: 'p', text: '侧栏是作用域，决定后续操作作用于哪个存档、哪个头像包。' },
        {
          kind: 'list',
          items: [
            '启动固定停在 No Profile（未激活、不可删除）。上次激活的存档不会自动恢复。',
            '下拉选择任意存档 = 浏览预览：可查看该档的组配置、RTF 设置与预览结果，但关联头像包、切换组、设置 RTF、执行替换等改动型操作置灰，并提示先激活。',
            '点「激活」才真正切档：旧档各组 config.xml 存回快照，新档快照恢复到对应头像包目录。已激活时按钮变灰。',
            '点「存档资料」区的 ＋ 新建 Profile：输入名称创建，建好即自动激活；同名档不能重建覆写。',
            '「删除当前 Profile」删除下拉选中的存档及全部组配置与映射备份；删除激活档会自动回到 No Profile。',
            '点「头像包（组）」区的 ＋ 关联头像包根目录（其下应是 African、Asian 等种族子目录）；一个档可关联多组。点组名切换当前组，组条目显示 RTF 是否已设置；行内红色 × 删除该组。',
            '关联、切换、设置 RTF、替换等写操作都需要先激活存档。',
          ],
        },
        { kind: 'note', tone: 'warn', text: '队列运行中切档、切组、关联头像包、新建档，或删除档 / 组时，会弹一次确认并取消未完成的 AI 生成任务；队列空闲时的切换不打扰，删除仍会确认。' },
        { kind: 'p', text: '顶栏右端常驻显示「档 · 组」，始终是真实激活作用域，不随侧栏浏览态变化。' },
      ],
    },
    {
      id: 'archive-workbench',
      title: '存档资料台：备料、检查与快照',
      summary: '当前任务卡、检查完整性、扫描、规则配置与国籍对照表。',
      blocks: [
        { kind: 'p', text: '存档资料台用于先看清手里有什么料，所有写操作仍以侧栏激活为前提。' },
        {
          kind: 'list',
          items: [
            '当前任务卡固定显示「头像包 / RTF 名单 / config.xml」三行路径。长路径默认显示尾部，悬停可看完整路径；未关联头像包时路径框为空、按钮置灰并提示原因。',
            '检查完整性：按规则 json 的 FolderList 判定头像包属于标准一 / 标准二 / 标准三，或「无法识别」；缺失目录时可点「一键创建缺失目录」；多出的目录不会进入抽图池。切换组或切档后检查结果自动重置。',
            '扫描头像：遍历头像包并缓存图片索引，摘要写入日志，同时记录目录签名；替换时优先复用缓存。',
            '选择RTF：挑选 FM 导出的 RTF；扫描RTF：立即解析并填充名单预览。浏览态下「选择RTF」置灰。',
            'config.xml 行：读取快照解析当前档的存档 XML 快照并在「快照预览」中展示映射；查看位置打开快照所在目录。两者都依赖快照文件已生成（执行替换后才有）。',
            '规则配置：结构化编辑标准规则，左侧分区为概览 / 目录清单 / 国籍映射 / 种族码修正 / 规则块 / 全局设置，也可切到「源码」直接改 JSON。保存前做结构校验，非法内容不会写盘。',
            '国籍对照表：维护三字码与各语言国籍全称、别名，支持新增语言；保存后下一次扫描 RTF、替换或提示词预览即生效。',
          ],
        },
        { kind: 'note', text: '检查、扫描类操作不受浏览态限制；设置 RTF、关联头像包、替换等写操作需要先激活。' },
      ],
    },
    {
      id: 'replace',
      title: '批量替换',
      summary: '三种模式、选项、两道确认与结果查看。',
      blocks: [
        { kind: 'p', text: '在批量 / 单人头像替换台左栏选择模式与选项，点「替换头像」开始。执行前需要已激活档、已选头像包、已设置 RTF（顶栏 ② 状态点变绿）。' },
        {
          kind: 'table',
          head: ['模式', '含义'],
          rows: [
            ['保留 Preserve', '保留已换过的球员，只为新球员补图；日常增量推荐'],
            ['覆写 Overwrite', '名单内球员全部重新抽图，名单外保留'],
            ['生成 Generate', '无视现有 config.xml，从零生成全部映射'],
          ],
        },
        { kind: 'p', text: '选项设置：允许复用图片（关闭时保留模式会先剔除 config.xml 已用图片）、仅随机人、写入前备份（与单人替换共用同一开关，默认开启）。' },
        { kind: 'p', text: '发起前有两道确认：' },
        {
          kind: 'list',
          ordered: true,
          items: [
            '完整性兜底校验：缺目录或结构无法识别时询问一次；完整时静默继续。',
            '既有映射预检：只有「生成」模式且本档档景非空时才问「将清空本档现有 N 条映射」。没确认的这一轮，写前拦截会中止，config.xml 与档景都不动。',
          ],
        },
        { kind: 'p', text: '执行中按钮内显示实时进度百分比，可点「取消」。完成后弹出「替换完成！」。头像包索引命中缓存时直接进入映射阶段，日志会写明「命中（复用，不扫盘）」。' },
        { kind: 'p', text: '结果区提供两个 Tab：' },
        {
          kind: 'list',
          items: [
            '名单预览：最近一次解析结果，支持排序、搜索、分页与快速跳页；拓展名单可切换「拓展列」、按「随机人」筛选、勾选当前页球员「加入队列」、预览头像。存档资料台显示完整列集，替换台右栏默认精简列集。',
            '快照预览：展示「读取快照」解析出的映射列表，可按 UID 筛选，显示头像相对路径与缩略图。',
          ],
        },
        { kind: 'note', text: '状态栏有两条独立进度通道：替换流水线 replace:progress 优先；检查完整性 / 扫描头像 / 扫描RTF / 读取快照走 task:progress。任务完成后 100% 会在状态栏停留约 20 秒再清空。' },
      ],
    },
    {
      id: 'single',
      title: '单人微调',
      summary: '搜索球员、指定替换、重掷替换与送生图。',
      blocks: [
        { kind: 'p', text: '单人替换卡常驻替换台左栏，与批量替换同为写操作。' },
        {
          kind: 'list',
          items: [
            '输入 UID 或关键词后回车搜索；命中多个时用「上一个 / 下一个」切换。',
            '卡片显示当前映射、球员信息与头像预览；字段较多时自动换行。',
            '点「浏览」选一张新图，路径需形如 ...\\<种族目录>\\<图片名>.png，再点「指定替换」，只改写这一条映射。',
            '「重掷替换」按标准规则给该球员重新抽图并写回。',
            '「送生图」把当前球员设为 AI 生成头像的数据源，并切到 AI 工作台。',
          ],
        },
        { kind: 'note', text: '单人替换与批量替换共用「写入前备份」开关。' },
      ],
    },
    {
      id: 'ai',
      title: 'AI 生成头像',
      summary: '服务商、数据源、提示词、参数、结果画廊与批量队列。',
      blocks: [
        { kind: 'p', text: 'AI 生成头像是可选工作台，需要自备 API Key（本地 ComfyUI 免 Key）。它不依赖名单里的种族目录，适合给单人或批量球员生成个性化头像。' },
        { kind: 'p', text: '服务商设置：内置 OpenAI 及兼容协议（如商汤 SenseNova）、阿里云百炼（千问 / 万相 / Z-Image）、火山方舟 Seedream、硅基流动、魔搭 ModelScope、本地 ComfyUI。点顶部状态灯或「服务商设置」可编辑 Key、Base URL、模型清单与模型能力参数。Key 由本机加密保存，仅当前系统账户可解；换设备或重装系统后需要重新填写。' },
        { kind: 'p', text: '数据源（球员档案卡）有三处入口：' },
        {
          kind: 'list',
          items: ['名单预览行尾「送生图」', '单人替换卡「送生图」', '档案卡内输入 UID 查询'],
        },
        { kind: 'p', text: '提示词模板：正文使用 {{变量|默认值}} 占位符，变量由球员档案自动填充。「管理模板」可新建 / 编辑 / 删除模板（内置 default 模板不可删除），可用变量胶囊点击插入光标处。负向提示词可选，模型不支持时仅记录不发送。' },
        { kind: 'p', text: '参数区按当前模型能力显隐：尺寸 / 比例、张数、质量、Seed、采样步数、引导系数、智能改写、思考模式、水印、调用方式、输出格式。尺寸不合规或提示词超出模型上限时「生成图片」会禁用并给出原因；Ctrl+Enter 与点按按钮走同一链路。' },
        { kind: 'p', text: '结果画廊同时显示本次会话结果与持久画廊记录。可灯箱查看大图、按原图参数「再生成」、另存为文件，或「保存并替换」。' },
        {
          kind: 'list',
          items: [
            '保存并替换会把图片写入 <头像包>/_generated/<池>/，落点可在弹窗中修改；随后走单人换脸链路写入当前档映射。',
            '如果图片已落盘但当前档 config.xml 缺失或结构异常，会警告「没能写入映射」，不会冒充成功。',
            '生成图只存于会话临时目录 .imgtmp\\，每次启动清理；不手动保存或保存并替换就会丢失。gallery.json 跨会话保留记录，但图片被清理后只显示占位。',
          ],
        },
        { kind: 'p', text: '批量队列：在名单预览勾选当前页球员后点「加入队列（N）」；AI 台底部队列坞常驻，顶栏 ③ 显示排队中 + 生成中数量角标。' },
        {
          kind: 'list',
          items: [
            '确认方式分手动确认 / 全自动。手动模式需逐条点队列行核对参数并点「确认」，未全部确认不能启动；全自动直接启动。',
            '启动前会确认任务数与费用；可只跑勾选条目，其余留在队列。',
            '运行中可取消：未提交的立即停止，已提交的等本次请求返回并保留结果（会产生费用）。',
            '队列条目绑定入队时的「档 · 组」；队列运行中切档 / 切组 / 关联头像包 / 新建档会弹确认并取消未完成任务，空闲时的删除也会确认。',
            '队列是会话内存态，重开应用即空。',
          ],
        },
      ],
    },
    {
      id: 'fm-reload',
      title: '回 FM 生效',
      summary: '重新加载皮肤缓存。',
      blocks: [
        { kind: 'p', text: '替换完成后回到 FM：偏好设置 → 界面，取消勾选再重新勾选皮肤，并重新加载皮肤缓存（Reload skin cache）。如果头像仍未变化，清理 FM 图形缓存后重启游戏。' },
      ],
    },
    {
      id: 'logs-data',
      title: '日志、数据与备份',
      summary: '日志抽屉、备份策略、数据目录与升级迁移。',
      blocks: [
        { kind: 'p', text: '底部状态栏常驻当前状态、迷你进度与最新一行日志；「日志」按钮出现红色角标表示有错误。点开日志抽屉可：' },
        {
          kind: 'list',
          items: ['按级别筛选，或勾选「仅该级别」', '「打开日志文件」用系统默认程序打开 newgan.log', '「清空显示」只清当前显示，不回填'],
        },
        { kind: 'p', text: '日志文件按 10 MB × 3 份滚动，备份命名为 newgan_日期_时间.log。' },
        { kind: 'p', text: '写入 config.xml 前，如果「写入前备份」开启，会生成 config_backup_YYYYMMDD-HHMMSS.xml，每个头像包目录最多保留最近 5 份；覆盖档景快照前也会生成 <快照词干>_backup_...xml，每份快照最多保留最近 5 份。早期版本的 config备份_ 文件不会被计入或自动清理。' },
        { kind: 'p', text: '正式安装后，运行数据都在安装目录 data\\：' },
        {
          kind: 'code',
          text: '.user\\                  存档、组配置与映射快照\nimage_ai.json           AI 服务商与生图设置\nnationalities.json      国籍对照表用户副本\nprompts\\templates.json  提示词模板用户副本\nrules\\standard2.json    标准二规则用户副本\nrules\\standard3.json    标准三规则用户副本\n.imgtmp\\                AI 会话临时图片（启动清理）\n.imgtmp\\gallery.json    画廊记录（跨会话保留）\nwebview\\                WebView2 数据\nnewgan.log              运行日志',
        },
        { kind: 'note', text: '只读映射表与标准规则出厂默认编译期嵌入二进制；首次启动会导出用户副本，删除用户副本后重启会重新导出。' },
      ],
    },
    {
      id: 'cache',
      title: '性能与索引缓存',
      summary: '为什么替换很快、何时需要强制重扫。',
      blocks: [
        { kind: 'p', text: '替换流水线分为解析 RTF → 扫盘建索引 → 抽图映射 → 写 config.xml → 同步快照。扫盘是大头，因此应用会把图片索引按「头像包根目录 + 目录签名」缓存。签名由各一级目录的名字、mtime 与直接子条目数组成。' },
        {
          kind: 'list',
          items: [
            '扫过一次且内容没变，之后每次替换直接复用缓存，日志显示「命中（复用，不扫盘）」。',
            '往头像包里加图却没被用上时，通常是因为签名没变。在对应一级目录里增删任意一张图片即可让签名变化，或重启应用。',
            '深层子目录里的改名不会被签名察觉；需要时同样在一级目录增删图片触发重扫。',
            '目录判定兼容符号链接与 Windows junction；链接目标不可达时该子树不会进入抽图池。',
          ],
        },
      ],
    },
  ],
  faq: [
    {
      id: 'faq-formats',
      title: '支持哪些 RTF 格式？旧版名单还能用吗？',
      blocks: [
        { kind: 'p', text: '支持 3 种列结构 × 2 种数据语言，共 6 种组合：基础名单（7 列）、拓展球员名单（≥18 列，第 15 列为所在地）、拓展职员名单（≥18 列，第 15 列为偏好工作）；每种都有中英文数据。旧版 14 列详细名单、性别在第 3 列的旧职员布局已不再支持，请用本仓库最新 views/ 重新导出。' },
      ],
    },
    {
      id: 'faq-invalid',
      title: '提示「RTF 格式不合法」怎么办？',
      blocks: [
        { kind: 'p', text: '说明表头或数据行不符合上述格式，提示会定位到「表头第 N 列」或「第 N 行「字段」」。检查是否用了旧视图 / 旧模板导出、列被手动增删、表头与数据行错位；用最新 views/ 重新导出即可。' },
      ],
    },
    {
      id: 'faq-dropped',
      title: '为什么有些球员整行被丢弃，或者没有生成映射？',
      blocks: [
        { kind: 'p', text: '两种情况：① 国籍显示文本不在国籍对照表里，整行丢弃并记警告日志；可点「国籍对照表」补一条映射或别名，保存后下次扫描 / 替换生效。② 国籍三字码不在标准规则的 Ethnics 映射里，球员被跳过；可在「规则配置」的国籍映射里补该码。' },
      ],
    },
    {
      id: 'faq-none',
      title: '名单里一个人都没换？',
      blocks: [
        { kind: 'p', text: '若勾了「仅随机人」，基础名单全员按随机人处理，但拓展名单要看第 10 列 Newgen；如果全为「否 / No」，过滤后就没有球员。关掉开关，或改用 is newgen 过滤视图重新导出。也可能是名单被筛空、UID 对不上，或头像包里没有图，导致本轮 0 条映射。' },
      ],
    },
    {
      id: 'faq-fast',
      title: '为什么替换秒过，日志显示「命中（复用，不扫盘）」？',
      blocks: [
        { kind: 'p', text: '这是正常行为：头像包索引命中了目录签名缓存，省掉了整包重扫。想强制重扫，在头像包的一级目录里增删任意一张图片，或重启应用。' },
      ],
    },
    {
      id: 'faq-add-image',
      title: '明明往头像包里加了图，替换却没用上？',
      blocks: [
        { kind: 'p', text: '索引缓存还在用旧签名。签名只覆盖一级目录的名字、mtime 与直接子条目数，深层子目录改名不会被察觉。在对应一级目录里增删任意图片即可，或重启应用。' },
      ],
    },
    {
      id: 'faq-profile',
      title: 'Profile 和「头像包（组）」是什么？为什么按钮是灰的？',
      blocks: [
        { kind: 'p', text: '一个 Profile = 一个存档的一套配置；一个 Profile 可以关联多个头像包目录（组），每组独立记录 RTF 与映射快照。下拉选择存档只是浏览，必须点「激活」才能关联头像包、切换组、设置 RTF、执行替换；浏览态下这些按钮会置灰并提示先激活。' },
      ],
    },
    {
      id: 'faq-noprofile',
      title: '启动为什么不是我上次的存档？No Profile 是什么？',
      blocks: [
        { kind: 'p', text: '启动固定从缺省档 No Profile 开始，且不自动恢复上次激活的档。No Profile 未激活、不可删除，用于先看清手里有什么料。选择其他存档并点「激活」才会切换。' },
      ],
    },
    {
      id: 'faq-modes',
      title: '保留、覆写、生成三种模式怎么选？',
      blocks: [
        {
          kind: 'table',
          head: ['模式', '适用场景'],
          rows: [
            ['保留', '保留已换过的球员，只补新球员；日常增量推荐'],
            ['覆写', '名单内全部重抽，名单外保留'],
            ['生成', '无视现有 config.xml，从零生成'],
          ],
        },
        { kind: 'p', text: '注意：生成模式若本轮一条也没抽到，会用空表覆盖本档现有映射；本档档景非空时发起前会先确认。想清空 config.xml，就走「生成」并确认。' },
      ],
    },
    {
      id: 'faq-backup',
      title: '备份在哪里？保留几份？可以关闭吗？',
      blocks: [
        { kind: 'p', text: 'config.xml 备份名为 config_backup_YYYYMMDD-HHMMSS.xml，放在对应头像包目录，每个目录最多保留最近 5 份；档景快照备份为 <快照词干>_backup_...xml，每份最多 5 份。替换台「选项设置」里的「写入前备份」可关闭，默认开启，与单人替换共用。早期 config备份_ 历史文件不会自动清理。' },
      ],
    },
    {
      id: 'faq-integrity',
      title: '「检查完整性」提示缺目录或无法识别怎么办？',
      blocks: [
        { kind: 'p', text: '缺目录：点「一键创建缺失目录」，或确认选的是头像包根目录。无法识别：子目录与任何标准的 FolderList 都不构成包含关系；替换时会把包内全部图片递归全池随机。可在「规则配置」把包内目录名补进清单，恢复按国籍 / 种族精确路由。' },
      ],
    },
    {
      id: 'faq-standard',
      title: '头像包需要哪些目录？标准一 / 二 / 三有什么区别？',
      blocks: [
        { kind: 'p', text: '标准一：图片直铺在根目录，无子目录，完全随机。标准二：14 个种族 / 大区目录，国籍查表后按种族码修正再抽图。标准三：国籍级 + 族群级的细分目录（具体条目以 standard3.json 的 FolderList 为准），按国家块 → 族群块 → _DEFAULT_ 规则块匹配。判定只看目录名：包含某标准 FolderList 的全部条目即归该标准。' },
      ],
    },
    {
      id: 'faq-oldpack',
      title: '目录名不在清单里的老头像包还能用吗？',
      blocks: [
        { kind: 'p', text: '能用。判定为「无法识别」时，替换会递归全池随机，仍能出图，但不按国籍与种族码路由。想恢复精确路由：在标准三规则的 FolderList 里补上包内目录名，或点「一键创建缺失目录」把清单补齐。' },
      ],
    },
    {
      id: 'faq-rules',
      title: '「规则配置」保存失败 / 「标准三规则无效」怎么办？',
      blocks: [
        { kind: 'p', text: '保存与加载都会做结构校验：版本必须为 4；规则块键必须是国籍码或清单内目录名；每个块必须 0–10 全枚举；规则指向的目录必须在 FolderList 内；加权两列必须等长且全为正数。非法内容不会写盘。按提示修正，或删掉 data\\rules\\standardN.json 让程序重新导出出厂默认；加载失败会报错并指明路径，不会静默回退。' },
      ],
    },
    {
      id: 'faq-nationality',
      title: '中文国籍能被识别吗？国籍对照表怎么改？',
      blocks: [
        { kind: 'p', text: '能。RTF 的国籍（中文全称、英文全称或三字码）先经国籍对照表归一化为三字码，再进入抽图与提示词。点「国籍对照表」可增删改码、各语言全称 / 别名和语言列表；同一码在同一语言下可以有多条（全称与别名），解析都算命中。保存后立即生效，也可直接编辑 data\\nationalities.json。' },
      ],
    },
    {
      id: 'faq-fm-no-avatar',
      title: '替换成功，FM 里头像没变？',
      blocks: [
        { kind: 'p', text: '先重新加载皮肤缓存：偏好设置 → 界面，取消再勾选皮肤并 Reload skin cache；必要时清 FM 图形缓存并重启。再确认 FM 图形目录里没有旧的 config.xml 副本，且头像包路径与 FM 配置一致。' },
      ],
    },
    {
      id: 'faq-permission',
      title: '写 config.xml 失败 / 权限错误？',
      blocks: [
        { kind: 'p', text: '以管理员（或 sudo）运行，或把头像包移到当前用户可写目录。注意不要放在只读盘、同步盘锁定的目录里。' },
      ],
    },
    {
      id: 'faq-ai-key',
      title: 'AI 生图需要什么？Key 存在哪里？',
      blocks: [
        { kind: 'p', text: '需要自备服务商 API Key（本地 ComfyUI 免 Key）。Key 只在本机加密保存，仅当前系统账户可解，不会随图片或配置明文导出；换设备或重装系统后需重新填写。点 AI 台顶部状态灯进入「服务商设置」填写 Key、Base URL 和模型。' },
      ],
    },
    {
      id: 'faq-comfy',
      title: '本地 ComfyUI 怎么用？需要 Key 吗？',
      blocks: [
        { kind: 'p', text: '不需要 Key。默认基址指向本机 ComfyUI，在「服务商设置」里改 Base URL 即可连自建 / 远程实例。参数对应工作流节点输入：提示词、尺寸、张数、负向词、Seed、步数、引导系数分别落到 CLIPTextEncode、EmptyLatentImage、KSampler 等节点。' },
      ],
    },
    {
      id: 'faq-ai-images',
      title: 'AI 生成的图存哪里？会丢吗？「保存并替换」失败？',
      blocks: [
        { kind: 'p', text: '生成图只存于 data\\.imgtmp\\，每次启动清理；不手动「另存」或「保存并替换」就会丢失。gallery.json 跨会话保留记录，但图片被清理后只显示占位。保存并替换会写入 <头像包>\\_generated\\<池>\\；若提示「没能写入映射」，说明图片已落盘，但当前档 config.xml 缺失或结构异常，需要检查头像包后重试（也可改用「指定替换」手动补）。' },
      ],
    },
    {
      id: 'faq-queue-scope',
      title: '队列运行中切存档或切组会怎样？',
      blocks: [
        { kind: 'p', text: '队列条目绑定入队时的「档 · 组」。队列运行中切档、切组、关联头像包、新建档，会弹一次确认，确认后取消未完成的生成任务并执行切换；取消则留在原作用域。删除档 / 组无论队列是否空闲都会确认（删除不可逆），运行中会把队列警告合并进同一个确认框。队列是会话内存态，重开应用即空。' },
      ],
    },
    {
      id: 'faq-ai-queue',
      title: '批量生成队列怎么用？',
      blocks: [
        { kind: 'p', text: '在名单预览勾选当前页球员 → 点「加入队列（N）」；AI 台底部队列坞常驻，顶栏 ③ 显示数量角标。确认方式分手动确认 / 全自动：手动模式需逐条核对参数并点「确认」，未全部确认不能启动；全自动直接启动。可只跑勾选条目，其余留在队列；运行中可取消（未提交立即停止，已提交等返回并计费）；失败条目可重试，错误会按类别给出处置建议。' },
      ],
    },
    {
      id: 'faq-log',
      title: '出问题怎么反馈？日志在哪？',
      blocks: [
        { kind: 'p', text: '点状态栏「日志」打开抽屉，把级别选到 DEBUG 复现一次，点「打开日志文件」直接打开 newgan.log（正式安装位于 <安装目录>\\data\\newgan.log）。反馈问题时请附日志与复现步骤。' },
      ],
    },
    {
      id: 'faq-smartscreen',
      title: '安装时 SmartScreen / 杀毒报毒怎么办？',
      blocks: [
        { kind: 'p', text: '应用未购买代码签名证书，SmartScreen 属于未知发布者提示；点「更多信息」→「仍要运行」。部分杀毒软件可能对未签名程序启发式报毒，可把安装目录加入信任区；源码完全公开，可自行审计或自行构建。' },
      ],
    },
    {
      id: 'faq-data',
      title: '数据能迁移吗？v1 怎么升级？',
      blocks: [
        { kind: 'p', text: 'v2 首次启动自动迁移：按顺序查找 v1 便携模式 <v2安装目录>\\data\\.user → v1 标准模式 %LOCALAPPDATA%\\NewGAN Manager\\.user，找到即整目录复制到 v2 的 data\\.user，Profile、组、RTF 设置、映射快照全部保留。迁移是复制不是移动，v1 不受影响、可回退。config.xml 生成规则逐字节兼容。' },
      ],
    },
  ],
};

const zhTW: HelpContent = {
  tutorial: [
    {
      id: 'install',
      title: '安裝與首次啟動',
      summary: '下載、執行環境、資料位置與安全提示。',
      blocks: [
        { kind: 'p', text: '到 Releases 下載對應平台的安裝包。Windows 使用 NSIS 安裝到目前使用者目錄，無需管理員權限；應用基於 Tauri 2，不依賴 Python 或 .NET 執行環境。' },
        {
          kind: 'list',
          items: [
            'Windows 10 1809+ / Windows 11 通常自帶 WebView2；若缺少，安裝程式會連網引導安裝。',
            '應用未購買程式碼簽章憑證，SmartScreen 或防毒軟體可能提示「Windows 已保護你的電腦」：點「更多資訊」→「仍要執行」，也可把安裝目錄加入防毒信任區。原始碼公開，可自行稽核或建置。',
            'macOS / Linux 使用 CI 建置產物。',
          ],
        },
        { kind: 'note', text: '應用資料全部可攜化：開發除錯寫在專案根 data\\，正式安裝寫在安裝目錄 data\\，不會寫到 C 槽 AppData。' },
        { kind: 'p', text: '從 v1 升級時，首次啟動會把 v1 的 .user 目錄複製到 v2 資料目錄（只複製、不移動，v1 安裝不受影響）。啟動固定停在缺省檔 No Profile，不會自動恢復上次啟用的存檔。' },
      ],
    },
    {
      id: 'fm-prep',
      title: '準備 FM 檢視與過濾器',
      summary: '把 views/ 與 filters/ 安裝到 FM 使用者目錄，並匯出正確欄位結構。',
      blocks: [
        { kind: 'p', text: '把儲存庫裡的 views/、filters/ 兩個資料夾複製到 FM 使用者目錄：' },
        { kind: 'code', text: 'Windows：文件\\Sports Interactive\\Football Manager 20XX\\\nmacOS：~/Library/Application Support/Sports Interactive/Football Manager 20XX/' },
        { kind: 'p', text: '在 FM 裡使用 SCRIPT FACES 的 player search / squad / shortlist / staff 檢視匯出名單；搭配 filters/ 裡的 is newgen 過濾器可以只看隨機人。' },
        { kind: 'note', text: '檢視與過濾器決定 RTF 的欄位結構。請使用本儲存庫最新 views/ 匯出；舊版 14 欄詳細名單、以及性別在第 3 欄的舊職員版面已不再支援。' },
      ],
    },
    {
      id: 'export-rtf',
      title: '在 FM 中匯出 RTF',
      summary: '三種欄位結構、表頭辨識與中英文資料判定。',
      blocks: [
        { kind: 'p', text: '把列表另存為 RTF。掃描以表頭為辨識依據：先看欄數，再核對前 4 欄「編號 / 國籍 / 第二國籍 / 姓名」，並抽查前 20 列資料。' },
        {
          kind: 'table',
          head: ['格式', '欄數', '欄序（從 1 起）'],
          rows: [
            ['基礎名單（球員 / 職員通用）', '7', 'UID、國籍、第二國籍、姓名、髮長碼、髮色碼、種族碼'],
            ['拓展球員名單', '≥18', '前 7 欄同上；第 8 膚色碼、第 9 頭像、第 10 Newgen、第 11 性別、第 12 年齡、第 13 身高、第 14 俱樂部、第 15 所在地、第 16 CA、第 17 PA、第 18 R.C.A；第 19 欄起為使用者自訂欄'],
            ['拓展職員名單', '≥18', '與拓展球員相同，僅第 15 欄為「偏好工作」'],
          ],
        },
        { kind: 'p', text: '資料語言不看表頭，而是拿資料列的國籍值查翻譯表，因此中文、英文匯出的名單都能辨識。基礎名單全員按隨機人處理（UID 加 r- 前綴）；拓展名單按第 10 欄 Newgen（是 / 否、Yes / No）判定。' },
        { kind: 'note', tone: 'warn', text: '國籍查不到的列會整列丟棄並寫入警告日誌，逐列跳過日誌最多 50 筆。整份檔案不合法時會彈出「RTF 格式不合法」，並定位到表頭欄或資料列欄位。' },
      ],
    },
    {
      id: 'sidebar-scope',
      title: '側欄：存檔與頭像包',
      summary: 'Profile 的瀏覽、啟用、新建、刪除，以及頭像包組的關聯與切換。',
      blocks: [
        { kind: 'p', text: '側欄是作用域，決定後續操作作用於哪個存檔、哪個頭像包。' },
        {
          kind: 'list',
          items: [
            '啟動固定停在 No Profile（未啟用、不可刪除）。上次啟用的存檔不會自動恢復。',
            '下拉選擇任意存檔 = 瀏覽預覽：可查看該檔的組設定、RTF 設定與預覽結果，但關聯頭像包、切換組、設定 RTF、執行替換等變更型操作會變灰，並提示先啟用。',
            '點「啟用」才真正切檔：舊檔各組 config.xml 存回快照，新檔快照恢復到對應頭像包目錄。已啟用時按鈕變灰。',
            '點「存檔資料」區的 ＋ 新建 Profile：輸入名稱建立，建好即自動啟用；同名檔不能重建覆寫。',
            '「刪除目前 Profile」刪除下拉選中的存檔及全部組設定與對應備份；刪除啟用檔會自動回到 No Profile。',
            '點「頭像包（組）」區的 ＋ 關聯頭像包根目錄（其下應是 African、Asian 等種族子目錄）；一個檔可關聯多組。點組名切換目前組，組條目顯示 RTF 是否已設定；列內紅色 × 刪除該組。',
            '關聯、切換、設定 RTF、替換等寫入操作都需要先啟用存檔。',
          ],
        },
        { kind: 'note', tone: 'warn', text: '佇列執行中切檔、切組、關聯頭像包、新建檔，或刪除檔 / 組時，會彈一次確認並取消未完成的 AI 生成任務；佇列閒置時的切換不打擾，刪除仍會確認。' },
        { kind: 'p', text: '頂欄右端常駐顯示「檔 · 組」，始終是真實啟用作用域，不隨側欄瀏覽狀態變化。' },
      ],
    },
    {
      id: 'archive-workbench',
      title: '存檔資料台：備料、檢查與快照',
      summary: '目前任務卡、檢查完整性、掃描、規則設定與國籍對照表。',
      blocks: [
        { kind: 'p', text: '存檔資料台用於先看清手上有什麼料，所有寫入操作仍以側欄啟用為前提。' },
        {
          kind: 'list',
          items: [
            '目前任務卡固定顯示「頭像包 / RTF 名單 / config.xml」三列路徑。長路徑預設顯示尾端，懸停可看完整路徑；未關聯頭像包時路徑框為空、按鈕變灰並提示原因。',
            '檢查完整性：按規則 json 的 FolderList 判定頭像包屬於標準一 / 標準二 / 標準三，或「無法辨識」；缺少目錄時可點「一鍵建立缺少目錄」；多出的目錄不會進入抽圖池。切換組或切檔後檢查結果自動重置。',
            '掃描頭像：走訪頭像包並快取圖片索引，摘要寫入日誌，同時記錄目錄簽章；替換時優先重用快取。',
            '選擇RTF：挑選 FM 匯出的 RTF；掃描RTF：立即解析並填充名單預覽。瀏覽狀態下「選擇RTF」變灰。',
            'config.xml 列：讀取快照解析目前檔的存檔 XML 快照並在「快照預覽」中展示對應；查看位置開啟快照所在目錄。兩者都依賴快照檔案已生成（執行替換後才有）。',
            '規則設定：結構化編輯標準規則，左側分區為概覽 / 目錄清單 / 國籍對應 / 種族碼修正 / 規則區塊 / 全域設定，也可切到「原始碼」直接改 JSON。儲存前做結構驗證，非法內容不會寫入磁碟。',
            '國籍對照表：維護三字碼與各語言國籍全稱、別名，支援新增語言；儲存後下一次掃描 RTF、替換或提示詞預覽即生效。',
          ],
        },
        { kind: 'note', text: '檢查、掃描類操作不受瀏覽狀態限制；設定 RTF、關聯頭像包、替換等寫入操作需要先啟用。' },
      ],
    },
    {
      id: 'replace',
      title: '批次替換',
      summary: '三種模式、選項、兩道確認與結果查看。',
      blocks: [
        { kind: 'p', text: '在批次 / 單人頭像替換台左欄選擇模式與選項，點「替換頭像」開始。執行前需要已啟用檔、已選頭像包、已設定 RTF（頂欄 ② 狀態點變綠）。' },
        {
          kind: 'table',
          head: ['模式', '含義'],
          rows: [
            ['保留 Preserve', '保留已換過的球員，只為新球員補圖；日常增量推薦'],
            ['覆寫 Overwrite', '名單內球員全部重新抽圖，名單外保留'],
            ['生成 Generate', '無視現有 config.xml，從零生成全部對應'],
          ],
        },
        { kind: 'p', text: '選項設定：允許重用圖片（關閉時保留模式會先剔除 config.xml 已用圖片）、僅隨機人、寫入前備份（與單人替換共用同一開關，預設開啟）。' },
        { kind: 'p', text: '發起前有兩道確認：' },
        {
          kind: 'list',
          ordered: true,
          items: [
            '完整性保險驗證：缺目錄或結構無法辨識時詢問一次；完整時靜默繼續。',
            '既有對應預檢：只有「生成」模式且本檔檔景非空時才問「將清空本檔現有 N 筆對應」。沒確認的這一輪，寫入前攔截會中止，config.xml 與檔景都不動。',
          ],
        },
        { kind: 'p', text: '執行中按鈕內顯示即時進度百分比，可點「取消」。完成後彈出「替換完成！」。頭像包索引命中快取時直接進入對應階段，日誌會寫明「命中（重用，不掃盤）」。' },
        { kind: 'p', text: '結果區提供兩個 Tab：' },
        {
          kind: 'list',
          items: [
            '名單預覽：最近一次解析結果，支援排序、搜尋、分頁與快速跳頁；拓展名單可切換「拓展欄」、按「隨機人」篩選、勾選目前頁球員「加入佇列」、預覽頭像。存檔資料台顯示完整欄集，替換台右欄預設精簡欄集。',
            '快照預覽：展示「讀取快照」解析出的對應清單，可按 UID 篩選，顯示頭像相對路徑與縮圖。',
          ],
        },
        { kind: 'note', text: '狀態列有兩條獨立進度通道：替換流程 replace:progress 優先；檢查完整性 / 掃描頭像 / 掃描RTF / 讀取快照走 task:progress。任務完成後 100% 會在狀態列停留約 20 秒再清空。' },
      ],
    },
    {
      id: 'single',
      title: '單人微調',
      summary: '搜尋球員、指定替換、重擲替換與送生圖。',
      blocks: [
        { kind: 'p', text: '單人替換卡常駐替換台左欄，與批次替換同為寫入操作。' },
        {
          kind: 'list',
          items: [
            '輸入 UID 或關鍵字後按 Enter 搜尋；命中多個時用「上一個 / 下一個」切換。',
            '卡片顯示目前對應、球員資訊與頭像預覽；欄位較多時自動換行。',
            '點「瀏覽」選一張新圖，路徑需形如 ...\\<種族目錄>\\<圖片名>.png，再點「指定替換」，只改寫這一筆對應。',
            '「重擲替換」按標準規則給該球員重新抽圖並寫回。',
            '「送生圖」把目前球員設為 AI 生成頭像的資料來源，並切到 AI 工作台。',
          ],
        },
        { kind: 'note', text: '單人替換與批次替換共用「寫入前備份」開關。' },
      ],
    },
    {
      id: 'ai',
      title: 'AI 生成頭像',
      summary: '服務商、資料來源、提示詞、參數、結果畫廊與批次佇列。',
      blocks: [
        { kind: 'p', text: 'AI 生成頭像是可選工作台，需要自備 API Key（本機 ComfyUI 免 Key）。它不依賴名單裡的種族目錄，適合給單人或批次球員生成個人化頭像。' },
        { kind: 'p', text: '服務商設定：內建 OpenAI 及相容協定（如商湯 SenseNova）、阿里雲百煉（千問 / 萬相 / Z-Image）、火山方舟 Seedream、矽基流動、魔搭 ModelScope、本機 ComfyUI。點頂部狀態燈或「服務商設定」可編輯 Key、Base URL、模型清單與模型能力參數。Key 由本機加密儲存，僅目前系統帳戶可解；換裝置或重裝系統後需要重新填寫。' },
        { kind: 'p', text: '資料來源（球員檔案卡）有三處入口：' },
        {
          kind: 'list',
          items: ['名單預覽列尾「送生圖」', '單人替換卡「送生圖」', '檔案卡內輸入 UID 查詢'],
        },
        { kind: 'p', text: '提示詞模板：正文使用 {{變數|預設值}} 佔位符，變數由球員檔案自動填充。「管理模板」可新建 / 編輯 / 刪除模板（內建 default 模板不可刪除），可用變數膠囊點擊插入游標處。負向提示詞可選，模型不支援時僅記錄不傳送。' },
        { kind: 'p', text: '參數區按目前模型能力顯示或隱藏：尺寸 / 比例、張數、品質、Seed、取樣步數、引導係數、智慧改寫、思考模式、浮水印、呼叫方式、輸出格式。尺寸不合規或提示詞超出模型上限時「生成圖片」會停用並給出原因；Ctrl+Enter 與點按按鈕走同一流程。' },
        { kind: 'p', text: '結果畫廊同時顯示本次工作階段結果與持久畫廊記錄。可燈箱查看大圖、按原圖參數「再生成」、另存為檔案，或「儲存並替換」。' },
        {
          kind: 'list',
          items: [
            '儲存並替換會把圖片寫入 <頭像包>/_generated/<池>/，落點可在彈窗中修改；隨後走單人換臉流程寫入目前檔對應。',
            '如果圖片已落盤但目前檔 config.xml 缺少或結構異常，會警告「沒能寫入對應」，不會冒充成功。',
            '生成圖只存於工作階段暫存目錄 .imgtmp\\，每次啟動清理；不手動儲存或儲存並替換就會遺失。gallery.json 跨工作階段保留記錄，但圖片被清理後只顯示佔位。',
          ],
        },
        { kind: 'p', text: '批次佇列：在名單預覽勾選目前頁球員後點「加入佇列（N）」；AI 台底部佇列塢常駐，頂欄 ③ 顯示排隊中 + 生成中數量角標。' },
        {
          kind: 'list',
          items: [
            '確認方式分手動確認 / 全自動。手動模式需逐筆點佇列列核對參數並點「確認」，未全部確認不能啟動；全自動直接啟動。',
            '啟動前會確認任務數與費用；可只跑勾選項目，其餘留在佇列。',
            '執行中可取消：未提交的立即停止，已提交的等本次請求返回並保留結果（會產生費用）。',
            '佇列項目綁定入佇列時的「檔 · 組」；佇列執行中切檔 / 切組 / 關聯頭像包 / 新建檔會彈確認並取消未完成任務，閒置時的刪除也會確認。',
            '佇列是工作階段記憶體狀態，重開應用即空。',
          ],
        },
      ],
    },
    {
      id: 'fm-reload',
      title: '回 FM 生效',
      summary: '重新載入面板快取。',
      blocks: [
        { kind: 'p', text: '替換完成後回到 FM：偏好設定 → 介面，取消勾選再重新勾選面板，並重新載入面板快取（Reload skin cache）。如果頭像仍未變化，清理 FM 圖形快取後重啟遊戲。' },
      ],
    },
    {
      id: 'logs-data',
      title: '日誌、資料與備份',
      summary: '日誌抽屜、備份策略、資料目錄與升級移轉。',
      blocks: [
        { kind: 'p', text: '底部狀態列常駐目前狀態、迷你進度與最新一列日誌；「日誌」按鈕出現紅色角標表示有錯誤。點開日誌抽屜可：' },
        {
          kind: 'list',
          items: ['按層級篩選，或勾選「僅該層級」', '「開啟日誌檔案」用系統預設程式開啟 newgan.log', '「清空顯示」只清目前顯示，不回填'],
        },
        { kind: 'p', text: '日誌檔案按 10 MB × 3 份輪替，備份命名為 newgan_日期_時間.log。' },
        { kind: 'p', text: '寫入 config.xml 前，如果「寫入前備份」開啟，會生成 config_backup_YYYYMMDD-HHMMSS.xml，每個頭像包目錄最多保留最近 5 份；覆寫檔景快照前也會生成 <快照主檔名>_backup_...xml，每份快照最多保留最近 5 份。早期版本的 config備份_ 檔案不會被計入或自動清理。' },
        { kind: 'p', text: '正式安裝後，執行資料都在安裝目錄 data\\：' },
        {
          kind: 'code',
          text: '.user\\                  存檔、組設定與對應快照\nimage_ai.json           AI 服務商與生圖設定\nnationalities.json      國籍對照表使用者副本\nprompts\\templates.json  提示詞模板使用者副本\nrules\\standard2.json    標準二規則使用者副本\nrules\\standard3.json    標準三規則使用者副本\n.imgtmp\\                AI 工作階段暫存圖片（啟動清理）\n.imgtmp\\gallery.json    畫廊記錄（跨工作階段保留）\nwebview\\                WebView2 資料\nnewgan.log              執行日誌',
        },
        { kind: 'note', text: '唯讀對應表與標準規則出廠預設編譯期嵌入二進位；首次啟動會匯出使用者副本，刪除使用者副本後重啟會重新匯出。' },
      ],
    },
    {
      id: 'cache',
      title: '效能與索引快取',
      summary: '為什麼替換很快、何時需要強制重掃。',
      blocks: [
        { kind: 'p', text: '替換流程分為解析 RTF → 掃盤建索引 → 抽圖對應 → 寫 config.xml → 同步快照。掃盤是大頭，因此應用會把圖片索引按「頭像包根目錄 + 目錄簽章」快取。簽章由各一級目錄的名稱、mtime 與直接子項目數組成。' },
        {
          kind: 'list',
          items: [
            '掃過一次且內容沒變，之後每次替換直接重用快取，日誌顯示「命中（重用，不掃盤）」。',
            '往頭像包裡加圖卻沒被用上時，通常是因為簽章沒變。在對應一級目錄裡增刪任意一張圖片即可讓簽章變化，或重啟應用。',
            '深層子目錄裡的改名不會被簽章察覺；需要時同樣在一級目錄增刪圖片觸發重掃。',
            '目錄判定相容符號連結與 Windows junction；連結目標不可達時該子樹不會進入抽圖池。',
          ],
        },
      ],
    },
  ],
  faq: [
    {
      id: 'faq-formats',
      title: '支援哪些 RTF 格式？舊版名單還能用嗎？',
      blocks: [
        { kind: 'p', text: '支援 3 種欄位結構 × 2 種資料語言，共 6 種組合：基礎名單（7 欄）、拓展球員名單（≥18 欄，第 15 欄為所在地）、拓展職員名單（≥18 欄，第 15 欄為偏好工作）；每種都有中英文資料。舊版 14 欄詳細名單、性別在第 3 欄的舊職員版面已不再支援，請用本儲存庫最新 views/ 重新匯出。' },
      ],
    },
    {
      id: 'faq-invalid',
      title: '提示「RTF 格式不合法」怎麼辦？',
      blocks: [
        { kind: 'p', text: '表示表頭或資料列不符合上述格式，提示會定位到「表頭第 N 欄」或「第 N 列「欄位」」。檢查是否用了舊檢視 / 舊模板匯出、欄位被手動增刪、表頭與資料列錯位；用最新 views/ 重新匯出即可。' },
      ],
    },
    {
      id: 'faq-dropped',
      title: '為什麼有些球員整列被丟棄，或者沒有生成對應？',
      blocks: [
        { kind: 'p', text: '兩種情況：① 國籍顯示文字不在國籍對照表裡，整列丟棄並記警告日誌；可點「國籍對照表」補一筆對應或別名，儲存後下次掃描 / 替換生效。② 國籍三字碼不在標準規則的 Ethnics 對應裡，球員被跳過；可在「規則設定」的國籍對應裡補該碼。' },
      ],
    },
    {
      id: 'faq-none',
      title: '名單裡一個人都沒換？',
      blocks: [
        { kind: 'p', text: '若勾了「僅隨機人」，基礎名單全員按隨機人處理，但拓展名單要看第 10 欄 Newgen；如果全為「否 / No」，篩選後就沒有球員。關掉開關，或改用 is newgen 過濾檢視重新匯出。也可能是名單被篩空、UID 對不上，或頭像包裡沒有圖，導致本輪 0 筆對應。' },
      ],
    },
    {
      id: 'faq-fast',
      title: '為什麼替換秒過，日誌顯示「命中（重用，不掃盤）」？',
      blocks: [
        { kind: 'p', text: '這是正常行為：頭像包索引命中了目錄簽章快取，省掉了整包重掃。想強制重掃，在頭像包的一級目錄裡增刪任意一張圖片，或重啟應用。' },
      ],
    },
    {
      id: 'faq-add-image',
      title: '明明往頭像包裡加了圖，替換卻沒用上？',
      blocks: [
        { kind: 'p', text: '索引快取還在用舊簽章。簽章只覆蓋一級目錄的名稱、mtime 與直接子項目數，深層子目錄改名不會被察覺。在對應一級目錄裡增刪任意圖片即可，或重啟應用。' },
      ],
    },
    {
      id: 'faq-profile',
      title: 'Profile 和「頭像包（組）」是什麼？為什麼按鈕變灰？',
      blocks: [
        { kind: 'p', text: '一個 Profile = 一個存檔的一套設定；一個 Profile 可以關聯多個頭像包目錄（組），每組獨立記錄 RTF 與對應快照。下拉選擇存檔只是瀏覽，必須點「啟用」才能關聯頭像包、切換組、設定 RTF、執行替換；瀏覽狀態下這些按鈕會變灰並提示先啟用。' },
      ],
    },
    {
      id: 'faq-noprofile',
      title: '啟動為什麼不是我上次的存檔？No Profile 是什麼？',
      blocks: [
        { kind: 'p', text: '啟動固定從缺省檔 No Profile 開始，且不自動恢復上次啟用的檔。No Profile 未啟用、不可刪除，用於先看清手上有什麼料。選擇其他存檔並點「啟用」才會切換。' },
      ],
    },
    {
      id: 'faq-modes',
      title: '保留、覆寫、生成三種模式怎麼選？',
      blocks: [
        {
          kind: 'table',
          head: ['模式', '適用場景'],
          rows: [
            ['保留', '保留已換過的球員，只補新球員；日常增量推薦'],
            ['覆寫', '名單內全部重抽，名單外保留'],
            ['生成', '無視現有 config.xml，從零生成'],
          ],
        },
        { kind: 'p', text: '注意：生成模式若本輪一筆也沒抽到，會用空表覆寫本檔現有對應；本檔檔景非空時發起前會先確認。想清空 config.xml，就走「生成」並確認。' },
      ],
    },
    {
      id: 'faq-backup',
      title: '備份在哪裡？保留幾份？可以關閉嗎？',
      blocks: [
        { kind: 'p', text: 'config.xml 備份名為 config_backup_YYYYMMDD-HHMMSS.xml，放在對應頭像包目錄，每個目錄最多保留最近 5 份；檔景快照備份為 <快照主檔名>_backup_...xml，每份最多 5 份。替換台「選項設定」裡的「寫入前備份」可關閉，預設開啟，與單人替換共用。早期 config備份_ 歷史檔案不會自動清理。' },
      ],
    },
    {
      id: 'faq-integrity',
      title: '「檢查完整性」提示缺目錄或無法辨識怎麼辦？',
      blocks: [
        { kind: 'p', text: '缺目錄：點「一鍵建立缺少目錄」，或確認選的是頭像包根目錄。無法辨識：子目錄與任何標準的 FolderList 都不構成包含關係；替換時會把包內全部圖片遞迴全池隨機。可在「規則設定」把包內目錄名補進清單，恢復按國籍 / 種族精確路由。' },
      ],
    },
    {
      id: 'faq-standard',
      title: '頭像包需要哪些目錄？標準一 / 二 / 三有什麼區別？',
      blocks: [
        { kind: 'p', text: '標準一：圖片直接放在根目錄，無子目錄，完全隨機。標準二：14 個種族 / 大區目錄，國籍查表後按種族碼修正再抽圖。標準三：國籍級 + 族群級的細分目錄（具體項目以 standard3.json 的 FolderList 為準），按國家區塊 → 族群區塊 → _DEFAULT_ 規則區塊對應。判定只看目錄名：包含某標準 FolderList 的全部項目即歸該標準。' },
      ],
    },
    {
      id: 'faq-oldpack',
      title: '目錄名不在清單裡的舊頭像包還能用嗎？',
      blocks: [
        { kind: 'p', text: '能用。判定為「無法辨識」時，替換會遞迴全池隨機，仍能出圖，但不按國籍與種族碼路由。想恢復精確路由：在標準三規則的 FolderList 裡補上包內目錄名，或點「一鍵建立缺少目錄」把清單補齊。' },
      ],
    },
    {
      id: 'faq-rules',
      title: '「規則設定」儲存失敗 / 「標準三規則無效」怎麼辦？',
      blocks: [
        { kind: 'p', text: '儲存與載入都會做結構驗證：版本必須為 4；規則區塊鍵必須是國籍碼或清單內目錄名；每個區塊必須 0–10 全枚舉；規則指向的目錄必須在 FolderList 內；加權兩欄必須等長且全為正數。非法內容不會寫入磁碟。按提示修正，或刪掉 data\\rules\\standardN.json 讓程式重新匯出出廠預設；載入失敗會報錯並指明路徑，不會靜默回退。' },
      ],
    },
    {
      id: 'faq-nationality',
      title: '中文國籍能被辨識嗎？國籍對照表怎麼改？',
      blocks: [
        { kind: 'p', text: '能。RTF 的國籍（中文全稱、英文全稱或三字碼）先經國籍對照表正規化為三字碼，再進入抽圖與提示詞。點「國籍對照表」可增刪改碼、各語言全稱 / 別名和語言清單；同一碼在同一語言下可以有多筆（全稱與別名），解析都算命中。儲存後立即生效，也可直接編輯 data\\nationalities.json。' },
      ],
    },
    {
      id: 'faq-fm-no-avatar',
      title: '替換成功，FM 裡頭像沒變？',
      blocks: [
        { kind: 'p', text: '先重新載入面板快取：偏好設定 → 介面，取消再勾選面板並 Reload skin cache；必要時清 FM 圖形快取並重啟。再確認 FM 圖形目錄裡沒有舊的 config.xml 副本，且頭像包路徑與 FM 設定一致。' },
      ],
    },
    {
      id: 'faq-permission',
      title: '寫 config.xml 失敗 / 權限錯誤？',
      blocks: [
        { kind: 'p', text: '以管理員（或 sudo）執行，或把頭像包移到目前使用者可寫目錄。注意不要放在唯讀磁碟、同步磁碟鎖定的目錄裡。' },
      ],
    },
    {
      id: 'faq-ai-key',
      title: 'AI 生圖需要什麼？Key 存在哪裡？',
      blocks: [
        { kind: 'p', text: '需要自備服務商 API Key（本機 ComfyUI 免 Key）。Key 只在本機加密儲存，僅目前系統帳戶可解，不會隨圖片或設定明文匯出；換裝置或重裝系統後需重新填寫。點 AI 台頂部狀態燈進入「服務商設定」填寫 Key、Base URL 和模型。' },
      ],
    },
    {
      id: 'faq-comfy',
      title: '本機 ComfyUI 怎麼用？需要 Key 嗎？',
      blocks: [
        { kind: 'p', text: '不需要 Key。預設基址指向本機 ComfyUI，在「服務商設定」裡改 Base URL 即可連自建 / 遠端執行個體。參數對應工作流節點輸入：提示詞、尺寸、張數、負向詞、Seed、步數、引導係數分別落到 CLIPTextEncode、EmptyLatentImage、KSampler 等節點。' },
      ],
    },
    {
      id: 'faq-ai-images',
      title: 'AI 生成的圖存哪裡？會遺失嗎？「儲存並替換」失敗？',
      blocks: [
        { kind: 'p', text: '生成圖只存於 data\\.imgtmp\\，每次啟動清理；不手動「另存」或「儲存並替換」就會遺失。gallery.json 跨工作階段保留記錄，但圖片被清理後只顯示佔位。儲存並替換會寫入 <頭像包>\\_generated\\<池>\\；若提示「沒能寫入對應」，表示圖片已落盤，但目前檔 config.xml 缺少或結構異常，需要檢查頭像包後重試（也可改用「指定替換」手動補）。' },
      ],
    },
    {
      id: 'faq-queue-scope',
      title: '佇列執行中切存檔或切組會怎樣？',
      blocks: [
        { kind: 'p', text: '佇列項目綁定入佇列時的「檔 · 組」。佇列執行中切檔、切組、關聯頭像包、新建檔，會彈一次確認，確認後取消未完成的生成任務並執行切換；取消則留在原作用域。刪除檔 / 組無論佇列是否閒置都會確認（刪除不可逆），執行中會把佇列警告合併進同一個確認框。佇列是工作階段記憶體狀態，重開應用即空。' },
      ],
    },
    {
      id: 'faq-ai-queue',
      title: '批次生成佇列怎麼用？',
      blocks: [
        { kind: 'p', text: '在名單預覽勾選目前頁球員 → 點「加入佇列（N）」；AI 台底部佇列塢常駐，頂欄 ③ 顯示數量角標。確認方式分手動確認 / 全自動：手動模式需逐筆核對參數並點「確認」，未全部確認不能啟動；全自動直接啟動。可只跑勾選項目，其餘留在佇列；執行中可取消（未提交立即停止，已提交等返回並計費）；失敗項目可重試，錯誤會按類別給出處置建議。' },
      ],
    },
    {
      id: 'faq-log',
      title: '出問題怎麼回報？日誌在哪？',
      blocks: [
        { kind: 'p', text: '點狀態列「日誌」開啟抽屜，把層級選到 DEBUG 重現一次，點「開啟日誌檔案」直接開啟 newgan.log（正式安裝位於 <安裝目錄>\\data\\newgan.log）。回報問題時請附日誌與重現步驟。' },
      ],
    },
    {
      id: 'faq-smartscreen',
      title: '安裝時 SmartScreen / 防毒報毒怎麼辦？',
      blocks: [
        { kind: 'p', text: '應用未購買程式碼簽章憑證，SmartScreen 屬於未知發行者提示；點「更多資訊」→「仍要執行」。部分防毒軟體可能對未簽章程式啟發式報毒，可把安裝目錄加入信任區；原始碼完全公開，可自行稽核或自行建置。' },
      ],
    },
    {
      id: 'faq-data',
      title: '資料能移轉嗎？v1 怎麼升級？',
      blocks: [
        { kind: 'p', text: 'v2 首次啟動自動移轉：按順序查找 v1 可攜模式 <v2安裝目錄>\\data\\.user → v1 標準模式 %LOCALAPPDATA%\\NewGAN Manager\\.user，找到即整目錄複製到 v2 的 data\\.user，Profile、組、RTF 設定、對應快照全部保留。移轉是複製不是移動，v1 不受影響、可回退。config.xml 生成規則逐位元組相容。' },
      ],
    },
  ],
};

const en: HelpContent = {
  tutorial: [
    {
      id: 'install',
      title: 'Install and first launch',
      summary: 'Download, runtime, data location, and safety notes.',
      blocks: [
        { kind: 'p', text: 'Download the package for your platform from Releases. On Windows, the NSIS installer installs per-user and does not require administrator rights. The app is built with Tauri 2 and does not need Python or the .NET runtime.' },
        {
          kind: 'list',
          items: [
            'Windows 10 1809+ and Windows 11 normally include WebView2. If it is missing, the installer offers to download and install it.',
            'The app is not code-signed, so SmartScreen or antivirus software may warn that Windows protected your PC. Choose More info, then Run anyway, or add the install folder to your antivirus allowlist. The source is public and can be audited or built yourself.',
            'macOS and Linux builds come from CI artifacts.',
          ],
        },
        { kind: 'note', text: 'All app data is portable: development uses data\\ under the project root, and an installed build uses data\\ under its install folder. Nothing is written to AppData on the system drive.' },
        { kind: 'p', text: 'When upgrading from v1, the first launch copies the v1 .user folder into the v2 data folder (copy only, no move, so v1 keeps working). Launch always starts at the No Profile slot and never auto-restores the last active profile.' },
      ],
    },
    {
      id: 'fm-prep',
      title: 'Install FM views and filters',
      summary: 'Copy views/ and filters/ into the FM user folder and export the right columns.',
      blocks: [
        { kind: 'p', text: 'Copy the repository views/ and filters/ folders into the FM user folder:' },
        { kind: 'code', text: 'Windows: Documents\\Sports Interactive\\Football Manager 20XX\\\nmacOS: ~/Library/Application Support/Sports Interactive/Football Manager 20XX/' },
        { kind: 'p', text: 'In FM, export lists from SCRIPT FACES player search, squad, shortlist, and staff views. The is newgen filter under filters/ shows only newgens.' },
        { kind: 'note', text: 'Views define the RTF columns. Use the latest views/ from this repository. The old 14-column detailed list and the old staff layout with gender in column 3 are no longer supported.' },
      ],
    },
    {
      id: 'export-rtf',
      title: 'Export an RTF in FM',
      summary: 'Three column layouts, header detection, and Chinese/English data.',
      blocks: [
        { kind: 'p', text: 'Save the list as RTF. Scanning detects the format from the header first, then checks the first four columns (UID, nationality, second nationality, name) and samples the first 20 data rows.' },
        {
          kind: 'table',
          head: ['Format', 'Columns', 'Order (1-based)'],
          rows: [
            ['Basic list (players or staff)', '7', 'UID, nationality, second nationality, name, hair length code, hair colour code, ethnicity code'],
            ['Extended player list', '≥18', 'First 7 as above; 8 skin tone, 9 face, 10 newgen, 11 gender, 12 age, 13 height, 14 club, 15 location, 16 CA, 17 PA, 18 R.C.A; column 19 onward is user-defined'],
            ['Extended staff list', '≥18', 'Same as extended players except column 15 is preferred job'],
          ],
        },
        { kind: 'p', text: 'Language is not read from headers. Each data row nationality value is looked up in the translation table, so both Chinese and English exports work. Basic lists treat every player as a newgen (UID gets an r- prefix). Extended lists use the Newgen value in column 10 (Yes/No).' },
        { kind: 'note', tone: 'warn', text: 'Rows with an unknown nationality are dropped as a whole and logged as warnings, with at most 50 per-row logs. If the whole file is invalid, an "Invalid RTF format" warning points to the bad header column or data field.' },
      ],
    },
    {
      id: 'sidebar-scope',
      title: 'Sidebar: profiles and face packs',
      summary: 'Browse, activate, create, delete profiles, and link/switch pack groups.',
      blocks: [
        { kind: 'p', text: 'The sidebar is the scope: it decides which profile and which face pack later actions affect.' },
        {
          kind: 'list',
          items: [
            'Launch always starts at No Profile (not active, cannot be deleted). The last active profile is never restored automatically.',
            'Picking a profile in the dropdown only browses it: you can inspect its groups, RTF setting, and preview results, but linking packs, switching groups, setting an RTF, and replacing are disabled with a hint to activate first.',
            'Click Activate to switch: each old group config.xml is saved back to its snapshot and the new profile snapshots are restored into its pack folders. The button is disabled while the profile is active.',
            'Click + in the Profiles area to create a profile. It is activated immediately. Existing names cannot be overwritten.',
            'Delete Current Profile removes the selected profile with all group configs and snapshot backups. Deleting the active profile returns to No Profile.',
            'Click + in the Face Packs (Groups) area to link a pack root (it should contain race folders such as African and Asian). One profile can link several groups. Click a group name to switch; each row shows whether an RTF is set, and the red × deletes the group.',
            'Linking, switching, setting an RTF, and replacing all require an active profile.',
          ],
        },
        { kind: 'note', tone: 'warn', text: 'While the AI queue is running, switching profile or group, linking a pack, creating a profile, or deleting a profile/group asks for confirmation and cancels unfinished generation jobs. Switching while idle is silent; deleting always asks.' },
        { kind: 'p', text: 'The top bar always shows profile · group, which is the real active scope and does not follow sidebar browsing.' },
      ],
    },
    {
      id: 'archive-workbench',
      title: 'Archive bench: prepare, check, and snapshots',
      summary: 'Task card paths, integrity check, scans, rules, and nationality table.',
      blocks: [
        { kind: 'p', text: 'The archive bench shows what you have to work with. Write operations still require an active profile.' },
        {
          kind: 'list',
          items: [
            'The task card shows three paths: face pack, RTF list, and config.xml. Long paths show their tail; hover for the full path. Without a linked pack the fields are empty and buttons are disabled with a reason.',
            'Check integrity compares pack folders with the rule FolderList and classifies the pack as standard 1, 2, 3, or unrecognized. Missing folders can be created in one click; extra folders never enter the draw pool. Switching group or profile resets the result.',
            'Scan faces walks the pack, caches a picture index, logs a summary, and records a directory signature. Replacing reuses this cache when possible.',
            'Choose RTF picks an FM export. Scan RTF parses it immediately and fills the roster preview. Choose RTF is disabled while browsing.',
            'The config.xml row offers Read Snapshot, which parses the current profile snapshot XML into the Snapshot preview, and Show Location, which opens the snapshot folder. Both need a snapshot file, which exists after a replace run.',
            'Rules opens the structured rule editor: Overview, Folder list, Nationality map, Ethnicity fixups, Rule blocks, and Global settings, plus a Source JSON tab. Saving validates structure and never writes invalid content.',
            'Nationalities maintains three-letter codes, full names and aliases per language, and lets you add languages. Saving applies to the next RTF scan, replace run, or prompt preview.',
          ],
        },
        { kind: 'note', text: 'Checks and scans work while browsing. Setting an RTF, linking a pack, and replacing require activation.' },
      ],
    },
    {
      id: 'replace',
      title: 'Batch replace',
      summary: 'Three modes, options, two confirmations, and results.',
      blocks: [
        { kind: 'p', text: 'Choose a mode and options in the replace workbench, then click Replace Faces. You need an active profile, a selected pack, and an RTF (top bar status ② turns green).' },
        {
          kind: 'table',
          head: ['Mode', 'Meaning'],
          rows: [
            ['Preserve', 'Keep replaced players and add images only for new players; recommended for routine top-ups'],
            ['Overwrite', 'Redraw every listed player and keep unlisted ones'],
            ['Generate', 'Ignore the existing config.xml and build all mappings from scratch'],
          ],
        },
        { kind: 'p', text: 'Options: allow image reuse (when off, Preserve first excludes images already used in config.xml), newgens only, and back up before writing (shared with single replace, on by default).' },
        { kind: 'p', text: 'Two confirmations can appear before a run:' },
        {
          kind: 'list',
          ordered: true,
          items: [
            'Integrity fallback: ask once if folders are missing or the pack is unrecognized; continue silently when complete.',
            'Existing mapping pre-check: only in Generate mode when the profile snapshot is not empty, asking whether to clear the existing N mappings. If not confirmed, the pre-write guard aborts and neither config.xml nor the snapshot is touched.',
          ],
        },
        { kind: 'p', text: 'The button shows live progress and can be cancelled. A Replace complete dialog appears when done. When the pack index cache hits, mapping starts immediately and the log says "cache hit (reused, no scan)."' },
        { kind: 'p', text: 'The results area has two tabs:' },
        {
          kind: 'list',
          items: [
            'Roster preview: the last parsed list with sorting, search, paging and quick jumps. Extended lists can show extended columns, filter newgens, add the current page to the AI queue, and preview faces. The archive bench shows the full column set; the replace bench defaults to a compact set.',
            'Snapshot preview: mappings parsed by Read Snapshot, filterable by UID, with face paths and thumbnails.',
          ],
        },
        { kind: 'note', text: 'The status bar has two progress channels: the replace pipeline (replace:progress) takes priority; integrity checks, face scans, RTF scans, and snapshot reads use task:progress. A finished task stays at 100% for about 20 seconds before clearing.' },
      ],
    },
    {
      id: 'single',
      title: 'Single-player tweaks',
      summary: 'Search, assign a face, re-roll, and send to AI generation.',
      blocks: [
        { kind: 'p', text: 'The single replace card lives in the replace workbench and is also a write operation.' },
        {
          kind: 'list',
          items: [
            'Type a UID or keyword and press Enter. Use Previous and Next when several players match.',
            'The card shows the current mapping, player details, and a face preview, wrapping automatically when fields are long.',
            'Browse selects a new image. The path must look like ...\\<race folder>\\<image>.png. Assign Replace then rewrites only that mapping.',
            'Re-roll draws a new face for this player using the standard rules and writes it back.',
            'Send to AI makes this player the AI generation source and switches to the AI workbench.',
          ],
        },
        { kind: 'note', text: 'Single and batch replace share the Back up before writing switch.' },
      ],
    },
    {
      id: 'ai',
      title: 'AI face generation',
      summary: 'Providers, data sources, prompts, parameters, gallery, and queue.',
      blocks: [
        { kind: 'p', text: 'AI face generation is optional and needs your own API key (local ComfyUI needs none). It does not depend on race folders and is suited to individual or batch portraits.' },
        { kind: 'p', text: 'Providers: OpenAI and compatible APIs (including SenseNova), Alibaba Cloud Bailian (Qwen / Wan / Z-Image), Volcano Ark Seedream, SiliconFlow, ModelScope, and local ComfyUI. Click the status light or Provider Settings to edit the key, base URL, model list, and capability parameters. Keys are encrypted on this machine and can only be decrypted by the current OS account; re-enter them after moving to another device or reinstalling the system.' },
        { kind: 'p', text: 'The player card can be filled from three places:' },
        {
          kind: 'list',
          items: ['Send to AI at the end of a roster row', 'Send to AI on the single replace card', 'Type a UID in the player card'],
        },
        { kind: 'p', text: 'Prompt templates use {{variable|default}} placeholders filled from the player data. Manage Templates creates, edits, and deletes templates (the built-in default cannot be deleted). Click a variable chip to insert it at the caret. Negative prompts are optional and are only logged, not sent, when the model does not support them.' },
        { kind: 'p', text: 'Parameters show or hide based on model capabilities: size / ratio, count, quality, seed, steps, guidance, prompt rewriting, thinking mode, watermark, call mode, and output format. Generate is disabled with an explanation when the size is invalid or the prompt exceeds the model limit. Ctrl+Enter uses the same path as the button.' },
        { kind: 'p', text: 'The gallery shows this session\'s results and persistent records. Open a lightbox, regenerate with the original parameters, save as a file, or Save and Replace.' },
        {
          kind: 'list',
          items: [
            'Save and Replace writes into <pack>/_generated/<pool>/ (the target can be edited) and then writes the mapping through the single replace path.',
            'If the file is saved but the profile config.xml is missing or malformed, the app warns that the mapping was not written instead of reporting success.',
            'Generated images live only in the session folder .imgtmp\\ and are cleared on every launch. Save or Save and Replace them or they are lost. gallery.json keeps records across sessions, but cleaned images show a placeholder.',
          ],
        },
        { kind: 'p', text: 'Batch queue: select current-page players in the roster and click Add to Queue (N). The queue dock stays at the bottom of the AI bench, and top bar ③ shows queued plus generating counts.' },
        {
          kind: 'list',
          items: [
            'Confirmation is Manual or Automatic. Manual requires checking each queue row and clicking Confirm; the run cannot start until all are confirmed. Automatic starts directly.',
            'Before starting, the app confirms the job count and cost. You can run only checked rows and leave the rest queued.',
            'A running queue can be cancelled: unsubmitted jobs stop immediately, submitted ones finish and keep their results (and still cost money).',
            'Queue rows bind the profile · group at enqueue time. Switching profile or group, linking a pack, or creating a profile while running asks for confirmation and cancels unfinished jobs. Deleting while idle also asks.',
            'The queue is session memory only and is empty after restarting.',
          ],
        },
      ],
    },
    {
      id: 'fm-reload',
      title: 'Make it show in FM',
      summary: 'Reload the skin cache.',
      blocks: [
        { kind: 'p', text: 'Back in FM, open Preferences, Interface, untick and re-tick the skin, and reload the skin cache. If faces still do not change, clear the FM graphics cache and restart the game.' },
      ],
    },
    {
      id: 'logs-data',
      title: 'Logs, data, and backups',
      summary: 'Log drawer, backup policy, data folder, and migration.',
      blocks: [
        { kind: 'p', text: 'The status bar shows the current state, mini progress, and latest log line. A red badge on Logs means errors are present. The log drawer lets you:' },
        {
          kind: 'list',
          items: ['Filter by level or show that level only', 'Open newgan.log with the system default app', 'Clear the display without backfilling'],
        },
        { kind: 'p', text: 'Logs roll at 10 MB × 3 files, with backups named newgan_date_time.log.' },
        { kind: 'p', text: 'Before writing config.xml, with Back up before writing enabled, the app creates config_backup_YYYYMMDD-HHMMSS.xml and keeps the latest 5 per pack folder. Snapshot overwrites also create <stem>_backup_...xml and keep the latest 5 per snapshot. Legacy config备份_ files are neither counted nor cleaned automatically.' },
        { kind: 'p', text: 'In an installed build all runtime data lives under data\\ in the install folder:' },
        {
          kind: 'code',
          text: '.user\\                  profiles, group configs, mapping snapshots\nimage_ai.json           AI provider and generation settings\nnationalities.json      user copy of the nationality table\nprompts\\templates.json  user copy of prompt templates\nrules\\standard2.json    user copy of standard 2 rules\nrules\\standard3.json    user copy of standard 3 rules\n.imgtmp\\                AI session images (cleared at launch)\n.imgtmp\\gallery.json    gallery records (kept across sessions)\nwebview\\                WebView2 data\nnewgan.log              runtime log',
        },
        { kind: 'note', text: 'Read-only mapping tables and standard rules are embedded in the binary at build time. User copies are exported on first launch, and deleting them re-exports the defaults on the next start.' },
      ],
    },
    {
      id: 'cache',
      title: 'Performance and index cache',
      summary: 'Why replaces are fast and when to force a rescan.',
      blocks: [
        { kind: 'p', text: 'A replace run parses the RTF, scans the pack into an index, draws mappings, writes config.xml, and syncs the snapshot. Scanning dominates, so the index is cached per pack root plus directory signature, built from each first-level folder name, mtime, and direct child count.' },
        {
          kind: 'list',
          items: [
            'After one scan with no changes, later runs reuse the cache and log "cache hit (reused, no scan)".',
            'If a newly added image is not used, the signature likely did not change. Add or remove any image in the relevant first-level folder, or restart the app.',
            'Renames deep inside a subfolder are not detected; trigger a rescan the same way when needed.',
            'Directory detection supports symlinks and Windows junctions. An unreachable link target keeps that subtree out of the draw pool.',
          ],
        },
      ],
    },
  ],
  faq: [
    {
      id: 'faq-formats',
      title: 'Which RTF formats are supported? Can I use old lists?',
      blocks: [
        { kind: 'p', text: 'Three column layouts × two data languages, six combinations: basic list (7 columns), extended player list (≥18 columns, column 15 is location), and extended staff list (≥18 columns, column 15 is preferred job), each in Chinese and English. The old 14-column detailed list and the old staff layout with gender in column 3 are no longer supported; export again with the latest views/.' },
      ],
    },
    {
      id: 'faq-invalid',
      title: 'What should I do when "Invalid RTF format" appears?',
      blocks: [
        { kind: 'p', text: 'The header or a data row does not match a supported layout. The message points to the bad header column or the row and field. Check for an old view or template, manually added or removed columns, or a header/data mismatch, then re-export with the latest views/.' },
      ],
    },
    {
      id: 'faq-dropped',
      title: 'Why are some players dropped or left without a mapping?',
      blocks: [
        { kind: 'p', text: 'Two cases: 1) The nationality text is not in the nationality table, so the row is dropped with a warning. Add a mapping or alias in Nationalities; saving applies to the next scan or replace. 2) The three-letter code is missing from the standard rule Ethnicity map, so the player is skipped. Add the code under Nationality map in Rules.' },
      ],
    },
    {
      id: 'faq-none',
      title: 'Why did no player get a face?',
      blocks: [
        { kind: 'p', text: 'With Newgens only on, basic lists treat everyone as a newgen, but extended lists read column 10. If every value is No, the filter leaves nobody. Turn it off or export again with the is newgen filter. Also check for an empty filtered list, UID mismatches, or a pack with no images, which can produce zero mappings.' },
      ],
    },
    {
      id: 'faq-fast',
      title: 'Why did replace finish instantly with "cache hit"?',
      blocks: [
        { kind: 'p', text: 'That is expected: the pack index matched the cached directory signature, so the full pack scan was skipped. To force a rescan, add or remove any image in a first-level pack folder, or restart the app.' },
      ],
    },
    {
      id: 'faq-add-image',
      title: 'I added images to the pack, but replace did not use them.',
      blocks: [
        { kind: 'p', text: 'The index cache is still using the old signature, which only covers first-level folder names, mtimes, and direct child counts. Renames deep in subfolders are invisible. Add or remove any image in the relevant first-level folder, or restart the app.' },
      ],
    },
    {
      id: 'faq-profile',
      title: 'What are a profile and a face-pack group? Why are buttons disabled?',
      blocks: [
        { kind: 'p', text: 'A profile holds one configuration per save. One profile can link multiple pack folders (groups), and each group has its own RTF and mapping snapshot. Picking a profile in the dropdown only browses it; click Activate to link a pack, switch groups, set an RTF, and replace. While browsing, those buttons are disabled with an activation hint.' },
      ],
    },
    {
      id: 'faq-noprofile',
      title: 'Why does the app not open my last profile? What is No Profile?',
      blocks: [
        { kind: 'p', text: 'The app always starts at No Profile and never auto-restores the last active profile. No Profile is inactive and cannot be deleted; it exists so you can look at your data first. Select another profile and click Activate to switch.' },
      ],
    },
    {
      id: 'faq-modes',
      title: 'Preserve, Overwrite, or Generate?',
      blocks: [
        {
          kind: 'table',
          head: ['Mode', 'When to use'],
          rows: [
            ['Preserve', 'Keep replaced players and fill new ones; recommended for routine top-ups'],
            ['Overwrite', 'Redraw all listed players and keep unlisted ones'],
            ['Generate', 'Ignore the existing config.xml and start from scratch'],
          ],
        },
        { kind: 'p', text: 'Caution: if Generate draws nothing in a run, it overwrites existing mappings with an empty table. When the profile snapshot is not empty, the app asks first. To clear config.xml on purpose, use Generate and confirm.' },
      ],
    },
    {
      id: 'faq-backup',
      title: 'Where are backups, how many are kept, and can I disable them?',
      blocks: [
        { kind: 'p', text: 'config.xml backups are config_backup_YYYYMMDD-HHMMSS.xml in the pack folder, with the latest 5 kept per folder. Snapshot backups are <stem>_backup_...xml, with the latest 5 per snapshot. Back up before writing can be turned off in Replace options; it is on by default and shared with single replace. Legacy config备份_ files are never cleaned automatically.' },
      ],
    },
    {
      id: 'faq-integrity',
      title: 'Check integrity says folders are missing or the pack is unrecognized.',
      blocks: [
        { kind: 'p', text: 'Missing folders: click Create Missing Folders or make sure you linked the pack root. Unrecognized: the subfolders do not contain any standard FolderList, so replace draws recursively from the whole pack. Add the folder names to the standard 3 FolderList in Rules to restore nationality and ethnicity routing.' },
      ],
    },
    {
      id: 'faq-standard',
      title: 'Which folders does a pack need? Standard 1, 2, or 3?',
      blocks: [
        { kind: 'p', text: 'Standard 1: images sit in the root with no subfolders and are drawn fully at random. Standard 2: 14 race or region folders; nationality is mapped and corrected by ethnicity code before drawing. Standard 3: fine-grained nationality and ethnicity folders (see the FolderList in standard3.json) matched by country block, ethnicity block, then _DEFAULT_ rules. Classification is by folder names only: containing all entries of a standard FolderList makes the pack that standard.' },
      ],
    },
    {
      id: 'faq-oldpack',
      title: 'Can I use an old pack with folders not in the list?',
      blocks: [
        { kind: 'p', text: 'Yes. An unrecognized pack still draws recursively from all images, but ignores nationality and ethnicity routing. To restore precise routing, add the pack folder names to the standard 3 FolderList in Rules, or use Create Missing Folders to complete the list.' },
      ],
    },
    {
      id: 'faq-rules',
      title: 'Rules saving fails, or "invalid standard 3 rules". What now?',
      blocks: [
        { kind: 'p', text: 'Saving and loading both validate structure: version must be 4; rule-block keys must be nationality codes or folders in the list; each block must enumerate 0–10; rules may only point to folders in the FolderList; and the weighted columns must be equal length and all positive. Invalid content is never written. Fix the message, or delete data\\rules\\standardN.json so the app re-exports the default. Loading failures report the path instead of silently falling back.' },
      ],
    },
    {
      id: 'faq-nationality',
      title: 'Does the app understand Chinese nationalities? How do I edit the table?',
      blocks: [
        { kind: 'p', text: 'Yes. RTF nationalities (Chinese full names, English full names, or three-letter codes) are normalized through the nationality table before drawing and prompting. Nationalities lets you add, edit, and delete codes, full names and aliases per language, and the language list. The same code can have several entries per language (full name and aliases), and all of them match. Saving applies immediately; you can also edit data\\nationalities.json directly.' },
      ],
    },
    {
      id: 'faq-fm-no-avatar',
      title: 'Replace succeeded, but FM still shows old faces.',
      blocks: [
        { kind: 'p', text: 'Reload the skin cache: Preferences, Interface, untick and re-tick the skin, then Reload skin cache. If needed, clear the FM graphics cache and restart. Also check that no old config.xml copy exists in the FM graphics folder and that the pack path matches the FM setup.' },
      ],
    },
    {
      id: 'faq-permission',
      title: 'config.xml writes fail or permission errors appear.',
      blocks: [
        { kind: 'p', text: 'Run as administrator (or with sudo), or move the pack to a folder the current user can write. Avoid read-only drives and folders locked by sync clients.' },
      ],
    },
    {
      id: 'faq-ai-key',
      title: 'What does AI generation need? Where is the key stored?',
      blocks: [
        { kind: 'p', text: 'You need your own provider API key (local ComfyUI needs none). Keys are encrypted locally and only the current OS account can decrypt them. They are never exported in plain text with images or settings; re-enter them after moving devices or reinstalling. Click the AI status light to open Provider Settings and fill in the key, base URL, and models.' },
      ],
    },
    {
      id: 'faq-comfy',
      title: 'How do I use local ComfyUI? Does it need a key?',
      blocks: [
        { kind: 'p', text: 'No key is needed. The default base URL points at local ComfyUI; change Base URL in Provider Settings to use a self-hosted or remote instance. Parameters map to workflow node inputs: prompt, size, count, negative prompt, seed, steps, and guidance go to CLIPTextEncode, EmptyLatentImage, KSampler, and similar nodes.' },
      ],
    },
    {
      id: 'faq-ai-images',
      title: 'Where are AI images stored? Can they be lost? What if Save and Replace fails?',
      blocks: [
        { kind: 'p', text: 'Generated images live only in data\\.imgtmp\\ and are cleared on every launch. Save as or Save and Replace them or they are lost. gallery.json keeps records across sessions, but cleaned images show a placeholder. Save and Replace writes into <pack>\\_generated\\<pool>\\. If it warns that the mapping was not written, the image was saved but the profile config.xml is missing or malformed; check the pack and retry, or use Assign Replace manually.' },
      ],
    },
    {
      id: 'faq-queue-scope',
      title: 'What happens if I switch profile or group while the queue runs?',
      blocks: [
        { kind: 'p', text: 'Queue rows bind the profile and group at enqueue time. Switching profile or group, linking a pack, or creating a profile while running asks for confirmation, then cancels unfinished jobs and switches; cancelling the dialog keeps the current scope. Deleting a profile or group always confirms because it is irreversible, and the queue warning is merged into the same dialog while running. The queue lives in session memory and is empty after a restart.' },
      ],
    },
    {
      id: 'faq-ai-queue',
      title: 'How does the batch generation queue work?',
      blocks: [
        { kind: 'p', text: 'Select current-page players in the roster and click Add to Queue (N). The queue dock stays at the bottom of the AI bench, and top bar ③ shows the count. Confirmation is Manual or Automatic: Manual requires checking each row and clicking Confirm, and the run cannot start until all are confirmed; Automatic starts immediately. You can run only checked rows. Cancel stops unsubmitted jobs at once and lets submitted ones finish and bill. Failed rows can be retried, with error guidance by category.' },
      ],
    },
    {
      id: 'faq-log',
      title: 'How do I report a problem? Where are the logs?',
      blocks: [
        { kind: 'p', text: 'Click Logs in the status bar, set the level to DEBUG, reproduce the issue, then click Open Log File to open newgan.log (installed builds use <install folder>\\data\\newgan.log). Include the log and reproduction steps when reporting.' },
      ],
    },
    {
      id: 'faq-smartscreen',
      title: 'SmartScreen or antivirus flags the installer.',
      blocks: [
        { kind: 'p', text: 'The app is not code-signed, so SmartScreen shows an unknown publisher warning. Choose More info, then Run anyway. Some antivirus tools heuristically flag unsigned apps; add the install folder to the allowlist. The source is public, so you can audit or build it yourself.' },
      ],
    },
    {
      id: 'faq-data',
      title: 'Can I migrate data? How do I upgrade from v1?',
      blocks: [
        { kind: 'p', text: 'v2 migrates on first launch: it checks the v1 portable path <v2 install>\\data\\.user, then the v1 standard path %LOCALAPPDATA%\\NewGAN Manager\\.user. If found, the whole folder is copied into v2 data\\.user, keeping profiles, groups, RTF settings, and mapping snapshots. This is a copy, not a move, so v1 still works and you can roll back. config.xml output stays byte-compatible.' },
      ],
    },
  ],
};

export const HELP: Record<Lang, HelpContent> = { 'zh-CN': zhCN, 'zh-TW': zhTW, en };

export function helpEntries(lang: Lang, tab: HelpTab): HelpEntry[] {
  return HELP[lang][tab];
}

function blockText(block: HelpBlock): string {
  switch (block.kind) {
    case 'p':
    case 'note':
    case 'code':
      return block.text;
    case 'list':
      return block.items.join(' ');
    case 'table':
      return [...block.head, ...block.rows.flat()].join(' ');
  }
}

/** 搜索用的纯文本索引：标题 + 摘要 + 全部内容块 */
export function helpSearchText(entry: HelpEntry): string {
  return [entry.title, entry.summary ?? '', ...entry.blocks.map(blockText)].join('\n').toLowerCase();
}
