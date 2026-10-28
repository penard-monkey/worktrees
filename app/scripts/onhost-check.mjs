// "Does this window run on the host", held (Phase 2 §4.4).
//
//   node app/scripts/onhost-check.mjs        # from the repo root; exits non-zero on failure
//
// The opener and dialog plugins act on the machine the WINDOW runs on. A
// window that drives a paired host would reveal, open or pick a path on the
// wrong machine, so every path-taking plugin call goes through
// `app/src/onhost.ts`, which asks `windowOnHost()` — the one predicate beside
// the door table in ipc.ts — and refuses with the reason when the answer is
// no. This check fails when:
//   - any other file under app/src imports `revealItemInDir` or `openPath` from
//     the opener, or anything from the dialog plugin (a path picked HERE and
//     handed to a host command);
//   - a wrapper in onhost.ts calls its plugin without asking the predicate, or
//     refuses without `notOnHost`'s reason (never silently);
//   - the predicate stops being ipc.ts's;
//   - a System Settings deep link (a `Denial.pane`, or any
//     `x-apple.systempreferences:` literal) is opened with `openUrl` anywhere
//     but onhost.ts: it opens the WINDOW's machine's Settings, so on a window
//     that drives a paired host it unlocks the wrong Mac.
// Any other `openUrl` is free: an http(s) URL means the same on every machine.
//
// Override, so a mutation can be tested without touching the tree:
//   ONHOST_SRC (onhost.ts), ONHOST_APPSRC (the tree to walk)
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const ONHOST = process.env.ONHOST_SRC ?? here("../src/onhost.ts");
const APPSRC = process.env.ONHOST_APPSRC ?? here("../src");
const IPC = here("../src/ipc.ts");
const read = (p) => fs.readFileSync(p, "utf8");

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);

// 1. Nobody else imports the path-taking plugin calls.
{
  const offenders = [];
  let files = 0;
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) { if (e.name !== "mock") walk(p); continue; }
      if (!/\.(ts|tsx)$/.test(e.name) || e.name.endsWith(".d.ts") || path.basename(p) === "onhost.ts") continue;
      files++;
      const src = read(p);
      for (const m of src.matchAll(/import\s*\{([^}]*)\}\s*from\s*"@tauri-apps\/plugin-opener"/g)) {
        const names = m[1].split(",").map((x) => x.trim().split(/\s+as\s+/)[0]).filter(Boolean);
        const bad = names.filter((n) => n !== "openUrl");
        if (bad.length) offenders.push(`${path.relative(APPSRC, p)}: ${bad.join(", ")} from the opener`);
      }
      if (/from\s*"@tauri-apps\/plugin-dialog"/.test(src)) offenders.push(`${path.relative(APPSRC, p)}: the dialog plugin`);
    }
  };
  walk(APPSRC);
  if (files < 20) fail(`only ${files} source files walked under ${APPSRC} — this check would pass on nothing`);
  else if (offenders.length) for (const o of offenders) fail(`a path-taking plugin call outside onhost.ts — ${o}`);
  else ok(`no file under app/src but onhost.ts imports a path-taking plugin call (${files} files; openUrl is free)`);
}

// 2. Each wrapper asks the predicate first, and refuses with the reason.
{
  const src = read(ONHOST);
  const fns = [
    ["revealOnHost", "revealItemInDir("],
    ["openOnHost", "openerOpenPath("],
    ["pickDirForHost", "dialogOpen("],
    ["openSettingsOnHost", "openUrl("],
  ];
  for (const [fn, call] of fns) {
    const body = new RegExp(`export (?:async )?function ${fn}\\([^)]*\\)[^{]*\\{([\\s\\S]*?)\\n\\}`).exec(src)?.[1] ?? "";
    if (!body) { fail(`onhost.ts: ${fn} not found`); continue; }
    const gate = body.search(/windowOnHost\(\)/);
    const at = body.indexOf(call);
    if (at < 0) fail(`onhost.ts: ${fn} does not call ${call.slice(0, -1)}`);
    else if (gate < 0 || gate > at) fail(`onhost.ts: ${fn} calls ${call.slice(0, -1)} without asking windowOnHost() first`);
    else if (!/notOnHost\(/.test(body)) fail(`onhost.ts: ${fn} refuses without notOnHost's reason — a greyed act must say why`);
    else ok(`${fn} asks windowOnHost() before ${call.slice(0, -1)} and refuses with the reason`);
  }
  const hostOnly = /export function hostOnly\(what: string\): \{[^}]*\} \{([\s\S]*?)\n\}/.exec(src)?.[1] ?? "";
  if (!/^\s*return windowOnHost\(\) \? \{\} : \{ disabled: true, title: notOnHost\(what\) \};\s*$/.test(hostOnly))
    fail("onhost.ts: hostOnly does not grey with notOnHost's reason exactly when windowOnHost() is false");
  else ok("hostOnly greys a button, with the reason as its title, exactly when the window is not on its host");
  if (!/import \{ notOnHost, windowOnHost \} from "\.\/ipc";/.test(src)) fail("onhost.ts does not take the predicate from ipc.ts");
  else ok("the predicate is ipc.ts's, beside the door table");
}

// 2b. A Settings deep link never reaches a bare `openUrl`. It is a URL, so rule
//     1 lets the import through; what makes it host-only is what it opens.
{
  const offenders = [];
  let files = 0;
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) { if (e.name !== "mock") walk(p); continue; }
      if (!/\.(ts|tsx)$/.test(e.name) || e.name.endsWith(".d.ts") || path.basename(p) === "onhost.ts") continue;
      files++;
      const src = read(p);
      for (const m of src.matchAll(/\bopenUrl\(([^)]*)\)/g)) {
        if (/\.pane\b|x-apple\.systempreferences/.test(m[1])) offenders.push(`${path.relative(APPSRC, p)}: openUrl(${m[1]})`);
      }
      if (/["'`]x-apple\.systempreferences:/.test(src)) offenders.push(`${path.relative(APPSRC, p)}: an x-apple.systempreferences: literal`);
    }
  };
  walk(APPSRC);
  if (files < 20) fail(`only ${files} source files walked under ${APPSRC} — this check would pass on nothing`);
  else if (offenders.length) for (const o of offenders) fail(`a System Settings link opened outside openSettingsOnHost — ${o}`);
  else ok(`no file but onhost.ts opens a System Settings link (${files} files)`);
}

// 3. The predicate is false exactly when something set the window away.
{
  const ipc = read(IPC);
  if (!/export function windowOnHost\(\): boolean \{\s*return away === null;\s*\}/.test(ipc)) fail("ipc.ts: windowOnHost is not `away === null`");
  else ok("windowOnHost() is true until the remote door (or the dev stack) says why not");
}

console.log(failed ? `\nonhost-check: ${failed} failure(s)` : "\nonhost-check: all good");
process.exit(failed ? 1 : 0);
