// Loading a client must not set host policy.
//
//   node app/scripts/hostpolicy-check.mjs      # exits non-zero on failure
//
// Settings → Codex → Permissions and Settings → pi → Project trust decide what
// an agent may do in EVERY lane the host launches (`worktrees-api` hostpolicy,
// OWASP review M2). Until this check, App.tsx pushed both from a `useEffect` on
// every hydration, so whichever client loaded last decided them — and under one
// engine per host, opening a page was enough. The rule now: the two setters are
// called by a Settings CLICK, and by nothing a page does on its own.
//
// Read through the TypeScript AST, not a regex, for two reasons. Comments and
// JSX text are not string-literal nodes, so prose about the rule cannot satisfy
// it. And "not in a useEffect" is too weak to be the rule: an effect can reach a
// setter through a helper (`const push = () => invoke(…)`, then
// `useEffect(() => push())`), and a component that calls one while it RENDERS
// sets it on load just as surely. So the rule is stated positively — every way
// to a setter ends in an `onClick` — and checked by walking each reference:
//
//   1. a setter is invoked inside an `onClick` handler, or inside a named
//      function whose EVERY reference in the file is itself click-only;
//   2. the setters appear only in the two Settings panels, so the hydration
//      owner, App.tsx, cannot hold them, and a new caller is an edit here;
//   3. each setter is reached from at least one `onClick` — a guard that a
//      deleted control also passes is a guard that cannot fail;
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
//   5. `set_term_history_opts` — a client preference the page DOES push, from
//      an effect — is pushed only once hydration has landed: directly inside a
//      `useEffect` whose callback opens with `if (!hydratedTick) return;` and
//      lists `hydratedTick` in its deps. Before hydration `settings` holds
//      DEFAULTS (both switches on), and the backend it reaches need not be
//      fresh — a webview reload, or any client of a long-running engine, finds
//      the user's "off" already there and would turn persistence back on until
//      hydration landed, long enough for the 15 s flush to write every dirty
//      shell's ring to disk.
//
// The pre-fix App.tsx fails 1-3 and 5; a lib.rs that still boots host policy,
// or a cmd_serve that boots it out of order or not at all, fails 4
// (`node app/scripts/hostpolicy-check.mjs [path/to/lib.rs] [path/to/serve/mod.rs]`).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const SRC = fileURLToPath(new URL("../src/", import.meta.url));
const LIB = process.argv[2] ?? fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url));
const SERVE = process.argv[3] ?? fileURLToPath(new URL("../../crates/worktrees-cli/src/serve/mod.rs", import.meta.url));
const SETTERS = ["set_codex_permissions", "set_pi_trust"];
const ALLOWED = new Set(["SettingsSheet.tsx", "PiPanel.tsx"]);
const EFFECTS = new Set(["useEffect", "useLayoutEffect", "useInsertionEffect"]);

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
const isSetter = (n) => (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) && SETTERS.includes(n.text);
const isFn = (n) => ts.isFunctionDeclaration(n) || ts.isFunctionExpression(n) || ts.isArrowFunction(n) || ts.isMethodDeclaration(n);
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

/**
 * Where `node` is reached from, walking out through anonymous functions:
 * `{ click }` inside an onClick attribute, `{ fn }` the first NAMED function,
 * or `{ other }` — an effect, another attribute, a render body, the module.
 */
function reachedFrom(node) {
  for (let p = node.parent; p; p = p.parent) {
    if (ts.isJsxAttribute(p)) return p.name.getText() === "onClick" ? { click: true } : { other: `the ${p.name.getText()} attribute` };
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

if (!failed) ok("every way to set_codex_permissions / set_pi_trust ends in an onClick");

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
  else if (!seen[s].clicked) fail(`${s} is not reached from any onClick — only a click may change host policy`);
  else ok(`${s} is reached from an onClick`);
}

// Rule 5.
{
  const PUSH = "set_term_history_opts";
  /** `if (!hydratedTick) return;` (or `{ return; }`) as a statement. */
  const isGuard = (st) => st && ts.isIfStatement(st) && !st.elseStatement
    && ts.isPrefixUnaryExpression(st.expression) && st.expression.operator === ts.SyntaxKind.ExclamationToken
    && ts.isIdentifier(st.expression.operand) && st.expression.operand.text === "hydratedTick"
    && (ts.isReturnStatement(st.thenStatement)
      || (ts.isBlock(st.thenStatement) && st.thenStatement.statements.length === 1 && ts.isReturnStatement(st.thenStatement.statements[0])));
  let pushes = 0;
  for (const file of files) {
    const rel = path.relative(SRC, file);
    const sf = ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true,
      file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const line = (n) => `${rel}:${sf.getLineAndCharacterOfPosition(n.getStart(sf)).line + 1}`;
    for (const m of descendants(sf, (n) => (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) && n.text === PUSH)) {
      pushes++;
      let effect = null;
      for (let p = m.parent; p && !effect; p = p.parent) if (ts.isCallExpression(p) && EFFECTS.has(calleeName(p))) effect = p;
      const cb = effect?.arguments[0];
      const deps = effect?.arguments[1];
      if (!cb || !(ts.isArrowFunction(cb) || ts.isFunctionExpression(cb)) || m.getStart(sf) < cb.getStart(sf)) {
        fail(`${line(m)} — ${PUSH} is invoked outside an effect; only a hydration-gated effect may push it`);
      } else if (!ts.isBlock(cb.body) || !isGuard(cb.body.statements[0])) {
        fail(`${line(m)} — ${PUSH} is pushed from an effect that does not open with \`if (!hydratedTick) return;\` — a reload would push DEFAULTS (persistence on) over the user's choice`);
      } else if (!deps || !ts.isArrayLiteralExpression(deps) || !deps.elements.some((e) => ts.isIdentifier(e) && e.text === "hydratedTick")) {
        fail(`${line(m)} — the effect pushing ${PUSH} is gated on hydratedTick but does not list it in its deps, so it never re-runs when hydration lands`);
      } else ok(`${line(m)} — ${PUSH} is pushed only after hydration`);
    }
  }
  if (!pushes) fail(`${PUSH} is pushed nowhere — the backend would never learn the user's terminal-history choice`);
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

console.log(failed ? `\nhostpolicy-check: ${failed} failure(s)` : "\nhostpolicy-check: all good");
process.exit(failed ? 1 : 0);
