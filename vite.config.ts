import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 桌面联调约定：固定 1420 端口，禁止终端清屏
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    // 系统 WebView 基线：macOS 13 WebKit / Windows WebView2
    target: ["safari16", "chrome110"],
    chunkSizeWarningLimit: 1000,
  },
});
