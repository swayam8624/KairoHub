import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] }
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  // Tauri ships a current embedded webview. Keep the frontend on Vite's
  // Rolldown/Oxc path instead of forcing the deprecated esbuild fallback.
  build: { target: "esnext", sourcemap: true }
});
