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
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::Manager;
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

/// 把 v1 的用户数据（.user/）迁移到便携数据目录（首次运行、目标缺失时）
/// v1 便携模式在 <exe>/data，标准模式在 %LOCALAPPDATA%/NewGAN Manager
///
/// 本项目从旧版（Python v1）重写而来，老用户升级后需要把以前的配置自动搬到新位置，
/// 避免重新配置；仅在「新目录还没有数据」时才搬。
fn migrate_legacy_user_data(data_dir: &std::path::Path) -> Result<(), String> {
    // 新位置已有 .user/cfg.json：迁移过或本来就是新用户，不覆盖。
    if data_dir.join(".user").join("cfg.json").exists() {
        return Ok(());
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
            let dst = data_dir.join(".user");
            copy_dir_recursive(&src, &dst)?;
            return Ok(());
        }
    }
    Ok(()) // 所有候选都没有旧数据，也算成功（新用户本来就没有）
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
        // .setup(...) 在应用启动、窗口创建前执行一次初始化。
        .setup(|app| {
            // 1) 便携数据目录（路径已在 run() 顶部确定，这里只补建目录）。
            std::fs::create_dir_all(&data_dir)?;
            // 2) 尝试把旧版数据迁移过来（首次运行时）。
            migrate_legacy_user_data(&data_dir)?;

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
            let pm = profile_manager::ProfileManager::new(None, &data_dir)?;

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
                running: Arc::new(AtomicBool::new(false)),       // 「正在替换」标志
                rtf_cache: Arc::new(Mutex::new(Vec::new())),     // 最近一次解析的 RTF 球员缓存
                mapping_cache: Arc::new(Mutex::new(Vec::new())), // 最近一次生成的映射缓存
                faces_cache: Arc::new(Mutex::new(None)), // 「扫描头像」的包索引缓存（初始为空）
                snapshot_cache: Arc::new(Mutex::new(None)), // 快照解析缓存（「读取快照」/替换收尾写入；初始为空）
                data_dir,                                   // 便携数据目录（标准规则在其下 rules/）
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
            commands::start_replace,   // 启动批量替换（后台线程）
            commands::cancel_replace,  // 请求取消正在进行的替换
            commands::query_players,   // 分页查询名单（预览）
            commands::read_snapshot,   // 读取快照：解析存档 XML 快照并缓存（快照预览数据源）
            commands::query_snapshot,  // 快照预览分页查询（UID 筛选 + 按页解析图片路径）
            commands::preview_player,  // 预览单个球员的映射与头像
            commands::replace_single_face, // 单人换脸
            commands::read_image,      // 读取图片字节（前端转 blob 显示）
            commands::resolve_avatar,  // 解析名单「头像」列原文为包内图片路径
            commands::query_logs       // 分页查询日志
        ])
        // .run(...) 启动事件循环，阻塞直到窗口关闭；启动失败直接 panic。
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
