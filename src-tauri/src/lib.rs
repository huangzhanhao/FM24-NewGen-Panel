//! Tauri 应用装配：模块声明、插件与状态注册、命令登记。
//!
//! 【装配流程】run() 按固定次序装配：确定便携数据目录（日志 target 注册时就要用）→
//! 注册插件（dialog / opener / 官方日志插件，日志第三路 target 接自研内存缓冲）→
//! setup 里建目录、迁移旧版数据、编程式创建主窗口、装日志门面与 panic hook、
//! 初始化 ProfileManager、导出标准规则默认文件、托管 AppState → 启动日志批推线程 →
//! 登记全部前端可调用命令。
//!
//! 【命令登记】前端 invoke("命令名") 调到 commands 里带 #[tauri::command] 的函数；
//! 新增命令必须同时登记在 invoke_handler 列表，否则前端调用报「命令未找到」。

mod assets; // 编译期内嵌的只读映射表（国籍翻译、config.xml 模板、标准规则默认数据）
mod commands; // Tauri 命令层：前端可调用的所有函数都在这里
mod face_mapper; // 核心算法：球员 → 头像图片的映射
mod facepack_scanner; // 头像包扫描器：包布局扫描与标准判定（FolderList 来自 templates，不硬编码）
mod image_ai; // AI 文生图（契约/模板渲染/生成编排/用量台账；命令在 commands.rs）
mod image_ai_providers; // AI 文生图适配器族（openai/custom/volcano/dashscope 协议适配）
mod image_ai_queue; // AI 文生图批量生成队列（条目状态机/并发调度/事件 Seam；命令在 commands.rs）
mod json_store; // JSON 配置的原子读写
mod logger; // 日志系统（log 门面 + 官方插件管道；自研内存缓冲与批量事件见模块头）
mod paging; // 通用分页契约（日志/名单/快照预览共用 total+items）
mod paths; // 路径规范化工具
mod profile_manager; // 存档（Profile）与头像包（组）管理
mod replace_flow; // 「Replace Faces」整条流水线
mod rtf_scanner; // RTF 扫描器：识别并解析 FM 导出的 RTF 名单（基础/拓展球员/拓展职员）
mod templates; // 头像包标准与抽图模型（一个标准一个自包含模块 + 规则配置聚合）
mod xml_parser; // 读写 config.xml（FM 认的头像映射配置）

use commands::AppState;
use logger::{LogMemory, LogSink, Logger};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::Manager; // app.handle() 取句柄（交给日志批推线程）
use tauri_plugin_log::{fern, RotationStrategy, Target, TargetKind, TimezoneStrategy};

/// 便携化数据目录：开发调试 → 项目根目录 data/；正式分发 → 可执行文件所在目录 data/
/// （NSIS 以 currentUser 模式安装到用户可写目录，安装目录内可直接写数据）
///
/// 传统应用把配置写到 C 盘 AppData，不方便携带。本项目把所有用户数据集中到程序旁边的
/// data/ 目录，拷走整个文件夹就能带走全部配置。失败时返回中文错误描述（供前端弹窗）。
fn portable_data_dir() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        // 开发模式：CARGO_MANIFEST_DIR 指向 src-tauri，取其父级（项目根）再拼 data。
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|root| root.join("data"))
            .ok_or_else(|| "无法定位项目根目录".to_string())
    } else {
        // 正式模式：可执行文件所在目录再拼 data。
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("data")))
            .ok_or_else(|| "无法定位可执行文件目录".to_string())
    }
}

