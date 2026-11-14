# 抽图模型设计-标准三-兼容NG-regens头像包

> 适用头像包：按 NG Regens Manager 25 目录体系组织的头像包。内置 `FolderList` 共 126 个相对目录：
> 62 个男球员族裔目录、32 个 `WF/*` 女球员目录、18 个 `Staff/*/male` 与 14 个 `Staff/*/female` 目录。
> 规则：`src-tauri/assets/standard3.json`（出厂内置）与 `data/rules/standard3.json`（用户可编辑副本）；
> 实现：`src-tauri/src/templates/standard3.rs`；分发：`src-tauri/src/templates/mod.rs`；包判定：`src-tauri/src/facepack_scanner.rs`。

## 1. 配置结构（schema 与命名语义）

| 键 | 含义 |
|---|---|
| `FolderList` | 包判定、规则候选和同类兜底共用的目录白名单。多级路径统一写 `/`；清单外目录与包根图片不参与标准三抽图。 |
| `Ethnics` | 国籍三字码 → 本国男球员基础池目录。内置 233 国，值域为 58 个基础池；其值同时是 `"default"` 的解析结果。 |
| `DefaultCodes` | 国籍 → 该国 regen 可能的种族码候选。保留配置并校验国籍键集，**不参与抽图**。 |
| `EthnicityMappingRules` | 男球员规则表：块键 → 种族码 → 规则。块键可以是国籍码、族群目录名或 `_DEFAULT_`。 |
| `FolderMappingFemale` | 男球员基础池目录 → 一个或多个 `WF/*` 目录。 |
| `FolderMappingStaff` | 男球员基础池目录 → 一个或多个 `Staff/<区域>`；选图时再拼接 `/male` 或 `/female`。 |
| `FolderMappingStaffFemale` | `Staff/<男职员区域>` → `Staff/<女职员区域>` 的覆盖表；未覆盖的区域沿用原区域。 |
| `Version` | 配置格式版本，必须为 `4`。 |
| `Fallback` | 本仓库扩展字段：`random`（默认）或 `skip`，控制候选与一级回退都失败后的处理。 |

`FolderMappingFemale` 与 `FolderMappingStaff` 的源键是**男球员基础池目录名**，不是国籍码；`FolderMappingStaffFemale` 的源键则是 `Staff/<区域>`。三张表都是带 `serde(default)` 的可选键：缺省 `FolderMappingFemale` 或 `FolderMappingStaff` 时，相应类别无映射候选；缺省 `FolderMappingStaffFemale` 时只是不换区域，仍沿用 `FolderMappingStaff` 的区域并拼接 `/female`。内置数量 58 / 58 / 4 是数据画像，不是自定义配置的数量要求。

内置规则有 43 块：38 个国家块、4 个族群块和 1 个全局块。4 个族群块为 `Afrocaribbean`、`CaucasianNA`、`Mestizo`、`SouthConeSA`。`Afroamerican`、`IndigenousSA`、`MixedRace`、`NorthernSA` 是纯目的地：会被规则指到，但不作为任何国家的基础池。

### 1.1 四种规则值

| 配置形态 | 解析结果 |
|---|---|
| `{"typeOfMapping":"directly","directory":X}` | 单目录候选 `X`。 |
| `{"typeOfMapping":"random","directory":[...]}` | 多目录等概率候选。 |
| `{"typeOfMapping":"randomWithWeight","directory":{"ethnicities":[...],"weights":[...]}}` | 多目录加权候选。 |
| `"default"` | 当前主国籍的 `Ethnics[N]`；查不到国籍时该规则解析不出候选。 |

`"default"` 在国家块、族群块和全局块中没有第二套含义。族群块内它也可理解为“该族群自己的目录”，因为该块正是通过 `Ethnics[N]` 查到的。

内置权重写整数，但反序列化接受 JSON 数值；校验只要求两列等长、非空且权重全为正。抽样按实际权重和归一，不假设总和为 100，例如 `[66,33]` 的比例是 `66/99 : 33/99`。

## 2. 种族代码与人员类别

