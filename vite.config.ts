import { defineConfig } from "vite";

// Tauri expects a fixed dev port and wants to own the terminal output.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
    // The browser preview's backend: cargo run -p writepad-devserver (see src/devbridge.ts).
    proxy: { "/api": { target: "http://127.0.0.1:1421", changeOrigin: true } },
  },
  build: { target: "safari15", outDir: "dist" },
});
