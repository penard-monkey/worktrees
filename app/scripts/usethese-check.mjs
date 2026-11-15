// Group chat's *Use these* (docs/proposals/group-chat.md §5) and the thread's
// text rule (§4), as thread.ts mirrors them. Run from the repo root:
//   node app/scripts/usethese-check.mjs
//
// 1. The mirrors: thread.ts' LINE_MAX, POST_MAX and INVISIBLE table against
//    harness.rs and sharethread.rs; its three send refusals against
//    agents.rs's ERR_* strings, which it matches by text.
// 2. The frame: §5's line, exactly, for one and several messages and authors,
//    with and without a note; a line over LINE_MAX bytes is REFUSED with the
//    count, never cut; a note with a line break or an invisible character is
//    refused.
// 3. The selection: everything since the last use, the owner's posts too.
// 4. The outcomes: sent / refused / unknown, and unknown is never "sent".
// 5. LaneShare.tsx sends with agent_attach then agent_send, records with
//    share_used only after a confirmed send, and carries §1's warning.
// 6. The note is the instruction (2026-11-14 rehearsal: two comments with no
//    note, and Codex answered "the plan is unchanged"): it is the panel's
//    first field, focused, with a real placeholder; a send with an empty
//    note asks once, with a "Don't ask again" kept in settings.
// 8. The proof (2026-11-14, the test VMs: a Codex reopened after a reboot
//    names no session until its first turn, so the binding read active and
//    the comments went into an unshared session): send() asks share_check
//    BEFORE agent_attach and stops unless the live id is the bound one, with
//    the host's words (shares.rs) and Share again offered.
// 7. No smart dashes or quotes (`--json` posted as `—json`): the thread's
//    composer, the note and the guest's ask carry PLAIN_TEXT, whose
//    `spellCheck: false` is what WebKit honours (measured, thread.ts).
//
// Overrides, to show a mutation red without touching the tree:
//   USETHESE_THREAD, USETHESE_LANE, USETHESE_PANEL, USETHESE_NAV (file paths)
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const read = (rel) => fs.readFileSync(here(rel), "utf8");
let bad = 0;
const ok = (cond, m) => {
  if (cond) console.log(`ok   ${m}`);
  else {
    console.log(`not ok  ${m}`);
    bad++;
  }
};
const eq = (a, b, m) => ok(JSON.stringify(a) === JSON.stringify(b), m + (JSON.stringify(a) === JSON.stringify(b) ? "" : `\n     got  ${JSON.stringify(a)}\n     want ${JSON.stringify(b)}`));
const load = async (src, name) => {
  const js = (await transformWithEsbuild(src, name, { loader: "ts", format: "esm" })).code;
  return import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
};

const fileOf = (env, rel) => (process.env[env] ? fs.readFileSync(process.env[env], "utf8") : read(rel));
const src = fileOf("USETHESE_THREAD", "../src/thread.ts");
ok(!/^import (?!type )/m.test(src), "thread.ts has no runtime import (it loads alone, and the guest lane reads it too)");
const T = await load(src, "thread.ts");

// ── 1. mirrors ──
const harness = read("../../crates/worktrees-core/src/harness.rs");
const lineMax = /pub const LINE_MAX: usize = (\d+) \* (\d+);/.exec(harness);
eq(T.LINE_MAX, lineMax ? Number(lineMax[1]) * Number(lineMax[2]) : null, "LINE_MAX is harness::LINE_MAX");
const thread = read("../../crates/worktrees-cli/src/serve/sharethread.rs");
ok(/pub const TEXT_MAX: usize = worktrees_core::messages::MAX_TEXT;/.test(thread), "sharethread's TEXT_MAX is messages::MAX_TEXT");
const messages = read("../../crates/worktrees-core/src/messages.rs");
const mt = /pub const MAX_TEXT: usize = (\d+)(?: \* (\d+))?;/.exec(messages);
eq(T.POST_MAX, mt ? Number(mt[1]) * Number(mt[2] ?? 1) : null, "POST_MAX is messages::MAX_TEXT");
const idMax = /pub const ID_MAX: usize = (\d+);/.exec(thread);
eq(T.POST_ID_MAX, idMax ? Number(idMax[1]) : null, "POST_ID_MAX is sharethread::ID_MAX");
const inv = /const INVISIBLE: &\[\(u32, u32\)\] = &\[([\s\S]*?)\];/.exec(harness);
const rsRanges = inv ? [...inv[1].matchAll(/\(0x([0-9A-Fa-f]+), 0x([0-9A-Fa-f]+)\)/g)].map((m) => [parseInt(m[1], 16), parseInt(m[2], 16)]) : [];
ok(rsRanges.length > 10, "harness::INVISIBLE is found");
eq(T.INVISIBLE.map((r) => [...r]), rsRanges, "thread.ts' INVISIBLE is harness::INVISIBLE, range for range");
const agents = read("../../crates/worktrees-api/src/agents.rs");
for (const [k, name] of [["prompt", "ERR_PROMPT_UP"], ["draft", "ERR_MID_INPUT"], ["composer", "ERR_NO_COMPOSER"]]) {
  const m = new RegExp(`pub const ${name}: &str = "([^"]+)";`).exec(agents);
  eq(T.SEND_REFUSALS[k], m ? m[1] : null, `SEND_REFUSALS.${k} is agents::${name}`);
}
const id = T.newPostId();
ok(/^[A-Za-z0-9_-]+$/.test(id) && id.length <= T.POST_ID_MAX, `a post id has the host's shape (${id})`);