### 2.1 种族码 0–10

| 代码 | 含义 | 代码 | 含义 |
|---|---|---|---|
| 0 | 北欧/白人 | 6 | 太平洋岛民 |
| 1 | 地中海/西语裔 | 7 | 美洲原住民 |
| 2 | 中东/北非 | 8 | 澳洲原住民 |
| 3 | 非洲/加勒比 | 9 | 黑白混血 |
| 4 | 南亚 | 10 | 东亚 |
| 5 | 东南亚 | | |

抽图直接使用 `PlayerRecord.ethnicity_code`，不按 `DefaultCodes[N]` 另抽一个码。标准三只使用主国籍 `primary_nat`，第二国籍 `sec_nat` 不参与目录决策。

### 2.2 `PlayerRecord.kind` 的来源

| 数据来源 | 生成的类别 |
|---|---|
| 基础 7 列 RTF | 没有职位与性别列，固定为 `PlayerMale`。 |
| 拓展球员 RTF | 按性别生成 `PlayerMale` / `PlayerFemale`。 |
| 拓展职员 RTF | 按性别生成 `StaffMale` / `StaffFemale`。 |
| `.fm` 内存记录 | 按记录中的职员与女性标记生成同样的四种类别。 |

RTF 的性别文本判定复用 `image_ai::is_female`；拓展名单中空白或无法识别为女性的文本走男性分支。人员类别与种族码是两个独立输入：女球员和职员不使用男球员规则块给出的候选目录。

## 3. 抽图解析流程（端到端）

```text
输入：主国籍 N、种族码 c、人员类别 kind
owner = Ethnics[N]                 // 未收录时为空

男球员：由三级规则块得到 candidates，一级回退目标为 owner
其他类别：由 owner 查类别映射得到 candidates，一级回退目标为映射列表首项

1) 从 candidates 中筛掉不可用目录并选择一个目录，尝试取图
2) 失败则尝试一级回退目标
3) 仍失败则执行 Fallback：
   random → 在同类、清单内目录的图片中抽一张
   skip   → 返回 None
4) 成功时返回实际目录与图片词干，写出的 from = "实际目录/词干"
```

### 3.1 男球员：按「块 × 码」逐级解析

`resolve(N, c)` 的规则链依次为：

1. `EthnicityMappingRules[N]`：国家块，存在才加入链；
2. `EthnicityMappingRules[owner]`：族群块，存在才加入链；
3. `EthnicityMappingRules["_DEFAULT_"]`：全局块，结构校验要求必须存在。

沿链寻找第一条能为码 `c` 解析出候选目录的规则。**不是先选一个块再停在该块内**：块存在但缺该码，或者未知国籍使 `"default"` 无 `owner` 可用，都会继续向后查找。用户自定义块因此可以只配置部分数字码。

未收录国籍没有专门的立即跳过分支：若全局块能用 `directly` 等规则给出候选，仍可直中；否则进入统一回退链。内置全局块的码 9 是 `directly → MixedRace`。国家块里的 `c9: "default"` 仍指向本国 `owner`，不会特殊改写成 `MixedRace`。

### 3.2 女球员与职员：只取 `owner`，再换目录类别

```text
PlayerFemale：
  owner → FolderMappingFemale[owner] → WF/*

StaffMale：
  owner → FolderMappingStaff[owner] → Staff/<区域>/male

StaffFemale：
  owner → FolderMappingStaff[owner]
       → FolderMappingStaffFemale 覆盖区域（若存在）
       → Staff/<区域>/female
```

这些分支仍调用 `resolve` 取得 `owner`，但不消费其 `candidates`，所以种族码不决定非男球员的目标目录。映射列表为空表示无候选；单值是单目录候选，多值是等概率候选。女职员目标不可用时不退到男职员目录。

### 3.3 可用目录、候选选择与回退边界

目录 `d` 可用需同时满足：

- `d` 完整匹配 `FolderList` 中的一项；
- 建规则时该目录在递归扫描池中非空；
- 当前仍有剩余图片，或有已发图片可在禁重复模式下洗回。

