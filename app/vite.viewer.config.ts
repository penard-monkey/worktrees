// Build for the docs viewer bundle — SEPARATE from the app's build, on purpose.
//
// This is the structural half of the mermaid exception (CLAUDE.md, "Design
// tokens"). Adding the viewer as a second input to `vite.config.ts` would let
// rollup hoist a shared chunk, and the moment `app/src/markdown.tsx` and the
// viewer share one, mermaid is reachable from the app's graph — "it is
// isolated" would then be a claim about a bundler heuristic rather than a
// boundary. Two configs, two `build.outDir`s, nothing shared but source files
// that the app already owned. `app/scripts/viewer-boundary-check.mjs` asserts
// it, because a config can be edited and a grep cannot be argued with.
//
//   pnpm -C app build:viewer     → app/viewer/dist/{viewer.js, shell.html}
//
// IIFE, one file, minified:
//   - a CLASSIC script is the only script a `file://` page can load at all
//     (measured in both Chromium and WebKit), and it is what shell.html uses;
//   - mermaid resolves its diagram types through `await import()`, which leaves
//     37 fetch sites in a code-split build and zero in a forced single file —
//     under `default-src 'none'` those 37 are 37 ways for a diagram to be
//     silently absent. `iife` inlines every one of them.
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { copyFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  plugins: [
    react(),
    {
      // The shell is a source file rather than a build product so the server
      // side can read it, and it is copied into `dist/` so the two halves the
      // server serves — the page and the bundle it references — always ship
      // from one directory and cannot be taken from different builds.
      name: "copy-shell",
      closeBundle() {
        copyFileSync(here("viewer/shell.html"), here("viewer/dist/shell.html"));
      },
    },
  ],
  // LIB MODE DOES NOT DEFINE `process.env.NODE_ENV`. That is correct for a
  // library (the consumer decides) and wrong for us: without it the bundle
  // ships React's DEVELOPMENT build — every invariant message, every hook
  // warning, `StrictMode`'s double-render — and it does so silently. It cost
  // 1.9 MB here, and the only symptom was a suspiciously large `viewer.js`
  // and four `import(` needles that turned out to be inside React's own
  // dev-only error strings.
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
  // The app's `public/` is the APP's — copying it here would put an icon the
  // server never serves into the directory the server does serve.
  publicDir: false,
  build: {
    outDir: here("viewer/dist"),
    emptyOutDir: true,
    target: "es2020",
    // Every stylesheet is imported with `?inline` and injected by main.tsx, so
    // there is no emitted .css for the server to serve as a second route.
    cssCodeSplit: false,
    lib: {
      entry: here("viewer/main.tsx"),
      formats: ["iife"],
      name: "WorktreesDocsViewer",
      fileName: () => "viewer.js",
    },
    rollupOptions: {
      output: {
        // Belt to `formats: ["iife"]`'s braces: an IIFE cannot be code-split,
        // and saying so means a future `manualChunks` fails the build instead
        // of quietly producing chunk fetches the CSP refuses.
        inlineDynamicImports: true,
      },
    },
  },
});
