// The three tmux attach rows and the one client that opens them must agree on
// what an attach ANSWERS.
//
// serve's `term_open`, `idea_term_open` and `workspace_term_open` rows
// (dispatch.rs) each answer the attach's id, a bare number. TerminalPane's
// tmux transport keeps that id, and `write`/`resize` send nothing while it is
// null. v0.54.0 read `workspace_term_open`'s answer as `[session, id]` and
// took `[1]` of a number: `undefined`, so a workspace chat's terminal drew
// Claude and took no key at all (2026-11-26, the owner could not answer an
// oh-my-zsh question in it). The mock answers a number too, so nothing that
// ran the harness saw it.
//
// Run from the repo root: node app/scripts/termopen-check.mjs
import { readFileSync } from "node:fs";

const dispatch = readFileSync("crates/worktrees-cli/src/serve/dispatch.rs", "utf8");
const term = readFileSync("crates/worktrees-api/src/term.rs", "utf8");
const pane = readFileSync("app/src/TerminalPane.tsx", "utf8");
let failed = 0;
const ok = (cond, what) => {
  if (cond) console.log(`ok - ${what}`);
  else { failed++; console.log(`not ok - ${what}`); }
};

// The row's body, from its name to the next row's `name: Kind [` line.
const rowBody = (name) => {
  const start = dispatch.search(new RegExp(`\\n\\s{4}${name}: \\w+ \\[`));
  if (start < 0) return null;
  const rest = dispatch.slice(start + 1);
  const next = rest.slice(1).search(/\n\s{4}(\/\/|[a-z_]+: \w+ \[)/);
  return next < 0 ? rest : rest.slice(0, next + 1);
};

for (const row of ["term_open", "idea_term_open", "workspace_term_open"]) {
  const body = rowBody(row);
  ok(body != null, `${row} is a row in dispatch.rs`);
  if (!body) continue;
  // `term_open` hands back the body's own answer, `term::term_open`'s.
  const bare = row === "term_open"
    ? /let id = term::term_open\(/.test(body) && /\n\s+id\n\s+\},?\s*$/.test(body) && /pub fn term_open\([^)]*\) -> Result<u32, String>/.test(term)
    : /Ok::<u32, String>\(id\)/.test(body);
  ok(bare, `${row} answers the attach id as a bare number`);
}

const open = pane.slice(pane.indexOf("const tmuxTransport"), pane.indexOf("const shellTransport"));
ok(open.length > 0, "TerminalPane has a tmux transport");
for (const row of ["term_open", "idea_term_open", "workspace_term_open"]) {
  ok(new RegExp(`invoke<number>\\("${row}"`).test(open), `the transport reads ${row}'s answer as a number`);
}
ok(!/\)\)\[\d\]/.test(open), "the transport indexes no attach answer as a tuple");

if (failed) {
  console.log(`\n${failed} failed`);
  process.exit(1);
}
