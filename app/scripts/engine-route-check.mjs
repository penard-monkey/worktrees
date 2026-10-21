// The app's routing table, held against the engine it routes to.
//
//   node app/scripts/engine-route-check.mjs        # from the repo root; exits non-zero on failure
//
// The shipped app is a client of this host's `worktrees serve`. Its door
// (`app/src/ipc.ts`) sends every command to serve over the socket EXCEPT the
// ones that act on the machine the window runs on, which stay on Tauri IPC:
// serve's `ClientLocal` rows (`CLIENT_LOCAL`) plus the shim's own two
// (`SHIM_COMMANDS`). Three lists that nothing compiles together say the same
// thing — ipc.ts's, dispatch.rs's `ClientLocal` rows, and the shim's
// `generate_handler!` — and each way they can drift fails silently somewhere
// else: a command routed to Tauri that the shim does not register rejects in
// the app; one routed to the socket that serve refuses answers `client-local`;
// one the shim registers that serve also runs is a second engine for it. The
// same shape as dnd-check.mjs: parse the Rust, parse the mirror, compare.
//
// Also held here, because they are the shim's side of the same contract:
//   - the shim's handshake (src-tauri/src/engine.rs) speaks serve's labels,
//     reads serve's file name, asks with an origin serve allows by name, and
//     tags the serve it starts with an owner serve accepts;
//   - the dev-only `--allow-origin` path cannot ship: the flag is built in ONE
//     place, from `dev_origin`, whose only `Some` comes from a `#[cfg(dev)]`
//     function with a `#[cfg(not(dev))]` twin that answers `None`;
//   - the shim stops only a serve it manages: `terminate` (SIGTERM of the old
//     serve) is reached from the Replace arm alone, which `decide` reaches only
//     from a PROVEN serve — never from a pid that failed the proof.
//
// Overrides, so a mutation can be tested without touching the tree:
//   ROUTE_IPC, ROUTE_LIB, ROUTE_ENGINE, ROUTE_DISPATCH, ROUTE_MOCK, ROUTE_APPSRC
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = {
  ipc: process.env.ROUTE_IPC ?? here("../src/ipc.ts"),
  lib: process.env.ROUTE_LIB ?? here("../src-tauri/src/lib.rs"),
  engine: process.env.ROUTE_ENGINE ?? here("../src-tauri/src/engine.rs"),
  dispatch: process.env.ROUTE_DISPATCH ?? here("../../crates/worktrees-cli/src/serve/dispatch.rs"),
  mock: process.env.ROUTE_MOCK ?? here("../src/mock/install.ts"),
  appsrc: process.env.ROUTE_APPSRC ?? here("../src"),
  policy: here("../../crates/worktrees-cli/src/serve/policy.rs"),
  servefile: here("../../crates/worktrees-cli/src/serve/servefile.rs"),
  servemod: here("../../crates/worktrees-cli/src/serve/mod.rs"),
};
const read = (p) => fs.readFileSync(p, "utf8");
/** Code only: `//` comments dropped, so prose about a rule cannot satisfy it. */
const code = (s) => s.split("\n").map((l) => l.replace(/\/\/.*$/, "")).join("\n");
const noTests = (s) => s.split("\n#[cfg(test)]")[0];

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const sameSet = (a, b) => a.length === b.length && [...a].sort().join() === [...b].sort().join();
const minus = (a, b) => a.filter((x) => !b.includes(x));

// ── the three lists ─────────────────────────────────────────────────────────

const ipc = read(SRC.ipc);
const tsList = (name) => {
  const m = new RegExp(`export const ${name}: readonly string\\[\\] = \\[([^\\]]*)\\]`).exec(ipc);
  return m ? [...m[1].matchAll(/"([a-z_0-9]+)"/g)].map((x) => x[1]) : null;
};
const LOCAL = tsList("CLIENT_LOCAL");
const SHIM = tsList("SHIM_COMMANDS");
if (!LOCAL?.length || !SHIM?.length) fail(`ipc.ts: CLIENT_LOCAL (${LOCAL}) or SHIM_COMMANDS (${SHIM}) not found — this check would pass on nothing`);

const dispatch = noTests(read(SRC.dispatch));
const open = dispatch.indexOf("commands! { cx, a;");
const block = open < 0 ? "" : dispatch.slice(open, dispatch.indexOf("\n}\n", open));
const rows = [...block.matchAll(/^ {4}([a-z][a-z0-9_]*): (Engine|ClientLocal|HostAdmin|Stream) =>/gm)].map((m) => [m[1], m[2]]);
if (rows.length < 100) fail(`dispatch.rs: only ${rows.length} rows parsed — is the commands! block where this check looks?`);
const ROWS = rows.map(([n]) => n);
const CLIENT_ROWS = rows.filter(([, e]) => e === "ClientLocal").map(([n]) => n);

