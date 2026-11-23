import { execSync } from "node:child_process";
import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  define: {
    __APP_COMMIT_HASH__: JSON.stringify(gitCommitHash()),
  },
  server: {
    host: "127.0.0.1",
    // The port is chosen by scripts/tauri-dev.mjs (a free one per worktree) and
    // handed down via VITE_PORT so Tauri's devUrl points at this exact server.
    // Falls back to 1421 for a bare `pnpm dev`.
    port: Number.parseInt(process.env.VITE_PORT ?? "", 10) || 1421,
    strictPort: true,
    // Vite's file watcher must not descend into the Rust build-output dirs.
    // On Windows, cargo locks `.exe`/pdb files in `target/` while linking, and
    // `fs.watch` throws EBUSY on locked files. An unhandled watcher error
    // crashes Vite and tears down `tauri dev` mid-compile. macOS's FSEvents
    // watcher doesn't hit this, so the gap only shows on Windows.
    watch: {
      ignored: ["**/src-tauri/target/**", "**/clovy-api/target/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    minify: !process.env.TAURI_DEBUG,
    sourcemap: Boolean(process.env.TAURI_DEBUG),
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        hud: fileURLToPath(new URL("./hud.html", import.meta.url)),
        "meeting-hud": fileURLToPath(new URL("./meeting-hud.html", import.meta.url)),
      },
      output: {
        manualChunks(id) {
          if (!id.includes("/node_modules/")) return;

          if (id.includes("/@tiptap/") || id.includes("/prosemirror-")) {
            return "vendor-editor";
          }
          if (id.includes("/react-dom/server.") || id.includes("/react-dom/cjs/react-dom-server")) {
            return "vendor-react-server";
          }
          if (id.includes("/react-dom/")) return "vendor-react-dom";
          if (id.includes("/react/")) return "vendor-react";
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["src/test/setup.ts"],
    include: ["src/test/**/*.{test,spec}.{ts,tsx,mjs}"],
    css: true,
    // Keep frontend tests inside one Node process. A serial worker-thread pool
    // cannot fan out into child processes if a worker exits unexpectedly.
    pool: "threads",
    fileParallelism: false,
    maxWorkers: 1,
    coverage: {
      provider: "v8",
      reporter: ["text", "html", "lcov"],
      reportsDirectory: "coverage/frontend",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/test/**", "src/**/*.d.ts", "src/main.tsx", "src/hud.ts", "src/agent-hud.ts"],
    },
  },
});

function gitCommitHash() {
  try {
    return (
      execSync("git rev-parse --short HEAD", {
        stdio: ["ignore", "pipe", "ignore"],
      })
        .toString()
        .trim() || "unknown"
    );
  } catch {
    return "unknown";
  }
}
