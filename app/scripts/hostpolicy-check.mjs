// Loading a client must not set host policy.
//
//   node app/scripts/hostpolicy-check.mjs      # exits non-zero on failure
//
// Host policy is what the host does on everyone's behalf (`worktrees-api`
// hostpolicy): Settings → Codex → Permissions and Settings → pi → Project
// trust decide what an agent may do in EVERY lane the host launches (OWASP
// review M2); Settings → Behavior → Auto-fetch origin is the one loop that
// fetches for the whole host; and Settings → Terminal's two switches decide
// what the host keeps of every dock shell, which every client that opens the
// tab shares (server-edition.md Appendix B, "One fix for all five"). Until
// this check, App.tsx pushed all five from `useEffect`s on every hydration,
// so whichever client loaded last decided them — and under one engine per
// host, opening a page was enough. The rule now: the four setters are called
// by the user's ACT on a Settings control, and by nothing a page does on its
// own.
//
// Read through the TypeScript AST, not a regex, for two reasons. Comments and
// JSX text are not string-literal nodes, so prose about the rule cannot satisfy
// it. And "not in a useEffect" is too weak to be the rule: an effect can reach a
// setter through a helper (`const push = () => invoke(…)`, then
// `useEffect(() => push())`), and a component that calls one while it RENDERS
// sets it on load just as surely. So the rule is stated positively — every way
// to a setter ends in an act — and checked by walking each reference. An act
// is an `onClick` handler, or an `onChange` on an intrinsic `<input>`,
// `<select>` or `<textarea>`, which React fires only for the user's own edit
// (a switch is a checkbox and a cadence is a select, and neither has a click
// of its own to hang the set on):
//
//   1. a setter is invoked inside an act's handler, or inside a named
//      function whose EVERY reference in the file is itself act-only;
//   2. the setters appear only in the Settings panels, so the hydration
//      owner, App.tsx, cannot hold them, and a new caller is an edit here;
//   3. each setter is reached from at least one act — a guard that a deleted
//      control also passes is a guard that cannot fail;
//   4. the host's own boot (`hostpolicy::boot`, which loads the file and runs
//      the one-time migration) is the ENGINE's act, and the engine is `worktrees
//      serve` — the desktop app is its client. So `cmd_serve` calls it exactly
//      once, after the one-engine refusal (`live_other`: boot may write the
//      file, and a second serve must not) and before `start(` binds the
//      listener (no client is served under the wrong policy); and the shim
//      calls it NOWHERE — a client that loaded host policy would be the "last
//      client to load decides" bug this check exists for, one process out, and
//      its copy of the overrides would sit in a process no lane is launched from.
//
//   5. serve tags every setter `HostConsole` or an `Engine` row whose
//      capability is `admin` (Phase 2 §3.4: Codex and pi console-only, the
//      other three `admin`): a `read` or `write` row is one any granted device
//      may call.
//
//   6. ui-state.json keeps all five keys, and the panel that sets each one
//      copies the host's stored value into it after a set the host accepted.
//      This build never reads them; v0.42.0 and older read ONLY ui-state.json
//      for the agent policies and v0.43 for all five, so a downgrade (a
//      reinstall, or "Install <tag>") that found them gone fell back to the
//      defaults — Auto-review, wider than a user's Ask — and wrote those back.
//      Run through the REAL `loadSettings` and its whole-blob save, with an
//      older-shaped blob on disk, so any way of dropping them fails.
//
//   7. that copy is WRITE-ONLY: a host key is read from a Settings object only
//      inside the function that also writes it into Settings — the mirror,
//      which reads it to skip a write that would change nothing. A control
//      that renders `settings.<key>`, or an effect that pushes it, is the
//      client's copy deciding for the host again.
//
// The pre-fix App.tsx fails 1-3 and 7 for every one of the three engine
// policies; a lib.rs that still boots host policy, or a cmd_serve that boots it
// out of order or not at all, fails 4
// (`node app/scripts/hostpolicy-check.mjs [path/to/lib.rs] [path/to/serve/mod.rs] [path/to/serve/dispatch.rs]`);
// the pre-fix dispatch.rs fails 5; the settings.ts that deleted the two agent
// keys on load fails 6, and so does a panel that never mirrors its key.
//
//   8. the auto-fetch and terminal-history controls, RUN under stubs: on load
//      they read and do nothing else; an act sends one set (a term switch only
//      itself); only an accepted key is copied, with the host's value; the
//      newest read wins; a refusal rolls back.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { transformWithEsbuild } from "vite";

