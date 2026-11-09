// An idea's file is canonical and byte-exact through the editor
// (app/companion/ideatext.ts, ideastate.ts), proved with the REAL
// @codemirror/state the companion is built with.
//
//   node app/scripts/ideas-text-check.mjs      # from the repo root; exits non-zero on failure
//
// For a document with frontmatter, an unknown frontmatter key, an HTML
// comment, a table and a mermaid fence, with LF, CRLF, mixed endings and a
// lone CR:
//   - opened and saved untouched, the text is the file's, byte for byte;
//   - one character typed, the saved text differs from the original by that
//     character only;
//   - Enter and a paste take the document's own line ending;
//   - the frontmatter's `---`, `id:` and closing lines cannot be changed —
//     not by a keystroke, not by an insertion before it, not by select-all
//     and delete, which leaves the three lines and still an id idea_write
//     accepts — while another key stays editable.
// And the pure helpers the Read view and the "save as new" act use.
//
// Override, so a mutation can be shown red without touching the tree:
//   IDEASTATE_SRC=<copy of ideastate.ts>   IDEATEXT_SRC=<copy of ideatext.ts>
import fs from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
let failed = 0;
const check = (c, m) => { if (!c) failed++; console.log(`${c ? "ok" : "not ok"} — ${m}`); };

const require = createRequire(here("../package.json"));
const STATE_URL = pathToFileURL(require.resolve("@codemirror/state").replace(/index\.cjs$/, "index.js")).href;
const toData = (code) => `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const ts = async (p) => (await transformWithEsbuild(fs.readFileSync(p, "utf8"), p, { loader: "ts" })).code;

const textSrc = fs.readFileSync(process.env.IDEATEXT_SRC ?? here("../companion/ideatext.ts"), "utf8");
check(!/^\s*import\s(?!type\b)/m.test(textSrc), "ideatext.ts has no runtime import (it is loaded as a data: module)");
const textUrl = toData(await ts(process.env.IDEATEXT_SRC ?? here("../companion/ideatext.ts")));
const stateCode = (await ts(process.env.IDEASTATE_SRC ?? here("../companion/ideastate.ts")))
  .replace(/from\s*["']\.\/ideatext["']/g, `from ${JSON.stringify(textUrl)}`)
  .replace(/from\s*["']@codemirror\/state["']/g, `from ${JSON.stringify(STATE_URL)}`);
check(!/from\s*["'](?!data:|file:)/.test(stateCode), "ideastate.ts imports only @codemirror/state and ./ideatext (both rewired here)");
const { textExtensions, savedText } = await import(toData(stateCode));
const T = await import(textUrl);
const { EditorState, EditorSelection } = await import(STATE_URL);

const ID = "01JC8Z4K3M9QX2T7VBN5R6WYHD";
const LINES = [
  "---",
  `id: ${ID}`,
  "created: 2026-11-08",
  "related: [01JC8Z4K3M9QX2T7VBN5R6WYHE]",
  "---",
  "",
  "# Jellyfin on the home server",
  "",
  "<!-- ¶ 7k2q -->",
  "Some *prose* with **weight** and `code`.",
  "",
  "| a | b |",
  "|---|---|",
  "| 1 | 2 |",
  "",
  "```mermaid",
  "graph TD; A-->B",
  "```",
  "",
];
const FIXTURES = {
  LF: LINES.join("\n"),
  CRLF: LINES.join("\r\n"),
  mixed: LINES.slice(0, 10).join("\r\n") + "\n" + LINES.slice(10).join("\n"),
  "lone CR": LINES.join("\n").replace("Some *prose*", "Some\r*prose*"),
  "no final newline, CRLF": LINES.slice(0, -1).join("\r\n"),
};

const open = (text) => EditorState.create({ doc: text, extensions: textExtensions(text) });
/** The editor position of byte offset `at` in `text`: one position per line
 *  break, as the state counts them. */
const posOf = (state, text, at) => {
  const sep = state.lineBreak;
  return text.slice(0, at).split(sep).join("\n").length;
};
const input = (state, spec, ev = "input.type") => state.update({ ...spec, userEvent: ev }).state;

for (const [name, text] of Object.entries(FIXTURES)) {
  const s0 = open(text);
  check(savedText(s0) === text, `${name}: opened and saved untouched, the bytes are the file's`);
  const mark = "Jellyfin";
  const at = text.indexOf(mark) + mark.length;
  const s1 = input(s0, { changes: { from: posOf(s0, text, at), insert: "!" } });
  const want = text.slice(0, at) + "!" + text.slice(at);
  check(savedText(s1) === want, `${name}: one character typed, the saved text differs by that character only`);
  // An edit deep in the document, past the CR or the change of endings.
  const at2 = text.indexOf("graph");
  const s2 = input(s0, { changes: { from: posOf(s0, text, at2), insert: "x" } });
  check(savedText(s2) === text.slice(0, at2) + "x" + text.slice(at2), `${name}: a character typed after the comment, table and fence is that character only`);
}

