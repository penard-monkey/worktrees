// chatmirror-check — the chat view (app/src/chat.ts) mirrors two things serve
// decides, and this holds the mirror to the Rust it copies:
//
//   node app/scripts/chatmirror-check.mjs      # exits non-zero on failure
//
// 1. The refusal strings its words key on (`AGENT_ERR`, and the patterns in
//    `answerWords` and `sendWords`), against `worktrees-api/src/agents.rs`.
//    A reworded refusal there drops the card's words to the generic "Not
//    confirmed: …" line, with every Rust test and every tsc run still green.
//    `stopWords` (the Stop button's, `agent_interrupt`) is held the same way.
// 2. The composer's line rule (`draftProblem`), against
//    `harness::typed_line_ok`, the one rule `agent_send`, `agent_answer_text`
//    and MCP `send` share. The REAL `draftProblem` runs over every code point
//    and is compared with a model of `typed_line_ok` built from the Rust:
//    its `INVISIBLE` table, its leading characters and `LINE_MAX`. A client
//    laxer than the host sends lines it refuses; a stricter one stops lines
//    the host would take.
// 3. The attachments (docs/proposals/chat-attachments.md): `ATTACH_ERR`'s
//    tokens against `worktrees_core::attach` and `budget::ATTACH_BUDGET`,
//    `ATTACH_MAX` and `ATTACH_PATH_MAX` against the host's cap and path
//    shape, and the line `attachLine` composes — the iOS app's shape too —
//    run through the model of `typed_line_ok` below at its longest.
// And the mock's agent rows (app/src/mock/install.ts) refuse only with
// strings the host can give, so a card driven in the harness meets the
// host's words. agent-rows-check.mjs holds the rows' capabilities.
//
// Like chatsteps-check, chat.ts runs with only its two imports stubbed.
// CHATMIRROR_{CHAT,API,HARNESS,TURNS,MOCK,ATTACH,BUDGET} point the check at a
// mutated copy.

import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = {
  chat: process.env.CHATMIRROR_CHAT ?? here("../src/chat.ts"),
  api: process.env.CHATMIRROR_API ?? here("../../crates/worktrees-api/src/agents.rs"),
  harness: process.env.CHATMIRROR_HARNESS ?? here("../../crates/worktrees-core/src/harness.rs"),
  turns: process.env.CHATMIRROR_TURNS ?? here("../../crates/worktrees-core/src/turns.rs"),
  mock: process.env.CHATMIRROR_MOCK ?? here("../src/mock/install.ts"),
  attach: process.env.CHATMIRROR_ATTACH ?? here("../../crates/worktrees-core/src/attach.rs"),
  budget: process.env.CHATMIRROR_BUDGET ?? here("../../crates/worktrees-cli/src/serve/budget.rs"),
};

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const read = (p) => fs.readFileSync(p, "utf8");
const done = () => {
  console.log(failed ? `\nchatmirror-check: ${failed} failure(s)` : "\nchatmirror-check: all good");
  process.exit(failed ? 1 : 0);
};

// ── the sources ──────────────────────────────────────────────────────────────

const chatSrc = read(SRC.chat);
const stubbed = chatSrc
  .replace(/^import \{ useSyncExternalStore \} from "react";$/m, "const useSyncExternalStore = () => undefined;")
  .replace(/^import \{ invoke, windowOnHost \} from "\.\/ipc";$/m, "const invoke = async () => { throw new Error(\"no ipc here\"); }; const windowOnHost = () => true;")
  .replace(/^import \{ convertPhoto, jpegName, photoDecision, photoKind, INPUT_MAX \} from "\.\/chat-image";$/m, "const INPUT_MAX = 40 * 1024 * 1024; const convertPhoto = () => undefined; const jpegName = () => undefined; const photoDecision = () => undefined; const photoKind = () => undefined;");
