// Build for the Ideas companion window — SEPARATE from the app's build, on
// purpose, the way the docs viewer's is (vite.viewer.config.ts).
//
// The companion is where the libraries ideas need will live (CodeMirror 6 in
// phase 1, docs/proposals/idea-board-phases.md §4.3), and the app's bundle
// must never gain a byte of them. A second input to `vite.config.ts` would let
// rollup hoist a chunk the two pages share, and then "it is isolated" would be
// a claim about a bundler heuristic. So: two configs, two outDirs, nothing
// shared but SOURCE files the app already owns (ipc.ts, ipcws.ts, tokens.css),
// which this build compiles into its own chunks. companion-boundary-check.mjs
// asserts it.
//
//   pnpm -C app build            → app/dist/ (the app), then app/dist/companion/
//   pnpm -C app build:companion  → app/dist/companion/ alone
//
// Unlike the viewer, this page is not served to a browser: the shim opens it in
// a second Tauri window as `companion/index.html` (companion.rs), resolved
// against `frontendDist` in a build and `devUrl` under `tauri dev` — where the
// app's own dev server serves `app/companion/index.html` as it is, so there is
// one dev server and no second port.
//
// It runs AFTER the app's build, which empties app/dist (its `emptyOutDir`),
// and empties only its own subdirectory.
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import { workspaceVersion } from "./vite.config";

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  root: here("companion"),
  // The page is `tauri://localhost/companion/index.html`; its assets sit beside
  // it, never in the app's `assets/`.
  base: "/companion/",
  plugins: [react()],
  // This frontend's version for the serve handshake, from the one source.
  define: { __WT_VERSION__: JSON.stringify(workspaceVersion()) },
  // The app's `public/` is the app's, already copied by its build.
  publicDir: false,
  build: {
    outDir: here("dist/companion"),
    emptyOutDir: true,
  },
});
