// Every copy in the app goes through `clipboard.ts::copyToClipboard`, and that
// helper asks the BACKEND first.
//
//   node app/scripts/clipboard-check.mjs      # exits non-zero on failure
//
// Why: `navigator.clipboard.writeText` is gated on transient user activation,
// and on macOS 27's WebKit every `evaluateJavaScript:` consumes it — including
// the ones Tauri uses to deliver each `emit` and each small `Channel` message
// (every chunk of terminal output). With a session streaming, one of those
// lands between mousedown and click almost every time, the click handler runs
// with no activation, and the write rejects with a raw `NotAllowedError` toast.
// Nothing about the call site looks wrong, so a new "Copy …" button written the
// obvious way reintroduces it; this is what notices.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
let failed = 0;
const ok = (name, cond, detail = "") => {
  if (cond) { console.log(`ok ${name}`); return; }
  failed++;
  console.log(`not ok — ${name}${detail ? `\n     ${detail}` : ""}`);
};

// ── no direct clipboard writes outside the helper ───────────────────────────
const src = here("../src");
const walk = (dir) => fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
  const p = path.join(dir, e.name);
  if (e.isDirectory()) return e.name === "mock" ? [] : walk(p);
  return /\.(ts|tsx)$/.test(e.name) ? [p] : [];
});
// Comments may name the API (this rule's own explanations do); code may not.
const code = (s) => s.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
const offenders = walk(src)
  .filter((f) => path.basename(f) !== "clipboard.ts")
  .filter((f) => /navigator\.clipboard/.test(code(fs.readFileSync(f, "utf8"))))
  .map((f) => path.relative(src, f));
ok("no navigator.clipboard outside clipboard.ts", offenders.length === 0, `found in: ${offenders.join(", ")}`);

// ── the helper: native first, web only as the fallback ──────────────────────
const helperPath = here("../src/clipboard.ts");
const helper = fs.existsSync(helperPath) ? code(fs.readFileSync(helperPath, "utf8")) : "";
ok("clipboard.ts exports copyToClipboard", /export async function copyToClipboard\b/.test(helper));
const native = helper.indexOf('invoke("copy_text"');
const web = helper.indexOf("navigator.clipboard.writeText");
ok("copyToClipboard asks copy_text first", native >= 0 && (web < 0 || native < web),
  `copy_text at ${native}, writeText at ${web}`);

// ── the command exists on both sides of the harness ─────────────────────────
const lib = fs.readFileSync(here("../src-tauri/src/lib.rs"), "utf8");
const handler = lib.slice(lib.indexOf("generate_handler!["), lib.indexOf("])", lib.indexOf("generate_handler![")));
ok("lib.rs registers copy_text", /\bcopy_text\b/.test(handler));
ok("lib.rs defines copy_text as an async command", /#\[tauri::command\]\s*async fn copy_text\b/.test(lib));
const mock = fs.readFileSync(here("../src/mock/install.ts"), "utf8");
ok("the mock harness answers copy_text", /case "copy_text"/.test(mock));

process.exit(failed ? 1 : 0);
