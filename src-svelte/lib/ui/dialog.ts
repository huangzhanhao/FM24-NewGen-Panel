/**
 * 统一的原生弹窗工具：message 来自 Tauri dialog 插件，弹原生系统对话框（比浏览器 alert 更贴近桌面应用）。
 */
import { message } from '@tauri-apps/plugin-dialog';
import { t } from '../text';
import type { ReplaceResult } from '../stores/app.svelte';

/** 把后端错误对象里的展示信息取出：`message` 末尾追加 `userAction` 建议（结构化错误专用）。 */
interface ErrorLike {
  message: string;
  userAction?: string | null;
}

function isErrorLike(e: unknown): e is ErrorLike {
  return (
    typeof e === 'object' &&
    e !== null &&
    'message' in e &&
    typeof (e as { message: unknown }).message === 'string'
  );
}

/** 错误 → 展示串：Error 取 message；结构化错误取 message 并追加「建议：…」；其余原样强转。
 *  生成链命令的 Err 已换成结构化对象，若仍按 Error/字符串处理会得到 [object Object]。 */
export function formatError(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (isErrorLike(e)) {
    return e.userAction ? `${e.message}\n${t.dialog.suggestion}${e.userAction}` : e.message;
  }
  return String(e);
}

/** 统一错误弹窗（后端命令返回 Err 时调用）。 */
export async function showError(e: unknown) {
  await message(formatError(e), { title: t.dialog.error });
}

/** 替换流水线收尾弹窗：成功/失败各弹一框告知结果，取消不打扰用户。
 *  App 挂载时注册为 initListeners 的 onFinished 回调——store 层不碰弹窗，经回调注入。 */
export async function notifyReplaceResult(result: ReplaceResult) {
  if (result === 'finished')
    await message(t.app.replaceFinished, { title: t.brand });
  else if (result === 'failed')
    await message(t.app.replaceFailed, { title: t.dialog.error });
}
