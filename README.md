# NewGAN Manager

> 给 Football Manager 的 NewGAN / 随机人批量匹配头像（facepack）的桌面工具。
> 读取 FM 导出的 RTF 名单 → 按「国籍 + 种族」自动挑一张头像图片 → 生成 FM 能识别的 `config.xml`。

![Tauri Build](https://github.com/huangzhanhao/NewGAN-Manager-ByHzH/actions/workflows/tauri.yml/badge.svg)

**当前版本 v2.0.0** ｜ Fork 自 [Maradonna90/NewGAN-Manager](https://github.com/Maradonna90/NewGAN-Manager)（上游已停止维护）｜ v2 基于 **Tauri 2（Rust 后端）+ Svelte 5（前端）** 重构：无需 Python / .NET 运行时，大名单（30 万行级）不卡顿，界面按操作旅程重排。

---

## 它解决什么问题

NewGAN（随机人）默认没有头像。手动给成百上千名随机人一张张贴图不现实。这个工具把 FM 导出的球员名单（RTF）和一套按种族分目录的头像包对应起来，**批量**生成 FM 读取的 `config.xml`，让随机人自动"长脸"。

**典型使用一句话：** 导出 RTF → 选头像包目录 → 点 `替换头像` → 回 FM 重载皮肤。

---

## 主要功能

- **多 Profile / 组管理**：不同存档各建一套 Profile；一个 Profile 下可关联多个头像包目录（组），每组独立记录 RTF 与映射快照。下拉切换先进入「浏览预览」，点 **激活** 才真正切档——把当前 `config.xml` 存回旧档快照、再把目标档快照写回头像包目录；新建 Profile 则建好即激活。启动固定从缺省档 `No Profile` 开始，不自动恢复上次激活的档。
- **多导出格式自动识别**：英文经典、中文经典与 FM 新版扩展（ext_new）三类导出格式校验时自动识别，归一到同一份球员记录；国籍中文名经 `nat_translation.json` 翻译回英文三字码再查种族映射——**无需把 FM 切成英文**。
- **只处理随机人**：`仅随机人` 开关按名单里的随机人标志列（经典布局在最后一列、新版布局在第 2 列，`Yes/No`、`是/否`）过滤，只给随机人换脸，被过滤者不写入 `config.xml`。
- **三种替换模式**：`保留`（Preserve）保留已换过的只补新的；`覆写`（Overwrite）对名单内球员全部重抽、名单外保留；`生成`（Generate）无视现有 `config.xml` 从零生成。
- **名单预览**：执行「替换头像」或「扫描RTF」后展示最近一次解析结果，分页预览表支持排序 / 搜索；数据保留在 Rust 侧，前端只取当前页，大名单也不卡。
- **头像扫描**：点 `扫描头像` 遍历头像包并缓存图片索引，摘要（图片总数、命中的标准）写入日志。
- **单个球员微调**：结果区切到 `单人微调`，输入 UID 回车 → 预览当前头像与球员信息 → 选一张图点 `替换`，只改写这一条映射。
- **规则可视化编辑**：`规则配置` 对话框直接查看、修改并保存标准规则 json（国籍→目录映射、FolderList 等），保存时自动做结构校验。
- **自动备份**：每次写入前生成 `config备份_YYYYMMDD-HHMMSS.xml`，每个头像包目录最多保留最近 10 份。
- **进度与取消**：解析 / 映射 / 写文件全部在 Rust 后台线程执行，进度经事件流推送；执行中界面保持流畅，可随时 `取消`。
- **应用内日志**：状态栏常驻状态与迷你进度，`日志` 按钮带错误角标；抽屉内可按级别筛选、只看某一级、一键打开日志文件；日志文件 10 MB × 3 份滚动。
- **完整性检查与一键创建**：`检查完整性` 按规则 json 的目录清单判定头像包归属哪个标准、缺哪些目录，缺失时一键批量创建（执行替换前仍兜底校验）。

---

## 快速开始（普通用户）

1. 到 **Releases** 下载对应平台安装包，安装运行（无需安装 Python / .NET）。
   - **Windows**：`NewGAN-Manager_2.0.0_x64-setup.exe`（NSIS，安装到当前用户目录，无需管理员权限）。
   - 系统使用自带 WebView2（Win10 1809+ / Win11 一般已内置）；若缺失，安装包会联网引导安装。
   - **首次运行提示**：应用未购买代码签名证书，Windows SmartScreen 或部分杀毒软件可能弹出警告（"Windows 已保护你的电脑"）。请点 **「更多信息」→「仍要运行」**；如在杀软信任区添加应用目录可避免反复提示。应用源码完全公开，可自行审计。
   - **macOS / Linux**：下载 CI 构建的对应平台产物。
2. 把仓库里的 `views/`、`filters/` 两个文件夹复制到 FM 用户目录：
   - Windows：`文档\Sports Interactive\Football Manager 20XX\`
   - macOS：`~/Library/Application Support/Sports Interactive/Football Manager 20XX/`
3. 按下面的[使用流程](#使用流程)操作。

---

## 使用流程

### 1. 在 FM 中导出 RTF
安装 `views/` 后，在 FM 里使用 **`SCRIPT FACES`** 的 `player search` / `squad` / `shortlist` / `staff` 视图（配合 `filters/` 里的 `is newgen search filter` 只看随机人），把列表**另存为 RTF**。

校验时按 **英文经典 → 中文经典 → 新版扩展** 依次尝试，自动识别出格式与语言，三种格式归一到同一份球员记录：

| 格式 | 列数 | 列序 |
| :--- | :--- | :--- |
| 经典名单（英文界面） | 7 | `UID \| Nat \| 2nd Nat \| Name \| 头发长度 \| 发色 \| 种族码` |
| 经典名单（中文界面） | 7 / 14 | 列序同上，表头与国籍为中文（国籍如「美国」，自动翻译回三字码） |
| 详细名单 | 14 | 经典列序扩展：多出 肤色码、Face、俱乐部、年龄、身高、体重、`是否为随机人` |
| 新版扩展名单（FM 新版导出） | 17 | `UID \| 随机人标志 \| 现状 \| \| 姓名 \| 国籍 \| 二国籍 \| 年龄 \| 身高 \| 俱乐部 \| 位置 \| 种族码 \| 肤色 \| 发长 \| 发色 \| 能力 \| 潜力` |

示例（经典 7 列）：

```
| UID       | Nat  | 2nd Nat | Name        | 头发长度 | 发色 | 种族码 |
| 2000472008| ESP  | USA     | Pepe Sáenz  | 1        | 12   | 1      |
```

> 随机人标志列的位置随格式不同：经典布局在最后一列，新版扩展布局在第 2 列（`Yes/No`、`是/否`）。UID 默认按随机人处理并加 `r-` 前缀；标志为 `No / 否` 时改回纯数字 UID。只有带随机人标志列时，`仅随机人` 过滤才生效。

### 2. 侧栏：浏览、激活 Profile 与头像包（组）
- 启动固定停在缺省档 **`No Profile`**（未激活，不可删除）；上次会话激活了哪档**不会**自动恢复。
- **下拉框选择任意存档 = 浏览预览**：可以查看该档的组配置与 RTF 设置，但关联头像包、切换组、设置 RTF、执行替换等改动型操作置灰，提示「请先点击激活」。
- 点蓝色 **`激活`** 按钮才真正切档：旧档各组 `config.xml` 存回快照、新档快照恢复到头像包目录。已激活时按钮变灰，激活状态由按钮本身表达（无单独的状态标签）。
- 点 `存档资料` 区的 ＋ **新建 Profile**：输入名称创建，**建好即自动激活**（同名档不得重建覆写）。
- 红色 **`删除当前 Profile`** 删除**下拉选中**的存档及其全部组配置与映射备份；若删的是激活档，自动回到 `No Profile`。
- 点 `头像包（组）` 区的 ＋ **关联头像包根目录**（其下是 `African`、`Asian`… 等种族子目录），一个 Profile 可关联多组；点组名切换当前组，组条目上显示 RTF 是否已设置；行内右侧的红色 `×`（点击区上下占满整行）删除该组。以上操作均需先激活。

### 3. 「当前任务」卡：检查、扫描与设置 RTF
- 未关联头像包时，这一卡仍完整显示「头像包 / RTF 名单 / config.xml」三行路径框与按钮（路径框为空、按钮置灰并提示原因），只在下方给一行灰色提示；关联后立即刷新。
- 路径框等宽对齐，长路径默认**显示尾部**（末级目录 / 文件名），鼠标悬停可看完整路径。
- **`检查完整性`**：按规则 json 的 `FolderList` 识别头像包归属哪个标准并校验目录是否齐全；缺失时 **`一键创建缺失目录`**；无法识别时提示替换时将递归全池随机（可在 `规则配置` 调整标准目录清单）。切换组/切档后检查结果自动重置。
- **`扫描头像`**：遍历头像包并缓存图片索引，摘要（图片总数、命中的标准）写入日志。
- **`选择RTF`** 挑选 FM 导出的 RTF 名单；**`扫描RTF`** 立即解析并填充名单预览（结果写入日志）。
- **config.xml 行**：`查看config` 打开 config.xml 所在目录；`查看快照` 打开存档 XML 快照所在目录。
- **`规则配置`**：打开规则编辑对话框，直接修改并保存标准规则 json。
- 浏览态（未激活）下 `选择RTF` 等改动型操作置灰并提示先激活；检查、扫描类操作不受限。

### 4. 执行面板：选模式与开关后执行
- **模式**：`保留` / `覆写` / `生成` 三个 chip 任选（对应 Preserve / Overwrite / Generate，含义见[主要功能](#主要功能)）。
- 开关（`选项设置：` 行）：`允许复用图片`（关闭时保留模式会先剔除 `config.xml` 已用图片）、`仅随机人`、`写入前备份`。
- 未就绪时按钮禁用并提示缺什么（选择 Profile · 选择头像包 · 设置 RTF 名单）；浏览态下提示替换只作用于已激活的存档。
- 点击 **`替换头像`**，进度条与状态文字实时推进，执行中可 `取消`，完成弹出 `替换完成！`。

### 5. 结果区：名单预览与单人微调
- **名单预览**：最近一次解析结果（替换头像或扫描RTF 产生）分页表格，可排序 / 搜索。
- **单人微调**：输入该球员 UID 回车 → 查看当前头像与球员信息 → `浏览` 选一张新图（路径需形如 `...\<种族目录>\<图片名>.png`）→ **`替换`**，仅改写这一条映射。字段较多，窗口较窄时会自动换行，无需拖动横向滚动条。

### 6. 回 FM 生效
`偏好设置 → 界面` 中**取消再勾选皮肤、重新加载皮肤缓存**（Reload skin cache），必要时清缓存后重启 FM。

### 7. 状态栏与日志
底部状态栏常驻显示当前状态（就绪 / 进度文字）；`日志` 按钮出现红色角标表示有错误，点开抽屉可按级别筛选、只看某一级、打开日志文件或清空显示。

> 抽屉里的列表来自 Rust 侧的环形缓冲（上限 1 万条，仅本次运行）：前端启动时会先向 `query_logs` 回填一次历史，再订阅 `log:batch` 增量。
> 因为启动日志（`Starting Application v2`）在 WebView 开始加载页面前就已被 emit，而 Tauri 不重放事件，所以这一行只能靠回填拿到。

---

## 头像包目录与种族

图片目录根下需要存在以下 14 个种族子目录（**名称必须完全一致**，含 `Seasian` 这一上游历史拼写与带空格的目录名）：

| 目录名 | 含义 | 目录名 | 含义 |
| :--- | :--- | :--- | :--- |
| `African` | 撒哈拉以南非洲 | `MESA` | 中东与南亚 |
| `Asian` | 东亚 | `SAMed` | 南大西洋与地中海 |
| `Caucasian` | 东欧 / 斯拉夫 | `Scandinavian` | 北欧 |
| `Central European` | 中欧、西欧 | `Seasian` | 东南亚 |
| `EECA` | 东欧与中亚 | `South American` | 拉美混血 |
| `Italmed` | 意大利与地中海 | `SpanMed` | 伊比利亚半岛 |
| `MENA` | 中东与北非 | `YugoGreek` | 巴尔干与希腊 |

- 选图池优先级：**国籍同名目录**（如 `ARG/`、`CHN/`）→ 种族目录 → 两者都不存在时使用头像包全部目录。
- FM 种族码（0–10）对照示例见 [FM NewGAN种族代码示例.html](docs/FM%20NewGAN种族代码示例.html)；标准三抽图模型的实测记录见 [ng-regens-v4-抽图模型.md](docs/ng-regens-v4-抽图模型.md)；实现见 [`face_mapper.rs`](src-tauri/src/face_mapper.rs)。
- 国籍 → 种族的对照表在 [`src-tauri/assets/eth_cfg.json`](src-tauri/assets/eth_cfg.json)（编译期嵌入，内容与 v1 一致），修改后需重新构建。

---

## 项目结构

```
├── src-svelte/                  # Svelte 5 前端
│   ├── main.ts / App.svelte / app.css
│   └── lib/
│       ├── types.ts / api.ts / dialog.ts   # IPC 类型与封装、统一错误弹窗
│       ├── text.ts              # 全局文案（所有用户可见文字统一维护，默认中文）
│       ├── actions.ts / icons.ts           # Svelte actions、内联 SVG 图标
│       ├── stores/              # app / log / ui（runes 状态）
│       └── components/          # Sidebar / SetupCard / ReplacePanel / ResultsSection /
│                                # PlayersTable / PlayerViewer / RuleEditor / StatusBar / LogDrawer
├── src-tauri/                   # Rust 后端
│   ├── Cargo.toml / build.rs / tauri.conf.json
│   ├── assets/                  # 嵌入资源：eth_cfg.json / nat_translation.json / config_template /
│   │                            #   standard2.json / standard3.json（标准规则出厂默认）
│   ├── capabilities/default.json
│   ├── icons/                   # 应用全套图标（OS 窗口 / 安装包用，由 app-icon.png 生成）
│   └── src/
│       ├── main.rs / lib.rs     # 入口 + Tauri Builder 装配（含便携数据目录与 v1 迁移）
│       ├── assets.rs            # 编译期加载映射表与标准规则出厂默认
│       ├── json_store.rs        # JSON 原子读写 + 损坏回退
│       ├── paths.rs             # 路径规范化（目录 key / 快照短命名）
│       ├── facepack.rs          # 头像包扫描 + 标准判定（直铺 / 标准N / 无法识别）
│       ├── templates/           # 每个标准配套的抽图模型：standard1.rs / standard2.rs / standard3.rs / unknown.rs
│       ├── rtf_parser.rs        # ← core/RtfParser.py（多格式识别 / 校验 / 解析 / 中文翻译）
│       ├── xml_parser.rs        # ← core/XmlParser.py（读/写/单条替换/备份轮换）
│       ├── profile_manager.rs   # ← core/ProfileManager.py（档/组 CRUD、浏览预览、激活快照交换）
│       ├── face_mapper.rs       # ← core/FaceMapper.py（标准路由 + 三模式映射）
│       ├── replace_flow.rs      # ← services/replace_service.py（替换流水线）
│       ├── logger.rs            # ← core/NewGanLogManager.py（环形缓冲 + 事件）
│       └── commands.rs          # ← services/* + 对话框原语（26 个 Tauri 命令）
├── tests/testing_data/          # RTF 测试样本（不入库）
├── .cargo/config.toml           # Cargo 构建配置（target-dir 指向根目录 target/）
├── package.json / vite.config.ts / svelte.config.js / tsconfig.json
├── views/ filters/ docs/ LICENSE  # FM 视图与过滤器、文档、许可
└── .github/workflows/tauri.yml    # CI：Rust/Web 测试门禁 + 三平台构建
```

> 生成目录（均已 gitignore）：`node_modules/`（pnpm 依赖）、`dist/`（前端构建）、`target/`（Rust 构建）、`data/`（开发调试时的运行数据）。

> 分层对应 v1：**UI（Svelte 组件）→ 命令层（Tauri commands）→ 业务核心（Rust 模块）**。数据永远留在 Rust 侧，IPC 只传分页 / 摘要，根治大 RTF 卡顿。
> v1（Python/Toga 版）的架构文档见 [docs/legacy-python/ARCHITECTURE.md](docs/legacy-python/ARCHITECTURE.md)。

---

## 开发与打包

```bash
# 0) 前置：Rust 工具链（MSVC target）+ Node.js ≥ 20 + pnpm；Win10/11 一般已自带 WebView2
# 1) 安装依赖
pnpm install

# 2) 开发模式（前端热重载 + Rust 增量编译；运行数据写入 <项目根>/data/）
#    若 5173 上已有本项目的 vite（先用 IDE 任务 / Chrome 调试起过前端），会自动复用，不会因端口占用而失败
pnpm tauri dev

# 3) 测试与检查
cargo test --manifest-path src-tauri/Cargo.toml   # Rust 单元测试
pnpm test                                          # vitest 前端单测
pnpm check                                         # svelte-check 类型检查

# 4) 打包（Windows 产出 NSIS 安装包，产物在 target/release/bundle/）
pnpm tauri build

# 5) 重新生成全套应用图标（源文件为根目录 app-icon.png）
pnpm tauri icon app-icon.png
```

> Rust 构建产物统一在项目根 `target/`（由 `.cargo/config.toml` 指定）；前端构建产物在 `dist/`。
>
> `tauri dev` 的 `beforeDevCommand` 不是直接 `pnpm dev`，而是 `node scripts/dev-server.mjs`：先探测 5173 上是否已是本项目的 dev server（拉取页面并校验其中是否引用了 `/src-svelte/main.ts`，避免误复用别的站点），是则直接退出复用，否则才拉起 `pnpm dev`。因为 vite 配了 `strictPort: true`（`devUrl` 写死 5173，改端口反而更糟），重复启动会报错并带崩 `tauri dev`。

---

## 从 v1 升级（数据自动迁移）

v2 首次启动时自动完成迁移，无需手动操作：

- **迁移来源**（按顺序查找，找到即止）：v1 便携模式 `<v2安装目录>\data\.user` → v1 标准模式 `%LOCALAPPDATA%\NewGAN Manager\.user`。
- **迁移目标**：v2 便携数据目录 `<安装目录>\data\.user`（开发调试时为 `<项目根>\data\.user`），整目录复制，Profile、组、RTF 设置、映射快照全部保留；旧版单档 Profile 格式自动升级为按组快照的新格式（v2 早期版本的扁平 `.user` 布局同样会在启动时自动迁移）。
- 迁移是**复制**而非移动，v1 安装不受影响，可随时回退。
- 产物 100% 兼容：`config.xml` 生成规则逐字节一致（同一模板、`r-` 前缀规则）、`.user/` 数据结构不变、14 个种族目录名不变、备份命名与保留策略不变。

---

## 应用数据与配置文件

只读映射表在编译期嵌入二进制（内容与 v1 `.config/` 完全一致）；用户运行时数据**全部便携化**——开发调试写入 `<项目根>\data\`，正式安装后写入 `<安装目录>\data\`（不落在 C 盘 AppData）：

```
# 只读配置（源码位置，编译期嵌入）
src-tauri/assets/
  eth_cfg.json          国籍（英文三字码）→ 种族目录 对照表
  nat_translation.json  中文国籍 → 英文三字码 对照表（解析中文 RTF 用）
  config_template       config.xml 的固定头尾模板，[players] 为占位符
  standard2.json        标准二规则出厂默认（14 种族目录 + 国籍映射 + 种族码修正）
  standard3.json        标准三规则出厂默认（62 图像池 + 族群映射规则块）

# 用户数据目录（开发：<项目根>/data/；安装后：<安装目录>/data/）
.user/
  profiles.json           全局配置：存档列表与激活位（旧名 cfg.json 启动时自动迁移）
  No Profile.json         缺省档配置（不套子目录，直接放在 .user 根）
  <Profile>/
    <Profile>.json        该存档的组配置与 RTF 设置
    <快照词干>.xml         组映射快照（切换/激活存档时交换；词干为「graphics_<后段>_snapshot」式短命名）
rules/
  standard2.json          标准二规则的用户副本（首次启动从出厂默认导出，改完下次替换即生效）
  standard3.json          标准三规则的用户副本（同上）
webview/                  WebView2 浏览器数据（缓存/localStorage，已重定向，不写 C 盘）
newgan.log                运行日志（10 MB × 3 滚动）
```

---

## 头像包标准与抽图规则

程序扫描头像包根目录后自动判定它属于哪个「标准」，再用该标准配套的抽图模型给每名球员选图。一个标准 = 一套目录组织约定 + 一份规则 json + 一个抽图模型，三者互不干涉。

| 标准 | 包结构 | 抽图逻辑 | 规则文件 |
| :--- | :--- | :--- | :--- |
| 标准一 | 图片直铺在根目录，无子目录 | 完全随机（忽略国籍与种族码） | 无（没有可配数据） |
| 标准二 | 14 个种族/大区目录 | 国籍查表 → 种族码修正 → 目录池随机 | `standard2.json` |
| 标准三 | 细分目录（国籍级 + 族群级，62 个图像池） | 规则块检索（见下） | `standard3.json` |
| 无法识别 / 空包 | 目录名与任何标准的清单都不构成包含关系 | 递归全池随机（包内所有子/孙目录 + 根目录图片） | 无 |

判定只看目录名：包内目录**包含**某标准 `FolderList` 的全部条目 → 归该标准（多出的目录不进抽图池，只在「检查完整性」里列为多余）；包内目录全部**落在**某标准清单内但有缺失 → 也归该标准，状态为可修复，可一键创建缺失目录。

### 标准三：三级规则块

按优先级选出规则块，再取块里球员种族码（0–10）对应的那条规则：

1. **国家块**：规则表里以该国籍三字码为键（如 `BRA`）；
2. **族群块**：以该国基础池目录名为键（如 `Mestizo`、`Afrocaribbean`）；
3. **全局默认块** `_DEFAULT_`：其余所有国籍。

每条规则有四种形态：

| 写法 | 含义 |
| :--- | :--- |
| `"default"` | 用归属者自己的目录（三级块下都等于该国的基础池 `Ethnics[国籍]`） |
| `{"typeOfMapping":"directly","directory":"Spain"}` | 直接映射到单一目录 |
| `{"typeOfMapping":"random","directory":["Japan","China"]}` | 等概率随机取一个 |
| `{"typeOfMapping":"randomWithWeight","directory":{"ethnicities":["Mestizo","NorthernSA"],"weights":[66,33]}}` | 按权重随机（权重按总和归一，不要求恰好 100） |

解析出的目录若在头像包里不存在或为空，按两级回退处理：**先退到该国基础池** `Ethnics[国籍]`，仍不可用再按 `Fallback` 字段处理（`random` = 从包内递归全池随机抽，默认值；`skip` = 跳过该球员不生成映射）。

### 编辑规则

界面里点「规则配置」直接改并保存，或用资源管理器打开 `data/rules/` 改 `standardN.json`（改完下次替换即生效）。保存与加载都会做结构校验：版本必须是 4、规则块键必须是国籍码或清单内目录名、每个块必须 0–10 全枚举、规则指向的目录必须在 `FolderList` 内、加权两列必须等长且全为正数。非法内容不会写盘，加载失败会报错并指明文件路径（不静默回退出厂默认，避免用改坏的规则抽图而不自知）。

`data/rules/template2.json` 这类旧命名文件会在启动时自动改名续用；已作废的旧标准三规则文件会被直接删除。

### 新增一个标准

判定层完全数据驱动，新增标准不用改判定代码，六步即可：`assets/standardN.json` → `src-tauri/src/templates/standardN.rs`（规则结构体 + 校验 + 抽图模型 + 单测，只依赖 `templates/mod.rs` 的接口）→ `assets.rs` 内嵌 → `templates/mod.rs` 登记（配置字段、加载导出、`folder_lists()`、`for_layout` 分支、`rule_files()`、已实现标准清单）→ 前端加一个 chip。命名一律跟着标准号走：`standardN.rs` ↔ `standardN.json` ↔ 类型 `StandardNConfig` / `StandardNRule`。

注意各标准的 `FolderList` 不能互相包含，否则判定会先命中先赢；加载时会校验这一点并报错。

### 目录名不在清单里的老头像包

若包里的目录（例如 `Scandinavian`）不在标准三的 62 目录清单内，判定结果是「无法识别」，替换时走递归全池随机：仍然能出图，但不按国籍与种族码路由。想恢复精确路由有两条路——在 `data/rules/standard3.json` 的 `FolderList` 里补上你包里的目录名，或点「一键创建缺失目录」把清单补齐。

---

## 故障排查

出问题时请先附上日志 `newgan.log` 再提 issue。最快的定位方式：点状态栏 `日志` 打开抽屉，把级别选到 `DEBUG` 复现一次，点 **打开日志文件** 直接打开日志（文件在数据目录：开发时为 `<项目根>\data\newgan.log`，安装后为 `<安装目录>\data\newgan.log`）。

| 现象 | 原因与处理 |
| :--- | :--- |
| `RTF 文件格式无效` | 三种已知导出格式（英文经典 / 中文经典 / 新版扩展）都不匹配：表头、列序或分隔符不符。用本仓库 `views/` 视图重新导出；日志会写明识别到的语言 |
| `读取 RTF 失败` / `请先选择 RTF 名单` | 文件被移动、改名或未设置，重新在「当前任务」卡点 `选择RTF` |
| `标准N：缺少 N 个目录（已有 x/y）：…` | 头像包缺该标准要求的目录；点 `一键创建缺失目录`，或确认选的是头像包**根目录** |
| `无法识别头像包结构` | 子目录与任何标准的 `FolderList` 清单都不构成包含关系；替换时将递归全池随机。可在 `规则配置` 中把包内目录名补进清单恢复精确路由 |
| `标准三规则无效：…` / `标准规则文件无法解析：…` | `data/rules/standardN.json` 被改坏（版本不符、块缺码、目录不在 `FolderList` 内、加权两列不等长等）。错误信息会指明文件与具体键；也可删掉该文件让程序重新导出一份出厂默认 |
| 部分球员被跳过、没生成映射 | 该球员国籍码不在映射表中（如 FM 用了非英文三字码），提 issue 反馈；也可修改 `src-tauri/assets/eth_cfg.json` 后自行构建 |
| 中文 RTF 中国籍未翻译 | `nat_translation.json` 缺该条目，提 issue 反馈；也可修改 `src-tauri/assets/nat_translation.json` 后自行构建 |
| 名单里一个人都没换 | 勾了 `仅随机人` 但 RTF 无随机人标志列，或该列全为 `No`；关掉开关，或改用带标志列的视图导出 |
| 写 `config.xml` 失败 / 权限错误 | 以管理员（或 `sudo`）运行，或把头像包移到当前用户可写目录 |
| FM 里头像没变 | 未重新加载皮肤缓存；或 FM 图形缓存目录里存在旧的 `config.xml` 副本 |

---

## 致谢

上游项目与其贡献者：
**[Maradonna](https://community.sigames.com/profile/50821-maradonna/) (gestalt)** — 发起、编码、图像生成；
**Samaroy** — 协调、图像生成；
**[HRiddick](https://sortitoutsi.net/user/profile/137954)** — 图片清理与后处理；
**[Krysler76](https://community.sigames.com/profile/157461-krysler76/)** — FM 视图制作；
**Ayal**、**[Zealand](https://www.youtube.com/user/FMBaseOfficial)**、**ZeBurgs** — 图像生成。

头像包的种族划分与图片由 NewGAN 项目组产出，本仓库只提供映射工具，不含任何头像图片。

## 许可证

GPL-3.0，详见 [LICENSE](LICENSE)。