const SRC = fileURLToPath(new URL("../src/", import.meta.url));
const LIB = process.argv[2] ?? fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url));
const SERVE = process.argv[3] ?? fileURLToPath(new URL("../../crates/worktrees-cli/src/serve/mod.rs", import.meta.url));
const DISPATCH = process.argv[4] ?? fileURLToPath(new URL("../../crates/worktrees-cli/src/serve/dispatch.rs", import.meta.url));
const SETTERS = ["set_codex_permissions", "set_pi_trust", "set_fetch_interval", "set_term_history_opts"];
/** Each host key in ui-state.json, the setter that changes it, and a value
 *  that is NOT its default (so DEFAULTS cannot pass rule 6 by accident). */
const KEYS = [
  ["codex_permissions", "set_codex_permissions", "ask"],
  ["pi_project_trust", "set_pi_trust", "ask"],
  ["fetch_interval_min", "set_fetch_interval", 15],
  ["term_persist_scrollback", "set_term_history_opts", false],
  ["term_per_tab_history", "set_term_history_opts", false],
];
const HOST_KEYS = new Set(KEYS.map(([k]) => k));
const ALLOWED = new Set(["SettingsSheet.tsx", "PiPanel.tsx", "HostPolicyControls.tsx"]);
const EFFECTS = new Set(["useEffect", "useLayoutEffect", "useInsertionEffect"]);
/** The intrinsic elements whose `onChange` only the user's own edit fires. */
const FORM_CONTROLS = new Set(["input", "select", "textarea"]);

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);

// Every source file the app ships. The mock is the BACKEND's stand-in, so it
// handles these commands rather than calling them.
const files = [];
const walk = (dir) => {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) { if (e.name !== "mock") walk(p); }
    else if (/\.tsx?$/.test(e.name) && !e.name.endsWith(".d.ts")) files.push(p);
  }
};
walk(SRC);

// `forEachChild` STOPS at the first callback that returns something truthy,
// so the recursion must return nothing — returning `out` walks first children
// only, and every rule here then passes on a file it never read.
const descendants = (node, pred, out = []) => {
  if (pred(node)) out.push(node);
  ts.forEachChild(node, (c) => { descendants(c, pred, out); });
  return out;
};
/** A name listed in one of ipc.ts's remote-door tables (`ADMIN_ROWS`,
 *  `CONSOLE_ROWS`): row names a remote window refuses for a device, compared
 *  with dispatch.rs by engine-route-check — a list, never a call. Exactly
 *  those two array literals in ipc.ts; anything else naming a setter counts. */
const ROUTE_TABLES = ["ADMIN_ROWS", "CONSOLE_ROWS"];
const inRouteTable = (n) => {
  const arr = n.parent, decl = arr?.parent;
  return !!arr && ts.isArrayLiteralExpression(arr) && !!decl && ts.isVariableDeclaration(decl) &&
    ROUTE_TABLES.includes(decl.name.getText()) && path.basename(decl.getSourceFile().fileName) === "ipc.ts";
};
const isSetter = (n) => (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) && SETTERS.includes(n.text) && !inRouteTable(n);
const isFn = (n) => ts.isFunctionDeclaration(n) || ts.isFunctionExpression(n) || ts.isArrowFunction(n) || ts.isMethodDeclaration(n);
/** `{ key: … }` (or `{ key }`) handed to a call: a write of `key` into Settings. */
const isKeyWrite = (n, key) => ts.isObjectLiteralExpression(n) && n.parent && ts.isCallExpression(n.parent)
  && n.properties.some((p) => (ts.isPropertyAssignment(p) || ts.isShorthandPropertyAssignment(p)) && p.name.getText() === key);