/// 把 v1 的用户数据（.user/）迁移到便携数据目录（首次运行、目标缺失时）。
/// v1 便携模式在 <exe>/data，标准模式在 %LOCALAPPDATA%/NewGAN Manager
///
/// 本项目从旧版（Python v1）重写而来，老用户升级后需要把以前的配置自动搬到新位置，
/// 避免重新配置；仅在「新目录还没有数据」时才搬。
///
/// 【迁移失败绝不阻断启动】原来第一个 IO 错误就用 `?` 抛出 `.setup()` → Tauri 起不来 →
/// 发布版没有控制台、用户看到的是「程序打不开」。现在一律记警告后继续跑：v1 原件仍在原处，
/// 用户可以手工拷贝，也可以下次启动重试。
/// 【为什么先写暂存目录再 rename】原来「已迁移」只看 cfg.json 是否存在，而 NTFS 目录读序
/// 大致按字母——拷贝到一半失败时 cfg.json 往往已经落位，下次启动就直接宣布「迁移完成」，
/// 带着静默缺失的 v1 数据继续跑。暂存目录 + 一次性 rename 让「就位」要么整体发生要么没发生。
fn migrate_legacy_user_data(data_dir: &std::path::Path) {
    let dst = data_dir.join(".user");
    if is_migrated(&dst) {
        return; // 已迁移过，或本来就是有数据的新用户：不覆盖
    }
    // 候选旧位置按优先级排列：先可执行文件旁的 data/.user，再 Windows 标准安装位置
    // %LOCALAPPDATA%/NewGAN Manager/.user（环境变量缺失时该候选为 None）。
    let mut candidates: Vec<Option<PathBuf>> = vec![std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("data").join(".user")))];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(Some(
            PathBuf::from(local).join("NewGAN Manager").join(".user"),
        ));
    }
    // 找到第一个真正含有 cfg.json 的旧目录，递归拷贝到新位置。
    for src in candidates.into_iter().flatten() {
        if src.join("cfg.json").is_file() {
            migrate_from(&src, data_dir);
            return;
        }
    }
    // 所有候选都没有旧数据：新用户本来就没有，什么都不做。
}

/// 迁移的核心一步：把 `src` 整体搬进 `data_dir/.user`（先写 `.user.migrating` 暂存，再一次 rename）。
/// 【为什么单独成函数】上面那层要从环境变量/可执行文件位置里挑候选目录，测试不便插手；
/// 这一层是纯路径参数，「绝不留下半个 .user」「绝不碰已有的 .user」两条都能直接校。
fn migrate_from(src: &std::path::Path, data_dir: &std::path::Path) {
    let dst = data_dir.join(".user");
    let tmp = data_dir.join(".user.migrating");
    // 上一次中断留下的暂存目录：里面只可能有本次迁移的副本，删掉重来才是正解。
    if tmp.exists() {
        log::warn!(target: "migrate", "清理上次未完成的迁移暂存目录：{}", tmp.display());
        if let Err(e) = std::fs::remove_dir_all(&tmp) {
            log::warn!(target: "migrate", "暂存目录清理失败，本次迁移放弃：{e}");
            return;
        }
    }
    if let Err(e) = copy_dir_recursive(src, &tmp) {
        log::warn!(
            target: "migrate",
            "旧版数据迁移未完成（原件仍在 {}，本次启动按现有配置继续，下次启动会重试）：{e}",
            src.display()
        );
        let _ = std::fs::remove_dir_all(&tmp); // 半成品整棵删掉，避免下次被当成完整数据
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, &dst) {
        // dst 已存在（例如迁移失败后用户已在 v2 里新建过配置）：不覆盖、不合并，把暂存丢掉，
        // v1 原件仍完整留在 src。这种并存的处置要用户自己决定，程序不该猜。
        log::warn!(
            target: "migrate",
            "迁移结果无法就位（{} 已存在或不可写）：{e}；旧数据仍完整保留在 {}，本次启动按现有配置继续",
            dst.display(),
            src.display()
        );
        let _ = std::fs::remove_dir_all(&tmp);
    } else {
        log::info!(target: "migrate", "旧版数据迁移完成：{} → {}", src.display(), dst.display());
    }
}

/// 「迁移已完成」的判据：存档索引与档名表都在。
/// 【为什么不能只看 cfg.json】它恰好是拷贝时最先落位的文件之一，半途失败时它已在、
/// profiles.json 却还没搬过来——按老口径下次启动就再也看不出这份数据是残缺的。
fn is_migrated(user_dir: &std::path::Path) -> bool {
    user_dir.join("cfg.json").is_file() && user_dir.join("profiles.json").is_file()
}

