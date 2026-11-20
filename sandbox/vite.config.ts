import { defineConfig } from "vite";
// The repo may live on a Windows drive under WSL (/mnt/...), where file-change
// events do not reach Linux: poll, or the dev server keeps serving stale code.
export default defineConfig({
  test: { globals: true },
  server: { port: 5173, strictPort: true, watch: { usePolling: true, interval: 500 } },
} as any);
