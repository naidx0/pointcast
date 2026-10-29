import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": resolve(__dirname, "src") },
  },
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
  preview: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        indicator: resolve(__dirname, "indicator.html"),
        overlay: resolve(__dirname, "overlay.html"),
        result: resolve(__dirname, "result.html"),
        onboarding: resolve(__dirname, "onboarding.html"),
      },
    },
  },
});
