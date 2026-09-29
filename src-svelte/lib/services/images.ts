/**
 * 图片字节 → blob URL 的统一封装。
 * WebView 不能直接用磁盘路径作 <img> 源，需把后端读回的字节包成浏览器可识别的 blob URL；
 * 各预览组件（名单预览/单人替换/快照预览）共用此处，MIME 推断规则单点维护。
 */
import { api } from './api';

/** 由文件扩展名推断图片 MIME（jpg → image/jpeg，其余 → image/<ext>；无扩展名按 png）。 */
function mimeOf(path: string): string {
  const ext = path.split('.').pop()?.toLowerCase() ?? 'png';
  return ext === 'jpg' ? 'image/jpeg' : `image/${ext}`;
}

/** 把图片字节按路径推断的 MIME 包成 blob URL。调用方负责在不再用时 revokeObjectURL。 */
export function bytesToBlobUrl(path: string, bytes: ArrayBuffer): string {
  return URL.createObjectURL(new Blob([bytes], { type: mimeOf(path) }));
}

/** 读取磁盘图片并转成 blob URL（bytesToBlobUrl 的便捷封装）。 */
export async function loadImageBlobUrl(path: string): Promise<string> {
  return bytesToBlobUrl(path, await api.readImage(path));
}
