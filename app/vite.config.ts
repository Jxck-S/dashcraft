import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

// Set by `tauri dev` when targeting a remote device; unset for desktop development.
const host = process.env.TAURI_DEV_HOST

// https://vite.dev/config/ — tuned for Tauri (https://v2.tauri.app/start/frontend/vite/)
export default defineConfig({
  plugins: [svelte()],
  // Keep Rust compiler errors visible in the terminal.
  clearScreen: false,
  server: {
    // Must match `build.devUrl` in src-tauri/tauri.conf.json.
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: 'ws', host, port: 1421 } : undefined,
    watch: {
      // Rust sources are rebuilt by the Tauri CLI, not Vite.
      ignored: ['**/src-tauri/**'],
    },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    // Tauri's webviews: WebView2 (Chromium) on Windows, WebKit on macOS/Linux.
    target: process.env.TAURI_ENV_PLATFORM === 'windows' ? 'chrome111' : 'safari16',
    minify: process.env.TAURI_ENV_DEBUG ? false : 'oxc',
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
})
