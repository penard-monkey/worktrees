// Every process the engine starts goes through worktrees_core::childenv.
// crates/worktrees-core/src/childenv.rs says why: an AppImage launch hands its
// AppDir — LD_LIBRARY_PATH, PYTHONHOME, a PATH that starts inside the mount —
// to everything spawned from the app, and a tmux server passes it on to every
// pane for its whole life. Enumerating the "user-facing" spawns is how two
// login shells were first missed, so the rule is ALL of them, and this check
// makes it structural rather than a habit:
//
//   - no bare `Command::new(` in the non-test part of any .rs file under
//     crates/worktrees-{core,api,cli}/src or app/src-tauri/src, childenv.rs
//     itself excepted — `childenv::command(..)` is how a process is made;
//   - no `Command as …` alias, which would hide one;
//   - every pty `CommandBuilder::new(` is followed, within three lines, by
//     `scrub_pty(&mut …)` (term.rs: a pty child cannot use `command`).
//
//   node app/scripts/childenv-check.mjs [<repo root>]   # exits non-zero on failure
//
// The optional root is for showing it red on a tree from before the rule.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = process.argv[2] ?? fileURLToPath(new URL("../..", import.meta.url));
const DIRS = [
  "crates/worktrees-core/src",
  "crates/worktrees-api/src",
  "crates/worktrees-cli/src",
  "app/src-tauri/src",
];

const rsFiles = (dir) =>
  fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) return rsFiles(p);
    return e.name.endsWith(".rs") ? [p] : [];
  });

// The non-test part: everything above the first `#[cfg(test)]` that opens a
// module (test modules sit at the bottom of a file here), with `//` comments
// blanked so a doc line naming `Command::new` is not a spawn. Line numbers
// are kept.
const code = (src) => {
  const m = /^#\[cfg\(test\)\]\s*\n(pub\s+)?mod \w+/m.exec(src);
  const head = m ? src.slice(0, m.index) : src;
  return head.split("\n").map((l) => l.replace(/\/\/.*$/, ""));
};

let failed = 0;
let files = 0;
let builders = 0;
const fail = (msg) => { failed++; console.log("FAIL " + msg); };

for (const d of DIRS) {
  const abs = path.join(ROOT, d);
  if (!fs.existsSync(abs)) { fail(`${d} does not exist under ${ROOT}`); continue; }
  for (const f of rsFiles(abs)) {
    files++;
    const rel = path.relative(ROOT, f);
    // A test module kept in its own file (`#[cfg(test)] mod tests;` in the
    // sibling mod.rs, as serve/tests.rs is) is test code from its first line.
    const parent = path.join(path.dirname(f), "mod.rs");
    if (path.basename(f) === "tests.rs" && fs.existsSync(parent) &&
        /^#\[cfg\(test\)\]\s*\nmod tests;/m.test(fs.readFileSync(parent, "utf8"))) continue;
    const lines = code(fs.readFileSync(f, "utf8"));
    lines.forEach((l, i) => {
      if (/\bCommand::new\(/.test(l) && !rel.endsWith("childenv.rs")) {
        fail(`${rel}:${i + 1}: bare Command::new — use childenv::command: ${l.trim()}`);
      }
      if (/\bprocess::Command\s+as\b|\bCommand\s+as\s+\w+\s*[;}]/.test(l)) {
        fail(`${rel}:${i + 1}: Command aliased, which would hide a spawn: ${l.trim()}`);
      }
      if (/\bCommandBuilder::new\(/.test(l)) {
        builders++;
        if (!lines.slice(i, i + 4).some((n) => /\bscrub_pty\(&mut\b/.test(n))) {
          fail(`${rel}:${i + 1}: pty CommandBuilder without scrub_pty in the next three lines`);
        }
      }
    });
  }
}

if (files === 0) fail("no .rs files found — wrong root?");
if (builders === 0) fail("no pty CommandBuilder found at all — has term.rs moved?");
console.log(`${files} files, ${builders} pty builders checked`);
if (failed) { console.log(`\n${failed} failure(s)`); process.exit(1); }
console.log("ok: every spawn goes through worktrees_core::childenv");
