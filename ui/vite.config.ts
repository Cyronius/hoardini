import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5193,
    watch: { ignored: ["**/mock/**"] },
    proxy: { "/api": { target: "http://127.0.0.1:7043", changeOrigin: false } },
  },
  build: { chunkSizeWarningLimit: 5000 },
});
