// 【新手导读】统一的错误弹窗工具。
// message 来自 Tauri 的 dialog 插件，弹出一个原生系统对话框（比浏览器 alert 更贴近桌面应用）。
import { message } from '@tauri-apps/plugin-dialog';

/** 统一错误弹窗（后端命令返回 Err 时调用）。
 * 参数类型为 unknown（可能是 Error、字符串或其他）：先判断是不是 Error，是就取 .message，否则强转字符串。 */
export async function showError(e: unknown) {
  await message(e instanceof Error ? e.message : String(e), { title: 'Error' });
}
