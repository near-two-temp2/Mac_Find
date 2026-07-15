import { defineConfig } from "vite";

// Tauri 期望前端固定跑在 1420 端口；build 产物落到 ../dist（对齐 tauri.conf.json 的
// frontendDist）。以下配置来自 Tauri 官方 vanilla-ts 模板。
export default defineConfig({
  // 防止 vite 清屏，遮住 rust 编译错误。
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // src-tauri 交给 cargo/tauri 监听，vite 不管。
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    // dist 输出目录（相对本文件）。
    outDir: "dist",
    // Safari / macOS WKWebView 目标。
    target: "safari14",
    minify: "esbuild",
    sourcemap: false,
  },
});
