// Round-trips diff.ts's unified-diff parser against REAL `git diff` output.
//
// The claim the Files tab's diff viewer rests on is that
// `git diff --unified=1000000` carries BOTH complete versions of a file, so the
// frontend can rebuild each side and syntax-highlight it as a whole file. This
// script checks exactly that claim, on whatever the current branch actually
// changed: for every changed path, the reconstructed old side must equal
// `git show <base>:<path>` and the new side must equal the file on disk, byte
// for byte. Anything the parser drops, duplicates or mis-orders shows up as a
// first-differing-line report rather than as a subtly wrong diff on screen.
//
// It imports the REAL diff.ts — no paraphrase — so running it before and after
// an edit tests the edit.
//
//   node app/scripts/diff-check.mjs [repo-path] [base-ref]
//
// Exits non-zero on any mismatch. To convince yourself it has teeth, make
// `flush()` push `d.text.toUpperCase()` and watch every multi-line file fail.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// Type-stripping via vite's own esbuild re-export, for the reason race-check.mjs
// gives: bare "esbuild" does not resolve from app/node_modules under pnpm's
// strict layout, but vite is a direct dependency and re-exports it.
import { transformWithEsbuild } from "vite";

const REPO = process.argv[2] || fileURLToPath(new URL("../..", import.meta.url));
const BASE_REF = process.argv[3] || null;
const DIFF_TS = fileURLToPath(new URL("../src/diff.ts", import.meta.url));

const { code } = await transformWithEsbuild(fs.readFileSync(DIFF_TS, "utf8"), DIFF_TS, { loader: "ts" });
const { parseUnifiedDiff, wordDiff, stripTrailingNewline } =
  await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);

// stderr is PIPED, not inherited: `tryGit` expects failures (a repo with no
// origin/master), and an inherited stream prints git's "fatal:" for each one
// above a run that is passing.
const git = (...a) =>
  execFileSync("git", ["-C", REPO, ...a], { encoding: "utf8", maxBuffer: 1 << 28, stdio: ["ignore", "pipe", "pipe"] });
const tryGit = (...a) => { try { return git(...a); } catch { return null; } };

// Same candidate precedence as lib.rs's BASE_CANDS — a check that resolved a
// different base than the app would be testing a diff the app never shows.
// Resolved lazily (a for-of, not map+find) so a repo with all four refs does not
// spawn three pointless merge-bases.
let base = BASE_REF ? tryGit("rev-parse", BASE_REF)?.trim() : null;
if (!BASE_REF) {
  for (const c of ["origin/main", "origin/master", "main", "master"]) {
    const sha = tryGit("merge-base", c, "HEAD")?.trim();
    if (sha) { base = sha; break; }
  }
}
if (!base) { console.log("no base branch to diff against — nothing to check"); process.exit(0); }

const files = git("diff", "--name-only", base).trim().split("\n").filter(Boolean);
console.log(`[diff-check] base=${base.slice(0, 8)} files=${files.length}`);

let ok = 0, bad = 0, skipped = 0;
for (const rel of files) {
  const patch = git("diff", "--no-ext-diff", "--no-color", "--unified=1000000", base, "--", rel);
  if (!patch.trim()) { skipped++; continue; }
  const p = parseUnifiedDiff(stripTrailingNewline(patch));
  if (p.binary) { skipped++; continue; }

  // An ADDED file has no base blob and a DELETED one has nothing on disk; both
  // are legitimate and both must reconstruct to an empty side.
  const wantOld = tryGit("show", `${base}:${rel}`) ?? "";
  let wantNew = "";
  try { wantNew = fs.readFileSync(`${REPO}/${rel}`, "utf8"); } catch { wantNew = ""; }

  const report = (side, want, got) => {
    const a = stripTrailingNewline(want).split("\n");
    const i = a.findIndex((l, k) => l !== got[k]);
    console.log(`  ${side}: want ${a.length} lines, got ${got.length}; first differing @${i}`);
    console.log(`    want ${JSON.stringify(a[i])}`);
    console.log(`    got  ${JSON.stringify(got[i])}`);
  };
  const eqOld = p.oldLines.join("\n") === stripTrailingNewline(wantOld);
  const eqNew = p.newLines.join("\n") === stripTrailingNewline(wantNew);
  if (eqOld && eqNew) { ok++; continue; }
  bad++;
  console.log(`FAIL ${rel}`);
  if (!eqOld) report("old", wantOld, p.oldLines);
  if (!eqNew) report("new", wantNew, p.newLines);
}

// Word diff: the spans must cut out exactly the changed substrings and nothing
// else. Whitespace is in the alphabet on purpose — a re-indented line is a real
// change and has to be visible, which is the one thing a word diff that split
// on whitespace could not say.
const WORD_CASES = [
  ["const foo = 1;", "const bar = 1;", ["foo"], ["bar"]],
  ["a b c d", "a b c d", [], []],
  ["  indent", "\tindent", ["  "], ["\t"]],
  ["x", "", ["x"], []],
  ["call(a, b)", "call(a, b, c)", [], [", c"]],
  // Two lines with no word in common mark NOTHING: the row's own del/add colour
  // already says the line was replaced, and marking every word on both sides is
  // noise. This shipped the other way once — the docstring promised empty spans
  // while the code returned the whole line — so the case is pinned here.
  ["foo", "12345", [], []],
  ["abc", "xyz", [], []],
];
let wbad = 0;
for (const [a, b, wa, wb] of WORD_CASES) {
  const w = wordDiff(a, b);
  const cut = (s, sp) => sp.map((x) => s.slice(x.start, x.end));
  const got = [cut(a, w.a), cut(b, w.b)];
  const want = [wa, wb];
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    wbad++;
    console.log(`FAIL wordDiff ${JSON.stringify(a)} → ${JSON.stringify(b)}`);
    console.log(`  want ${JSON.stringify(want)}`);
    console.log(`  got  ${JSON.stringify(got)}`);
  }
}

console.log(`[diff-check] round-trip ok=${ok} bad=${bad} skipped=${skipped}; wordDiff bad=${wbad}`);
process.exit(bad + wbad ? 1 : 0);
