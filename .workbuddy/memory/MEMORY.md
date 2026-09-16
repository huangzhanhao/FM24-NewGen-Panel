# 项目长期记忆 · NewGAN Manager v2

## 用户偏好（前端）

- **不要自造状态判据，先照已有实现抄。** 需要判断「数据是否可读/是否为空」时，直接用 appStore 里的
  全局标记 + 既有组件的同款写法（快照预览：`readable = 展示档快照路径 === appStore.snapshotReadPath`；
  名单预览：`readable = appStore.rosterReady`），不要从查询结果反推（如「total === 0 就是缓存为空」）
  而新增 `hasCache` 之类的派生变量——用户会明确回退这类实现。
- 「后端有没有数据」这类事实只在 store 里维护一个标记，由**所有**填充/清空后端缓存的入口负责置位。
- 改动尽量小、贴合现有代码风格（逐行中文注释解释「为什么」，协议值不入 text.ts）。

## 关键约束

- 窗口默认 1000×640，由 `src-tauri/src/lib.rs` 的 `inner_size` 决定（不在 tauri.conf.json 里）。
- 结果区三 Tab 共用 `.table-toolbar`（app.css）：它是 nowrap flex，靠 `.search`（flex 0 1 320px，
  min-width 120px）收缩让位；分页器 `TablePager` 的 `.pager` 是 `nowrap + flex-shrink: 0`，不再折行。
- 后端 `rtf_cache`（名单预览的数据源）由两处写入：`commands.rs::scan_rtf`（成功才填）与
  `commands.rs::start_replace` 的收尾（`*rtf_cache = result.rtf_data.clone()`，**失败/取消时 rtf_data 为空
  → 等于清空缓存**）。前端对应标记：`lastRtfScan`（扫描摘要，由 SetupCard 的「扫描RTF」**或**替换成功时
  随 `replace:finished` 载荷 `{ result, roster }` 回传）+ `rosterReady`（可读标记，扫描成功或替换 finished
  置 true，失败/取消置 false）+ `rosterVersion`（重载）。摘要口径单点在 `rtf_scanner::RosterSummary::from_records`。
- 后端 `snapshot_cache`（快照预览的数据源）同样两处写入：`commands.rs::read_snapshot`（用户点「读取快照」）
  与 `commands.rs::start_replace` 收尾的 `sync_group_snapshot`（成功时把 `img_dir/config.xml` 拷到
  `group_xml_path(user_dir, cur_prf, cur_group)` 快照位置再重解析；失败/取消不动）。前端对应：
  `snapshotReadPath`（SetupCard::readSnapshot **或** 替换成功时取 `snapshot.configXml.path`）+ `snapshotVersion`。
  缓存按路径命中，因此「文件变了但路径没变」时必须主动重写缓存，否则预览是旧数据。
- 快照 XML（`.user/<档>/<stem>_snapshot.xml`）是 config.xml 的存档副本：切档时 `save_group_xml` 存、
  切入时 `restore_group_xml` 取；替换收尾的同步只是提前做同一件事，不改变语义。
- Rust 侧 `RunResult`（replace_flow）是「事件载荷的来源」：新增要回传前端的字段就加在这里，
  命令层 `start_replace` 负责拼 `replace:finished` 的 JSON，前端 store 只读不猜。
- 状态栏进度有**两条通道**（别合并成一条，但**载荷形状必须一致**）：替换流水线走 `replace:progress`
  → `appStore.progress`；「检查完整性/扫描头像/扫描RTF/读取快照」走 `task:progress` → `appStore.task`。
  两者都是 `{status, value, current, total}`——形状一致，StatusBar 才能用 `shown = showTask ? task :
  progress` 一套逻辑渲染百分比与「n/total」计数。纯阶段锚点传 current/total = 0（前端只显示百分比）。
  要给某个扫描类命令加进度：命令签名加 `app: AppHandle` + `TaskProgress::new(&app, "任务名")`，
  在对应扫描器上加 `*_progress(..., &mut dyn FnMut(usize, usize))` 变体并先算出总数当分母。
  四个扫描命令都必须是 **async fn**（同步命令跑在主线程，会把 emit 与重绘一起堵住）。
