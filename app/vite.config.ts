import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// The repo lives on a Windows drive under WSL (/mnt/...): file events do not
// reach Linux, so the dev server polls (same as the console).
export default defineConfig({
  plugins: [react()],
  test: { globals: true },
  server: { port: 5180, strictPort: true, watch: { usePolling: true, interval: 500 } },
} as any);