`directly` 只检查单目录；`random` 在剩余可用目录中等概率选择；`randomWithWeight` 先剔除不可用目录，再按剩余权重重新归一。没有任何可用候选时才进入后续回退。

回退有两个实现细节：

- 男球员的一级回退是 `Ethnics[N]`，即本国基础池。
- 非男球员的一级回退变量被设为**类别映射列表首项**。当整个候选列表都不可用时，首项通常也不可用；真正提供其他目录的是后面的同类清单兜底。它不会退回原男球员基础池。

`Fallback = random` 时，目录类别由路径形态确定：无 `/` 为男球员，`WF/` 为女球员，`Staff/*/male` 为男职员，`Staff/*/female` 为女职员。兜底只在与记录同类的清单目录中进行，把这些目录的当前图片**展平后等概率抽一张**；不是先等概率抽目录，图片越多的目录被选中的概率越高。清单外的 `Scandinavian/`、包根图片和其他人员类别都不会被使用。

`Fallback = skip` 时返回 `RuleSkip`；`random` 下同类清单目录仍无可用图片时返回 `EmptyPack`。标准三的 Deck 会重洗，因此正常流程不会因“禁重复抽干”返回 `PoolExhausted`。

### 3.4 运行统计

| `stats()` 键 | 计数口径 |
|---|---|
| `owner_fallback` | 候选未出图后，一级回退成功；男球员即本国基础池回落。 |
| `pool_fallback` | 前两级未出图后，经同类清单兜底成功。 |
| `wf_family` | 女球员成功出图数，失败记录不计。 |
| `staff_family` | 男、女职员合并的成功出图数，失败记录不计。 |

映射结束时仅把非零统计写入 INFO 日志；男球员成功数不另设类别计数。

## 4. 禁重复与 Preserve 剔除

- **允许复用（`duplicates = true`）**：候选目录内有放回独立均匀抽取，池内容不变；若进入同类清单兜底，则在同类目录的图片全集上展平等概率抽取。
- **禁止复用（`duplicates = false`）**：每个目录维护“剩余牌 + 已发牌”的 Deck。建池时洗牌，依次发图；目录剩余牌发完后，将该目录已发牌重洗再继续。同类清单兜底会先洗回其中已发完的目录，再展平抽取。
- **禁重复的边界**：同一目录在同一轮内不放回，每张图使用次数最多差 1；人数超过图片数会重开下一轮，所以不是整批永久唯一，也不会仅因人数多于图片而跳过。
- **Preserve 剔除的触发条件**：仅当处理模式为 `Preserve` 且禁重复时，才把现有 `config.xml` 映射占用的图片从池中剔除；允许复用时不做这一步。
- **剔除方式**：按完整 `from = "目录/词干"` 精确匹配，同时从剩余牌和已发牌中移除，避免已发图在重洗后再次出现。不同目录的同名图互不影响。
- **图片身份**：扫描与输出都使用完整文件词干，例如 `a.1.png` → `a.1`。`PickedImage` 保留实际取图目录，规则候选、一级回退和同类兜底都按同一身份写入 `from`。

## 5. 头像包判定（标准归属）

判定与抽图分离，唯一入口是 `facepack_scanner::classify`。输入同时包含一级目录名和 `WF/x`、`Staff/r/male` 等多级相对路径；包根键、垃圾目录以及未在任何清单中登记的纯容器前缀不参与目录集合比对。

按标准登记顺序（标准二、标准三）进行两轮判断：

1. **超集轮**：包内目录包含某标准全部 `FolderList` 路径 → 判为该标准且 `complete = true`，允许额外目录。
2. **子集轮**：有效目录全部落在某标准清单内 → 判为该标准但 `complete = false`，前端可创建缺失目录。
3. 都不满足 → `Unknown`，改走独立的全池随机规则；没有子目录时，无根图为 `Empty`、有根图为 `Flat`。

