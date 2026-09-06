//! 【新手导读】Tauri 应用的「总装配车间」。这里把后端所有零件拼成一个可运行的桌面应用：
//!   1) 声明并引入各个功能模块（mod xxx）；
//!   2) 用 tauri::Builder 搭建应用：注册插件、初始化状态、创建主窗口；
//!   3) 注册所有「命令」（command）——前端 JS 通过 invoke 调用它们来和 Rust 后端通信。
//!
//! 关键概念：Tauri = Rust 后端 + 网页前端（本项目用 Svelte）。
//!   两者通过 IPC（进程间通信）交互：前端 invoke("命令名", 参数) → 后端带 #[tauri::command] 的函数。

// 下面这些 `mod xxx;` 声明「本 crate 包含哪些模块」，每个对应同目录下的一个 .rs 文件。
// 只有在这里声明过的模块，才能被 crate 内部使用（Rust 的模块需要显式声明）。
mod assets;          // 编译期内嵌的只读映射表（国籍翻译、config.xml 模板、模板规则默认数据）
mod commands;        // Tauri 命令层：前端可调用的所有函数都在这里
mod face_mapper;     // 核心算法：球员 → 头像图片的映射
mod facepack;        // 头像包扫描与模板判定（FolderList 来自 rule_config，不硬编码）
mod json_store;      // JSON 配置的原子读写
mod logger;          // 日志系统（环形缓冲 + 滚动文件 + 事件推送）
mod paths;           // 路径规范化工具
mod pick_rules;      // 抽图模板规则（按模板分发的具体选图逻辑）
mod profile_manager; // 存档（Profile）与头像包（组）管理
mod rule_config;     // 模板规则配置（内置默认 + 用户可编辑 json 的加载/导出）
mod replace_flow;    // 「Replace Faces」整条流水线
mod rtf_parser;      // 解析 FM 导出的 RTF 球员名单
mod xml_parser;      // 读写 config.xml（FM 认的头像映射配置）

use commands::AppState;                 // 应用全局状态（被 Tauri 托管，命令里通过 State 取用）
use logger::Logger;                     // 日志器
use std::path::{Path, PathBuf};         // Rust 的跨平台路径类型（Path 借用、PathBuf 拥有）
use std::sync::atomic::AtomicBool;      // 原子布尔量：多线程间安全地传递「取消/运行中」标志
use std::sync::{Arc, Mutex};            // Arc=多线程共享所有权；Mutex=互斥锁，保护共享数据
use tauri::Manager;                     // 引入 Manager trait 才能用 app.manage() / app.handle()

/// 便携化数据目录：开发调试 → 项目根目录 data/；正式分发 → 可执行文件所在目录 data/
/// （NSIS 以 currentUser 模式安装到用户可写目录，安装目录内可直接写数据）
///
/// 【为什么这么做】传统应用把配置写到 C 盘 AppData，不方便携带。本项目把所有用户数据
/// 集中到程序旁边的 data/ 目录，拷走整个文件夹就能带走全部配置。
/// 返回类型 Result<PathBuf, String>：成功给出路径，失败给出中文错误描述（供前端弹窗）。
fn portable_data_dir() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        // 开发模式：cfg!(debug_assertions) 是编译期布尔判断，调试构建时为 true。
        // CARGO_MANIFEST_DIR = <项目根>/src-tauri，是编译期常量，由 env! 宏在编译时展开。
        // .parent() 取其上级目录（即项目根），再拼接 data。
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|root| root.join("data"))
            .ok_or_else(|| "无法定位项目根目录".to_string())
    } else {
        // 正式模式：拿当前可执行文件的路径，取其所在目录再拼 data。
        // 链式调用中用 ok()/and_then() 把可能失败的步骤优雅地串起来，任一步失败则回退到错误。
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("data")))
            .ok_or_else(|| "无法定位可执行文件目录".to_string())
    }
}

