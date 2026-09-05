import { message } from '@tauri-apps/plugin-dialog';

/** 统一错误弹窗（后端命令返回 Err 时调用） */
export async function showError(e: unknown) {
  await message(e instanceof Error ? e.message : String(e), { title: 'Error' });
}
