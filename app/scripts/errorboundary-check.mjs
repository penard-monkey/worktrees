// The crash screen above each React root (AppErrorBoundary.tsx), against the
// REAL class (bundled, `./ipc` replaced by a recorder):
//
//   1. A caught throw draws a screen with the message and a Reload button,
//      whatever was thrown — null and undefined included.
//   2. The crash goes to `log_event`, through ipc.ts (the one door); a log that
//      fails is said on the console, never swallowed, never thrown.
//   3. Both roots mount it OUTERMOST, through a dynamic import: a static one
//      would load ipc.ts before the mock harness is installed.
//
// react-dom/server does not run error boundaries, so the class is driven by
// the calls React makes: getDerivedStateFromError, componentDidCatch, render.
//
// Run from the repo root, as CI does:
//   node app/scripts/errorboundary-check.mjs     exits non-zero on failure
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const esbuild = createRequire(require.resolve("vite"))("esbuild");
const APP = new URL("../", import.meta.url).pathname;
const FILE = path.join(APP, "src/AppErrorBoundary.tsx");

let failed = 0;
const check = (cond, msg) => { if (!cond) failed++; console.log(`${cond ? "ok" : "not ok"} — ${msg}`); };

// ── 1 and 2. the class ───────────────────────────────────────────────────────
if (!fs.existsSync(FILE)) check(false, "app/src/AppErrorBoundary.tsx exists");
else {
  const out = await esbuild.build({
    stdin: {
      contents: `
        export { AppErrorBoundary } from "./AppErrorBoundary";
        export { renderToStaticMarkup } from "react-dom/server";
        export { createElement } from "react";`,
      resolveDir: path.join(APP, "src"), loader: "ts",
    },
    plugins: [{
      name: "ipc", setup(b) {
        b.onResolve({ filter: /^\.\/ipc$/ }, () => ({ path: "ipc", namespace: "stub" }));
        b.onLoad({ filter: /.*/, namespace: "stub" }, () => ({ contents: "export const invoke = (...a) => globalThis.__invoke(...a);" }));
      },
    }],
    bundle: true, write: false, format: "cjs", platform: "node", jsx: "automatic",
    define: { "process.env.NODE_ENV": '"production"' }, logLevel: "silent",
  });
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "errorboundary-check-"));
  fs.writeFileSync(path.join(dir, "bundle.cjs"), out.outputFiles[0].text);
  const { AppErrorBoundary: B, renderToStaticMarkup, createElement } = require(path.join(dir, "bundle.cjs"));
  fs.rmSync(dir, { recursive: true, force: true });

  const calls = [];
  globalThis.__invoke = async (cmd, args) => { calls.push([cmd, args]); };
  let reloads = 0;
  globalThis.window = { location: { reload: () => { reloads++; } } };
  const mount = () => new B({ where: "main window", children: createElement("p", null, "the app") });
  const caught = (thrown) => {
    const b = mount();
    b.state = { ...b.state, ...B.getDerivedStateFromError(thrown) };
    return b;
  };
  const html = (b) => renderToStaticMarkup(b.render());
  /** The element tree's first button. */
  const button = (n) => {
    if (!n || typeof n !== "object") return null;
    if (Array.isArray(n)) return n.map(button).find(Boolean) ?? null;
    return n.type === "button" ? n : button(n.props?.children);
  };

  check(html(mount()) === "<p>the app</p>", "nothing thrown: the app, and nothing of the boundary's");
  const e = new Error("kaboom <b>");
  const screen = html(caught(e));
  check(/role="alert"/.test(screen) && screen.includes("kaboom &lt;b&gt;") && !screen.includes("the app"), "a throw: an alert with its message, in place of the app");
  check(/<button[^>]*>Reload<\/button>/.test(screen), "…and a Reload button");
  button(caught(e).render())?.props.onClick();
  check(reloads === 1, "Reload reloads the window");
  for (const [v, want] of [[null, "null"], [undefined, "undefined"], ["a string", "a string"], [new Error(""), "Error"]])
    check(/role="alert"/.test(html(caught(v))) && html(caught(v)).includes(want), `${v instanceof Error ? "an Error with no message" : `a thrown ${JSON.stringify(v) ?? "undefined"}`} is a crash too, and says ${JSON.stringify(want)}`);

  caught(e).componentDidCatch(e, { componentStack: "\n    at Broken\n    at App" });
  const [cmd, args] = calls[0] ?? [];
  check(calls.length === 1 && cmd === "log_event" && args?.level === "error", "the crash is one log_event at error");
  check(/main window/.test(args?.msg ?? "") && /kaboom/.test(args?.msg ?? "") && /at Broken/.test(args?.msg ?? "") && (args?.msg ?? "").includes(e.stack), "…naming the window, the message, the component stack and the stack");
  // serve keeps 4000 bytes of a line (meta.rs UI_LINE_MAX): the component
  // stack, which names the broken component, must not be what is cut.
  const long = new Error("long"); long.stack = "x".repeat(9000);
  caught(long).componentDidCatch(long, { componentStack: "\n    at Broken" });
  check((calls[1]?.[1].msg ?? "").slice(0, 4000).includes("at Broken"), "…with the component stack before the long one");

  const said = [];
  const real = console.error;
  console.error = (...a) => said.push(a.join(" "));
  let unhandled = 0;
  const onUnhandled = () => { unhandled++; };
  process.on("unhandledRejection", onUnhandled);
  globalThis.__invoke = () => Promise.reject("the engine is not connected");
  caught(e).componentDidCatch(e, { componentStack: "" });
  await new Promise((r) => setTimeout(r, 0));
  globalThis.__invoke = () => { throw new Error("no door"); };
  let threw = false;
  try { caught(e).componentDidCatch(e, {}); } catch { threw = true; }
  await new Promise((r) => setTimeout(r, 0));
  process.off("unhandledRejection", onUnhandled);
  console.error = real;
  check(said.some((s) => s.includes("not connected")) && unhandled === 0, "a log that fails (no engine) is said on the console, not swallowed");
  check(!threw && said.some((s) => s.includes("no door")), "…and a door that throws does not throw out of the boundary");

  const src = fs.readFileSync(FILE, "utf8");
  check(/^import \{ invoke \} from "\.\/ipc";$/m.test(src) && !/@tauri-apps\//.test(src.split("\n").filter((l) => /\bimport\b|\bfrom\b/.test(l)).join("\n")), "it reaches the backend through ./ipc and imports no @tauri-apps module");
}

