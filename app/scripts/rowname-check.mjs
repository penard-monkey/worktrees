// Guards renaming a row in the nav (rowname.ts, RowNameField.tsx).
//
//   node app/scripts/rowname-check.mjs        # exits non-zero on failure
//
// Drives the REAL src/rowname.ts (through esbuild, as afterglow-check does),
// then reads the sources for the wiring a browser drive cannot see cheaply:
// that the field's Escape goes through useEscape, that it is module scope,
// that every nameable nav row offers Rename in its right-click menu, that F2
// reaches the rule and never from a text field, and that no rename control
// carries user text into a usage key.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const eq = (got, want, what) => { if (JSON.stringify(got) !== JSON.stringify(want)) fail(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`); };
const has = (src, needle, what) => { if (!src.includes(needle)) fail(`${what}: missing ${JSON.stringify(needle)}`); };

const js = (await transformWithEsbuild(read("../src/rowname.ts"), "rowname.ts", { loader: "ts", format: "esm" })).code;
const R = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

// ── a lane: the field holds the name on screen ──
const none = { kind: "none" }, set = (title) => ({ kind: "set", title });
eq(R.laneChange("Release prep", undefined, "feat-x"), set("Release prep"), "a lane with no title takes one");
eq(R.laneChange("  Release prep  ", "Old", "feat-x"), set("Release prep"), "a title is trimmed");
eq(R.laneChange("Old", "Old", "feat-x"), none, "the same title is no change");
eq(R.laneChange("feat-x", undefined, "feat-x"), none, "an untitled lane left as its slug is no change");
eq(R.laneChange("feat-x", "Old", "feat-x"), set(""), "the slug typed back clears the title");
eq(R.laneChange("", "Old", "feat-x"), set(""), "an empty field clears the title");
eq(R.laneChange("   ", undefined, "feat-x"), none, "an empty field on an untitled lane is no change");

// ── a chat or a workspace always has a name ──
eq(R.namedChange("Order of release", "New chat"), set("Order of release"), "a chat takes a new title");
eq(R.namedChange("", "New chat"), none, "an empty chat title is a cancel");
eq(R.namedChange(" New chat ", "New chat"), none, "the same chat title is no change");

// ── F2 ──
const k = (o = {}) => ({ key: "F2", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, repeat: false, ...o });
eq(R.isRenameKey(k(), false), true, "plain F2 renames");
eq(R.isRenameKey(k(), true), false, "F2 in a text field (the terminal's textarea too) is the field's");
for (const m of ["metaKey", "ctrlKey", "altKey", "shiftKey", "repeat"]) eq(R.isRenameKey(k({ [m]: true }), false), false, `F2 with ${m} is not a rename`);
eq(R.isRenameKey(k({ key: "F3" }), false), false, "another key is not a rename");

// ── the field ──
const field = read("../src/RowNameField.tsx");
has(field, "useEscape(", "the field's Escape goes through useEscape");
if (/addEventListener\(\s*["']keydown/.test(field)) fail("the field adds a keydown listener of its own");
if (!/^export function RowNameField\(/m.test(field)) fail("RowNameField is not a module-scope export");
has(field, "onBlur=", "blur saves");
has(field, "onPointerDown={stop}", "a press in the field never arms the row's drag");
has(field, "onDoubleClick={stop}", "a double-click in the field never enters the place");
if (/data-track=\{/.test(field)) fail("the field computes its usage key");

// ── the rows ──
const app = read("../src/App.tsx");
const chat = read("../src/WorkspaceChat.tsx");
has(app, 'data-testid="ctx-rename" data-track="nav.row.rename"', "a lane's right-click menu has Rename");
has(app, "laneField(repo, p, true)", "a lane's ghost row can hold the field");
has(app, "laneField(repo, p, false)", "a lane's row can hold the field");
has(app, "isRenameKey(e, inField)", "F2 reaches the rule");
has(app, "laneChange(typed, p.declared?.title, p.slug)", "a lane's field saves by laneChange");
has(app, "namedChange(typed, current)", "a chat row's field saves by namedChange");
has(chat, 'data-testid="ws-chat-row-rename" data-track="workspace.chat.row.rename"', "a chat row's right-click menu has Rename");
has(chat, "namedChange(t, c.title)", "the chat header's field saves by namedChange");
// A renamed lane keeps its slug, branch and session: the only engine call is set_title.
const save = app.slice(app.indexOf("const laneField ="), app.indexOf("const laneField =") + 900);
if (/invoke\(/.test(save)) fail("a lane's field calls the engine itself (it must go through setTitle)");

if (bad) { console.error(`${bad} failure(s)`); process.exit(1); }
console.log("rowname-check: ok");
