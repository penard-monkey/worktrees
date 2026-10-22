// Every `.mini` the app renders has to be SEEABLE where it is rendered.
//
//   node app/scripts/minireveal-check.mjs [src-dir]   # exits non-zero on failure
//
// `.mini` is `opacity: 0` — it is the project header's hover-revealed button,
// and `.project-h:hover` / `:focus-within` is what shows it. A `.mini` placed
// anywhere else has no such hover, and the two that were (the git-init
// prompt's ✕, the init nudge's ✕) sat invisible from the day they were added:
// a card whose only way out could not be seen. Their context rules set
// `margin-left: auto` and nothing else, which reads like a styled control. No
// suite can see it — tsc, bats and the mock all pass, and the button is even
// clickable if you happen to hit the spot. Read through the TypeScript AST, as
// hostpolicy-check.mjs does, so the ancestor chain is the real JSX tree:
//
//   1. every JSX element whose STATIC className carries `mini` has a rule that
//      sets `opacity: 1` for it — a modifier (`.mini.pnew`) or a host it sits
//      inside (`.newform-h .mini`, `.project-h:hover .mini`);
//   2. a host that styles `button` with a background (`.newform button`, the
//      card's primary: 0,1,1 beats `.mini`'s 0,1,0) makes the revealing rule
//      restate `background`, or the ✕ is painted as the accent button;
//   3. a revealing rule that sets `background` brings its own `:hover` rule,
//      since it ties `.mini:hover` (0,2,0) and wins or loses on source order;
//   4. that hover's surface differs from the card's in EVERY theme (both
//      cards are `--bg-elev`, which is exactly `.mini:hover`'s fill);
//   5. the git-init prompt is on the Escape stack (`useEscape`), calling its
//      `onCancel` — xterm swallows a bubbling Escape, and nothing in the card
//      takes focus.
//
// A `.mini` rendered by a component used INSIDE another component's host is
// invisible to rule 1's ancestor walk (the tree stops at the component), and
// fails here; give it a host rule of its own. The pre-fix tree fails 1 and 5
// (2-4 only ask about a rule that reveals). Each of 2-4 was shown red on its
// own: `opacity: 1` alone, the obvious fix, fails 2; dropping the `:hover`
// rule fails 3; a hover of `--bg-elev`, `--line` or `--bg-panel` fails 4 (the
// last two are 0-13 per channel off the card in four of six themes).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const SRC = process.argv[2] ?? fileURLToPath(new URL("../src/", import.meta.url));

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);

const tsx = [];
const cssText = [];
const walk = (dir) => {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) { if (e.name !== "mock") walk(p); }
    else if (e.name.endsWith(".tsx")) tsx.push(p);
    else if (e.name.endsWith(".css")) cssText.push(fs.readFileSync(p, "utf8"));
  }
};
walk(SRC);

