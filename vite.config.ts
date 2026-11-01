import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // 1420 是 Tauri 模板惯用端口。别改回 5173：Windows 会把 5141–5240 整段保留给 Hyper-V/WSL，
  // 绑定时直接 EACCES，而 strictPort 会让整个 tauri dev 失败。
  server: { port: 1420, strictPort: true },
  envPrefix: 'TAURI_',
  build: { target: 'chrome105', minify: 'esbuild', sourcemap: false },
});