因此，有效目录中同时出现“缺清单目录”与“清单外目录”时不会进入子集兜底，而是 `Unknown`。`complete` 只看目录路径是否存在，不要求目录非空；空目录可以算完整，但不会进入标准三的可用规则池。

`validate_registry` 要求任意两份标准清单的**交集为 0**，不只是“互不包含”，从加载层排除共享目录导致的先命中误归类。标准三被识别后，完整性报告中的 `missing` 按 126 个完整相对路径计算，`extra` 只列未配置且非纯容器的一级目录；额外目录不会被标准三主规则或兜底使用。

## 6. 规则文件、可编辑性与结构校验

- **内置默认**：`src-tauri/assets/standard3.json` 编译期内嵌；反序列化内置资产失败属于程序资产错误。
- **用户副本**：首次运行导出 `<数据目录>/rules/standard3.json`，已存在则不覆盖。
- **加载优先级**：用户文件存在则覆盖内置值，缺失则保留内置默认；随后执行标准三 `validate()` 与跨标准 `validate_registry()`。JSON 损坏或语义无效会报出文件路径，不静默回退。
- **编辑器保存**：先解析为 `Standard3Config`，再跑本标准 `validate()`，最后 pretty 格式化并原子写回；解析或校验失败不写盘。跨标准清单交集检查在整体规则加载时执行。

`Standard3Config::validate()` 的主要约束：

- `Version = 4`；`FolderList` 非空且不重复；必须存在 `_DEFAULT_` 块。
- `Ethnics` 的目标必须在清单内；`DefaultCodes` 的国籍键必须存在于 `Ethnics`。
- 规则块键必须是已配置国籍码、清单内目录名或 `_DEFAULT_`。
- `directly` / `random` / 加权规则引用的目录必须在清单内；`random` 与加权列表非空，加权两列等长且权重全为正。
- `FolderMappingFemale` 的目标必须在清单内且以 `WF/` 开头。
- `FolderMappingStaff` 的每个区域拼接 `/male` 后必须在清单内；`FolderMappingStaffFemale` 的源区域与目标区域都必须出现在 `FolderMappingStaff` 的区域值中。

块内缺少 `"0"`–`"10"` 中的某个码是合法的，含义是继续下一级。`_country_specific_default_` 等非数字码不参与三级查找，加载校验会记录 WARN；其规则值本身仍会经过目录、列表和权重校验。把 `_country_specific_default_` 当作独立**块键**则仍是结构错误。

## 7. 来源、确认状态与内置数据

### 7.1 已采用的反推结论

- **`"default"`**：统一解析为当前主国籍的 `Ethnics[N]`；只有全局块码 9 固定指向 `MixedRace`。
- **`_country_specific_default_`**：不作为额外执行层。扰动实测中，“先消费它再穿透”的池覆盖率为 97.90%，低于同口径基线 99.69%，不支持优先执行该键的解释。
- **禁重复模型**：NG 的 172,494 条产物中，94 个池均呈现“每张图使用次数最多差 1”的 Deck 特征，且映射不越出对应目录清单与人员类别，因此本实现采用每目录发完重洗和同类隔离。

这些结论描述兼容依据；实际运行行为以 §3–§6 的代码路径为准。

### 7.2 当前内置数据画像

| 项目 | 内置值 |
|---|---:|
| `FolderList` | 126 = 62 男球员 + 32 女球员 + 18 男职员 + 14 女职员 |
| `Ethnics` | 233 国 → 58 个基础池目录 |
| 规则块 | 43 = 38 国家 + 4 族群 + 1 全局 |
| 类别映射 | 女球员 58 条、职员 58 条、女职员换区 4 条 |
| 男球员完整矩阵 | 233 × 11 = 2563 组，全部可解析到清单内目录 |
| 62 个男球员目录 | 全部至少被一条规则指到，无死目录 |

2563 组静态展开的规则形态与命中层级：

| 维度 | 分布 |
|---|---|
| 规则形态 | `default` 1833、`directly` 591、`random` 128、加权 11 |
| 命中层级 | 国家块 418、族群块 451、全局块 1694 |
| 权重和 | 99 共 4 条，100 共 7 条；运行时均按实际总和归一 |