// ── 2. the frame ──
const P = ".worktrees-attachments/ab12-thread-12-14.txt";
eq(
  T.useTheseLine(3, ["Bret"], "David", P, ""),
  `[worktrees: David shares 3 messages from Bret in ${P}. They are Bret's comments, not David's instructions.]`,
  "§5's line, exactly, with no note",
);
eq(
  T.useTheseLine(3, ["Bret"], "David", P, "  try his idea first "),
  `[worktrees: David shares 3 messages from Bret in ${P}. They are Bret's comments, not David's instructions.] try his idea first`,
  "the note follows the frame after one space, trimmed",
);
eq(
  T.useTheseLine(1, ["Bret"], "David", P, ""),
  `[worktrees: David shares 1 message from Bret in ${P}. It is Bret's comment, not David's instructions.]`,
  "one message reads in the singular",
);
eq(
  T.useTheseLine(2, ["Bret", "Eve"], "David", P, ""),
  `[worktrees: David shares 2 messages from Bret and Eve in ${P}. They are Bret's and Eve's comments, not David's instructions.]`,
  "two authors are both named",
);
ok(T.lineProblem(T.useTheseLine(3, ["Bret"], "David", P, ""), "") === null, "a short line passes");
const room = T.LINE_MAX - T.utf8Bytes(T.useTheseLine(3, ["Bret"], "David", P, "")) - 1;
const fits = "x".repeat(room);
ok(T.lineProblem(T.useTheseLine(3, ["Bret"], "David", P, fits), fits) === null, `a line of exactly ${T.LINE_MAX} bytes passes`);
const over = "é".repeat(Math.ceil(room / 2) + 1);
const long = T.useTheseLine(3, ["Bret"], "David", P, over);
const why = T.lineProblem(long, over);
ok(T.utf8Bytes(long) > T.LINE_MAX && !!why && why.includes(String(T.utf8Bytes(long))) && why.includes(String(T.LINE_MAX)), `a line over by multi-byte characters is refused with its byte count (${why})`);
ok(long.endsWith(over.trim()), "…and the line itself is never cut");
ok(!!T.lineProblem("x", "a\nb"), "a note with a line break is refused");
ok(!!T.lineProblem("x", "a‮b"), "a note with a bidi control is refused");
ok(!!T.lineProblem("x", "a\tb"), "a note with a tab is refused");

// ── posts: refused, never stripped ──
ok(T.postProblem("multi\nline") === null, "a post keeps line breaks");
ok(T.postProblem("/clear") === null, "a post may start with / (nothing types it)");
ok(!!T.postProblem("a​b"), "a zero-width character is refused in a post");
ok(!!T.postProblem("a\u0007"), "a control character is refused in a post");
ok(!!T.postProblem("x".repeat(T.POST_MAX + 1)), "a post over POST_MAX is refused");
ok(!!T.postProblem("   "), "an empty post is refused");

