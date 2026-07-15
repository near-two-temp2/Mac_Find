import { defineConfig } from "vite";

// Tauri expects a fixed dev port and no clearing of the terminal so its own
// output stays visible. `dist` is what `frontendDist` in tauri.conf.json reads.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    // Safari 13+ / macOS 11+ target — matches LSMinimumSystemVersion.
    target: "safari13",
    outDir: "dist",
    emptyOutDir: true,
    // Sourcemaps only in dev builds.
    sourcemap: false,
  },
});