const lib = noTests(read(SRC.lib));
const gh = /tauri::generate_handler!\[([\s\S]*?)\]\)/.exec(lib);
const HANDLERS = (gh?.[1] ?? "").split(",").map((s) => s.trim()).filter(Boolean);
if (!gh || !HANDLERS.length) fail("lib.rs: generate_handler![…] not found or empty");

if (LOCAL && SHIM && rows.length && HANDLERS.length) {
  // 1. ipc.ts's client-local list IS serve's ClientLocal rows, both ways.
  if (sameSet(LOCAL, CLIENT_ROWS)) ok(`ipc.ts CLIENT_LOCAL is dispatch.rs's ClientLocal rows, both ways (${LOCAL.length}: ${LOCAL.join(", ")})`);
  else {
    const extra = minus(LOCAL, CLIENT_ROWS), missing = minus(CLIENT_ROWS, LOCAL);
    if (extra.length) fail(`routed to Tauri but serve runs them (not ClientLocal rows): ${extra.join(", ")} — the app would answer them from a shim that has no engine`);
    if (missing.length) fail(`ClientLocal in serve but routed to the socket: ${missing.join(", ")} — serve answers them "client-local" and nothing runs`);
  }
  // 2. The shim registers exactly what the door routes to it.
  const routed = [...LOCAL, ...SHIM];
  if (sameSet(HANDLERS, routed)) ok(`generate_handler! is CLIENT_LOCAL ∪ SHIM_COMMANDS, both ways (${HANDLERS.length})`);
  else {
    const notRouted = minus(HANDLERS, routed), notRegistered = minus(routed, HANDLERS);
    if (notRouted.length) fail(`the shim registers commands the door never routes to it: ${notRouted.join(", ")}${notRouted.some((n) => ROWS.includes(n)) ? " — and serve runs them too: a second engine for those" : ""}`);
    if (notRegistered.length) fail(`the door routes to the shim commands it does not register: ${notRegistered.join(", ")} — every one rejects in the app`);
  }
  // 3. The shim's own commands are never serve's.
  const clash = SHIM.filter((n) => ROWS.includes(n));
  if (clash.length) fail(`SHIM_COMMANDS that are also dispatch rows: ${clash.join(", ")} — the token or the window would be served over the socket`);
  else ok(`no shim command is a dispatch row (${SHIM.join(", ")})`);
}

// 4. Every command the page names is one somebody answers.
{
  const appsrc = SRC.appsrc;
  const unknown = [];
  let calls = 0;
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) { if (e.name !== "mock") walk(p); continue; }
      if (!/\.(ts|tsx)$/.test(e.name) || e.name.endsWith(".d.ts")) continue;
      for (const m of read(p).matchAll(/\binvoke(?:<[^>()]*(?:<[^>]*>[^>()]*)*>)?\(\s*"([a-z_0-9]+)"/g)) {
        calls++;
        if (!ROWS.includes(m[1]) && !(SHIM ?? []).includes(m[1])) unknown.push(`${path.relative(appsrc, p)}: ${m[1]}`);
      }
    }
  };
  walk(appsrc);
  if (calls < 100) fail(`only ${calls} invoke("…") literals found under app/src — is the pattern still right?`);
  else if (unknown.length) fail(`the page invokes commands that are neither serve rows nor shim commands: ${unknown.join("; ")}`);
  else ok(`every one of the page's ${calls} invoke("…") literals names a serve row or a shim command`);
}

// 5. The mock answers the shim's commands too (AGENTS.md: it tracks every command the shim registers).
{
  const mock = read(SRC.mock);
  const missing = (SHIM ?? []).filter((n) => !mock.includes(`case "${n}":`));
  if (missing.length) fail(`mock/install.ts does not answer: ${missing.join(", ")}`);
  else ok(`the mock answers every shim command (${(SHIM ?? []).join(", ")})`);
}

// ── the shim's side of serve's contract ─────────────────────────────────────