/// 把 v1 的用户数据（.user/）迁移到便携数据目录（首次运行、目标缺失时）
/// v1 便携模式在 <exe>/data，标准模式在 %LOCALAPPDATA%/NewGAN Manager
///
/// 【背景】本项目是从旧版（Python v1）重写而来。老用户升级后，需要把他们以前的配置
/// 自动搬到新位置，避免重新配置。此函数就是干这件事，仅在「新目录还没有数据」时才搬。
fn migrate_legacy_user_data(data_dir: &std::path::Path) -> Result<(), String> {
    // 如果新位置已经存在 .user/cfg.json，说明已经迁移过或本来就是新用户，直接返回，不覆盖。
    if data_dir.join(".user").join("cfg.json").exists() {
        return Ok(()); // 已有新数据
    }
    // 候选旧位置列表（按优先级）：先看可执行文件旁的 data/.user。
    // Vec<Option<PathBuf>> 允许某些候选「拿不到」（比如环境变量缺失）时用 None 占位。
    let mut candidates: Vec<Option<PathBuf>> = vec![
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("data").join(".user"))),
    ];
    // 再追加 Windows 标准安装位置 %LOCALAPPDATA%/NewGAN Manager/.user（环境变量存在时）。
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(Some(PathBuf::from(local).join("NewGAN Manager").join(".user")));
    }
    // into_iter().flatten() 把 Vec<Option<T>> 展平为只包含 Some 值的迭代器（跳过 None）。
    // 找到第一个真正含有 cfg.json 的旧目录，递归拷贝到新位置，然后返回。
    for src in candidates.into_iter().flatten() {
        if src.join("cfg.json").is_file() {
            let dst = data_dir.join(".user");
            copy_dir_recursive(&src, &dst)?;
            return Ok(());
        }
    }
    Ok(()) // 所有候选都没找到旧数据，也算成功（新用户本来就没有）
}

/// 递归拷贝整个目录（含子目录和文件）。Rust 标准库没有现成的「拷目录」，故手写。
/// 【错误处理】map_err(|e| e.to_string()) 把系统 IO 错误转成 String，方便统一返回给前端。
/// 末尾的 `?` 是 Rust 的错误传播运算符：出错就立即 return Err，否则取出 Ok 里的值继续。
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

