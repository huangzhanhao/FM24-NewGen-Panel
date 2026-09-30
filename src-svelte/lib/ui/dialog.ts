/**
 * 统一的原生弹窗工具：message 来自 Tauri dialog 插件，弹原生系统对话框（比浏览器 alert 更贴近桌面应用）。
 */
import { message } from '@tauri-apps/plugin-dialog';
import { t } from '../text';
import type { ReplaceResult } from '../stores/app.svelte';

/** 统一错误弹窗（后端命令返回 Err 时调用）。
 * 参数 e 类型为 unknown（可能是 Error、字符串或其他）：是 Error 就取 .message，否则强转字符串。 */
export async function showError(e: unknown) {
  await message(e instanceof Error ? e.message : String(e), { title: t.dialog.error });
}

/** 替换流水线收尾弹窗：成功/失败各弹一框告知结果，取消不打扰用户。
 *  App 挂载时注册为 initListeners 的 onFinished 回调——store 层不碰弹窗，经回调注入。 */
export async function notifyReplaceResult(result: ReplaceResult) {
  if (result === 'finished')
    await message(t.app.replaceFinished, { title: t.brand });
  else if (result === 'failed')
    await message(t.app.replaceFailed, { title: t.dialog.error });
}
