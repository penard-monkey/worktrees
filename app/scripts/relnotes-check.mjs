// Links in the release notes, against the REAL code: `renderInline` and
// `ReleaseNotes` are sliced out of App.tsx and bundled with the real
// `safeHref`, with `openUrl` replaced by a recorder.
//
//   - `[text](url)` is an anchor when its target is http(s), and its label
//     alone, inert, when it is anything else (`javascript:`, a relative path).
//   - A click and a middle-click go through openUrl and never to the WebView; a
//     failed open reaches the caller's error handler; a click on a link in a
//     headline does not also toggle the entry.
//   - A headline with nothing behind it is not a disabled button: a link in
//     one could not be pressed.
//
// Run from the repo root, as CI does:
//   node app/scripts/relnotes-check.mjs [App.tsx]     exits non-zero on failure
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const esbuild = createRequire(require.resolve("vite"))("esbuild");
const SRC = new URL("../src/", import.meta.url).pathname;
const app = fs.readFileSync(process.argv[2] || path.join(SRC, "App.tsx"), "utf8");

const from = app.indexOf("const INLINE = ");
const to = app.indexOf("\n}\n", app.indexOf("\nfunction ReleaseNotes(", from));
if (from < 0 || to < 0) throw new Error("INLINE … ReleaseNotes not found in App.tsx");

const out = await esbuild.build({
  stdin: {
    contents: `
      import * as React from "react";
      import { safeHref } from "./markdown";
      import * as Icons from "./icons";
      const openUrl = (u) => globalThis.__openUrl(u);
      type NotesSection = any;
      ${app.slice(from, to + 3)}
      export { renderInline, ReleaseNotes };
      export { renderToStaticMarkup } from "react-dom/server";
      export { createElement } from "react";`,
    resolveDir: SRC, loader: "tsx",
  },
  bundle: true, write: false, format: "cjs", platform: "node", jsx: "automatic",
  define: { "process.env.NODE_ENV": '"production"' }, logLevel: "silent",
});
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "relnotes-check-"));
fs.writeFileSync(path.join(dir, "bundle.cjs"), out.outputFiles[0].text);
const { renderInline, ReleaseNotes, renderToStaticMarkup, createElement } = require(path.join(dir, "bundle.cjs"));
fs.rmSync(dir, { recursive: true, force: true });

let failed = 0;
const eq = (a, b, msg) => {
  const same = JSON.stringify(a) === JSON.stringify(b);
  if (!same) failed++;
  console.log(`${same ? "ok" : "not ok"} — ${msg}${same ? "" : ` — got ${JSON.stringify(a)}, want ${JSON.stringify(b)}`}`);
};
const errors = [];
const onError = (e) => errors.push(String(e));
const nodes = (s) => renderInline(s, onError);
const html = (s) => renderToStaticMarkup(createElement("p", null, ...nodes(s))).slice(3, -4);
const bare = (s) => html(s).replace(/ (?:title|class)="[^"]*"/g, "");

// ── what a link draws ────────────────────────────────────────────────────────
eq(bare("see [#12](https://github.com/o/r/pull/12)."), 'see <a href="https://github.com/o/r/pull/12">#12</a>.', "an https target is a link");
eq(bare("[a](http://x.dev) and [b](https://y.dev)"), '<a href="http://x.dev">a</a> and <a href="https://y.dev">b</a>', "two in one line");
eq(bare("[`code` and **bold**](https://x.dev)"), '<a href="https://x.dev"><code>code</code> and <strong>bold</strong></a>', "markup inside a label");
eq(bare("**see [x](https://x.dev) now**"), '<strong>see <a href="https://x.dev">x</a> now</strong>', "a link inside bold");
for (const t of ["javascript:alert%281%29", "JaVaScRiPt:alert`1`", "data:text/html,x", "file:///etc/passwd", "vbscript:x", "x-apple.systempreferences:y", "//evil.dev/x", "DESIGN.md", "#anchor"])
  eq([/<a\b|href=/.test(html(`a [text](${t}) b`)), bare(`a [text](${t}) b`).replace(/<\/?span>/g, "")], [false, "a text b"], `${t}: inert text`);
eq(bare("`[x](https://x.dev)`"), "<code>[x](https://x.dev)</code>", "in code it is literal");
eq(bare("a **b** `c` *d*"), "a <strong>b</strong> <code>c</code> <em>d</em>", "code, strong and em as before");
eq(bare("[x] (y) arr[0](z"), "[x] (y) arr[0](z", "brackets that are not a link are left alone");
eq(bare('[x](https://x.dev/"onclick=1)').includes('"onclick'), false, "a quote in a target cannot add an attribute");

// ── what a press does ────────────────────────────────────────────────────────
const opened = [];
globalThis.__openUrl = async (u) => { opened.push(u); };
const link = nodes("[x](https://x.dev)").find((n) => n?.type === "a");
const press = (handler, button = 0) => {
  const ev = { button, prevented: false, stopped: false, preventDefault() { this.prevented = true; }, stopPropagation() { this.stopped = true; } };
  link?.props[handler]?.(ev);
  return [ev.prevented, ev.stopped, opened.splice(0)];
};
eq(press("onClick"), [true, true, ["https://x.dev"]], "a click opens it outside, and is not also the headline's toggle");
eq(press("onAuxClick", 1), [true, true, ["https://x.dev"]], "so does a middle-click, which fires no click");
eq(press("onAuxClick", 2), [true, true, []], "a right-click opens nothing, and the WebView follows nothing");
globalThis.__openUrl = () => Promise.reject("opener: denied");
press("onClick");
await new Promise((r) => setTimeout(r, 0));
eq(errors, ["open https://x.dev: opener: denied"], "a failed open is reported, not swallowed");

// ── in a headline ────────────────────────────────────────────────────────────
const draw = (item, props = {}) => renderToStaticMarkup(createElement(ReleaseNotes, {
  sections: [{ version: "1.0.0", date: "", groups: [{ name: "Added", items: [item] }] }], notes: "", open: new Set(["0:0:0"]), onToggle() {}, onError, ...props,
}));
const flat = draw({ lead: "All lead, [x](https://x.dev)", rest: "" });
eq([/<button/.test(flat), /<a [^>]*href="https:\/\/x\.dev"/.test(flat), /class="rel-lead/.test(flat)], [false, true, true], "a headline with nothing to open is not a disabled button, and its link is live");
const full = draw({ lead: "Lead [x](https://x.dev)", rest: "body [y](https://y.dev)" });
eq([/<button[^>]*aria-expanded="true"/.test(full), /href="https:\/\/x\.dev"/.test(full), /<p class="rel-body">body <a /.test(full)], [true, true, true], "one with a paragraph still opens, and both halves' links are live");
eq(/<WhatsNewModal[\s\S]{0,900}?onError=\{fail\}/.test(app) && /<ReleaseNotes [^>]*onError=\{onError\}/.test(app), true, "App hands its fail() to the notes");

console.log(failed ? `\n${failed} failed` : "\nall ok");
process.exit(failed ? 1 : 0);