- 替换流水线的阶段内进度：`replace_flow` 用 `ProgressFn = &mut dyn FnMut(&str, u8, usize, usize)`，
  阶段锚点沿用 v1 的 0/20/40/80/90/100，阶段内部用 `item()` 把 done/total 折算进
  `STAGE_RTF(20,40)/STAGE_INDEX(40,46)/STAGE_XML(46,52)/STAGE_PLAYERS(52,80)`。
  **区间必须相邻不重叠**，否则后一阶段的 0% 会把进度条拉回去。face_mapper 只报原始计数
  （`MapStep::{ReadXml, Players}` + 步骤起始的 `(step, 0, 0)` 锚点），百分比折算在 replace_flow。
- `replace` 通道的事件去重器是 `commands.rs::ProgressDedup`，键 = **(阶段文案, 百分比)**。
  **键里绝不能带 current/total**——阶段内逐项上报时计数每项都变，带上去等于没去重、刷爆 IPC。
  有单测 `progress_dedup_keys_on_status_and_value_only` 守这条。
- `runTask(label, fn)` 是 task 通道的**唯一生命周期管理者**，契约四条：
  1) 进入时 `task = { status: label, value: 0, current: 0, total: 0 }`（否则第一条事件到达前会闪出
     上一个任务的名称与计数）；2) 命令成功后补 `value = 100, current = total`（小包几毫秒、后端来不及
     `finish()`，不补就一闪而过等于没反馈；失败则保持 0 不谎报）；3) `finally` 复位 `taskRunning`；
  4) 收尾排 `scheduleBarClear()`：**跑完的 100% 在状态栏留 `BAR_LINGER_MS`（20 秒）再清空**，
     新任务开始 / `replace:progress` 推进时撤掉旧定时器，定时器回调里再判一次
     `taskRunning || replacing`。`replace:finished` 也排一次清空。
  前端所有调用四条命令的地方都要经它（SetupCard 四个按钮 + `fixMissing` 复检 + ReplacePanel 替换前兜底校验）。
- StatusBar 取值的优先级：`replacing` > 任务通道。`showTask = !replacing && (taskRunning || task.value > 0)`
  ——`taskRunning=false 但 value>0` 表示「刚跑完还没到清空时刻」，进度条保留 100% 让用户看到
  （即「● 就绪 ▬▬▬ 100%」）。因此 `replace:progress` 监听里必须把 `task` 清零，否则取消替换后
  状态栏会退回显示旧任务的 100%。
  前端单测在 `src-svelte/lib/stores/app.store.test.ts`，改动 runTask / StatusBar 先跑它。
- 无进度版的扫描/解析包装函数（`facepack_scanner::scan_pack`、`RtfScanner::parse_roster`）在生产
  已无调用方，保留给测试用，标了 `#[cfg(test)]` —— 这样发布构建不报 dead_code，测试又不用到处写
  空回调。（更早的 `check_integrity`/`scan_and_total` 当时是直接删掉并改测试调用点，两种做法都行，
  以「少改测试」优先时选 cfg(test)。）
- **图片词干只有一个口径**：`facepack_scanner::image_stem`（`file_stem`，`"a.1.png"`→`"a.1"`）。
  `PackScan` 的三份索引全部经它：`root_images`、`recursive_pools`、以及 **faces_map——faces_map 的值
  直接取自 recursive_pools**（`scan_pack_progress` 第二趟先 `walk_recursive` 再 `get(fname)`），
  因此标准二/三主池与标准三兜底池逐字同键，禁重复 / Preserve 剔除摘一份即两份生效。
  历史上 faces_map 用过 `legacy_face_stem`（取首个 `.` 之前）——已删除，勿再引入第二套词干规则。
  非图片文件与 `.` 开头文件不入池（在 `walk_recursive` 里统一过滤）。
- 改完 Rust 自检：`$env:CARGO_TARGET_DIR='<repo>\target'; cargo test --manifest-path src-tauri/Cargo.toml`
  （构建产物在仓库根的 target，不在 src-tauri/target）。仓库整体不是 `cargo fmt --check` 干净的，
  别全量 fmt，只保证新代码不新增 fmt 差异。