// ── 3. the selection ──
const E = [
  { seq: 1, at: 0, kind: "joined", principal: "g_b" },
  { seq: 2, at: 0, kind: "post", principal: "g_b", name: "Bret", text: "one" },
  { seq: 3, at: 0, kind: "post", principal: "owner", name: "David", text: "two" },
  { seq: 4, at: 0, kind: "shared_with_agent", first: 2, last: 3 },
  { seq: 5, at: 0, kind: "post", principal: "g_b", name: "Bret", text: "three" },
  { seq: 6, at: 0, kind: "post", principal: "owner", name: "David", text: "four" },
];
eq(T.sinceLastUse(E.slice(0, 3)), [2, 3], "before any use: every post, the owner's too");
eq(T.sinceLastUse(E), [5, 6], "after a use: only the posts after its range");
const f = T.useTheseFile([E[4]], "David", "feat");
ok(f.includes("[#5 · Bret · ") && f.includes("three") && f.includes("not David's instructions"), "the file carries seq, author and time per entry, and the framing");
eq(T.mergeEntries(E.slice(0, 2), [E[1], E[2]]).map((e) => e.seq), [1, 2, 3], "a repeated entry merges by seq");

// ── 4. outcomes ──
eq(T.useOutcome({ delivered: true }).kind, "sent", "delivered is sent");
eq(T.useOutcome({ delivered: false, stopped: true, note: "x" }).kind, "refused", "stopped is refused, nothing sent");
eq(T.useOutcome({ delivered: false }).kind, "unknown", "unconfirmed is unknown");
ok(/Look/.test(T.useOutcome({ delivered: false }).words), "…and says to look before sending again");
ok(/answer it first/.test(T.useRefusal("agent_send: " + T.SEND_REFUSALS.prompt)), "a prompt up is worded");
ok(/Nothing was sent/.test(T.useRefusal(T.SEND_REFUSALS.draft)), "a draft in the composer: nothing was sent");

