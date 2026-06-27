import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { resolve } from 'node:path'

// Tauri expects a fixed port and host; see tauri.conf.json -> build.devUrl.
const host = process.env.TAURI_DEV_HOST

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte()],
  // Tauri dev server settings
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: 'ws', host, port: 1421 }
      : undefined,
    watch: {
      // Ignore the Rust target dir to avoid Vite reloading on Cargo changes.
      ignored: ['**/src-tauri/**'],
    },
  },
  // Vite options tailored for Tauri development
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    // Tauri uses WebView2 (Edge Chromium) on Windows and WKWebView on macOS.
    // Both support modern JS; we target a recent baseline to keep minification simple.
    target: 'es2022',
    minify: !process.env.TAURI_ENV_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    rollupOptions: {
      input: {
        // Main dashboard window
        main: resolve(__dirname, 'index.html'),
        // Floating pill window — always-on-top status / visualizer widget.
        // Lives at the project root (alongside index.html) so Vite emits
        // it to dist/pill.html rather than dist/src/pill.html.
        pill: resolve(__dirname, 'pill.html'),
      },
    },
  },
})