if (/^import /m.test(stubbed)) { fail("chat.ts gained an import this check does not stub — extend the stubs above"); done(); }
const js = (await transformWithEsbuild(stubbed, "chat.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
for (const name of [
  "AGENT_ERR", "draftProblem", "answerWords", "sendWords", "stopWords", "SEND_MAX", "TURNS_MAX",
  "ATTACH_ERR", "ATTACH_MAX", "ATTACH_PATH_MAX", "ATTACH_PER_MESSAGE", "attachLine", "attachLead", "attachProblem", "attachWords", "splitAttached",
]) {
  if (C[name] === undefined) fail(`chat.ts exports no ${name}`);
}
if (failed) done();

/** Rust source without its unit-test module (AGENTS.md: cut at the
 *  `#[cfg(test)]` that heads `mod tests`, not the first one). */
const live = (s) => s.split(/\n#\[cfg\(test\)\]\s*\nmod tests\b/)[0];

/** The string literals of Rust source, unescaped, with comments, char
 *  literals and lifetimes skipped. Raw strings are read as written. */
function rustStrings(rs) {
  const out = [];
  let i = 0;
  while (i < rs.length) {
    const c = rs[i];
    if (c === "/" && rs[i + 1] === "/") { const e = rs.indexOf("\n", i); i = e < 0 ? rs.length : e; continue; }
    if (c === "/" && rs[i + 1] === "*") { const e = rs.indexOf("*/", i + 2); i = e < 0 ? rs.length : e + 2; continue; }
    const raw = /^b?r(#*)"/.exec(rs.slice(i, i + 40));
    if (raw && !/\w/.test(rs[i - 1] ?? "")) {
      const end = `"${raw[1]}`, s = i + raw[0].length, e = rs.indexOf(end, s);
      out.push(rs.slice(s, e));
      i = e + end.length;
      continue;
    }
    if (c === "'") {
      const m = /^'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^\\'])'/u.exec(rs.slice(i, i + 16));
      i += m ? m[0].length : 1;
      continue;
    }
    if (c === '"') {
      let s = "";
      i++;
      while (i < rs.length && rs[i] !== '"') {
        if (rs[i] !== "\\") { s += rs[i++]; continue; }
        const n = rs[i + 1];
        if (n === "\n") { i += 2; while (/\s/.test(rs[i])) i++; continue; }
        const esc = { n: "\n", t: "\t", r: "\r", 0: "\0", "\\": "\\", '"': '"', "'": "'" }[n];
        if (esc !== undefined) { s += esc; i += 2; continue; }
        const u = /^\\u\{([0-9a-fA-F_]+)\}/.exec(rs.slice(i, i + 16));
        if (u) { s += String.fromCodePoint(parseInt(u[1].replace(/_/g, ""), 16)); i += u[0].length; continue; }
        if (n === "x") { s += String.fromCharCode(parseInt(rs.slice(i + 2, i + 4), 16)); i += 4; continue; }
        throw new Error(`a string escape this check cannot read: ${rs.slice(i, i + 8)}`);
      }
      out.push(s);
      i++;
      continue;
    }
    i++;
  }
  return out;
}

/** What a format string reads as once its arguments are in: every `{…}` is
 *  `7`, `{{` and `}}` are braces. The mock's `${…}` gets the same `7`. */
const rendered = (s) => s.replace(/\{\{|\}\}|\{[^{}]*\}/g, (m) => (m === "{{" ? "{" : m === "}}" ? "}" : "7"));

/** A function's body, from `head` to the first line that is only `}`. */
function body(src, head) {
  const at = src.indexOf(head);
  if (at < 0) return null;
  const end = src.indexOf("\n}\n", at);
  return src.slice(at, end < 0 ? src.length : end + 2);
}

const api = live(read(SRC.api));
const harness = live(read(SRC.harness));
const apiSaid = rustStrings(api).map(rendered);
const typed = body(harness, "pub fn typed_line_ok(");
if (!typed) { fail("harness.rs has no `pub fn typed_line_ok(` for this check to read"); done(); }
const typedSaid = rustStrings(typed).map(rendered);
const hostSaid = new Set([...apiSaid, ...typedSaid]);

// ── 1. the refusal strings ───────────────────────────────────────────────────

// AGENT_ERR's keys with a const in agents.rs, the rest by where they are said.
const CONSTS = {
  changed: "ERR_CHANGED", noChoice: "ERR_NO_CHOICE", promptUp: "ERR_PROMPT_UP", midInput: "ERR_MID_INPUT",
  noComposer: "ERR_NO_COMPOSER", fieldHeld: "ERR_FIELD_HELD", multi: "ERR_MULTI", notMulti: "ERR_NOT_MULTI", notesOpen: "ERR_NOTES_OPEN",
  notBusy: "ERR_NOT_BUSY", stopDraft: "ERR_STOP_DRAFT", noSessionYet: "ERR_NO_SESSION_YET",
};
const WHOLE = ["noBox", "noAgent"];
const PREFIXES = ["noLanePrefix", "turnsUnsupportedPrefix", "stopUnsupportedPrefix"];
const constOf = (name) => {
  const m = new RegExp(`pub const ${name}: &str = ("(?:[^"\\\\]|\\\\.)*");`).exec(api);
  return m ? rustStrings(m[1])[0] : undefined;
};
for (const [key, value] of Object.entries(C.AGENT_ERR)) {
  if (key in CONSTS) {
    const want = constOf(CONSTS[key]);
    if (want === undefined) fail(`AGENT_ERR.${key} mirrors agents.rs's ${CONSTS[key]}, which is gone`);
    else if (value !== want) fail(`AGENT_ERR.${key} is ${JSON.stringify(value)}; agents.rs's ${CONSTS[key]} is ${JSON.stringify(want)}`);
    else ok(`AGENT_ERR.${key} is ${CONSTS[key]}`);
  } else if (WHOLE.includes(key)) {
    if (!apiSaid.includes(value)) fail(`AGENT_ERR.${key} (${JSON.stringify(value)}) is no refusal agents.rs gives`);
    else ok(`AGENT_ERR.${key} is a refusal agents.rs gives`);
  } else if (PREFIXES.includes(key)) {
    if (!apiSaid.some((s) => s.startsWith(value) && s.length > value.length)) fail(`AGENT_ERR.${key} (${JSON.stringify(value)}) heads no refusal agents.rs gives`);
    else ok(`AGENT_ERR.${key} heads a refusal agents.rs gives`);
  } else {
    fail(`AGENT_ERR.${key} is a key this check does not know — say in CONSTS, WHOLE or PREFIXES where agents.rs says it`);
  }
}

// The patterns the words key on: each `/…/.test(err)` must still match
// something agents.rs says, and each `err.startsWith("…")` head one.
for (const fn of ["answerWords", "sendWords", "stopWords"]) {
  const src = body(chatSrc, `export function ${fn}(`);
  if (!src) { fail(`chat.ts has no \`export function ${fn}(\` for this check to read`); continue; }
  const patterns = [...src.matchAll(/\/((?:\\.|[^/\n\\])+)\/([a-z]*)\.test\(err\)/g)].map((m) => new RegExp(m[1], m[2]));
  for (const re of patterns) {
    if (!apiSaid.some((s) => re.test(s))) fail(`${fn} keys on ${re}, which matches no refusal agents.rs gives`);
    else ok(`${fn}'s ${re} matches a refusal agents.rs gives`);
  }
  const heads = [...src.matchAll(/err\.startsWith\(("(?:[^"\\]|\\.)*")\)/g)].map((m) => JSON.parse(m[1]));
  for (const h of heads) {
    // `"text "` is typed_line_ok's: every one of its refusals must start so.
    const from = h === "text " ? typedSaid : apiSaid;
    if (!from.length || (h === "text " ? !from.every((s) => s.startsWith(h)) : !from.some((s) => s.startsWith(h)))) {
      fail(`${fn} keys on refusals starting ${JSON.stringify(h)}, and ${h === "text " ? "not every typed_line_ok refusal does" : "none from agents.rs does"}`);
    } else ok(`${fn}'s head ${JSON.stringify(h)} is ${h === "text " ? "every typed_line_ok refusal's" : "a refusal agents.rs gives"}`);
  }
  if (!patterns.length && !heads.length && fn === "answerWords") fail("answerWords keys on no pattern — the pattern this check reads no longer matches");
}

// A stale provider must not survive into the red composer line once the host
// says no agent is there. The host cannot say which provider ran last.
const noAgentLine = "No agent is running in this place.";
for (const name of ["Claude", "Codex"]) {
  const send = C.sendWords(C.AGENT_ERR.noLanePrefix + "old-session", name).line;
  const stop = C.stopWords(C.AGENT_ERR.noAgent, name).line;
  if (send !== noAgentLine || stop !== noAgentLine) fail(`no-agent words for ${name}: ${JSON.stringify({ send, stop })}`);
  else ok(`no-agent words do not guess ${name}`);
}

// ── 2. the line rule ─────────────────────────────────────────────────────────

const LINE_MAX = (() => {
  const m = /pub const LINE_MAX: usize = ([^;]+);/.exec(harness);
  return m ? m[1].replace(/_/g, "").split("*").reduce((a, s) => a * Number(s.trim()), 1) : NaN;
})();
if (C.SEND_MAX !== LINE_MAX) fail(`chat.ts's SEND_MAX is ${C.SEND_MAX}; harness::LINE_MAX is ${LINE_MAX}`);
else ok(`SEND_MAX is harness::LINE_MAX (${LINE_MAX})`);
const maxTurns = Number(/pub const MAX_TURNS: usize = (\d+);/.exec(read(SRC.turns))?.[1]);
if (C.TURNS_MAX !== maxTurns) fail(`chat.ts's TURNS_MAX is ${C.TURNS_MAX}; turns::MAX_TURNS is ${maxTurns}`);
else ok(`TURNS_MAX is turns::MAX_TURNS (${maxTurns})`);

// The model below is typed_line_ok as it reads today, in its order. A rule
// added there, or one changed, fails here until the model (and draftProblem)
// follow it.
const SHAPE = [
  ["text.trim().is_empty()", "a blank line is refused"],
  ["text.len() > LINE_MAX", "the bound is in bytes"],
  ["text.chars().any(char::is_control)", "a control character (category Cc) is refused"],
  ["text.chars().find(|c| invisible(*c))", "an INVISIBLE character is refused"],
  ["text.trim_start().chars().next()", "the first character after leading white space is the one that runs something"],
];
let shapeOk = true;
for (const [needle, what] of SHAPE) {
  if (!typed.includes(needle)) { shapeOk = false; fail(`typed_line_ok no longer reads \`${needle}\` (${what}) — update this check's model, and draftProblem`); }
}
const refusals = (typed.match(/return Err\(/g) ?? []).length;
if (refusals !== SHAPE.length) { shapeOk = false; fail(`typed_line_ok has ${refusals} refusals; this check models ${SHAPE.length}`); }
const lead = /matches!\(c, ((?:'[^']+'\s*\|?\s*)+)\)/.exec(typed);
const LEAD = new Set(lead ? [...lead[1].matchAll(/'([^']+)'/g)].map((m) => m[1]) : []);
if (!LEAD.size) { shapeOk = false; fail("typed_line_ok's leading characters (`matches!(c, '/' | …)`) are not where this check looks"); }
const table = /const INVISIBLE: &\[\(u32, u32\)\] = &\[([\s\S]*?)\];/.exec(harness);
const INVISIBLE = table ? [...table[1].matchAll(/\(0x([0-9A-Fa-f]+),\s*0x([0-9A-Fa-f]+)\)/g)].map((m) => [parseInt(m[1], 16), parseInt(m[2], 16)]) : [];
if (!INVISIBLE.length) { shapeOk = false; fail("harness.rs's INVISIBLE table is not where this check looks"); }
if (!harness.includes("INVISIBLE.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))")) { shapeOk = false; fail("harness::invisible no longer reads INVISIBLE as inclusive ranges"); }
if (shapeOk) ok(`typed_line_ok reads as modelled: ${SHAPE.length} refusals, leading ${[...LEAD].join(" ")}, ${INVISIBLE.length} invisible ranges`);

/** What typed_line_ok refuses `text` for, in its order, or null. */
function host(text) {
  if (!text.trim()) return "blank";
  if (new TextEncoder().encode(text).length > LINE_MAX) return "length";
  const cps = [...text].map((ch) => ch.codePointAt(0));
  if (cps.some((cp) => cp <= 0x1f || (cp >= 0x7f && cp <= 0x9f))) return "control";
  if (cps.some((cp) => INVISIBLE.some(([lo, hi]) => cp >= lo && cp <= hi))) return "invisible";
  const first = [...text].find((ch) => !/\p{White_Space}/u.test(ch));
  return LEAD.has(first) ? "lead" : null;
}
/** Which of typed_line_ok's refusals draftProblem's words are. */
function client(text, what) {
  const p = C.draftProblem(text, what);
  if (p === null) return null;
  if (p.startsWith("Use one line")) return "control";
  if (p.startsWith("Remove the invisible")) return "invisible";
  if (p.startsWith("Too long")) return "length";
  if (p.startsWith("Slash commands") || / can’t start with /.test(p)) return "lead";
  return `unknown words: ${p}`;
}

if (shapeOk) {
  // Every code point, alone at the head of a line and inside one.
  const differ = [];
  for (let cp = 0; cp <= 0x10ffff && differ.length < 12; cp++) {
    if (cp >= 0xd800 && cp <= 0xdfff) continue;
    const ch = String.fromCodePoint(cp);
    for (const [text, where] of [[`${ch}x`, "heading a line"], [`x${ch}x`, "inside a line"]]) {
      const h = host(text), c = client(text);
      if (h !== c) differ.push(`U+${cp.toString(16).toUpperCase().padStart(4, "0")} ${where}: host ${h ?? "takes it"}, draftProblem ${c ?? "takes it"}`);
    }
  }
  if (differ.length) {
    fail("draftProblem and typed_line_ok disagree — a Unicode update can do this too (INVISIBLE is generated from one version, \\p{Cf} is node's):\n       " + differ.join("\n       "));
  } else ok("draftProblem refuses what typed_line_ok refuses, for every code point, with the same reason");

  const lines = [
    ["x".repeat(LINE_MAX), null], ["x".repeat(LINE_MAX + 1), "length"],
    ["é".repeat(LINE_MAX / 2), null], ["é".repeat(LINE_MAX / 2) + "x", "length"],
    ["😀".repeat(LINE_MAX / 4), null], ["😀".repeat(LINE_MAX / 4) + "x", "length"],
    ["  /x", "lead"], ["　#x", "lead"], ["x /y", null], ["   ", null],
  ];
  const wrong = lines.filter(([text, want]) => client(text) !== want || (want && host(text) !== want));
  if (wrong.length) fail(`the bound or the leading rule differs: ${wrong.map(([t, w]) => `${JSON.stringify(t.length > 12 ? `${t.slice(0, 4)}…(${t.length})` : t)} should be ${w ?? "taken"}, is ${client(t) ?? "taken"}`).join("; ")}`);
  else ok(`the ${LINE_MAX}-byte bound counts UTF-8 bytes, and leading white space does not hide a ${[...LEAD].join(" ")}`);
  for (const what of ["answer", "note"]) {
    if (client("/x", what) !== "lead" || client("a\tb", what) !== "control") fail(`draftProblem(…, "${what}") does not apply the same rule`);
  }
}

// ── 3. attachments ───────────────────────────────────────────────────────────

const attach = live(read(SRC.attach));
const budget = live(read(SRC.budget));
const strConst = (src, name) => {
  const m = new RegExp(`pub const ${name}: &str = ("(?:[^"\\\\]|\\\\.)*");`).exec(src);
  return m ? rustStrings(m[1])[0] : undefined;
};
const TOKENS = { type: "ERR_TYPE", size: "ERR_SIZE", files: "ERR_FILES", bytes: "ERR_BYTES", unsafe: "ERR_UNSAFE" };
const hostTokens = [];
for (const [key, value] of Object.entries(C.ATTACH_ERR)) {
  const want = key === "budget" ? strConst(budget, "ATTACH_BUDGET") : strConst(attach, TOKENS[key] ?? "?");
  hostTokens.push(want);
  if (want === undefined) fail(`ATTACH_ERR.${key} mirrors ${key === "budget" ? "budget.rs's ATTACH_BUDGET" : `attach.rs's ${TOKENS[key] ?? "(no const)"}`}, which is gone`);
  else if (value !== want) fail(`ATTACH_ERR.${key} is ${JSON.stringify(value)}; the host's is ${JSON.stringify(want)}`);
  else ok(`ATTACH_ERR.${key} is the host's ${JSON.stringify(want)}`);
}
const shifted = /pub const MAX_BYTES: usize = (\d+) << (\d+);/.exec(attach);
const maxBytes = shifted ? Number(shifted[1]) * 2 ** Number(shifted[2]) : NaN;
if (C.ATTACH_MAX !== maxBytes) fail(`chat.ts's ATTACH_MAX is ${C.ATTACH_MAX}; attach::MAX_BYTES is ${maxBytes}`);
else ok(`ATTACH_MAX is attach::MAX_BYTES (${maxBytes})`);
// `<DIR>/<yyyymmdd>-<16 hex>.<ext>`, at its longest extension.
const dir = strConst(attach, "DIR");
const exts = /const EXTS: &\[&str\] = &\[([^\]]*)\];/.exec(attach);
const longest = exts ? Math.max(...[...exts[1].matchAll(/"([a-z]+)"/g)].map((m) => m[1].length)) : NaN;
const nameShape = /format!\("\{day\}-\{:016x\}\.\{ext\}"/.test(attach);
const pathMax = dir ? dir.length + 1 + 8 + 1 + 16 + 1 + longest : NaN;
if (!nameShape) fail("attach.rs no longer names a file `{day}-{:016x}.{ext}` — update ATTACH_PATH_MAX and the path shape in splitAttached");
else if (C.ATTACH_PATH_MAX !== pathMax) fail(`chat.ts's ATTACH_PATH_MAX is ${C.ATTACH_PATH_MAX}; the host's longest path is ${pathMax}`);
else ok(`ATTACH_PATH_MAX is the host's longest path (${pathMax})`);
const accepted = strConst(attach, "ACCEPTED");
if (!accepted?.startsWith("accepted: ")) fail(`attach::ACCEPTED (${JSON.stringify(accepted)}) no longer starts "accepted: ", which attachWords cuts off`);
// Each token's words are the chat's own, never the bare token.
const samples = {
  type: `${C.ATTACH_ERR.type}: HEIC (${accepted})`, size: `${C.ATTACH_ERR.size}: ${maxBytes}`, files: `${C.ATTACH_ERR.files}: 50`,
  bytes: `${C.ATTACH_ERR.bytes}: ${100 * 2 ** 20}`, budget: `${C.ATTACH_ERR.budget}: 42`, unsafe: C.ATTACH_ERR.unsafe,
};
const generic = Object.entries(samples).filter(([, err]) => C.attachWords(err, "x.heic").line === `Not attached: ${err}`).map(([k]) => k);
if (generic.length) fail(`attachWords has no words of its own for ${generic.join(", ")}`);
else ok(`attachWords has words for every token (${Object.keys(samples).length})`);
if (!C.attachWords(samples.type, "x.heic").line.includes("HEIC") || C.attachWords(samples.type, "x").line.includes("accepted:")) fail("attachWords does not name what the bytes were, or keeps the host's `(accepted: …)`");
// OWASP M1: a text file with a hidden or control character is refused by the
// host alone (no client copy of the set), and the words keep its code point.
const hiddenWhat = /err_type\(&format!\("(text with a hidden or control character, U\+)\{:04X\}"/.exec(attach)?.[1];
if (!hiddenWhat) fail("attach.rs no longer refuses `text with a hidden or control character, U+…` as an attach-type");
else {
  const w = C.attachWords(`${C.ATTACH_ERR.type}: ${hiddenWhat}202E (${accepted})`, "notes.txt").line;
  if (!w.includes("text with a hidden or control character, U+202E") || w.includes("accepted:")) fail(`attachWords loses the hidden character's words: ${JSON.stringify(w)}`);
  else ok("a text file's hidden or control character is refused by the host, and the words name it");
}
// The line, as (main) fixed it for both clients.
const P1 = ".worktrees-attachments/20261101-1f3a9c2e5b7d0a46.png";
const P2 = ".worktrees-attachments/20261101-77b0d4e1aa000000.pdf";
const shapes = [
  [C.attachLine("Look at this.", [P1]), `Look at this. [attached: ${P1}]`],
  [C.attachLine("Compare", [P1, P2]), `Compare [attached: ${P1}, ${P2}]`],
  [C.attachLine("", [P1]), `Look at the attached file. [attached: ${P1}]`],
  [C.attachLine("  ", [P1, P2]), `Look at the attached files. [attached: ${P1}, ${P2}]`],
  [C.attachLine("fix it", []), "fix it"],
];
const bad = shapes.filter(([got, want]) => got !== want);
if (bad.length) fail(`attachLine's shape: ${bad.map(([g, w]) => `${JSON.stringify(g)} should be ${JSON.stringify(w)}`).join("; ")}`);
else ok("attachLine writes `<text> [attached: <path>, <path>]`, and the lead-in when there is no text");
const split = C.splitAttached(C.attachLine("Compare", [P1, P2]));
if (split.text !== "Compare" || split.paths.join("|") !== `${P1}|${P2}`) fail(`splitAttached does not undo attachLine: ${JSON.stringify(split)}`);
else ok("splitAttached undoes attachLine");
if (C.splitAttached("see [attached: /etc/passwd]").paths.length) fail("splitAttached takes a path the host never writes");
// At its longest, every line attachProblem lets through passes typed_line_ok.
if (shapeOk) {
  const longPath = (i) => `${dir}/20261101-${i.toString(16).padStart(16, "0")}.${"x".repeat(longest)}`;
  const worst = (text, n) => C.attachLine(text, Array.from({ length: n }, (_, i) => longPath(i)));
  const wrong = [];
  for (let n = 1; n <= C.ATTACH_PER_MESSAGE; n++) {
    for (const text of ["", "x".repeat(200), "x".repeat(LINE_MAX - 12 - n * (pathMax + 2) + 1), "x".repeat(LINE_MAX - 12 - n * (pathMax + 2)), "x".repeat(LINE_MAX)]) {
      const allowed = C.attachProblem(text, n) === null;
      if (allowed && host(worst(text, n)) !== null) wrong.push(`${n} file(s), ${text.length} chars: let through, the host refuses it (${host(worst(text, n))})`);
      if (!allowed && host(worst(text, n)) === null && C.draftProblem(text) === null) wrong.push(`${n} file(s), ${text.length} chars: refused, the host takes it`);
    }
  }
  if (wrong.length) fail(`attachProblem and typed_line_ok disagree about a line with attachments:\n       ${wrong.slice(0, 6).join("\n       ")}`);
  else ok(`attachProblem refuses before upload exactly the lines typed_line_ok would, 1–${C.ATTACH_PER_MESSAGE} files at the longest path`);
  if (C.attachProblem("/clear", 1) === null || C.attachProblem("", 1) !== null) fail("attachProblem does not check the text by today's rule, or refuses a message of attachments alone");
}

// ── 4. the mock's agent rows ─────────────────────────────────────────────────

const mock = read(SRC.mock);
const from = mock.indexOf('case "agent_prompt"');
const last = mock.indexOf('case "agent_turns"');
const to = last < 0 ? -1 : mock.indexOf('\n    case "', last + 1);
if (from < 0 || to < 0) fail("the mock's agent rows (`case \"agent_prompt\"` … `case \"agent_turns\"`) are not where this check looks");
else {
  const rows = mock.slice(from, to);
  const thrown = [...rows.matchAll(/throw ("(?:[^"\\]|\\.)*"|`(?:[^`\\]|\\.)*`)/g)]
    .map((m) => (m[1][0] === '"' ? JSON.parse(m[1]) : m[1].slice(1, -1).replace(/\$\{[^}]*\}/g, "7")));
  const invented = thrown.filter((s) => !hostSaid.has(s));
  if (!thrown.length) fail("the mock's agent rows throw nothing — the pattern this check reads no longer matches");
  else if (invented.length) fail(`the mock's agent rows refuse with words the host never gives: ${invented.map((s) => JSON.stringify(s)).join(", ")}`);
  else ok(`the mock's agent rows refuse only as the host does (${thrown.length} throws)`);
}
// The mock's agent_attach refuses only with the host's tokens.
const ma = mock.indexOf('case "agent_attach"');
const mb = ma < 0 ? -1 : mock.indexOf('\n    case "', ma + 1);
if (ma < 0 || mb < 0) fail("the mock has no `case \"agent_attach\"` for this check to read");
else {
  const throws = [...mock.slice(ma, mb).matchAll(/throw ("(?:[^"\\]|\\.)*"|`(?:[^`\\]|\\.)*`)/g)].map((m) => (m[1][0] === '"' ? JSON.parse(m[1]) : m[1].slice(1, -1)));
  const stray = throws.filter((t) => !hostTokens.some((tok) => tok && (t === tok || t.startsWith(`${tok}: `))));
  const typeWords = throws.filter((t) => t.startsWith(`${C.ATTACH_ERR.type}: `)).filter((t) => !t.endsWith(` (${accepted})`));
  if (!throws.length) fail("the mock's agent_attach throws nothing — the pattern this check reads no longer matches");
  else if (stray.length || typeWords.length) fail(`the mock's agent_attach refuses as the host never does: ${[...stray, ...typeWords].map((x) => JSON.stringify(x)).join(", ")}`);
  else ok(`the mock's agent_attach refuses only with the host's tokens (${throws.length} throws)`);
}

done();
