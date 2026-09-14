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
- 改完 Rust 自检：`$env:CARGO_TARGET_DIR='<repo>\target'; cargo test --manifest-path src-tauri/Cargo.toml`
  （构建产物在仓库根的 target，不在 src-tauri/target）。仓库整体不是 `cargo fmt --check` 干净的，
  别全量 fmt，只保证新代码不新增 fmt 差异。

