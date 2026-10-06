// Guards the two rules behind "+ is always visible" and ⌘N. Source-level, like
// navwire-check: it reads the real files, so a drift fails here, not in review.
//  1. the header + carries `pnew` and `.mini.pnew` is opacity 1 — the other
//     minis stay hover/focus-within revealed.
//  2. ⌘N is bound in the ONE global keydown handler, after the modal guard.
import { readFileSync } from "node:fs";
const dir = new URL("../src/", import.meta.url);
const app = readFileSync(new URL("App.tsx", dir), "utf8");
const css = readFileSync(new URL("App.css", dir), "utf8");
const fails = [];
const need = (ok, msg) => { if (!ok) fails.push(msg); };

need(/\.mini\.pnew\s*\{\s*opacity:\s*1;?\s*\}/.test(css), ".mini.pnew must be opacity: 1");
need(/\.project-h:focus-within \.mini/.test(css), "hover actions must also reveal on :focus-within");
need(/\.mini \{[^}]*opacity: 0/.test(css), ".mini must stay hidden at rest");
need(/className="mini pnew"[^>]*nav\.project\.new/.test(app) || /className="mini pnew" title="new worktree \(⌘N\)"/.test(app),
  "the header + needs `mini pnew` and ⌘N in its title");

const guard = app.indexOf("if ((e.metaKey || e.ctrlKey) && modalOpen())");
const chord = app.indexOf("newWorktreeRef.current()");
need(guard > 0 && chord > guard, "⌘N must be handled after the modal guard");
need(/\(k === "n" \|\| e\.code === "KeyN"\)/.test(app), "⌘N must match e.key with a KeyN fallback");

if (fails.length) { console.error(fails.join("\n")); process.exit(1); }
console.log("newworktree-check ok");
