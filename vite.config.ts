import { defineConfig, Plugin } from "vite";
import react from "@vitejs/plugin-react";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

function venturiSessionDevSyncPlugin(): Plugin {
  return {
    name: "venturi-session-dev-sync",
    configureServer(server) {
      server.middlewares.use("/__venturi_session", (req, res, next) => {
        if (req.method !== "GET") return next();
        const url = new URL(req.url || "/", "http://localhost");
        const queryPath = url.searchParams.get("file");
        const sessionPath =
          queryPath ||
          process.env.VENTURI_SESSION_FILE ||
          path.join(os.homedir(), ".venturi", "active_session.json");
        try {
          if (fs.existsSync(sessionPath)) {
            const raw = fs.readFileSync(sessionPath, "utf-8");
            res.setHeader("Content-Type", "application/json");
            res.setHeader("Cache-Control", "no-store");
            res.end(raw);
            return;
          }
          res.statusCode = 404;
          res.end(JSON.stringify({ error: "No active session file" }));
        } catch (err) {
          res.statusCode = 500;
          res.end(JSON.stringify({ error: String(err) }));
        }
      });
    },
  };
}

export default defineConfig({
  plugins: [react(), venturiSessionDevSyncPlugin()],
  clearScreen: false,
  build: {
    rollupOptions: {
      output: {
        manualChunks: {
          three: ["three"],
          vendor: ["react", "react-dom", "lucide-react"],
        },
      },
    },
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
