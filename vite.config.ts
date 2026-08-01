import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed port and needs to know the host when developing on a device.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],

  // Tauri produces its own, more useful errors — keep vite quiet about the ones it can't fix.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      // The Rust side has its own watcher; double-watching just burns CPU.
      ignored: ["**/src-tauri/**"],
    },
  },

  // WebView2 on Windows and WKWebView on macOS both handle modern output fine.
  build: {
    target: "esnext",
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