// ── 3. both roots ────────────────────────────────────────────────────────────
for (const [rel, spec, inner, where] of [
  ["src/main.tsx", "./AppErrorBoundary", "EngineGate", "main window"],
  ["companion/main.tsx", "../src/AppErrorBoundary", "Companion", "Ideas window"],
]) {
  const s = fs.readFileSync(path.join(APP, rel), "utf8");
  check(s.includes(`const { AppErrorBoundary } = await import("${spec}");`), `${rel} loads it with a dynamic import`);
  check(!/^import[^;]*AppErrorBoundary/m.test(s), `${rel} does not import it statically`);
  const tree = /createRoot\(document\.getElementById\("root"\)[\s\S]*?\n  \);/.exec(s)?.[0] ?? "";
  const m = /<React\.StrictMode>\s*<AppErrorBoundary where="([^"]*)">([\s\S]*)<\/AppErrorBoundary>\s*<\/React\.StrictMode>/.exec(tree);
  check(!!m && m[1] === where && m[2].includes(`<${inner}`), `${rel} mounts it outermost, around everything the root draws`);
}
const css = fs.readFileSync(path.join(APP, "src/App.css"), "utf8");
check(/^\.app-crash \{/m.test(css), "App.css (both pages load it) has the screen's styles");

console.log(failed ? `\n${failed} failed` : "\nall ok");
process.exit(failed ? 1 : 0);