/** The name a function is bound to: `function f`, `const f = () =>`, `f: () =>`. */
const boundName = (fn) => {
  if ((ts.isFunctionDeclaration(fn) || ts.isMethodDeclaration(fn)) && fn.name) return fn.name.getText();
  const p = fn.parent;
  if (p && (ts.isVariableDeclaration(p) || ts.isPropertyAssignment(p)) && ts.isIdentifier(p.name)) return p.name.text;
  return null;
};
const calleeName = (call) => {
  const e = call.expression;
  return ts.isIdentifier(e) ? e.text : ts.isPropertyAccessExpression(e) ? e.name.text : null;
};
/** An identifier that names a binding, as opposed to `x.name` or `{ name: … }`. */
const isReference = (id) => {
  const p = id.parent;
  if (ts.isPropertyAccessExpression(p) && p.name === id) return false;
  if (ts.isPropertyAssignment(p) && p.name === id) return false;
  if ((ts.isVariableDeclaration(p) || ts.isFunctionDeclaration(p) || ts.isParameter(p)) && p.name === id) return false;
  return true;
};

/** `onClick` anywhere, or `onChange` on an intrinsic form control. */
const isAct = (attr) => {
  const name = attr.name.getText();
  if (name === "onClick") return true;
  const el = attr.parent?.parent;
  return name === "onChange" && el && (ts.isJsxOpeningElement(el) || ts.isJsxSelfClosingElement(el))
    && ts.isIdentifier(el.tagName) && FORM_CONTROLS.has(el.tagName.text);
};

/**
 * Where `node` is reached from, walking out through anonymous functions:
 * `{ click }` inside an act's handler, `{ fn }` the first NAMED function,
 * or `{ other }` — an effect, another attribute, a render body, the module.
 */
function reachedFrom(node) {
  for (let p = node.parent; p; p = p.parent) {
    if (ts.isJsxAttribute(p)) return isAct(p) ? { click: true } : { other: `the ${p.name.getText()} attribute` };
    if (ts.isCallExpression(p) && EFFECTS.has(calleeName(p)) && node.getStart() > p.expression.getEnd()) return { other: `a ${calleeName(p)}` };
    if (isFn(p)) { const name = boundName(p); if (name) return { fn: name }; }
  }
  return { other: "module scope" };
}

const seen = Object.fromEntries(SETTERS.map((s) => [s, { files: new Set(), clicked: false }]));

for (const file of files) {
  const rel = path.relative(SRC, file);
  const sf = ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true,
    file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const line = (n) => `${rel}:${sf.getLineAndCharacterOfPosition(n.getStart(sf)).line + 1}`;
  const mentions = descendants(sf, isSetter);
  if (!mentions.length) continue;
  const ids = descendants(sf, (n) => ts.isIdentifier(n) && isReference(n));

  // A named function is click-only when every reference to it is: inside an
  // onClick, or inside another click-only function. Memoised, and a cycle
  // (mutual recursion) proves nothing, so it counts as not click-only.
  const memo = new Map();
  const clickOnly = (name) => {
    if (memo.has(name)) return memo.get(name);
    memo.set(name, { ok: false, why: `${name} is part of a cycle`, clicked: false });
    const refs = ids.filter((id) => id.text === name);
    let res = { ok: refs.length > 0, why: `${name} is used nowhere in this file`, clicked: false };
    for (const r of refs) {
      const from = reachedFrom(r);
      if (from.click) { res.clicked = true; continue; }
      if (from.fn && from.fn !== name) {
        const up = clickOnly(from.fn);
        if (up.ok) { res.clicked ||= up.clicked; continue; }
        res = { ok: false, why: `${line(r)}: ${name} is used from ${from.fn}, and ${up.why}`, clicked: false };
        break;
      }
      res = { ok: false, why: `${line(r)}: ${name} is used from ${from.other ?? "itself"}`, clicked: false };
      break;
    }
    memo.set(name, res);
    return res;
  };

  // Rule 1.
  for (const m of mentions) {
    seen[m.text].files.add(rel);
    const from = reachedFrom(m);
    if (from.click) { seen[m.text].clicked = true; continue; }
    if (from.other) { fail(`${line(m)} — ${m.text} is invoked from ${from.other}; loading a client would set host policy`); continue; }
    const r = clickOnly(from.fn);
    if (r.ok) { seen[m.text].clicked ||= r.clicked; continue; }
    fail(`${line(m)} — ${m.text} is invoked by ${from.fn}, which is not click-only: ${r.why}`);
  }
}

if (!failed) ok(`every way to ${SETTERS.join(" / ")} ends in the user's act`);

