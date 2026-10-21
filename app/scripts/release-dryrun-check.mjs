// A packaging pull request runs the release's own steps (release.yml) as a dry
// run, and that is only safe while nothing on that path can publish. The shape
// that makes it so, and what this pins:
//
//   release.yml          — the steps. Triggered ONLY by workflow_call, with a
//                          required boolean `dry_run`; no write permission and
//                          no publishing operation anywhere in it; the signing
//                          secret read once, in the bundle step, which refuses
//                          the real key in a dry run, signs with a throwaway one
//                          there, and refuses to publish without the real one.
//                          Every job but `plan` takes the tag from plan, never
//                          from GITHUB_REF_NAME (a pull request's is "N/merge").
//   publish.yml          — a v* tag push. Calls release.yml with dry_run: false
//                          and the key, and is the one file that creates the
//                          release or holds `contents: write`, on that job only.
//   any other caller     — dry_run: true, no `secrets:` at all, never
//                          pull_request_target (which runs with the base
//                          repository's secrets and a write token).
//
//   node app/scripts/release-dryrun-check.mjs [<repo root>]   # exits non-zero on failure
//
// The optional root is for showing it red on a broken copy, or on a tree from
// before the dry run existed.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = process.argv[2] ?? fileURLToPath(new URL("../..", import.meta.url));
const WF = path.join(ROOT, ".github/workflows");
const SHARED = "release.yml";
const PUBLISHER = "publish.yml";
const KEY = "secrets.TAURI_SIGNING_PRIVATE_KEY";

let failed = 0;
const fail = (msg) => { failed++; console.log("FAIL " + msg); };

