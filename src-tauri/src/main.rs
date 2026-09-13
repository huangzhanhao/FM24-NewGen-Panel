//! 桌面应用的可执行入口（binary crate）。
//!
//! 【职责分工】Tauri 约定把启动逻辑放进 lib.rs 的 run()，本文件只调用它——桌面端与移动端
//! 复用同一份 lib 代码，这里只是桌面这一侧的薄壳。
//!
//! 【windows_subsystem 属性】只在 release 构建（非 debug_assertions）时声明
//! windows_subsystem = "windows"，正式版以图形窗口程序运行、不弹出控制台；
//! 调试构建不声明，保留控制台便于 cargo run 时直接看输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 库名 newganmanager_lib 来自 Cargo.toml 的 [lib] 段；run() 负责装配并启动整个 Tauri 应用。
fn main() { newganmanager_lib::run() }