// ── inserted line breaks take the document's ending ─────────────────────────
for (const [name, sep] of [["LF", "\n"], ["CRLF", "\r\n"]]) {
  const text = FIXTURES[name];
  const s0 = open(text);
  check(s0.lineBreak === sep, `${name}: the editor's line break is ${JSON.stringify(sep)}`);
  const at = text.indexOf("Some");
  const enter = input(s0, { changes: { from: posOf(s0, text, at), insert: s0.lineBreak } });
  check(savedText(enter) === text.slice(0, at) + sep + text.slice(at), `${name}: Enter inserts ${JSON.stringify(sep)}`);
  for (const pasted of ["p\nq", "p\r\nq", "p\rq"]) {
    const s = input(s0, { changes: { from: posOf(s0, text, at), insert: pasted } }, "input.paste");
    check(savedText(s) === text.slice(0, at) + `p${sep}q` + text.slice(at), `${name}: a paste of ${JSON.stringify(pasted)} lands as ${JSON.stringify(`p${sep}q`)}`);
  }
}
{
  // A reload is not input: a programmatic replacement keeps its bytes.
  const s0 = open(FIXTURES.LF);
  const s = s0.update({ changes: { from: 0, to: s0.doc.length, insert: s0.toText("---\nid: X\n---\nraw\r\n") }, filter: false }).state;
  check(savedText(s) === "---\nid: X\n---\nraw\r\n", "a change that is not a person's input (no userEvent) is not rewritten");
}

// ── the frontmatter guard ───────────────────────────────────────────────────
for (const [name, text] of Object.entries(FIXTURES)) {
  const s0 = open(text);
  const idAt = text.indexOf(ID);
  const typed = input(s0, { changes: { from: posOf(s0, text, idAt), insert: "Z" } });
  check(savedText(typed) === text, `${name}: a keystroke in the id is refused`);
  const before = input(s0, { changes: { from: 0, insert: "x" } });
  check(savedText(before) === text, `${name}: an insertion before the opening --- is refused`);
  const closeAt = text.indexOf("---", 4);
  const del = input(s0, { changes: { from: posOf(s0, text, closeAt), to: posOf(s0, text, closeAt) + 1 } }, "delete.backward");
  check(savedText(del) === text, `${name}: deleting from the closing --- is refused`);
  const all = input(s0.update({ selection: EditorSelection.single(0, s0.doc.length) }).state, { changes: { from: 0, to: s0.doc.length, insert: "" } }, "delete.selection");
  const left = savedText(all);
  const e = text.startsWith("---\r\n") ? "\r\n" : "\n";
  const sep = s0.lineBreak;
  check(left === `---${e}id: ${ID}${e}---${e}`, `${name}: select-all and delete leaves the three guarded lines (${JSON.stringify(left)})`);
  check(T.frontOf(all.doc.toString(), sep)?.id != null, `${name}: and what is left still carries the id`);
  const idHead = input(s0, { changes: { from: posOf(s0, text, text.indexOf("id:")), insert: "z" } });
  check(savedText(idHead) === text, `${name}: a keystroke at the head of the id line is refused`);
  const idEnd = input(s0, { changes: { from: posOf(s0, text, idAt + ID.length), insert: "z" } });
  check(savedText(idEnd) === text, `${name}: a keystroke at the end of the id line is refused`);
  const bs = input(s0, { changes: { from: posOf(s0, text, text.indexOf("\n# Jelly")) - 1, to: posOf(s0, text, text.indexOf("\n# Jelly")) } }, "delete.backward");
  check(savedText(bs).includes(`---`) && T.frontOf(bs.doc.toString(), sep)?.id != null, `${name}: Backspace at the head of the body keeps the frontmatter whole`);
  const typedAll = input(s0.update({ selection: EditorSelection.single(0, s0.doc.length) }).state, { changes: { from: 0, to: s0.doc.length, insert: "x" } });
  check(savedText(typedAll) === `---${e}id: ${ID}${e}---${e}x`, `${name}: select-all and a keystroke leaves the three lines and the keystroke after them (${JSON.stringify(savedText(typedAll))})`);
  const keyBefore = input(s0, { changes: { from: posOf(s0, text, text.indexOf("created:")), insert: "z" } });
  check(savedText(keyBefore) === text.replace("created:", "zcreated:"), `${name}: a keystroke at the head of an unguarded key's line is allowed`);
  const cAt = text.indexOf("2026-11-08");
  const cr = input(s0, { changes: { from: posOf(s0, text, cAt), to: posOf(s0, text, cAt) + 4, insert: "2027" } });
  check(savedText(cr) === text.replace("2026-11-08", "2027-11-08"), `${name}: another key (created) stays editable`);
}
{
  const plain = "# No frontmatter\n\ntext\n";
  const s = input(open(plain), { changes: { from: 0, insert: "x" } });
  check(savedText(s) === "x" + plain, "a document with no frontmatter guards nothing");
}

// ── the pure helpers ────────────────────────────────────────────────────────
check(T.bodyOf(FIXTURES.LF).startsWith("\n# Jellyfin") && T.bodyOf(FIXTURES.CRLF).startsWith("\r\n# Jellyfin"), "bodyOf cuts just after the closing --- line, CRLF too");
check(T.bodyOf("# x\n") === "# x\n", "bodyOf leaves a document without frontmatter whole");
check(T.commentOnly("<!-- ¶ 7k2q -->\n") && T.commentOnly("<!-- a -->  <!-- b -->") && !T.commentOnly("<b>x</b>") && !T.commentOnly("<!-- a --><b>"), "commentOnly: only comments and space");
const NEW = "01JC8Z4K3M9QX2T7VBN5R6WYHF";
check(T.withId(FIXTURES.CRLF, NEW) === FIXTURES.CRLF.replace(ID, NEW), "withId swaps the id's value and keeps every other byte (CRLF)");
check(T.withId("# x\n", NEW) === null, "withId refuses a text with no frontmatter id");
check(T.lineBreakOf("") === "\n" && T.lineBreakOf("a\r\nb\n") === "\n" && T.lineBreakOf("a\r\nb\r\n") === "\r\n", "lineBreakOf: CRLF only when every LF is part of one");

console.log(failed ? `\nideas-text-check: ${failed} failure(s)` : "\nideas-text-check: all good");
process.exit(failed ? 1 : 0);