// YAML comments out (a `#` at the start or after whitespace), line numbers kept.
// Good enough for these files: no `#` inside a quoted YAML scalar, and a shell
// `${x#y}` never has whitespace before its `#`.
const code = (src) => src.split("\n").map((l) => l.replace(/(^|\s)#.*$/, ""));
// The lines of a top-level key's block, or of a job's.
const block = (lines, re, indent) => {
  const i = lines.findIndex((l) => re.test(l));
  if (i < 0) return null;
  const out = [lines[i]];
  for (const l of lines.slice(i + 1)) {
    if (l.trim() && l.search(/\S/) <= indent) break;
    out.push(l);
  }
  return out;
};
const jobs = (lines) => {
  const j = block(lines, /^jobs:\s*$/, 0) ?? [];
  const names = j.map((l) => /^  ([\w-]+):\s*$/.exec(l)?.[1]).filter(Boolean);
  return Object.fromEntries(names.map((n) => [n, block(j, new RegExp(`^  ${n}:\\s*$`), 2)]));
};
const triggers = (lines) =>
  (block(lines, /^on:\s*$/, 0) ?? []).map((l) => /^  ([\w-]+):/.exec(l)?.[1]).filter(Boolean);
const has = (lines, re) => lines.some((l) => re.test(l));

const WRITE = /(:\s*write\s*$|write-all)/;
const PUBLISHES = [
  /\bgh\s+release\b/, /\bgh\s+api\b/, /action-gh-release/, /create-release/,
  /upload-release-asset/, /\bgit\s+push\b/, /\bgit\s+tag\b/, /\b(npm|pnpm|cargo)\s+publish\b/,
];
const CALL = /^\s*uses:\s*\.\/\.github\/workflows\/release\.yml\s*$/;

if (!fs.existsSync(path.join(WF, SHARED))) {
  fail(`${SHARED} does not exist under ${WF}`);
  process.exit(1);
}
const files = fs.readdirSync(WF).filter((f) => /\.ya?ml$/.test(f));
const src = Object.fromEntries(files.map((f) => [f, code(fs.readFileSync(path.join(WF, f), "utf8"))]));

// ── release.yml: the steps every packaging PR runs ──────────────────────────
{
  const L = src[SHARED];
  const on = triggers(L);
  if (on.join(",") !== "workflow_call") {
    fail(`${SHARED}: its only trigger must be workflow_call, found [${on.join(", ")}] — a direct trigger runs it without a caller saying dry_run`);
  }
  const input = block(L, /^      dry_run:\s*$/, 6) ?? [];
  if (!has(input, /^\s*type:\s*boolean\s*$/) || !has(input, /^\s*required:\s*true\s*$/)) {
    fail(`${SHARED}: the dry_run input must be \`type: boolean\` and \`required: true\` — an optional one reads empty, and empty == false`);
  }
  L.forEach((l, i) => {
    if (WRITE.test(l)) fail(`${SHARED}:${i + 1}: a write permission in the workflow a pull request runs: ${l.trim()}`);
    for (const re of PUBLISHES) {
      if (re.test(l)) fail(`${SHARED}:${i + 1}: a publishing operation in the workflow a pull request runs (it belongs in ${PUBLISHER}): ${l.trim()}`);
    }
    for (const m of l.matchAll(/secrets\.[\w]+/g)) {
      if (m[0] !== KEY) fail(`${SHARED}:${i + 1}: ${m[0]} — the only secret release.yml may read is ${KEY}`);
    }
  });
  const keyAt = L.flatMap((l, i) => (l.includes(KEY) ? [i] : []));
  if (keyAt.length !== 1) {
    fail(`${SHARED}: ${KEY} must be read exactly once (the bundle step), found ${keyAt.length}`);
  } else {
    // The step holding it: back to its `- name:`, forward to the next step.
    let s = keyAt[0];
    while (s > 0 && !/^\s*- (name|uses):/.test(L[s])) s--;
    const step = block(L, new RegExp(`^${L[s].replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`), L[s].search(/-/));
    const body = (step ?? []).join("\n");
    const need = [
      [/DRY_RUN:\s*\$\{\{\s*inputs\.dry_run\s*\}\}/, "DRY_RUN from inputs.dry_run"],
      [/if \[ -n "\$TAURI_SIGNING_PRIVATE_KEY" \]/, "the dry run's refusal of a real key"],
      [/tauri signer generate/, "the dry run's throwaway key"],
      [/elif \[ -z "\$TAURI_SIGNING_PRIVATE_KEY" \]/, "the release's refusal of a missing key"],
    ];
    for (const [re, what] of need) {
      if (!re.test(body)) fail(`${SHARED}:${keyAt[0] + 1}: the step that reads ${KEY} has lost ${what}`);
    }
  }
  const J = jobs(L);
  if (!J.plan) fail(`${SHARED}: no \`plan\` job — the tag and targets are decided there`);
  else if (!has(J.plan, /refs\/tags\/v\*\)/)) fail(`${SHARED}: plan no longer refuses a publishing run that is not on a v* tag`);
  for (const [name, lines] of Object.entries(J)) {
    if (name === "plan") continue;
    lines.forEach((l) => {
      if (/GITHUB_REF_NAME|github\.ref_name/.test(l)) {
        fail(`${SHARED}: job ${name} reads the ref name — take the tag from plan (TAG); a pull request's is "N/merge": ${l.trim()}`);
      }
    });
  }
}

// ── every caller ─────────────────────────────────────────────────────────────
let publishers = 0;
let dryRuns = 0;
for (const f of files) {
  const L = src[f];
  if (f !== SHARED && f !== PUBLISHER) {
    L.forEach((l, i) => {
      for (const re of PUBLISHES) {
        if (re.test(l)) fail(`${f}:${i + 1}: a publishing operation outside ${PUBLISHER}: ${l.trim()}`);
      }
      if (l.includes(KEY)) fail(`${f}:${i + 1}: ${KEY} outside ${SHARED} and ${PUBLISHER}`);
    });
  }
  for (const [name, lines] of Object.entries(jobs(L))) {
    if (!has(lines, CALL)) continue;
    const dry = lines.map((l) => /^\s*dry_run:\s*(\S+)\s*$/.exec(l)?.[1]).filter(Boolean);
    if (f === PUBLISHER) {
      publishers++;
      if (dry.join() !== "false") fail(`${f}: job ${name} must call release.yml with dry_run: false, found [${dry}]`);
      if (!has(lines, new RegExp(`TAURI_SIGNING_PRIVATE_KEY:\\s*\\$\\{\\{\\s*${KEY.replace(".", "\\.")}\\s*\\}\\}`))) {
        fail(`${f}: job ${name} does not pass ${KEY} — the release would refuse to sign`);
      }
    } else {
      dryRuns++;
      if (dry.join() !== "true") fail(`${f}: job ${name} calls release.yml without dry_run: true (found [${dry}])`);
      if (has(lines, /^\s*secrets:/)) fail(`${f}: job ${name} passes secrets to a dry run — none may reach it, \`inherit\` included`);
      const on = triggers(L);
      if (on.includes("pull_request_target")) {
        fail(`${f}: a dry run on pull_request_target runs a PR's code with the base repository's secrets and a write token`);
      }
      L.forEach((l, i) => {
        if (WRITE.test(l)) fail(`${f}:${i + 1}: a write permission in a dry run's caller: ${l.trim()}`);
      });
    }
  }
}

// ── publish.yml: the one place that publishes ────────────────────────────────
if (!src[PUBLISHER]) {
  fail(`${PUBLISHER} does not exist — nothing would create the release`);
} else {
  const L = src[PUBLISHER];
  const on = block(L, /^on:\s*$/, 0) ?? [];
  if (triggers(L).join(",") !== "push" || !has(on, /tags:\s*\[\s*'v\*'\s*\]/)) {
    fail(`${PUBLISHER}: must trigger on a v* tag push and nothing else`);
  }
  if (publishers !== 1) fail(`${PUBLISHER}: expected one job calling ${SHARED}, found ${publishers}`);
  const top = block(L, /^permissions:\s*$/, 0) ?? [];
  if (!has(top, /^\s*contents:\s*read\s*$/)) fail(`${PUBLISHER}: top-level permissions must be contents: read`);
  for (const [name, lines] of Object.entries(jobs(L))) {
    const writes = has(lines, WRITE);
    const releases = has(lines, /\bgh\s+release\s+create\b/);
    if (writes && !releases) fail(`${PUBLISHER}: job ${name} holds a write permission and does not create the release`);
    if (releases && !has(lines, /^\s*needs:\s*\S/)) fail(`${PUBLISHER}: job ${name} creates the release without needing the build`);
  }
}
if (dryRuns === 0) fail("no workflow runs release.yml as a dry run — the first release would be its first run");

console.log(`${files.length} workflows, ${publishers} publishing call, ${dryRuns} dry-run call(s) checked`);
if (failed) { console.log(`\n${failed} failure(s)`); process.exit(1); }
console.log("ok: nothing a pull request runs can publish, and the release runs the steps it rehearsed");