## 8. 与标准二的当前实现差异

| 维度 | 标准二 | 标准三 |
|---|---|---|
| 目录范围 | 14 个一级种族/大区目录 | 126 个相对目录，含男球员、女球员与男女职员 |
| 人员范围 | 球员路径；职员返回 `StaffNotApplicable` | 按 `kind` 分四类路由与兜底 |
| 国籍输入 | 主国籍 + 第二国籍 | 只用主国籍，第二国籍不参与 |
| 决策模型 | `Ethnics` + `CodeRules.prefer/fallback` + 国籍同名目录优先 | 三级规则块；非男球员改走三张类别映射表 |
| 缺目录时 | 目标池不可用后，在自身 `FolderList` 全池回落 | 候选 → 一级回退 → 同类清单兜底；兜底可配置为跳过 |
| 禁重复 | 抽中即移除，池可耗尽并返回 `PoolExhausted` | 每目录 Deck 发完重洗，不因人数超过池容量而耗尽 |
| 分发入口 | `PackLayout::Standard { standard: 2 }` | `PackLayout::Standard { standard: 3 }`，使用递归扫描池 |

两套规则目前都执行各自的语义 `validate()`；整体加载还要求两份 `FolderList` 交集为 0。

## 9. 本仓库实现与验证

- 配置与算法：[standard3.json](../src-tauri/assets/standard3.json)、[standard3.rs](../src-tauri/src/templates/standard3.rs)。
- 加载、登记、分发与共享抽样工具：[mod.rs](../src-tauri/src/templates/mod.rs)。`for_layout` 命中标准三时，以 `scan.recursive_pools` 构造 `Standard3Rule`。
- 扫描与判定：[facepack_scanner.rs](../src-tauri/src/facepack_scanner.rs)；人员类别：[rtf_scanner.rs](../src-tauri/src/rtf_scanner.rs)；统计与跳过日志：[face_mapper.rs](../src-tauri/src/face_mapper.rs)。

常态测试覆盖：

1. `resolve_covers_chain_defaults_and_unknown_nationality`、`mapping_rule_roundtrips_all_four_shapes`：三级解析、未知国籍与四种规则读写。
2. `validate_rejects_broken_configs`：结构约束、类别映射与特殊键校验。
3. `builtin_rules_resolve_every_country_and_code`：2563 组全解析、规则形态分布与 62 目录覆盖。
4. `pick_walks_the_fallback_chain_then_skips`、`fallback_random_never_leaves_the_folder_list`：两级回退、跳过原因与同类清单边界。
5. `sampling_models_cover_deck_and_iid`、`pool_state_covers_usable_and_exclude_used`：有放回、Deck 重洗、可用性及 Preserve 双池剔除。
6. `family_routing_covers_female_and_staff_dirs`、`staff_rosters_from_real_rtfs_land_in_their_own_family`：四类路由、女职员换区、真实职员名单与成功计数。
7. `pick_weighted_rule_only_uses_available_dirs`：不可用目录剔除与剩余权重归一。
8. `real_shape_pack_scans_classifies_and_picks_without_leaving_the_list`、`dotted_filename_has_single_identity_in_the_pool`：真实目录形态全链路与带点文件名身份。
9. `standard3_rule_file_registration`、`ensure_then_load_roundtrip_and_corrupt_reports_path`：规则保存、首启导出、不覆盖用户文件与损坏路径报错。

2026-09-10 的社区标准三真实包实测中，男球员 2563 组全部直中规则候选、零回退，禁重复轮的 `from` 全部唯一，且每条映射都能定位到真实文件。对任意本机包可运行冒烟测试：

```powershell
$env:NEWGAN_REAL_PACK_DIR = "D:\path\to\facepack"
cargo test --manifest-path src-tauri/Cargo.toml real_pack_smoke -- --ignored --nocapture
```

冒烟测试硬断言“人人有图、图真实存在”；直中/回退占比与清单内空目录只打印，不把合理的缺目录回退判为失败。
