// BranchCombo's highlight, against the REAL component (sliced out of App.tsx
// and evaluated under React stubs): after the list shrinks under a highlight
// (a re-read of the branches, with the field's text unchanged), the row drawn
// as highlighted, the row a screen reader is told about and the row Enter
// commits are ONE row, and it exists.
//
// Run from the repo root, as CI does:
//   node app/scripts/combo-check.mjs [App.tsx]     exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const APP = process.argv[2] || fileURLToPath(new URL("../src/App.tsx", import.meta.url));
const src = fs.readFileSync(APP, "utf8");
const from = src.indexOf("\nfunction BranchCombo({");
const to = src.indexOf("\n}\n", from);
if (from < 0 || to < 0) throw new Error("function BranchCombo not found in " + APP);

const js = (await transformWithEsbuild(`
type BranchList = any; type React = any; declare namespace React { type KeyboardEvent = any }
export function build(env: any) {
  const { useState, useRef, useEffect, useId, useEscape, Icons, h, F } = env;
${src.slice(from, to + 3)}
  return BranchCombo;
}`, "combo.tsx", { loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F" })).code;
const { build } = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

let failed = 0;
const eq = (a, b, msg) => {
  const same = JSON.stringify(a) === JSON.stringify(b);
  if (!same) failed++;
  console.log(`${same ? "ok" : "not ok"} — ${msg}${same ? "" : ` — got ${JSON.stringify(a)}, want ${JSON.stringify(b)}`}`);
};

/** One mounted combo: its state survives `render(props)`. */
function mount(props) {
  const slots = [];
  let at = 0, tree = null;
  const committed = [];
  const env = {
    useState: (v) => {
      const i = at++;
      if (!(i in slots)) slots[i] = v;
      return [slots[i], (x) => { slots[i] = typeof x === "function" ? x(slots[i]) : x; }];
    },
    useRef: (v) => { const i = at++; return (slots[i] ??= { current: v }); },
    useEffect: () => {}, useId: () => "u", useEscape: () => {},
    Icons: new Proxy({}, { get: () => "icon" }),
    h: (type, p, ...kids) => ({ type, p: p ?? {}, kids: kids.flat(Infinity).filter(Boolean) }), F: "f",
  };
  const Combo = build(env);
  const render = (over = {}) => {
    at = 0;
    Object.assign(props, over);
    tree = Combo({ placeholder: "", inputClass: "", ariaLabel: "", onChange() {}, onOpen() {}, onCommit: (b) => committed.push(b), ...props });
    return tree;
  };
  const all = (n = tree) => [n, ...n.kids.filter((k) => k && k.type).flatMap(all)];
  const input = () => all().find((n) => n.type === "input").p;
  render();
  return {
    render, committed,
    key: (key) => { input().onKeyDown({ key, preventDefault() {}, stopPropagation() {} }); render(); },
    focus: () => { input().onFocus(); render(); },
    /** The row drawn highlighted, the row announced, the row marked selected. */
    seen: () => {
      const rows = all().filter((n) => n.p.role === "option");
      const name = (n) => n && (n.p.className.includes("create") ? "create" : n.kids.join(""));
      const active = input()["aria-activedescendant"];
      return {
        lit: name(rows.find((n) => / hi\b/.test(n.p.className))) ?? null,
        selected: name(rows.find((n) => n.p["aria-selected"])) ?? null,
        announced: name(rows.find((n) => n.p.id === active)) ?? (active ? `dangling ${active}` : null),
      };
    },
  };
}
const one = (b) => ({ lit: b, selected: b, announced: b });
const list = (...branches) => ({ branches, default_base: "main" });

{
  const c = mount({ value: "", data: list("a", "b", "c", "d") });
  c.focus();
  eq(c.seen(), one("a"), "the list opens on its first row");
  for (let i = 0; i < 3; i++) c.key("ArrowDown");
  eq(c.seen(), one("d"), "↓ three times: the fourth row");
  c.render({ data: list("a", "b") });
  eq(c.seen(), one("b"), "the list shrank under the highlight: the last row is lit, selected and announced");
  c.key("Enter");
  eq(c.committed, ["b"], "…and Enter commits exactly that row");
}
{
  const c = mount({ value: "", data: list("a", "b", "c", "d", "e") });
  c.focus();
  for (let i = 0; i < 4; i++) c.key("ArrowDown");
  c.render({ data: list("a", "b", "c") });
  c.key("ArrowUp");
  eq(c.seen(), one("b"), "↑ after a shrink steps from the row that is lit");
  c.render({ data: list("a", "b", "c", "d", "e") });
  for (let i = 0; i < 3; i++) c.key("ArrowDown");
  c.render({ data: list("a", "b", "c") });
  c.key("ArrowDown");
  eq(c.seen(), one("a"), "↓ after a shrink wraps from the row that is lit");
}
{
  const c = mount({ value: "fe", data: list("feat-a", "feat-b", "feat-c") });
  c.focus();
  for (let i = 0; i < 2; i++) c.key("ArrowDown");
  c.render({ data: list() });
  eq(c.seen(), one("create"), "only the create row is left: it is the highlighted one");
  c.key("Enter");
  eq(c.committed, ["fe"], "…and Enter takes the typed text");
}
{
  const c = mount({ value: "x", data: list("xa", "xb"), allowCreate: false });
  c.focus();
  c.key("ArrowDown");
  c.render({ data: list() });
  eq(c.seen(), one(null), "no rows: nothing lit, nothing announced");
  c.key("Enter");
  eq(c.committed, ["x"], "…and Enter takes the typed text");
}

console.log(failed ? `\n${failed} failed` : "\nall ok");
process.exit(failed ? 1 : 0);