// Rule 2.
for (const s of SETTERS) {
  const where = [...seen[s].files];
  const stray = where.filter((f) => !ALLOWED.has(path.basename(f)));
  if (stray.length) fail(`${s} appears outside the Settings panels: ${stray.join(", ")}`);
  else if (where.length) ok(`${s} appears only in ${where.join(", ")}`);
}

// Rule 3.
for (const s of SETTERS) {
  if (!seen[s].files.size) fail(`${s} is invoked nowhere — the Settings control that sets it is gone`);
  else if (!seen[s].clicked) fail(`${s} is not reached from any act — only the user's act may change host policy`);
  else ok(`${s} is reached from an act`);
}

// Rule 5. Parsed the way engine-route-check.mjs parses the table: rows only,
// with `//` comments dropped so prose naming a row cannot be one.
{
  const table = fs.readFileSync(DISPATCH, "utf8").split(/\n#\[cfg\(test\)\]\s*\nmod tests\b/)[0]
    .split("\n").map((l) => l.replace(/\/\/.*$/, "")).join("\n");
  for (const s of SETTERS) {
    const row = new RegExp(`^ {4}${s}: ([A-Za-z]+)(?: \\[(\\w+)[^\\]]*\\])? =>`, "m").exec(table);
    const tag = row && (row[2] ? `${row[1]} [${row[2]}]` : row[1]);
    if (!tag) fail(`serve's dispatch table has no row for ${s}`);
    else if (row[1] !== "HostConsole" && tag !== "Engine [admin]") fail(`serve tags ${s} \`${tag}\` — host policy is \`admin\` at least: any granted device may call a read or write row`);
    else ok(`serve tags ${s} ${tag}`);
  }
}

// Rule 4. Line comments are dropped first, so prose naming the call cannot
// count as the call.
{
  const strip = (s) => s.split("\n#[cfg(test)]")[0].split("\n").map((l) => l.replace(/\/\/.*$/, "")).join("\n");
  const lib = strip(fs.readFileSync(LIB, "utf8"));
  const inShim = (lib.match(/hostpolicy::boot\(/g) ?? []).length;
  if (inShim) fail(`the shim calls hostpolicy::boot ${inShim} time(s) — it is serve's client, not the engine; loading host policy is serve's act (cmd_serve)`);
  else ok("the shim never loads host policy — it is the engine's client");
  const serve = strip(fs.readFileSync(SERVE, "utf8"));
  const body = /pub fn cmd_serve\(args: &\[String\]\) -> i32 \{([\s\S]*?)\n\}/.exec(serve)?.[1] ?? "";
  const boots = [...body.matchAll(/worktrees_api::hostpolicy::boot\(/g)].map((m) => m.index);
  const refusal = body.indexOf("servefile::live_other(");
  const bind = body.search(/\bstart\(opts\.config_dir/);
  if (!body) fail("cmd_serve is not where this rule looks (crates/worktrees-cli/src/serve/mod.rs)");
  else if (boots.length !== 1) fail(`cmd_serve calls hostpolicy::boot ${boots.length} times — expected exactly 1`);
  else if (refusal < 0 || boots[0] < refusal) fail("cmd_serve loads host policy BEFORE the one-engine refusal — a second serve on the same config dir could write the file under a live one");
  else if (bind < 0 || boots[0] > bind) fail("cmd_serve loads host policy AFTER start( binds the listener — a client could be served under the wrong policy");
  else ok("cmd_serve loads host policy once, after the one-engine refusal and before the listener binds");
}

// Rule 6. The real settings.ts, inlined the way layout-check.mjs does it (a
// data: URL has no base for `./afterglow`); `invoke` is the only stub, serving
// the blob on disk and recording what the whole-blob save writes.
{
  const MIRROR = Object.fromEntries(KEYS.map(([k, , v]) => [k, v]));
  const read = (rel) => fs.readFileSync(path.join(SRC, rel), "utf8");
  const inline = (rel) => read(rel).replace(/^export /gm, "") + "\n";
  const stripImport = (src, mod) => {
    const re = new RegExp(`^import \\{[^}]*\\} from "\\./${mod}";$`, "m");
    if (!re.test(src)) fail(`settings.ts: no \`import { … } from "./${mod}"\` line — rule 6's inlining is stale`);
    return src.replace(re, "");
  };
  let stored = null;
  const saved = [];
  globalThis.__hostpolicyInvoke = (cmd, a) => {
    if (cmd === "set_settings") saved.push(a.settings);
    return Promise.resolve(cmd === "get_settings" ? structuredClone(stored) : null);
  };
  const src = inline("afterglow.ts") + inline("harness.ts") +
    stripImport(stripImport(read("settings.ts"), "afterglow"), "harness")
      .replace(/^import \{ invoke \}.*$/m, "const invoke = (cmd, a) => globalThis.__hostpolicyInvoke(cmd, a);");
  const js = (await transformWithEsbuild(src, "settings.ts", { loader: "ts", format: "esm" })).code;
  const S = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
  const pick = (o) => Object.fromEntries(KEYS.map(([k]) => [k, o?.[k]]));
  const keeps = (o) => KEYS.every(([k]) => o?.[k] === MIRROR[k]);

  // A v0.42.0 blob: no settings_rev, so `migrate` runs and saves at once —
  // the very save that wrote the keys out in the rehearsal.
  stored = { theme: "nord", ...MIRROR };
  const s = await S.loadSettings();
  await new Promise((r) => setTimeout(r, 400)); // saveSettings debounces 250ms
  if (!keeps(s)) fail(`loadSettings dropped the downgrade mirror: loaded ${JSON.stringify(pick(s))} from ${JSON.stringify(MIRROR)} — an older build would fall back to its defaults`);
  else ok(`loadSettings keeps ${KEYS.map(([k]) => k).join(" / ")} for an older build`);
  if (!saved.length) fail("loading a v0.42.0 blob saved nothing — the migration's save is gone, so this rule no longer exercises the whole-blob write");
  else if (!keeps(saved[saved.length - 1])) fail(`the whole-blob save wrote ${JSON.stringify(pick(saved.at(-1)))} to ui-state.json — a downgrade would read the defaults`);
  else ok("the whole-blob save writes them back to ui-state.json");

  // …and each panel keeps the copy CURRENT: the file that sets a key must
  // write that key into Settings, or the mirror holds the pre-upgrade value
  // forever and a downgrade undoes every change made since.
  for (const [key, setter] of KEYS) {
    const where = [...seen[setter].files];
    const missing = where.filter((rel) => {
      const sf = ts.createSourceFile(rel, read(rel), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
      return !descendants(sf, (n) => isKeyWrite(n, key)).length;
    });
    if (!where.length) fail(`nothing sets \`${key}\` (${setter}) — rule 6 has no panel to hold to the mirror`);
    else if (missing.length) fail(`${missing.join(", ")} sets \`${key}\` (${setter}) but never writes it into Settings — the downgrade mirror would go stale at the first change`);
    else ok(`${where.join(", ")} mirrors \`${key}\` into Settings`);
  }
}

// Rule 7. Every READ of a host key off an object — `x.key`, `x["key"]`,
// `const { key } = x` — must sit in the same function as a write of that key
// into Settings, i.e. inside the mirror. A render body that also holds a
// control's onChange does not count: the write is in the handler, a function
// of its own.
{
  const innermostFn = (n) => { for (let p = n.parent; p; p = p.parent) if (isFn(p)) return p; return null; };
  const readsOf = (n) => {
    if (ts.isPropertyAccessExpression(n) && HOST_KEYS.has(n.name.text)) return n.name.text;
    if (ts.isElementAccessExpression(n) && ts.isStringLiteralLike(n.argumentExpression) && HOST_KEYS.has(n.argumentExpression.text)) return n.argumentExpression.text;
    if (ts.isBindingElement(n) && ts.isObjectBindingPattern(n.parent)) {
      const k = (n.propertyName ?? n.name).getText();
      if (HOST_KEYS.has(k)) return k;
    }
    return null;
  };
  const bad = Object.fromEntries(KEYS.map(([k]) => [k, []]));
  for (const file of files) {
    const rel = path.relative(SRC, file);
    const sf = ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true,
      file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const line = (n) => `${rel}:${sf.getLineAndCharacterOfPosition(n.getStart(sf)).line + 1}`;
    for (const r of descendants(sf, (n) => readsOf(n) !== null)) {
      const key = readsOf(r);
      const fn = innermostFn(r);
      const mirrored = fn && descendants(fn, (n) => isKeyWrite(n, key) && innermostFn(n) === fn).length > 0;
      if (!mirrored) bad[key].push(line(r));
    }
  }
  for (const [key] of KEYS) {
    if (bad[key].length) fail(`\`${key}\` is read from the client's settings outside the mirror (${bad[key].join(", ")}) — the control must render, and the host must take, the HOST's value`);
    else ok(`\`${key}\` is read from the client's settings only to keep the mirror`);
  }
}

// Rule 8. The two engine-policy controls, RUN: the real HostPolicyControls.tsx
// under a hook runtime of a few lines and an `invoke` the test resolves by
// hand (ctxmenu-check.mjs's shape). Rules 1-7 read the code; this one holds
// what D6 made binding to what the controls DO: on load, a read and nothing
// else — no set, and no copy into Settings even when the client's copy
// disagrees with the host; an act sends one set, and the term switches send
// only the switch that moved; only a key the host ACCEPTED is copied, with the
// value the host then reports; the newest read wins; a refusal rolls back.
{
  const FILE = path.join(SRC, "HostPolicyControls.tsx");
  if (!fs.existsSync(FILE)) fail("HostPolicyControls.tsx is missing — the auto-fetch and terminal-history controls must render the host's value");
  else {
    const body = fs.readFileSync(FILE, "utf8").split("\n")
      .filter((l) => !/^import\s/.test(l)).join("\n")
      .replace(/^export function /gm, "function ");
    const wrapped = `export function build(env: any) {
  const { useCallback, useEffect, useRef, useState, h, F, invoke } = env;
${body}
  return { FetchIntervalSection, TermHistorySection };
}`;
    const js = (await transformWithEsbuild(wrapped, "hostpolicy-controls.tsx", {
      loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F",
    })).code;
    const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
    const same = (a, b) => !!a && !!b && a.length === b.length && a.every((x, i) => Object.is(x, b[i]));
    const tick = () => new Promise((r) => setTimeout(r, 0));

    /** One mount: hooks by call order, effects after render, setState marks
     *  the instance dirty and `settle` re-renders it. */
    const mount = (name, props) => {
      const slots = [];
      const effects = [];
      const cleanups = {};
      const calls = [];
      let i = 0, dirty = false, tree = null;
      const env = {
        useState: (init) => {
          const k = i++;
          if (!(k in slots)) slots[k] = { v: init };
          const s = slots[k];
          return [s.v, (n) => { const v = typeof n === "function" ? n(s.v) : n; if (!Object.is(v, s.v)) { s.v = v; dirty = true; } }];
        },
        useRef: (init) => { const k = i++; if (!(k in slots)) slots[k] = { current: init }; return slots[k]; },
        useCallback: (fn, deps) => { const k = i++; if (slots[k] && same(slots[k].deps, deps)) return slots[k].fn; slots[k] = { fn, deps }; return fn; },
        useEffect: (fn, deps) => { const k = i++; if (slots[k] && same(slots[k].deps, deps)) return; slots[k] = { deps }; effects.push([k, fn]); },
        h: (type, p, ...children) => ({ type, props: p ?? {}, children: children.flat(Infinity) }),
        F: "F",
        invoke: (cmd, args) => new Promise((resolve, reject) => calls.push({ cmd, args, resolve, reject })),
      };
      const C = build(env)[name];
      const render = () => {
        i = 0; dirty = false; tree = C(props);
        for (const [k, fn] of effects.splice(0)) { cleanups[k]?.(); const c = fn(); if (typeof c === "function") cleanups[k] = c; }
      };
      render();
      const nodes = (t, out = []) => { if (t && typeof t === "object") { out.push(t); for (const c of t.children) nodes(c, out); } return out; };
      return {
        calls,
        find: (pred) => nodes(tree).filter(pred),
        setProps: (p) => { props = { ...props, ...p }; render(); },
        async settle() { for (let n = 0; n < 5; n++) { await tick(); if (dirty) render(); } },
        /** The newest call to `cmd`, which must be pending. */
        last: (cmd) => calls.filter((c) => c.cmd === cmd).at(-1),
        count: (cmd) => calls.filter((c) => c.cmd === cmd).length,
      };
    };
    const is = (got, want, what) => {
      if (JSON.stringify(got) === JSON.stringify(want)) ok(what);
      else fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
    };

    // ── Auto-fetch ──
    {
      const mirrored = [];
      const failed = [];
      const onFail = (e) => failed.push(String(e));
      const m = mount("FetchIntervalSection", { settings: { fetch_interval_min: 60 }, onChange: (p) => mirrored.push(p), onFail });
      const value = () => m.find((n) => n.type === "select")[0]?.props.value;
      const pick = (v) => m.find((n) => n.type === "select")[0].props.onChange({ currentTarget: { value: String(v) } });
      is(m.calls.map((c) => c.cmd), ["get_fetch_interval"], "auto-fetch: loading the control reads the host and sets nothing");
      m.last("get_fetch_interval").resolve(5);
      await m.settle();
      is(value(), 5, "auto-fetch: the control shows the HOST's cadence, not the client's copy (60)");
      is(mirrored, [], "auto-fetch: nothing is copied into Settings on load, though the client's copy disagrees");

      pick(15);
      is([m.count("set_fetch_interval"), m.last("set_fetch_interval")?.args], [1, { mins: 15 }], "auto-fetch: an act sends one set, of what was picked");
      m.last("set_fetch_interval").resolve(null);
      await m.settle();
      m.last("get_fetch_interval").resolve(15);
      await m.settle();
      is(mirrored, [{ fetch_interval_min: 15 }], "auto-fetch: after the host accepts, the value it reports is copied into Settings");
      m.setProps({ settings: { fetch_interval_min: 15 } });

      // Two acts in flight: the read after the first comes back LAST and stale.
      pick(60);
      m.last("set_fetch_interval").resolve(null);
      await m.settle();
      const stale = m.last("get_fetch_interval");
      pick(5);
      m.last("set_fetch_interval").resolve(null);
      await m.settle();
      m.last("get_fetch_interval").resolve(5);
      await m.settle();
      stale.resolve(60);
      await m.settle();
      is([value(), mirrored.at(-1)], [5, { fetch_interval_min: 5 }], "auto-fetch: a read overtaken by a newer one is dropped");

      pick(60);
      m.last("set_fetch_interval").reject("host-policy.json is not valid JSON");
      await m.settle();
      is([value(), failed.length === 1 && /Auto-fetch/.test(failed[0])], [5, true], "auto-fetch: a refused set rolls back and is reported");
    }

    // ── Terminal history ──
    {
      const mirrored = [];
      const failed = [];
      const m = mount("TermHistorySection", {
        settings: { term_persist_scrollback: true, term_per_tab_history: true },
        onChange: (p) => mirrored.push(p), onFail: (e) => failed.push(String(e)),
      });
      const boxes = () => m.find((n) => n.type === "input" && n.props.type === "checkbox");
      const checked = () => boxes().map((b) => b.props.checked);
      const flip = (i, on) => boxes()[i].props.onChange({ currentTarget: { checked: on } });
      is(m.calls.map((c) => c.cmd), ["get_term_history_opts"], "terminal history: loading the control reads the host and sets nothing");
      m.last("get_term_history_opts").resolve({ persist_scrollback: false, per_tab_history: true });
      await m.settle();
      is(checked(), [false, true], "terminal history: the switches show the HOST's values, not the client's copy");
      is(mirrored, [], "terminal history: nothing is copied into Settings on load, though the client's copy disagrees");

      flip(1, false);
      is([m.count("set_term_history_opts"), m.last("set_term_history_opts")?.args], [1, { perTabHistory: false }],
        "terminal history: a switch sends ONLY itself, never this window's copy of the other");
      m.last("set_term_history_opts").resolve(null);
      await m.settle();
      m.last("get_term_history_opts").resolve({ persist_scrollback: false, per_tab_history: false });
      await m.settle();
      is(mirrored, [{ term_per_tab_history: false }],
        "terminal history: only the switch the host accepted is copied into Settings — the other may be its default speaking");

      flip(0, true);
      m.last("set_term_history_opts").reject("host-policy.json is not valid JSON");
      await m.settle();
      is([checked()[0], failed.length === 1 && /Terminal history/.test(failed[0])], [false, true], "terminal history: a refused set rolls back and is reported");
    }
  }
}

console.log(failed ? `\nhostpolicy-check: ${failed} failure(s)` : "\nhostpolicy-check: all good");
process.exit(failed ? 1 : 0);
