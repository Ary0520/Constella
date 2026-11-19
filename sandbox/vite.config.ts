import { defineConfig } from "vite";
export default defineConfig({ test: { globals: true }, server: { port: 5173, strictPort: true } } as any);
