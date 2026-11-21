import { defineConfig, type Plugin } from "vitest/config";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";
import path from "node:path";

/**
 * Dev-surface only (configureServer never runs under `vite build`): serves
 * the updater stub fixtures from src-tauri/updater-stub/ at /__updater-stub/,
 * the endpoint `tauri.dev.conf.json` points `tauri dev` at. Strict whitelist
 * — two exact routes, no traversal, no caching. Editing a fixture sits inside
 * the tauri dev watch scope, so it rebuilds + relaunches the app — the mount
 * auto-check then reflects the new version with no manual clicking. See
 * src-tauri/updater-stub/README.md.
 */
function updaterStub(): Plugin {
  const stubDir = path.resolve(__dirname, "src-tauri", "updater-stub");
  const routes: Record<string, { file: string; type: string }> = {
    "/__updater-stub/latest.json": { file: "latest.json", type: "application/json" },
    "/__updater-stub/artifact.bin": { file: "artifact.bin", type: "application/octet-stream" },
  };
  return {
    name: "updater-stub",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const route = routes[req.url?.split("?")[0] ?? ""];
        if (!route) return next();
        try {
          res.writeHead(200, {
            "Content-Type": route.type,
            "Cache-Control": "no-store",
          });
          res.end(readFileSync(path.join(stubDir, route.file)));
        } catch (err) {
          res.writeHead(500, { "Content-Type": "text/plain" });
          res.end(`updater-stub: failed to read fixture — ${err}`);
        }
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), updaterStub()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
  },
});