// ── the stylesheet: innermost `selector { decls }` blocks, comments gone.
// `@media … {` never matches `[^{}]+\{[^{}]*\}`, so a nested rule comes out
// with its own selector, which is all the questions below need.
const rules = [];
for (const text of cssText) {
  const css = text.replace(/\/\*[\s\S]*?\*\//g, "");
  for (const m of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const decls = Object.fromEntries(
      m[2].split(";").map((d) => d.split(":")).filter((d) => d.length >= 2)
        .map(([k, ...v]) => [k.trim(), v.join(":").trim()]),
    );
    for (const sel of m[1].split(",")) rules.push({ sel: sel.trim(), decls });
  }
}
const bgOf = (d) => d.background ?? d["background-color"];
const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const PSEUDO = String.raw`(?::[\w-]+(?:\([^)]*\))?)*`;
/** The rule that reveals a `.mini` through modifier `m`, or through host `c`. */
const revealByModifier = (m) => rules.find((r) => r.sel === `.mini.${m}` && r.decls.opacity === "1");
const revealByHost = (c) => rules.find((r) =>
  new RegExp(`^\\.${esc(c)}${PSEUDO}\\s+\\.mini$`).test(r.sel) && r.decls.opacity === "1");

// ── the JSX: static class names only. A class added by a condition
// (`(armed ? " armed" : "")`) is not always there, so it reveals nothing.
const words = (s) => s.split(/\s+/).filter(Boolean);
function staticClasses(attr) {
  const init = attr.initializer;
  if (!init) return [];
  if (ts.isStringLiteral(init)) return words(init.text);
  if (!ts.isJsxExpression(init) || !init.expression) return [];
  const out = [];
  const visit = (e) => {
    if (ts.isParenthesizedExpression(e)) return; // a conditional piece
    if (ts.isStringLiteral(e) || ts.isNoSubstitutionTemplateLiteral(e)) out.push(...words(e.text));
    else if (ts.isBinaryExpression(e) && e.operatorToken.kind === ts.SyntaxKind.PlusToken) { visit(e.left); visit(e.right); }
    else if (ts.isTemplateExpression(e)) {
      // only the words that stand clear of a `${…}` on both sides
      const parts = [e.head.text, ...e.templateSpans.map((s) => s.literal.text)];
      parts.forEach((t, i) => {
        const w = words(t);
        if (i > 0 && !/^\s/.test(t)) w.shift();
        if (i < parts.length - 1 && !/\s$/.test(t)) w.pop();
        out.push(...w);
      });
    }
  };
  visit(init.expression);
  return out;
}
const classAttr = (attrs) => attrs.properties.find((p) => ts.isJsxAttribute(p) && p.name.getText() === "className");

const minis = []; // { file, line, classes, chain: string[][] (nearest ancestor first) }
for (const file of tsx) {
  const sf = ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const chain = [];
  const here = (node, attrs) => {
    const a = classAttr(attrs);
    const classes = a ? staticClasses(a) : [];
    if (classes.includes("mini")) {
      const { line } = sf.getLineAndCharacterOfPosition(node.getStart());
      minis.push({ file: path.basename(file), line: line + 1, classes, chain: [...chain].reverse() });
    }
    return classes;
  };
  // `forEachChild` stops at the first truthy return: this callback returns nothing.
  const visit = (node) => {
    if (ts.isJsxElement(node)) {
      chain.push(here(node, node.openingElement.attributes));
      node.children.forEach(visit);
      chain.pop();
    } else if (ts.isJsxSelfClosingElement(node)) {
      here(node, node.attributes);
    } else {
      ts.forEachChild(node, (c) => { visit(c); });
    }
  };
  visit(sf);
}

if (minis.length === 0) fail("no `.mini` found in any .tsx — did the class get renamed? this guard would pass on nothing");
else ok(`${minis.length} .mini element(s) found across the app's .tsx`);

// ── theme tokens, for rule 4
const themes = {};
for (const text of cssText) {
  for (const m of text.matchAll(/:root(?:\[data-theme="([^"]+)"\])?\s*\{([^}]*)\}/g)) {
    const t = (themes[m[1] ?? "tokyo-night (:root)"] ??= {});
    for (const v of m[2].matchAll(/(--[\w-]+):\s*(#[0-9a-fA-F]{6})\b/g)) t[v[1]] = v[2];
  }
}
const rgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
const tokenOf = (v) => /^var\((--[\w-]+)\)$/.exec(v ?? "")?.[1];

const hostBg = (chain) => {
  for (const classes of chain) for (const c of classes) {
    const r = rules.find((r) => r.sel === `.${c}` && bgOf(r.decls));
    if (r) return { host: c, bg: bgOf(r.decls) };
  }
  return null;
};

for (const m of minis) {
  const where = `${m.file}:${m.line} (.${m.classes.join(".")})`;
  const mod = m.classes.filter((c) => c !== "mini").map((c) => [c, revealByModifier(c)]).find(([, r]) => r);
  let host = null;
  for (const classes of m.chain) {
    for (const c of classes) { const r = revealByHost(c); if (r) { host = [c, r]; break; } }
    if (host) break;
  }
  const reveal = mod ? { via: `.mini.${mod[0]}`, rule: mod[1] } : host ? { via: host[1].sel, rule: host[1] } : null;
  // 1
  if (!reveal) {
    const inside = m.chain.map((cs) => cs.join(".")).filter(Boolean).slice(0, 3).join(" < ") || "no static ancestor";
    fail(`${where} — nothing sets opacity: 1 for it (inside ${inside}); .mini is opacity 0 outside a hovered .project-h, so it is invisible`);
    continue;
  }
  ok(`${where} is shown by \`${reveal.via}\``);
  // 2
  const trap = m.chain.flat().map((c) => rules.find((r) => r.sel === `.${c} button` && bgOf(r.decls))).find(Boolean);
  if (trap && !bgOf(reveal.rule.decls))
    fail(`${where} — \`${trap.sel}\` paints every button in it, and \`${reveal.via}\` does not restate background: the ✕ takes the primary button's fill`);
  else if (trap) ok(`${where} restates the mini look over \`${trap.sel}\``);
  // 3 + 4
  if (!host || mod || !bgOf(reveal.rule.decls)) continue;
  const c = host[0];
  const hover = rules.find((r) => new RegExp(`^\\.${esc(c)}\\s+\\.mini:hover$`).test(r.sel) && bgOf(r.decls));
  if (!hover) { fail(`${where} — \`${reveal.via}\` sets a background but there is no \`.${c} .mini:hover\`; it ties .mini:hover and hides it`); continue; }
  const card = hostBg(m.chain);
  const a = tokenOf(bgOf(hover.decls)), b = tokenOf(card?.bg);
  if (!card || !a || !b) { fail(`${where} — cannot resolve hover (${bgOf(hover.decls)}) or card (${card?.bg}) to a single token; compare them by hand in every theme and teach this check`); continue; }
  const flat = Object.entries(themes).filter(([, t]) => t[a] && t[b])
    .map(([name, t]) => [name, Math.max(...rgb(t[a]).map((v, i) => Math.abs(v - rgb(t[b])[i])))])
    .filter(([, d]) => d < 16);
  if (Object.keys(themes).length < 6) fail(`found ${Object.keys(themes).length} theme blocks, expected 6 — the theme parse broke`);
  else if (flat.length) fail(`${where} — hover ${a} is within ${flat.map(([n, d]) => `${d} of ${b} in ${n}`).join(", ")} (under 16 per channel reads as no hover)`);
  else ok(`${where} hover ${a} differs from the card's ${b} (.${card.host}) by ≥16 per channel in all six themes`);
}

// 5 — the git-init prompt owns Escape while it is up
{
  const app = path.join(SRC, "App.tsx");
  const sf = ts.createSourceFile(app, fs.readFileSync(app, "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  let fn = null;
  ts.forEachChild(sf, (n) => { if (ts.isFunctionDeclaration(n) && n.name?.text === "InitRepoPrompt") fn = n; });
  if (!fn) fail("App.tsx has no `function InitRepoPrompt` — renamed? re-point this check");
  else {
    const calls = [];
    const find = (n) => {
      if (ts.isCallExpression(n) && n.expression.getText() === "useEscape") calls.push(n);
      ts.forEachChild(n, (c) => { find(c); });
    };
    find(fn.body);
    const cancels = calls.filter((c) => c.arguments[0] && /\bonCancel\s*\(/.test(c.arguments[0].getText()));
    if (cancels.length) ok("InitRepoPrompt calls useEscape → onCancel");
    else fail("InitRepoPrompt does not call `useEscape` with its onCancel — Escape cannot dismiss it while the terminal has focus");
  }
}

if (failed) { console.log(`\n${failed} failure(s)`); process.exit(1); }
console.log("\nall .mini controls are revealed where they render");