const engine = noTests(read(SRC.engine));
const engineCode = code(engine);
const rustStr = (src, name) => {
  const m = new RegExp(`pub const ${name}: &str = "((?:[^"\\\\]|\\\\.)*)";`).exec(src);
  return m ? JSON.parse(`"${m[1]}"`) : null;
};
{
  const policy = read(SRC.policy);
  const pairs = [
    ["CLIENT_LABEL", rustStr(engine, "CLIENT_LABEL"), rustStr(policy, "CLIENT_LABEL")],
    ["SERVER_LABEL", rustStr(engine, "SERVER_LABEL"), rustStr(policy, "SERVER_LABEL")],
    ["the file", rustStr(engine, "SERVE_FILE"), rustStr(read(SRC.servefile), "FILE")],
  ];
  const bad = pairs.filter(([, a, b]) => !a || a !== b);
  for (const [k, a, b] of bad) fail(`engine.rs ${k} is ${JSON.stringify(a)}, serve's is ${JSON.stringify(b)}`);
  if (!bad.length) ok(`engine.rs speaks serve's handshake labels and reads serve's file (${pairs.map(([, a]) => JSON.stringify(a)).join(" ")})`);
  const builtin = /pub const BUILTIN_ORIGINS: &\[&str\] = &\[([^\]]*)\]/.exec(policy)?.[1] ?? "";
  const appOrigin = rustStr(engine, "APP_ORIGIN");
  if (!appOrigin || !builtin.includes(`"${appOrigin}"`)) fail(`engine.rs APP_ORIGIN ${JSON.stringify(appOrigin)} is not one of serve's BUILTIN_ORIGINS (${builtin}) — a release app's probe would be refused`);
  else ok(`the release probe's Origin (${appOrigin}) is one serve allows by name`);
  const owners = /pub const OWNERS: &\[&str\] = &\[([^\]]*)\]/.exec(read(SRC.servemod))?.[1] ?? "";
  const owner = rustStr(engine, "OWNER");
  if (!owner || !owners.includes(`"${owner}"`)) fail(`engine.rs OWNER ${JSON.stringify(owner)} is not one of serve's --owner values (${owners}) — every serve the app starts would refuse its argv`);
  else ok(`the owner the app starts serve with (${owner}) is one serve's --owner takes`);
}

