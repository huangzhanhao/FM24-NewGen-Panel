# NewGAN Manager

> 给 Football Manager 的 NewGAN / 随机人批量匹配头像（facepack）的桌面工具。
> 读取 FM 导出的 RTF 名单 → 按「国籍 + 种族」自动挑一张头像图片 → 生成 FM 能识别的 `config.xml`。

![Tauri Build](https://github.com/huangzhanhao/NewGAN-Manager-ByHzH/actions/workflows/tauri.yml/badge.svg)

**当前版本 v2.0.0** ｜ Fork 自 [Maradonna90/NewGAN-Manager](https://github.com/Maradonna90/NewGAN-Manager)（上游已停止维护）｜ v2 基于 **Tauri 2（Rust 后端）+ Svelte 5（前端）** 重构：无需 Python / .NET 运行时，大名单（30 万行级）不卡顿，界面按操作旅程重排。

---

## 它解决什么问题

NewGAN（随机人）默认没有头像。手动给成百上千名随机人一张张贴图不现实。这个工具把 FM 导出的球员名单（RTF）和一套按种族分目录的头像包对应起来，**批量**生成 FM 读取的 `config.xml`，让随机人自动"长脸"。

**典型使用一句话：** 导出 RTF → 选头像包目录 → 点 `Replace Faces` → 回 FM 重载皮肤。

---

## 主要功能

- **多 Profile / 组管理**：不同存档各建一套 Profile；一个 Profile 下可关联多个头像包目录（组），每组独立记录 RTF 与映射快照。切换 Profile / 组时自动把当前 `config.xml` 存回快照、再把目标快照写回头像包目录。
- **中文数据库支持**：可识别中文表头、中文国籍的 RTF。国籍中文名经 `nat_translation.json` 翻译回英文三字码再查种族映射——**无需把 FM 切成英文**。
- **只处理随机人**：`仅随机人` 开关按 RTF 最后一列（`Yes/No`、`是/否`）过滤，只给随机人换脸，被过滤者不写入 `config.xml`。
- **三种替换模式**：`Preserve` 保留已换过的只补新的；`Overwrite` 对名单内球员全部重抽、名单外保留；`Generate` 无视现有 `config.xml` 从零生成。
- **名单预览（v2 新增）**：RTF 解析结果分页预览表，支持排序 / 搜索；数据保留在 Rust 侧，前端只取当前页，大名单也不卡。
- **单个球员微调**：结果区切到 `单人微调`，输入 UID 回车 → 预览当前头像与球员信息 → 选一张图 `Replace it`，只改写这一条映射。
- **自动备份**：每次写入前生成 `config备份_YYYYMMDD-HHMMSS.xml`，每个头像包目录最多保留最近 10 份。
- **进度与取消**：解析 / 映射 / 写文件全部在 Rust 后台线程执行，进度经事件流推送；执行中界面保持流畅，可随时 `取消`。
- **应用内日志**：状态栏常驻状态与迷你进度，`日志` 按钮带错误角标；抽屉内可按级别筛选、只看某一级、一键打开日志文件；日志文件 10 MB × 3 份滚动。
- **缺失目录检查与一键创建**：配置卡可主动检查头像包 14 个种族目录是否齐全，缺失时一键批量创建（执行前仍兜底校验）。

---

## 快速开始（普通用户）

1. 到 **Releases** 下载对应平台安装包，安装运行（无需安装 Python / .NET）。
   - **Windows**：`NewGAN-Manager_2.0.0_x64-setup.exe`（NSIS，安装到当前用户目录，无需管理员权限）。系统使用自带 WebView2（Win10 1809+ / Win11 一般已内置）；若缺失，安装包会联网引导安装。
   - **macOS / Linux**：下载 CI 构建的对应平台产物。
2. 把仓库里的 `views/`、`filters/` 两个文件夹复制到 FM 用户目录：
   - Windows：`文档\Sports Interactive\Football Manager 20XX\`
   - macOS：`~/Library/Application Support/Sports Interactive/Football Manager 20XX/`
3. 按下面的[使用流程](#使用流程)操作。

---

## 使用流程

### 1. 在 FM 中导出 RTF
安装 `views/` 后，在 FM 里使用 **`SCRIPT FACES`** 的 `player search` / `squad` / `shortlist` / `staff` 视图（配合 `filters/` 里的 `is newgen search filter` 只看随机人），把列表**另存为 RTF**。

导出的 RTF 至少需要以下列，**顺序固定**：

```
| UID       | Nat  | 2nd Nat | Name        | 头发长度 | 发色 | 种族码 |
| 2000472008| ESP  | USA     | Pepe Sáenz  | 1        | 12   | 1      |
```

额外支持：
- **中文表头 / 中文国籍**（如 `| 编号 | 国籍/地区籍 | … | 姓名 |`，国籍写「美国」这类中文名）；
- **14 列扩展格式**，多出：肤色码、Face、俱乐部、年龄、身高、体重、`是否为随机人`——只有带最后一列时，`仅随机人` 过滤才生效。

> UID 默认按随机人处理并加 `r-` 前缀；当 `是否为随机人 = No / 否` 时改回纯数字 UID。

### 2. 侧栏：准备 Profile 与头像包
- 点 `Profile` 区的 ＋ 新建并选择一个 Profile（`No Profile` 为占位状态，不能删除）。
- 点 `头像包（组）` 区的 ＋ **关联头像包根目录**（其下是 `African`、`Asian`… 等种族子目录），一个 Profile 可关联多组；点组名切换当前组，组条目上显示 RTF 是否已设置。

### 3. 配置卡：检查完整性并设置 RTF
- 点 **`检查完整性`**：校验 14 个种族目录是否齐全，缺失时可 **`一键创建缺失目录`**。
- 点 **`选择文件`** 挑选上一步导出的 RTF。

### 4. 执行面板：选模式与开关后执行
- **模式**：`Preserve` / `Overwrite` / `Generate` 三个 chip 任选，含义见[主要功能](#主要功能)。
- 开关：`允许复用图片`（关闭时 Preserve 模式会先剔除 `config.xml` 已用图片）、`仅随机人`、`写入前备份`。
- 未就绪时按钮禁用并提示缺什么（选择 Profile · 选择头像包 · 设置 RTF 名单）。
- 点击 **Replace Faces**，进度条与状态文字实时推进，执行中可 `取消`，完成弹出 `Finished! :)`。

### 5. 结果区：名单预览与单人微调
- **名单预览**：刚解析的名单分页表格，可排序 / 搜索。
- **单人微调**：输入该球员 UID 回车 → 查看当前头像 → `Browse` 选一张新图（路径需形如 `...\<种族目录>\<图片名>.png`）→ **Replace it**，仅改写这一条映射。

### 6. 回 FM 生效
`偏好设置 → 界面` 中**取消再勾选皮肤、重新加载皮肤缓存**（Reload skin cache），必要时清缓存后重启 FM。

### 7. 状态栏与日志
底部状态栏常驻显示当前状态（Ready / 进度文字）；`日志` 按钮出现红色角标表示有错误，点开抽屉可按级别筛选、只看某一级、打开日志文件或清空显示。

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
- FM 种族码（0–10）如何参与修正选池，见 [种族特征分类说明.md](docs/种族特征分类说明.md) 与 [种族代码示例.xlsx](docs/FM%20NewGAN种族代码示例.xlsx)；实现见 [`face_mapper.rs`](src-tauri/src/face_mapper.rs)。
- 国籍 → 种族的对照表在 [`src-tauri/assets/eth_cfg.json`](src-tauri/assets/eth_cfg.json)（编译期嵌入，内容与 v1 一致），修改后需重新构建。

---

## 项目结构

```
├── src-svelte/                  # Svelte 5 前端
│   ├── main.ts / App.svelte / app.css
│   └── lib/
│       ├── types.ts / api.ts / dialog.ts   # IPC 类型与封装、统一错误弹窗
│       ├── stores/            # app / log / ui（runes 状态）
│       └── components/        # Sidebar / SetupCard / ReplacePanel / ResultsSection /
│                               # PlayersTable / PlayerViewer / StatusBar / LogDrawer
├── src-tauri/                  # Rust 后端
│   ├── Cargo.toml / build.rs / tauri.conf.json
│   ├── assets/                 # 嵌入资源：eth_cfg.json / nat_translation.json / config_template
│   ├── capabilities/default.json
│   ├── icons/                  # 应用全套图标（OS 窗口 / 安装包用，由 app-icon.png 生成）
│   └── src/
│       ├── main.rs / lib.rs    # 入口 + Tauri Builder 装配（含便携数据目录与 v1 迁移）
│       ├── assets.rs           # 编译期加载映射表
│       ├── json_store.rs       # JSON 原子读写 + 损坏回退
│       ├── paths.rs            # 路径规范化（目录 key / 快照路径）
│       ├── rtf_parser.rs       # ← core/RtfParser.py（校验/解析/中文翻译）
│       ├── xml_parser.rs       # ← core/XmlParser.py（读/写/单条替换/备份轮换）
│       ├── profile_manager.rs  # ← core/ProfileManager.py（档/组 CRUD、快照交换）
│       ├── face_mapper.rs      # ← core/FaceMapper.py（种族修正 + 三模式映射）
│       ├── replace_flow.rs     # ← services/replace_service.py（替换流水线）
│       ├── logger.rs           # ← core/NewGanLogManager.py（环形缓冲 + 事件）
│       └── commands.rs         # ← services/* + 对话框原语（17 个 Tauri 命令）
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

---

## 从 v1 升级（数据自动迁移）

v2 首次启动时自动完成迁移，无需手动操作：

- **迁移来源**（按顺序查找，找到即止）：v1 便携模式 `<v2安装目录>\data\.user` → v1 标准模式 `%LOCALAPPDATA%\NewGAN Manager\.user`。
- **迁移目标**：v2 便携数据目录 `<安装目录>\data\.user`（开发调试时为 `<项目根>\data\.user`），整目录复制，Profile、组、RTF 设置、映射快照全部保留；旧版单档 Profile 格式自动升级为按组快照的新格式。
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

# 用户数据目录（开发：<项目根>/data/；安装后：<安装目录>/data/）
.user/
  cfg.json              Profile 列表与激活状态（仅一个为 true）
  <Profile>.json        单个 Profile 的头像包目录（组）与 RTF 设置
  <Profile>/
    <sanitized_dir>.xml 组映射快照（切换 Profile / 组时交换）
newgan.log              运行日志（10 MB × 3 滚动）
```

---

## 故障排查

出问题时请先附上日志 `newgan.log` 再提 issue。最快的定位方式：点状态栏 `日志` 打开抽屉，把级别选到 `DEBUG` 复现一次，点 **打开日志文件** 直接打开日志（文件在数据目录：开发时为 `<项目根>\data\newgan.log`，安装后为 `<安装目录>\data\newgan.log`）。

| 现象 | 原因与处理 |
| :--- | :--- |
| `The RTF file is invalid!` | RTF 表头/列顺序不符，或用了其他分隔符导出。用本仓库 `views/` 视图重新导出；日志会写明识别到的语言 |
| `The RTF file doesn't exist!` | 文件被移动或改名，重新在配置卡中 `选择文件` |
| `缺少 N 个种族目录` | 头像包缺该种族目录；点 `一键创建缺失目录`，或确认选的是头像包**根目录** |
| 大量 `primary nationality ... is None - skipping` | 映射表缺该国籍码（如 FM 用了非英文三字码），提 issue 反馈；也可修改 `src-tauri/assets/eth_cfg.json` 后自行构建 |
| 中文 RTF 中国籍未翻译 | `nat_translation.json` 缺该条目，提 issue 反馈；也可修改 `src-tauri/assets/nat_translation.json` 后自行构建 |
| 名单里一个人都没换 | 勾了 `仅随机人` 但 RTF 无「是否为随机人」列，或该列全为 `No`；关掉开关，或改用 14 列视图导出 |
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
