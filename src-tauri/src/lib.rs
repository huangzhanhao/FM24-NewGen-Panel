//! Tauri 应用装配：状态初始化、旧版数据迁移、日志事件、命令注册
mod assets;
mod commands;
mod face_mapper;
mod json_store;
mod logger;
mod paths;
mod profile_manager;
mod replace_flow;
mod rtf_parser;
mod xml_parser;

use commands::AppState;
use logger::Logger;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// 便携化数据目录：开发调试 → 项目根目录 data/；正式分发 → 可执行文件所在目录 data/
/// （NSIS 以 currentUser 模式安装到用户可写目录，安装目录内可直接写数据）
fn portable_data_dir() -> Result<PathBuf, String> {
    if cfg!(debug_assertions) {
        // CARGO_MANIFEST_DIR = <项目根>/src-tauri，编译期常量
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|root| root.join("data"))
            .ok_or_else(|| "无法定位项目根目录".to_string())
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("data")))
            .ok_or_else(|| "无法定位可执行文件目录".to_string())
    }
}

/// 把 v1 的用户数据（.user/）迁移到便携数据目录（首次运行、目标缺失时）
/// v1 便携模式在 <exe>/data，标准模式在 %LOCALAPPDATA%/NewGAN Manager
fn migrate_legacy_user_data(data_dir: &std::path::Path) -> Result<(), String> {
    if data_dir.join(".user").join("cfg.json").exists() {
        return Ok(()); // 已有新数据
    }
    let mut candidates: Vec<Option<PathBuf>> = vec![
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("data").join(".user"))),
    ];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(Some(PathBuf::from(local).join("NewGAN Manager").join(".user")));
    }
    for src in candidates.into_iter().flatten() {
        if src.join("cfg.json").is_file() {
            let dst = data_dir.join(".user");
            copy_dir_recursive(&src, &dst)?;
            return Ok(());
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = portable_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            migrate_legacy_user_data(&data_dir)?;

            // 主窗口编程式创建（而非 tauri.conf.json 声明）：
            // 只有 Builder API 能指定 WebView2 数据目录为绝对路径，把浏览器缓存/localStorage
            // 一并便携化到 <data>\webview，避免 WebView2 默认写入 C 盘 AppData
            let webview_data = data_dir.join("webview");
            std::fs::create_dir_all(&webview_data)?;
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::default())
                .title("NewGAN Manager")
                .inner_size(1000.0, 640.0)
                .resizable(true)
                .data_directory(webview_data)
                .build()?;
            let (logger, log_rx) = Logger::new(&data_dir)?;
            let logger = Arc::new(logger);
            logger.info("app", "Starting Application v2");
            let pm = profile_manager::ProfileManager::new(None, &data_dir)?;
            app.manage(AppState {
                pm: Mutex::new(pm),
                logger: logger.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                running: Arc::new(AtomicBool::new(false)),
                rtf_cache: Arc::new(Mutex::new(Vec::new())),
                mapping_cache: Arc::new(Mutex::new(Vec::new())),
            });
            logger::spawn_emitter(app.handle().clone(), log_rx);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_state,
            commands::create_profile,
            commands::delete_profile,
            commands::switch_profile,
            commands::select_image_directory,
            commands::set_current_rtf,
            commands::set_current_group,
            commands::delete_group,
            commands::validate_image_dirs,
            commands::create_missing_dirs,
            commands::start_replace,
            commands::cancel_replace,
            commands::query_players,
            commands::preview_player,
            commands::replace_single_face,
            commands::read_image,
            commands::query_logs
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
