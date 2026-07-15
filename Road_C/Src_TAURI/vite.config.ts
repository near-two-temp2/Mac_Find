import { defineConfig } from "vite";

// Vite config tuned for Tauri: fixed dev port, no clearScreen so cargo output
// stays visible, and emit to ../dist which tauri.conf.json points at.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    // Target Safari/WKWebView; Tauri v2 on macOS ships a modern engine.
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
});
