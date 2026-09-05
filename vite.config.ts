import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  envPrefix: 'TAURI_',
  build: { target: 'chrome105', minify: 'esbuild', sourcemap: false },
});
