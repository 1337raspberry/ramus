import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// `TAURI_DEV_HOST` is set by `cargo tauri ios dev <device>` (and the
// Android equivalent) when running against a real device. Vite's default
// `localhost` bind means the phone can't reach the dev server; honouring
// the env var tells vite to listen on the LAN IP instead. Desktop builds
// leave it unset → vite binds to localhost as before.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: host || false,
    port: 5173,
    strictPort: true,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
  },
  build: {
    outDir: "dist",
  },
});
