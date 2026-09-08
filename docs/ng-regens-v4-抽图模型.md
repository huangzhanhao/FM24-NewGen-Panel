# NG Regens Manager v4 抽图模型（反推）

> 来源：项目记忆 `project-ng-regens-v4-rebuild.md`；依据 v4 配置编辑器截图（`v4_temp/国家映射|图像池|高级规则`）与重建的 `base_config.json`（Version 4）反推。
> 旧版参照：`C:\Program Files (x86)\NG_Regens Manager 25\data\flutter_assets\assets\base_config.json`（Version 3）。

## 1. 配置结构（v4 schema 与命名语义）

| 键 | 含义 |
|---|---|
| `FolderList` | 图像池（62 个）。其中 IndigenousSA / MixedRace / NorthernSA / Afroamerican 为“纯目的地”：只被路由进入，从不作为国家基础池。 |
| `Ethnics` | 国家码 → 基础池文件夹；同时决定该国在无国家规则块时归属哪个族群块。 |
| `DefaultCodes` | 国家码 → 该国 regen 会落到的种族代码候选池（0–10 的子集）。 |
| `EthnicityMappingRules` | 规则表。键分三类：国家码；族群名（仅 4 个：Afrocaribbean / CaucasianNA / Mestizo / SouthConeSA，编辑器中列于“种族群组”）；`_DEFAULT_`（编辑器“全局默认值”）。 |

规则值的四种形态与编辑器文案对应：

| 形态 | 编辑器文案 | 解析 |
|---|---|---|
| `{"typeOfMapping":"directly","directory":X}` | 直接映射到: X | 取单一文件夹 X |
| `{"typeOfMapping":"random","directory":[...]}` | 随机映射到: A, B | 列表等概率 |
| `{"typeOfMapping":"randomWithWeight","directory":{"ethnicities":[...],"weights":[...]}}` | 加权 | 按整数百分比加权（v4；v3 为小数） |
| `"default"` | 使用此群组的文件夹 / 使用指定的国家文件夹 | 见下“归属者自身文件夹” |

**`"default"` 的统一语义 = 当前规则块“归属者”自己的文件夹：**
- 国家块内 → 该国基础池 `Ethnics[国家]`；
- 族群块内 → 该族群同名文件夹（族群名即文件夹名）；
- `_DEFAULT_` 内 → 被查询国家的 `Ethnics[国家]`。

## 2. 种族代码 0–10 语义

| 代码 | 含义 | 代码 | 含义 |
|---|---|---|---|
| 0 | 北欧/白人 | 6 | 太平洋岛民 |
| 1 | 地中海/西语裔 | 7 | 美洲原住民 |
| 2 | 中东/北非 | 8 | 澳洲原住民 |
| 3 | 非洲/加勒比 | 9 | 黑白混血 |
| 4 | 南亚 | 10 | 东亚 |
| 5 | 东南亚 | | |

## 3. 抽图解析流程（端到端）

```
输入: 国籍 N, 种族代码 c (c 取自 DefaultCodes[N] 或 FM 存档)

1) 选规则块（优先级从高到低）:
   ① Rules[N]                      国家级块（若存在，0–10 全枚举）
   ② Rules[Ethnics[N]]             族群级块（仅当基础池是 4 族群之一）
   ③ Rules["_DEFAULT_"]            全局块
2) 取该块中代码 c 的规则
3) 解析规则 → 文件夹 F:
   directly → F = directory
   random   → F = 等概率抽自 directory[]
   weighted → F = 按 weights 加权抽自 directory.ethnicities[]
   "default"→ F = 归属者自身文件夹（见上）
4) 从 F 中随机抽一张真人脸贴给该 regen
```

全局块不变量：**c = 9 恒为 MixedRace**，其余代码回国家基础池；这是全配置唯一的跨国家硬编码。

## 4. 结构性不变量（可用于校验/补数据）