/// 【应用启动主函数】整个 Tauri 应用从这里装配并运行。
/// 采用「建造者模式（Builder Pattern）」：一连串 .xxx() 链式调用逐步配置应用，最后 .run() 启动。
#[cfg_attr(mobile, tauri::mobile_entry_point)] // 移动端构建时标记为入口点（桌面端忽略）
pub fn run() {
    // tauri::Builder::default() 创建一个默认的应用构建器，接下来链式地往上叠加配置。
    tauri::Builder::default()
        // .plugin(...) 注册官方插件：
        //   dialog —— 提供原生弹框（提示/确认）和文件、目录选择对话框；
        //   opener —— 提供打开外部链接、本地文件的能力。
        // 注册后前端才能通过对应的 JS API 调用它们。
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // .setup(...) 在应用启动、窗口创建前执行一次初始化。闭包参数 app 是应用句柄。
        // 返回 Result：若初始化出错（用 ? 传播），应用会启动失败。
        .setup(|app| {
            // 1) 准备便携数据目录，并确保它存在（不存在就创建）。
            let data_dir = portable_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            // 2) 尝试把旧版数据迁移过来（首次运行时）。
            migrate_legacy_user_data(&data_dir)?;

            // 3) 主窗口编程式创建（而非在 tauri.conf.json 里声明）：
            // 只有 Builder API 能指定 WebView2 数据目录为绝对路径，把浏览器缓存/localStorage
            // 一并便携化到 <data>\webview，避免 WebView2 默认写入 C 盘 AppData。
            //（WebView2 是 Windows 上承载前端网页的内嵌浏览器内核）
            let webview_data = data_dir.join("webview");
            std::fs::create_dir_all(&webview_data)?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("NewGAN Manager")   // 窗口标题
                .inner_size(1000.0, 640.0) // 初始内容区尺寸（宽×高，单位逻辑像素）
                .resizable(true)           // 允许用户拖拽改变窗口大小
                .data_directory(webview_data) // 关键：把浏览器数据目录指向便携位置
                .build()?;                 // 真正创建窗口

            // 4) 初始化日志器。Logger::new 返回（日志器, 接收端）；日志器用 Arc 包裹以便多线程共享。
            let (logger, log_rx) = Logger::new(&data_dir)?;
            let logger = Arc::new(logger);
            logger.info("app", "Starting Application v2"); // 写入启动日志

            // 5) 初始化存档管理器（ProfileManager），读取或创建用户配置。
            let pm = profile_manager::ProfileManager::new(None, &data_dir)?;

            // 5.5) 模板规则：首次运行把内置默认（assets/templateN.json）导出为用户可编辑文件
            //      （data/rules/templateN.json，已存在不覆盖）。失败不阻断启动，仅记警告。
            if let Err(e) = rule_config::ensure_user_files(&data_dir) {
                logger.warn("rules", format!("导出模板规则默认文件失败: {e}"));
            }

            // 6) app.manage(...) 把全局状态交给 Tauri 托管。
            //    之后任何命令函数只需在参数里写 State<'_, AppState>，Tauri 就会自动把这个状态传进去。
            //    AppState 内部用 Mutex/AtomicBool/Arc 包裹字段，保证多线程（命令、后台线程）安全访问。
            app.manage(AppState {
                pm: Mutex::new(pm),                          // 存档管理器（互斥锁保护）
                logger: logger.clone(),                      // 日志器（Arc 共享）
                cancel: Arc::new(AtomicBool::new(false)),    // 「请求取消」标志
                running: Arc::new(AtomicBool::new(false)),   // 「正在替换」标志
                rtf_cache: Arc::new(Mutex::new(Vec::new())), // 最近一次解析的 RTF 球员缓存
                mapping_cache: Arc::new(Mutex::new(Vec::new())), // 最近一次生成的映射缓存
                faces_cache: Arc::new(Mutex::new(None)),     // 「扫描头像」的包索引缓存（初始为空）
                data_dir,                                    // 便携数据目录（模板规则在其下 rules/）
            });

            // 7) 启动一个后台线程，把日志按批通过事件（log:batch）推给前端。
            logger::spawn_emitter(app.handle().clone(), log_rx);
            Ok(())
        })
        // .invoke_handler(...) 注册「前端可调用」的命令列表。
        // generate_handler! 是 Tauri 宏，把这些 Rust 函数暴露为 IPC 命令；
        // 前端 api.ts 里 invoke("get_app_state") 就会调到 commands::get_app_state。
        // 【重要】新增命令时，必须同时在这里登记，否则前端调用会报「命令未找到」。
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,        // 获取应用全局快照（存档/组/RTF/运行状态）
            commands::create_profile,       // 新建并激活存档
            commands::delete_profile,       // 删除存档
            commands::switch_profile,       // 切换存档（含快照存取）
            commands::select_image_directory, // 关联头像包目录（建组）
            commands::set_current_rtf,      // 设置当前组的 RTF 名单
            commands::set_current_group,    // 切换当前头像包（组）
            commands::delete_group,         // 删除组
            commands::validate_image_dirs,  // 校验 14 个种族目录是否齐全
            commands::check_integrity,      // 检查完整性：识别三套头像包标准（图片直铺/14/65）并给出缺失目录
            commands::create_missing_dirs,  // 批量创建缺失目录
            commands::scan_images,          // 扫描头像：建图片索引缓存（供替换复用），结果写日志
            commands::scan_rtf,             // 扫描RTF：解析名单并填充预览缓存，结果写日志
            commands::open_rules_dir,       // 打开模板规则目录（data/rules/，用户编辑 templateN.json）
            commands::open_pack_config_dir, // 打开头像包目录（config.xml 所在目录，查看config）
            commands::open_snapshot_dir,    // 打开存档 XML 快照所在目录（查看快照）
            commands::preview_profile,      // 预览指定存档（下拉浏览展示，不切换当前档）
            commands::read_rule_file,       // 读取模板规则文件内容（规则编辑对话框）
            commands::save_rule_file,       // 保存模板规则文件（先校验 schema 再原子写回）
            commands::start_replace,        // 启动批量替换（后台线程）
            commands::cancel_replace,       // 请求取消正在进行的替换
            commands::query_players,        // 分页查询名单（预览）
            commands::preview_player,       // 预览单个球员的映射与头像
            commands::replace_single_face,  // 单人换脸
            commands::read_image,           // 读取图片字节（前端转 blob 显示）
            commands::query_logs            // 分页查询日志
        ])
        // .run(...) 真正启动事件循环（阻塞直到窗口关闭）。
        // generate_context! 宏在编译期读取 tauri.conf.json 生成上下文（图标、配置等）。
        // expect(...) 表示：如果启动失败就直接 panic 并打印这句话。
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