// The dev-only --allow-origin path cannot ship.
{
  const flagAt = [...engineCode.matchAll(/"--allow-origin"/g)].map((m) => m.index);
  const sa = /pub fn serve_args\([^)]*\) -> Vec<OsString> \{([\s\S]*?)\n\}/.exec(engineCode);
  const inArgs = sa && flagAt.length === 1 && flagAt[0] > sa.index && flagAt[0] < sa.index + sa[0].length;
  if (!inArgs) fail(`"--allow-origin" must appear exactly once in engine.rs, inside serve_args (found ${flagAt.length})`);
  else if (!/if let Some\(o\) = dev_origin \{\s*a\.push\("--allow-origin"\.into\(\)\);/.test(sa[1])) fail("serve_args adds --allow-origin other than under `if let Some(o) = dev_origin`");
  else ok("engine.rs builds --allow-origin once, in serve_args, only from Some(dev_origin)");
  const libCode = code(lib);
  if (/--allow-origin|allow_origin/.test(libCode)) fail("lib.rs names --allow-origin / allow_origin itself — the flag is serve_args' alone");
  const devFns = [...libCode.matchAll(/(#\[cfg\((not\(dev\)|dev)\)\]\s*)?fn dev_origin\([^)]*\) -> Option<String> \{([\s\S]*?)\n\}/g)];
  const onDev = devFns.filter((m) => m[2] === "dev");
  const offDev = devFns.filter((m) => m[2] === "not(dev)");
  if (devFns.length !== 2 || onDev.length !== 1 || offDev.length !== 1) fail(`lib.rs must define dev_origin exactly twice, one #[cfg(dev)] and one #[cfg(not(dev))] (found ${devFns.length}: ${devFns.map((m) => m[2] ?? "ungated").join(", ")})`);
  else if (offDev[0][3].trim() !== "None") fail(`the #[cfg(not(dev))] dev_origin answers ${JSON.stringify(offDev[0][3].trim())}, not None — a release build would start serve with a dev origin`);
  else ok("dev_origin is #[cfg(dev)] with a #[cfg(not(dev))] twin answering None, so a release build passes no dev origin");
  const passes = [...libCode.matchAll(/engine::Engine::new\(([\s\S]*?)\)\)\);/g)];
  if (passes.length !== 1 || !/let dev = dev_origin\(app\);/.test(libCode) || !/^\s*dev,\s*$/m.test(passes[0][1])) fail("the Engine is not built with `dev_origin(app)` as its dev origin — something else could feed serve_args");
  else ok("the Engine's dev origin is dev_origin(app) and nothing else");
  const binDev = /engine::serve_bin\(\s*cfg!\(dev\),/.test(libCode);
  if (!binDev) fail("serve_bin is not told cfg!(dev) — WORKTREES_SERVE_BIN or the workspace target could be honoured by a release build");
  else ok("serve_bin's dev knobs (WORKTREES_SERVE_BIN, the workspace target) are gated on cfg!(dev)");
}

// The shim stops only what it manages, and never a pid that failed the proof.
{
  const decide = /pub fn decide\(seen: Seen, ours: &str\) -> Decision \{([\s\S]*?)\n\}/.exec(engineCode)?.[1] ?? "";
  const unproven = /Seen::Unproven\([^)]*\) => Decision::(\w+)/.exec(decide)?.[1];
  const replaceFrom = [...decide.matchAll(/(Seen::\w+)\([^)]*\)[^\n]*?=> Decision::Replace/g)].map((m) => m[1]);
  if (unproven !== "Refuse") fail(`decide() answers an UNPROVEN holder of the port with ${unproven ?? "nothing"}, not Refuse`);
  else if (!replaceFrom.length || replaceFrom.some((s) => s !== "Seen::Proven")) fail(`decide() reaches Replace from ${replaceFrom.join(", ") || "nowhere"} — only a PROVEN serve may be replaced`);
  else ok("decide() refuses an unproven port holder and replaces only a proven serve");
  const connect = /pub fn connect\(&self, ask: Ask, phase: &dyn Fn\(&str\)\) -> Result<Endpoint, ConnectError> \{([\s\S]*?)\n    \}/.exec(engineCode)?.[1] ?? "";
  if (!connect) fail("engine.rs: `pub fn connect(&self, …) -> Result<Endpoint, ConnectError>` not found — the checks below would pass on nothing");
  const terms = [...engineCode.matchAll(/self\.terminate\(/g)].length;
  const inReplace = /Decision::Replace\(sf, theirs\) => \{(?:(?!Decision::)[\s\S])*?self\.terminate\(&sf\)\.map_err\(ConnectError::failed\)\?;/.test(connect);
  if (terms !== 1 || !inReplace) fail(`self.terminate( is called ${terms} time(s); it must be called once, from connect()'s Decision::Replace arm`);
  else ok("the old serve is SIGTERMed only from the Replace arm");

  // The page's `start: false` is the one thing the shim obeys of its retry
  // policy (app/src/reconnect.ts): nothing is started or replaced under it.
  if (!/match permit\(decide\(seen, &self\.version\), ask\.may_start\) \{/.test(connect)) fail("connect() does not decide through permit(decide(…), ask.may_start) — a try the page sent with start:false could start or replace a serve");
  else ok("connect() decides through permit(…, ask.may_start)");
  const permitFn = /pub fn permit\(d: Decision, may_start: bool\) -> Decision \{([\s\S]*?)\n\}/.exec(engineCode)?.[1] ?? "";
  const gated = ["Start(why)", "Replace(sf, theirs)"].filter((arm) => new RegExp(`Decision::${arm.replace(/[()]/g, "\\$&")} if !may_start => \\{?\\s*Decision::Decline`).test(permitFn));
  if (gated.length !== 2) fail(`permit() must turn BOTH Start and Replace into Decline under !may_start (gated: ${gated.join(", ") || "none"})`);
  else ok("permit() declines both a start and a replace when the try may not start");
  const starts = [...engineCode.matchAll(/self\.start\(/g)].length;
  const inConnect = [...connect.matchAll(/self\.start\(ask\.manual\)/g)].length;
  if (starts !== 2 || inConnect !== 2) fail(`self.start( is called ${starts} time(s), ${inConnect} from connect(); it must be exactly the Start and Replace arms, after permit()`);
  else ok("serve is spawned only from connect()'s Start and Replace arms");
  if (!/let seen = if ask\.may_start \{ self\.settle\(seen, STOP_WAIT, phase\) \} else \{ seen \};/.test(connect)) fail("connect() does not wait (settle, ≤ STOP_WAIT) for a serve still stopping before it may start one");
  else ok("a try that may start waits for a serve still stopping first (settle, ≤ STOP_WAIT)");
  if (!/engine::Ask \{ reason: reason\.as_deref\(\), may_start: start, manual \}/.test(code(lib))) fail("lib.rs engine_connect does not hand the page's `start` to connect() as may_start");
  else ok("engine_connect hands the page's start through as may_start, unaltered");
  if (/SIGKILL/.test(engineCode)) fail("engine.rs sends SIGKILL somewhere — a serve is never killed, only asked to stop");
  else ok("engine.rs never sends SIGKILL");
  if (/libc::kill\(/.test(code(lib))) fail("lib.rs signals a process itself — stopping serve is engine.rs's stop_owned()");
}

console.log(failed ? `\nengine-route-check: ${failed} failure(s)` : "\nengine-route-check: all good");
process.exit(failed ? 1 : 0);