/// 递归拷贝整个目录（含子目录和文件）。Rust 标准库没有现成的「拷目录」，故手写。
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &to)?; // 子目录：递归拷贝
        } else {
            std::fs::copy(entry.path(), &to).map_err(|e| e.to_string())?; // 文件：直接拷
        }
    }
    Ok(())
}

/// 应用装配与启动主函数。
#[cfg_attr(mobile, tauri::mobile_entry_point)] // 移动端构建时标记为入口点（桌面端忽略）
pub fn run() {
    // 数据目录必须先于插件装配确定：日志文件的 Folder target 在 .plugin() 注册时就要用到它
    // （插件 setup 先于应用 setup 运行，全局 logger 在应用 setup 第一行日志前已就位）。
    let data_dir = portable_data_dir().expect("无法定位便携数据目录");
    // 「正在替换」标志：交给 AppState 做重入保护与界面禁用（setup 里托管时用同一份 Arc）。
    let running = Arc::new(AtomicBool::new(false));
    // 内存环形缓冲 + 事件通道（logger.rs 自研补齐的部分）：Arc 一份给插件 fern 闭包，
    // 一份给 setup 里的 Logger；接收端最后交给后台聚批线程。
    let (memory, log_rx) = LogMemory::channel();
    let memory = Arc::new(memory);
    let memory_for_plugin = Arc::clone(&memory); // fern 闭包拿一份，setup 里的 Logger 拿原件
    tauri::Builder::default()
        // 官方插件：dialog 提供原生弹框与文件/目录选择，opener 提供打开外部链接/本地文件的
        // 能力；注册后前端才能通过对应的 JS API 调用它们。
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // 官方日志插件（tauri-plugin-log）：log 门面 + stdout + 滚动文件三合一，
        // 依赖库（tauri/tao/wry…）的日志从此也汇入同一条管道。第三个 target 把
        // 原始记录交给自研 LogMemory（官方没有「回查历史」能力，见 logger.rs）。
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info) // 全局阈值：INFO 及以上才进管道
                .timezone_strategy(TimezoneStrategy::UseLocal) // 本地时区（与 v1 一致）
                .max_file_size(10 * 1024 * 1024) // 单文件 10MB，超过滚动（对齐 v1）
                .rotation_strategy(RotationStrategy::KeepSome(3)) // 保留最近 3 份滚动备份（对齐 v1 的 .1~.3）
                // 必须清掉插件根格式：fern 的父级格式化输出会「替换」record.args 再传给子
                // target（实测正文会被嵌套成 [时间][级别] 前缀套 v1 格式）。清掉后根直接透传
                // 原始 Record，各 target 自带格式互不污染（文件 v1 行/内存原始串）。
                .clear_format()
                .targets([
                    // ① 控制台：官方默认行样式（clear_format 后根不再兜底，此处自带；
                    //    样式对齐插件原输出：[日期][时间][级别][模块] 正文）。
                    Target::new(TargetKind::Stdout).format(|out, message, record| {
                        out.finish(format_args!(
                            "[{}][{}][{}] {}",
                            chrono::Local::now().format("%Y-%m-%d][%H:%M:%S"),
                            record.level(),
                            record.target(),
                            message
                        ))
                    }),
                    // ② 滚动文件：v1 兼容行格式（格式契约在 logger::v1_line，勿改）。
                    //    官方 RotatingFile 负责开关文件与滚动，替代原手写 write_file/rotate。
                    //    【file_name 传词干】插件内部 join(词干).with_extension("log") 拼出
                    //    newgan.log（与 v1 同名）；滚动备份命名为 newgan_日期_时间.log 并按
                    //    KeepSome(3) 保留最近 3 份（官方语义，替代 v1 的 .1/.2/.3）。
                    Target::new(TargetKind::Folder {
                        path: data_dir.clone(),
                        file_name: Some("newgan".into()),
                    })
                    .format(|out, _message, record| {
                        out.finish(format_args!(
                            "{}",
                            logger::v1_line(
                                &chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                                logger::level_label(record.level()),
                                record.target(),
                                &record.args().to_string(),
                            )
                        ))
                    }),
                    // ③ 内存环形缓冲（自研补齐）：fern 自定义输出拿到未经格式化的原始
                    // Record，组装 LogLine 入缓冲 + 事件通道，前端回查/实时增量都靠它。
                    Target::new(TargetKind::Dispatch(fern::Dispatch::new().chain(
                        fern::Output::call(move |record| memory_for_plugin.record(record)),
                    ))),
                ])
                .build(),
        )
        // 关窗不做任何拦截：× 直接退出（Tauri 默认行为）。替换中的 config.xml 与档景快照都走
        // tmp+rename 原子落盘，进程退出时正式文件要么旧要么新，不会有半截映射表。
        // .setup(...) 在应用启动、窗口创建前执行一次初始化。
        // 【move 捕获】Tauri 要求该闭包 'static，故把 data_dir / logger / running 等句柄的
        // 所有权交给它。
        .setup(move |app| {
            // 1) 便携数据目录（路径已在 run() 顶部确定，这里只补建目录）。
            std::fs::create_dir_all(&data_dir)?;
            // 2) 尝试把旧版数据迁移过来（首次运行时）。失败只记警告，绝不阻断启动。
            //    这里用 log 门面而不是 Logger：官方日志插件在 setup 之前已装好全局 logger，
            //    自研 Logger 门面要到第 4 步才建起来，而迁移的失败恰恰必须被看见。
            migrate_legacy_user_data(&data_dir);

            // AI 文生图：清理上次临时图片、导出/补齐配置文件。失败不阻断启动。
            if let Err(e) = image_ai::Config::prepare_session(&data_dir) {
                log::warn!(target: "image_ai", "临时目录清理失败: {e}");
            }
            if let Err(e) = image_ai::Config::ensure_user_files(&data_dir) {
                log::warn!(target: "image_ai", "生图配置初始化失败: {e}");
            }

            // 3) 主窗口编程式创建（而非在 tauri.conf.json 里声明）：
            // 只有 Builder API 能指定 WebView2 数据目录为绝对路径，把浏览器缓存/localStorage
            // 一并便携化到 <data>\webview，避免 WebView2 默认写入 C 盘 AppData。
            let webview_data = data_dir.join("webview");
            std::fs::create_dir_all(&webview_data)?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("NewGAN Manager") // 窗口标题
                .inner_size(1280.0, 800.0) // 初始内容区尺寸（宽×高，单位逻辑像素）
                .min_inner_size(860.0, 600.0) // 窗口拖小下限（低于此宽布局不再保证）
                .resizable(true) // 允许用户拖拽改变窗口大小
                .data_directory(webview_data) // 浏览器数据目录指向便携位置（便携化关键）
                .build()?;

            // 4) 日志门面：官方插件已在 setup 之前装好全局 logger，这里起手即可写。
            //    Logger 只携带「打开文件路径 + 回查缓冲」，写入经 log 门面走官方管道。
            let logger = Arc::new(Logger::new(&data_dir, memory));
            // panic hook：官方生态不覆盖 panic——补上，让崩溃现场进日志文件（默认 hook
            // 保留，stderr 照常输出）。
            let default_hook = std::panic::take_hook();
            let panic_logger = logger.clone();
            std::panic::set_hook(Box::new(move |info| {
                default_hook(info);
                panic_logger.error("panic", format!("{info}"));
            }));
            logger.info("app", "Starting Application v2");

            // 5) 初始化存档管理器（ProfileManager），读取或创建用户配置。
            //    并把日志出口交给它：切档时的档景（快照）存取异常此前是静默的，
            //    而快照正是替换的判据，静默失败＝下次替换按旧档景判断（见 profile_manager 的 warn）。
            let mut pm = profile_manager::ProfileManager::new(None, &data_dir)?;
            pm.set_logger(logger.clone());

            // 5.5) 标准规则：首次运行把内置默认（assets/standardN.json）导出为用户可编辑文件
            //      （data/rules/standardN.json，已存在不覆盖）。失败不阻断启动，仅记警告。
            if let Err(e) = templates::ensure_user_files(&data_dir) {
                logger.warn("rules", format!("导出标准规则默认文件失败: {e}"));
            }

            // 6) app.manage(...) 把全局状态交给 Tauri 托管：之后任何命令函数只需在参数里写
            //    State<'_, AppState>，Tauri 就会自动把这个状态传进去。
            app.manage(AppState {
                pm: Mutex::new(pm),                              // 存档管理器（互斥锁保护）
                logger: logger.clone(),                          // 日志器（Arc 共享）
                cancel: Arc::new(AtomicBool::new(false)),        // 「请求取消」标志
                running: Arc::clone(&running),                   // 「正在替换」标志
                rtf_cache: Arc::new(Mutex::new(Vec::new())),     // 最近一次解析的 RTF 球员缓存
                mapping_cache: Arc::new(Mutex::new(Vec::new())), // 最近一次生成的映射缓存
                faces_cache: Arc::new(Mutex::new(None)), // 「扫描头像」的包索引缓存（初始为空）
                snapshot_cache: Arc::new(Mutex::new(None)), // 快照解析缓存（「读取快照」/替换收尾写入；初始为空）
                image: Arc::new(Mutex::new(
                    image_ai::Config::load(&data_dir).unwrap_or_else(|e| {
                        log::warn!(target: "image_ai", "读取生图配置失败，使用出厂默认: {e}");
                        image_ai::Config::builtin()
                    }),
                )),
                image_keys: Arc::new(Mutex::new(HashMap::new())),
                queue: Arc::new(Mutex::new(image_ai_queue::QueueState::default())), // 批量生成队列（条目 + cancel 标志）
                data_dir, // 便携数据目录（标准规则在其下 rules/）
            });

            // 7) 启动一个后台线程，把日志按批通过事件（log:batch）推给前端。
            logger::spawn_emitter(app.handle().clone(), log_rx);
            Ok(())
        })
        // .invoke_handler(...) 登记「前端可调用」的命令列表：前端 api.ts 里
        // invoke("get_app_state") 就会调到 commands::get_app_state。新增命令必须在此登记。
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,          // 获取应用全局快照（存档/组/RTF/运行状态）
            commands::create_profile,         // 新建并激活存档
            commands::delete_profile,         // 删除存档
            commands::switch_profile,         // 切换存档（含快照存取）
            commands::select_image_directory, // 关联头像包目录（建组）
            commands::set_current_rtf,        // 设置当前组的 RTF 名单
            commands::set_current_group,      // 切换当前头像包（组）
            commands::delete_group,           // 删除组
            commands::validate_image_dirs,    // 校验 14 个种族目录是否齐全
            commands::check_integrity, // 检查完整性：识别三套头像包标准（图片直铺/14/65）并给出缺失目录
            commands::create_missing_dirs, // 批量创建缺失目录
            commands::scan_images,     // 扫描头像：建图片索引缓存（供替换复用），结果写日志
            commands::scan_rtf,        // 扫描RTF：解析名单并填充预览缓存，结果写日志
            commands::open_rules_dir,  // 打开标准规则目录（data/rules/，用户编辑 templateN.json）
            commands::open_snapshot_dir, // 打开存档 XML 快照所在目录（查看位置）
            commands::preview_profile, // 预览指定存档（下拉浏览展示，不切换当前档）
            commands::read_rule_file,  // 读取标准规则文件内容（规则编辑对话框）
            commands::save_rule_file,  // 保存标准规则文件（先校验 schema 再原子写回）
            commands::preflight_replace, // 发起前预检：本次会不会清空本档既有映射（需要就先弹确认）
            commands::start_replace,   // 启动批量替换（后台线程）
            commands::cancel_replace,  // 请求取消正在进行的替换
            commands::query_players,   // 分页查询名单（预览）
            commands::read_snapshot,   // 读取快照：解析存档 XML 快照并缓存（快照预览数据源）
            commands::query_snapshot,  // 快照预览分页查询（UID 筛选 + 按页解析图片路径）
            commands::preview_player,  // 预览单个球员的映射与头像
            commands::query_avatars,   // 名单预览头像列：按 UID 批量查快照/映射缓存里的头像路径
            commands::replace_single_face, // 单人换脸
            commands::read_image,      // 读取图片字节（前端转 blob 显示）
            commands::resolve_avatar,  // 解析名单「头像」列原文为包内图片路径
            commands::query_logs,      // 分页查询日志
            commands::get_image_ai_state,
            commands::render_prompt,
            commands::preview_prompt, // 按球员数据源渲染模板（uid → build_vars → render）
            commands::save_provider,
            commands::delete_provider,
            commands::set_provider_key,
            commands::save_prompt_template,
            commands::delete_prompt_template,
            commands::generate_image,
            commands::save_generated_image,
            commands::save_and_replace, // 保存并替换：生成图落盘 _generated 后复用单人换脸链
            commands::query_usage,
            commands::add_to_queue,      // 批量生成：追加条目（返回快照）
            commands::remove_from_queue, // 批量生成：按 id 移除条目
            commands::clear_queue,       // 批量生成：清空队列
            commands::start_queue,       // 批量生成：启动队列（后台异步跑 run_queue）
            commands::cancel_queue,      // 批量生成：请求取消（未启动条目判「已取消」）
            commands::query_queue        // 批量生成：查询队列全量快照
        ])
        // .run(...) 启动事件循环，阻塞直到窗口关闭；启动失败直接 panic。
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一份 v1 形状的 .user：存档索引 + 档名表 + 一个档的子目录。
    fn v1_fixture(src: &Path) {
        std::fs::create_dir_all(src.join("档A")).unwrap();
        std::fs::write(src.join("cfg.json"), r#"{"prf":"档A"}"#).unwrap();
        std::fs::write(src.join("profiles.json"), r#"{"Profile":{"档A":true}}"#).unwrap();
        std::fs::write(
            src.join("档A").join("档A.json"),
            r#"{"img_dir":"C:\\pack"}"#,
        )
        .unwrap();
    }

    #[test]
    fn migrate_from_moves_the_whole_tree_and_leaves_no_temp_dir() {
        let data = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        v1_fixture(src.path());
        migrate_from(src.path(), data.path());

        let dst = data.path().join(".user");
        assert!(dst.join("cfg.json").is_file(), "存档索引没搬过来");
        assert!(dst.join("profiles.json").is_file(), "档名表没搬过来");
        assert!(
            dst.join("档A").join("档A.json").is_file(),
            "档子目录里的 json 没搬过来（递归拷贝漏了）"
        );
        assert!(
            !data.path().join(".user.migrating").exists(),
            "暂存目录没清掉"
        );
        // 原件留在原地：迁移是「搬一份副本过来」，不是「搬走」——失败时用户的数据必须还在。
        assert!(src.path().join("cfg.json").is_file());
    }

    #[test]
    fn migrate_from_never_touches_an_existing_user_dir() {
        // 迁移半途失败后用户已在 v2 里新建过配置：.user 已存在（残缺），绝不能被覆盖或合并。
        let data = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        v1_fixture(src.path());
        let dst = data.path().join(".user");
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("cfg.json"), "v2 自己的内容").unwrap();

        migrate_from(src.path(), data.path());

        assert_eq!(
            std::fs::read_to_string(dst.join("cfg.json")).unwrap(),
            "v2 自己的内容"
        );
        assert!(
            !dst.join("profiles.json").is_file(),
            "不该把 v1 的文件补进 v2 的数据里"
        );
        assert!(
            !data.path().join(".user.migrating").exists(),
            "失败路径也要清掉暂存目录"
        );
    }

    #[test]
    fn half_populated_user_dir_is_not_declared_migrated() {
        // 老口径只看 cfg.json：而它是拷贝时最先落位的文件之一 → 半途失败会被当成「已完成」，
        // 从此再没人知道这份数据缺了 profiles.json 和各档的 json。
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cfg.json"), "{}").unwrap();
        assert!(!is_migrated(dir.path()));
        std::fs::write(dir.path().join("profiles.json"), "{}").unwrap();
        assert!(is_migrated(dir.path()));
    }
}