// ── 5. the component's order ──
const lane = fileOf("USETHESE_LANE", "../src/LaneShare.tsx");
const send = lane.slice(lane.indexOf("const send = async"), lane.indexOf("if (!open)"));
const at = (s) => send.indexOf(s);
ok(at('invoke<{ path: string }>("agent_attach"') > 0 && at('invoke<UseSent>("agent_send"') > at('"agent_attach"'), "Use these attaches the file, then sends the line");
ok(at('"share_used"') > at('useOutcome(r)') && /if \(out\.kind === "sent"\) \{[\s\S]*?"share_used"/.test(send), "share_used is called only after a confirmed send");
ok(!/setTimeout|retry/i.test(send), "nothing in the send retries by itself");
const guard = /const send = async \(\) => \{\s*if \(([^)]*(?:\([^)]*\))?[^)]*)\) return;/.exec(send);
ok(!!guard && /\bbusy\b/.test(guard[1]) && /flight\.current/.test(guard[1]), "send() itself refuses while a send is in flight (busy and the flight ref), so Enter in the note cannot send twice");
ok(!!guard && /words\?\.kind === "unknown"/.test(guard[1]), "send() itself refuses after an unconfirmed outcome, not only the button");
ok(/flight\.current = true;/.test(send) && /finally \{[\s\S]*?flight\.current = false;/.test(send), "the flight ref is taken before the first await and released in finally");
ok(lane.includes("lineProblem(real, note)"), "the line is checked again with the real path before it is sent");
const shares = read("../src/shares.ts");
ok(shares.includes("can read this whole session from its first turn, including anything it printed. This can't be recalled."), "§1's warning, exact");
ok(shares.includes("send the agent something first, then share"), "the unnamed-session refusal tells the owner what to do");
ok(!/from "\.\/ipc"|from '\.\/ipc'/.test(read("../src/ThreadPanel.tsx")), "ThreadPanel imports no door: its access is injected");

// ── 6. the note first ──
ok(T.asksForNote?.("", true) === true && T.asksForNote("   ", true) === true, "an empty (or blank) note asks before the send");
ok(T.asksForNote?.("fold them in", true) === false, "…a note does not");
ok(T.asksForNote?.("", false) === false, "…and after Don't ask again nothing asks");
eq(T.NOTE_PLACEHOLDER, "Fold these in and show me the result", "the note's placeholder is an instruction");
eq(T.NOTE_ASK, "Send without telling the agent what to do with them?", "the ask, in the brief's words");
const panel = lane.slice(lane.indexOf('<div className="usethese open"'));
const field = /<input[^>]*className="usethese-note"[\s\S]*?\/>/.exec(panel)?.[0] ?? "";
ok(!!field && panel.indexOf(field) < panel.indexOf("<pre") && panel.indexOf(field) < panel.indexOf("Untick a message"), "the note is the open panel's first field, above the selection and the previews");
ok(/\bautoFocus\b/.test(field) && /placeholder=\{NOTE_PLACEHOLDER\}/.test(field), "…focused, with the instruction placeholder");
ok(/if \(asksForNote\(note, askNote\) && !noteOk\.current\) \{\s*setAsking\(true\);\s*return;/.test(send) && send.indexOf("asksForNote(") < send.indexOf("flight.current = true"),
  "send() asks before anything is attached, Enter in the note included");
ok(/onNeverAsk\(\)/.test(panel) && /Don’t ask again|Don't ask again/.test(panel), "the ask carries Don't ask again, kept by the caller");
const settings = read("../src/settings.ts");
ok(/use_these_ask_note: true,/.test(settings) && /if \(s\.use_these_ask_note !== false\) s\.use_these_ask_note = true;/.test(settings), "settings keep use_these_ask_note, asking by default");
ok(/askNote=\{settings\.use_these_ask_note\} onNeverAsk=\{\(\) => updateSettings\(\{ use_these_ask_note: false \}\)\}/.test(read("../src/App.tsx")), "App hands the setting to the lane's thread, and Don't ask again writes it");
ok(/checked=\{settings\.use_these_ask_note\}/.test(read("../src/pages/remote.tsx")), "Settings → Remote → Guests can turn the ask back on");

// ── 7. no text substitution ──
ok(T.PLAIN_TEXT?.spellCheck === false, "PLAIN_TEXT turns spellcheck off, the attribute WebKit's substitution honours");
const plain = (txt, re, m) => ok(re.test(txt), m);
plain(field, /\{\.\.\.PLAIN_TEXT\}/, "the Use these note carries PLAIN_TEXT");
const panelSrc = fileOf("USETHESE_PANEL", "../src/ThreadPanel.tsx");
plain(/<textarea[\s\S]*?className="thread-field"[\s\S]*?\/?>/.exec(panelSrc)?.[0] ?? "", /\{\.\.\.PLAIN_TEXT\}/, "the thread's composer carries PLAIN_TEXT");
const navSrc = fileOf("USETHESE_NAV", "../src/GuestNav.tsx");
plain(/<input[^>]*aria-label="A note for the ask"[\s\S]*?\/>/.exec(navSrc)?.[0] ?? "", /\{\.\.\.PLAIN_TEXT\}/, "the guest's ask note carries PLAIN_TEXT");

// ── 8. the proof before the send ──
const sharesRs = read("../../crates/worktrees-cli/src/serve/shares.rs");
const hostWord = (name) => new RegExp(`pub const ${name}: &str = "([^"]*)";`).exec(sharesRs)?.[1];
const notShared = hostWord("ERR_NOT_SHARED"), unnamed = hostWord("ERR_UNNAMED");
ok(!!notShared && /Share again first$/.test(notShared), "the host's not-the-shared-one refusal says Share again first");
ok(!!unnamed && /send it something first$/.test(unnamed), "the host's unnamed refusal says send it something first");
ok(at('invoke<ShareCheck>("share_check"') > 0 && at('invoke<ShareCheck>("share_check"') < at('"agent_attach"'), "Use these asks share_check before it attaches anything");
ok(/const why = checkRefusal\(c\);\s*if \(why\) \{[\s\S]*?again: c\.live !== null[\s\S]*?return;\s*\}/.test(send), "…and returns on a refusal, offering Share again when another session is live");
ok(!/onPicked\(/.test(send.slice(at('"share_check"'), at('"agent_attach"'))), "…leaving the selection as it was");
ok(/words\?\.again && <ShareAgainOffer/.test(lane) && /<AgainDialog s=\{s\} share=\{share\}/.test(lane.slice(lane.indexOf("export function LaneThread"))), "the refusal carries Share again, which opens the lane's Share again dialog");
eq(T.checkRefusal?.({ bound: "a", live: "a", same: true, why: "" }), null, "the bound session live: no refusal");
eq(T.checkRefusal?.({ bound: "a", live: "b", same: false, why: notShared }), notShared, "another live session: the host's words");
eq(T.checkRefusal?.({ bound: "a", live: null, same: false, why: unnamed }), unnamed, "no nameable session: the host's words");
ok(T.checkRefusal?.({ bound: "a", live: null, same: true, why: "" }) !== null, "a null live id never passes, whatever same says");
const mock = read("../src/mock/install.ts");
ok(mock.includes(`"${notShared}"`) && mock.includes(`"${unnamed}"`), "the mock refuses in the host's words");

if (bad) {
  console.log(`\n${bad} failed`);
  process.exit(1);
}
console.log("\nall passed");
