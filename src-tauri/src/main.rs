//! 【新手导读】整个桌面应用的「可执行文件入口」（binary crate）。
//! Tauri 的惯例：把真正的启动逻辑放进 lib.rs 的 run() 函数，main.rs 只负责调用它。
//! 这样拆分的好处：桌面端和移动端能复用同一份 lib 代码，main.rs 只是 Windows/桌面这一侧的薄壳。

// Windows 发布版不显示控制台（黑色命令行窗口）。
// 语法拆解：
//   #![...]  —— 「crate 级属性」，作用于整个文件（注意 #! 带感叹号，写在文件顶部）。
//   cfg_attr(条件, 属性) —— 「满足条件时才启用后面的属性」。
//   not(debug_assertions) —— 非调试构建，也就是 release 正式版。
//   windows_subsystem = "windows" —— 告诉 Windows 以「图形窗口程序」运行，而非「控制台程序」。
// 合起来：只有在打正式版时才隐藏控制台；开发调试（cargo run）时仍保留控制台方便看输出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 程序真正的起点：调用库 crate（newganmanager_lib，名字来自 Cargo.toml 的 [lib] 段）里的 run()。
// run() 定义在 lib.rs，负责装配并启动整个 Tauri 应用。
fn main() { newganmanager_lib::run() }