- 任意 (N, c) 必可解析（三级兜底保证无悬空）；编辑器「验证映射」页即在检查此条。
- 国家级块与族群块若存在则 0–10 全列，无缺口（v4 已取消 v3 的 `_country_specific_default_`）。
- 文件夹要么是某国基础池，要么是纯目的地。

## 5. 相对 v3 的变化点

- 族群块 2 → 4 个（新增 CaucasianNA、SouthConeSA）；Mestizo 的 c9 由 MixedRace 改为 NorthernSA。
- 取消 `_country_specific_default_`，国家块改为 0–10 全枚举。
- 移除 Scandinavian，拆为 Denmark / Norway / Sweden 等单国池；新增 13 个文件夹。
- 权重单位由小数（0.6）改为整数百分比（60）。
- 新增国家 CIS / URS / ISR 等；移除 AXL / GRL / LBN / NIU / PLW / SDN / SIN / SWZ / VAT。

## 6. 确认状态

- 已确认：全局默认值（c9→MixedRace）；4 个族群块；加权权重为整数（用户核对）。
- 仍属推断（高置信、未逐国实证）：国家块中 c9 的 “default → MixedRace” 译法。
- 编辑器中的国家数字 ID（如 ARM-754）为显示用途，未写入配置。

## 7. 本仓库实现状态（2026-09-08）

模型已落地为**标准三**的抽图规则：配置 `src-tauri/assets/standard3.json`（内容即本文档反推的 Version 4 表）、
实现 `src-tauri/src/templates/standard3.rs`、设计与验收清单 `docs/模板三抽图模型重构计划.md`、
用户视角说明 README「头像包标准与抽图规则」。

**实测到的数据不变量**（233 国 × 11 码 = 2563 组合，已写成回归测试 `builtin_rules_resolve_every_country_and_code`）：

- 全部组合可解析，目标目录 100% 落在 62 项 `FolderList` 内；62 个目录都至少被一条规则指到（无死目录）。
- 形态分布：`default` 1833 / `directly` 591 / `random` 128 / `weighted` 11。
- 选块层级：`_DEFAULT_` 1694 / 族群块 451 / 国家块 418。
- 加权权重和只有 **99 与 100** 两种（如 `DOM/1 = [66,33]`）→ 实现按 Σ 归一，不假设 100。

**对 §6 那条推断项的取舍**：实现采用 §1/§3 的统一语义 —— `"default"` 一律解析成**归属者自己的目录**
（国家块与 `_DEFAULT_` 下都等于 `Ethnics[国籍]`，族群块下等于该族群目录，三者数值恒相同）。
因此国家块里 `c9: "default"`（如 ARM）解析为 `Armenian`，**不是** MixedRace；
只有 `_DEFAULT_` 块的 `c9` 是写死的 `directly → MixedRace`。
若日后实证发现上游对国家块 c9 另有特殊处理，改动点在 `templates/standard3.rs` 的 `to_candidates`（一处）。

**本仓库的两处扩展**（上游配置里没有、也不写回配置文件）：

1. `Fallback` 字段（`random` / `skip`，默认 `random`）：规则解析出的目录在头像包内缺失或为空时的第二级兜底。
2. 两级回退链：候选目录不可用 → 先退本国基础池 `Ethnics[N]` → 仍不可用 → 按 `Fallback`。
   上游假定包内目录齐全，本程序要面对只收录部分目录的社区包，故补这条链。

**结构校验**：`Standard3Config::validate()` 把 §4 的不变量变成硬约束（`Version` 必须为 4、块键必须是国籍码或清单内目录名、
每块 0–10 全枚举、规则目录必须在 `FolderList` 内、加权两列等长且全为正），
保存规则文件与加载规则时都会跑；`_country_specific_default_` 这类异体键会被判为「未知规则块键」，不做兼容。

**`DefaultCodes` 的角色**：保留在配置里但不参与抽图 —— 本程序的种族码来自 RTF 名单中球员自己的字段，
不需要按国家抽一个码；该字段仅在结构校验里与 `Ethnics` 的键集对照。
