// tauri dev 的 beforeDevCommand 包装：端口已被本项目的 vite 占用时直接复用。
// 场景：先用 IDE 任务「Tauri Dev - 启动前端」/「Chrome - 调试前端」起了 vite，
// 再跑 pnpm tauri dev —— vite 配了 strictPort，重复启动会直接失败并带崩 tauri dev。
import { spawn } from 'node:child_process';

// 与 vite.config.ts 的 server.port 保持一致：对不上会复用到别的页面，或让 devUrl 指向空端口。
const PORT = Number(process.env.TAURI_DEV_PORT ?? 1420);
const URL = `http://127.0.0.1:${PORT}/`;

// 只有确实是本项目的 dev server（index.html 里引用了 src-svelte/main.ts）才复用，
// 否则交给 vite 因端口占用报错，避免把别人的页面加载进应用窗口。
const alreadyUp = await fetch(URL, { signal: AbortSignal.timeout(1500) })
  .then((r) => r.text())
  .then((html) => html.includes('/src-svelte/main.ts'))
  .catch(() => false);

if (alreadyUp) {
  console.log(`- vite dev server already running at ${URL}, reusing it`);
  process.exit(0);
}

const child = spawn('pnpm dev', { stdio: 'inherit', shell: true });
child.on('exit', (code, signal) => process.exit(signal ? 0 : (code ?? 0)));
