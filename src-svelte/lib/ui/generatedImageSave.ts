/**
 * 生成图「另存为」共用动作：结果画廊的「保存」与队列坞缩略图的「保存」是同一段流程
 * （save 对话框 → save_generated_image → 成功提示/失败弹窗），原先在两组件各抄一份 10 行，
 * 是「改一处漏一处」的面（将来加默认目录/文件名规则时只改这里）。
 * 放 ui/ 层而非 store：本动作含 Tauri 对话框与弹窗 UI，与 dialog.ts 同层（store 不碰 UI）。
 */
import { save, message } from "@tauri-apps/plugin-dialog";
import { api } from "../services/api";
import { t } from "../text";
import { showError } from "./dialog";

/** 打开另存为对话框（默认 PNG 文件名）并把生成图写入目标；用户取消返回 false，失败弹窗。 */
export async function saveGeneratedImageAs(imgId: string): Promise<boolean> {
  const target = await save({
    defaultPath: `${imgId}.png`,
    filters: [{ name: "PNG", extensions: ["png"] }],
  });
  if (!target) return false; // 用户取消
  try {
    await api.saveGeneratedImage(imgId, target);
    await message(t.gen.saved, { title: t.dialog.info });
    return true;
  } catch (e) {
    await showError(e);
    return false;
  }
}
