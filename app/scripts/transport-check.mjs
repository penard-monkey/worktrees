// The frontend's door to the backend, held against the server it talks to.
//
//   node app/scripts/transport-check.mjs        # from the repo root; exits non-zero on failure
//
// `app/src/ipc.ts` is the only module that talks to the backend. The shipped
// app is a client of its host's `worktrees serve`: its door (`engineDoor`)
// sends every command over a WebSocket (`app/src/ipcws.ts`) except the
// client-local ones, which stay on Tauri IPC; the dev stack uses the same door
// in a browser; the mock harness keeps plain Tauri IPC. None of this is
// visible to any other suite — the mock answers below `@tauri-apps/api/core`
// and never sees the socket, and serve's Rust tests never see a browser. So
// this check does four things, the same way `api-boundary-check.mjs` and
// `termreplay-check.mjs` do:
//
//   1. PINS THE WIRE. Every frame key, the channel spelling, the first `seq`
//      and the end frame's, the client-local marker, the handshake's keys,
//      labels and spelling (one key a frame, lowercase hex), the size limits
//      and the byte-channel argument are read out of wire.rs / policy.rs /
//      conn.rs / servefile.rs / dispatch.rs and compared with `ipcws.ts`. And the behavioural tests below build their frames
//      from the RUST-parsed keys, not from WIRE — so a rename on the Rust side
//      fails them too, rather than letting a stub agree with a stale client.
//   2. DRIVES THE REAL CLIENT on a virtual clock against a fake socket.
//      Handshake v1 first (the fix for OWASP H1): the client proves it holds
//      the token and makes the server prove it too, with proofs this check
//      computes itself with node:crypto from the spec; the token is never on
//      the wire; a server that cannot prove itself — the review's stand-in on
//      a dead serve's port among them — is sent nothing, and its version
//      claim is not believed. Then replies routed by id (a non-string `err`
//      passed through unchanged), channels in `seq` order including a
//      reordered arrival, a channel's end (and the `err` that also ends it)
//      releasing its handler, events and unlisten, a request too big for serve
//      refused before it can end the connection, a version mismatch refused, a
//      refused handshake and a dead socket surfaced — never swallowed.
//   3. DRIVES THE ENGINE DOOR (ipc.ts) over real ipcws clients, its retries
//      (reconnect.ts, inlined as the real source) on a virtual clock:
//      client-local commands to the local door and nothing else, a lost socket
//      redialled ON ITS OWN on the backoff, every listener carried across
//      reconnects — subscribed exactly once per socket after five of them —
//      the engine's state (starting → up → waiting → up, `gen` counting
//      connections), a failed dial or a refused handshake retried in the
//      shim's own words, a hidden window deferring, someone else's serve never
//      restarted, and the crash-loop limit stopping the retries. A command's
//      own refusal (term_open for a session that died while the engine was
//      down) is the caller's error, never a lost engine: no retry, no dial.
//      And the termreplay contract through the whole chain:
//      TerminalPane's `useTerm` + `shellTransport` → ipc.ts → ipcws.ts → the
//      fake socket. A replay is a channel's first frame and stays unanswered;
//      a live query after it is answered.
//   4. GUARDS THE EDGES: one door (nothing else imports core/event), the dev
//      token endpoint's client reachable only behind `import.meta.env.DEV &&
//      VITE_SERVE` while the app's boot takes the token from the shim's
//      `engine_connect` alone, the endpoint refusing cross-origin asks, the
//      version source, and — when `app/dist` exists — the socket client IN the
//      build (the app is a client) and the dev endpoint's client NOT in it.
//
// Overrides, so a mutation can be tested without touching the tree:
//   TRANSPORT_IPCWS, TRANSPORT_IPC, TRANSPORT_RECONNECT, TRANSPORT_WIRE, TRANSPORT_POLICY,
//   TRANSPORT_DISPATCH, TRANSPORT_CONN, TRANSPORT_SERVEFILE, TRANSPORT_MAIN,
//   TRANSPORT_DOOR, TRANSPORT_DIST
import { createHmac, randomBytes } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const SRC = {
  ipcws: process.env.TRANSPORT_IPCWS ?? here("../src/ipcws.ts"),
  ipc: process.env.TRANSPORT_IPC ?? here("../src/ipc.ts"),
  reconnect: process.env.TRANSPORT_RECONNECT ?? here("../src/reconnect.ts"),
  ipcInTree: here("../src/ipc.ts"),
  pane: here("../src/TerminalPane.tsx"),
  main: process.env.TRANSPORT_MAIN ?? here("../src/main.tsx"),
  door: process.env.TRANSPORT_DOOR ?? here("../vite.serve-door.ts"),
  vite: here("../vite.config.ts"),
  envd: here("../src/vite-env.d.ts"),
  appsrc: here("../src"),
  dist: process.env.TRANSPORT_DIST ?? here("../dist"),
  wire: process.env.TRANSPORT_WIRE ?? here("../../crates/worktrees-cli/src/serve/wire.rs"),
  policy: process.env.TRANSPORT_POLICY ?? here("../../crates/worktrees-cli/src/serve/policy.rs"),
  dispatch: process.env.TRANSPORT_DISPATCH ?? here("../../crates/worktrees-cli/src/serve/dispatch.rs"),
  conn: process.env.TRANSPORT_CONN ?? here("../../crates/worktrees-cli/src/serve/conn.rs"),
  servefile: process.env.TRANSPORT_SERVEFILE ?? here("../../crates/worktrees-cli/src/serve/servefile.rs"),
  cliToml: here("../../crates/worktrees-cli/Cargo.toml"),
  wsToml: here("../../Cargo.toml"),
};
const read = (p) => fs.readFileSync(p, "utf8");

let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (cond, m) => (cond ? ok(m) : fail(m));
const tick = () => new Promise((r) => setTimeout(r, 0));
const ticks = async (n = 4) => { for (let i = 0; i < n; i++) await tick(); };
const b64 = (s) => Buffer.from(s).toString("base64");
const text = (ab) => new TextDecoder().decode(new Uint8Array(ab));
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

async function load(file, js) {
  return import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));
}
async function tsModule(file) {
  const out = await transformWithEsbuild(read(file), path.basename(file), { loader: "ts", format: "esm" });
  return out.code;
}

// ── 1. the wire, read out of the Rust ───────────────────────────────────────

/** The body of `fn name` in a Rust source (to the first line that closes it).
 *  Exactly ONE definition, or null and a failure: a check that took the first
 *  of two could be repointed at the wrong one (the docsviewer-check trap). */
function rustFn(src, name) {
  const re = new RegExp(`fn ${name}\\b[^{]*\\{([\\s\\S]*?)\\n\\}`, "gm");
  const all = [...src.matchAll(re)];
  if (all.length > 1) fail(`${all.length} definitions of fn ${name} — which one is the wire?`);
  return all.length === 1 ? all[0][1] : null;
}
/** A Rust `&str` const, with its escapes read the way rustc reads them. */
const rustStr = (src, name) => {
  const m = new RegExp(`pub const ${name}: &str = "((?:[^"\\\\]|\\\\.)*)";`).exec(src);
  return m ? JSON.parse(`"${m[1]}"`) : null;
};
/** A `usize` const: a literal, or `a << b`. */
const rustUsize = (src, name) => {
  const m = new RegExp(`pub const ${name}: usize = ([^;]+);`).exec(src);
  const e = m?.[1].trim() ?? "";
  if (/^\d+$/.test(e)) return Number(e);
  const sh = /^(\d+)\s*<<\s*(\d+)$/.exec(e);
  return sh ? Number(sh[1]) * 2 ** Number(sh[2]) : NaN;
};
/** The string keys of the first `json!({ … })` in `body`, in order. */
function jsonKeys(body) {
  const m = body && /json!\(\{([\s\S]*?)\}\)/.exec(body);
  return m ? [...m[1].matchAll(/"([^"]+)"\s*:/g)].map((x) => x[1]) : null;
}
const camel = (s) => s.replace(/_([a-z])/g, (_, c) => c.toUpperCase());

/** Handshake v1 as (main)'s spec states it (owasp-serve/handshake-v1.md,
 *  2026-10-18, the fix for OWASP H1). This is the check's OWN copy: the fake
 *  server in §2 builds every handshake frame from it and computes every proof
 *  with node:crypto, never from ipcws.ts, so a client that drifts from the
 *  spec fails on behaviour and not merely on a key comparison. */
const SPEC = {
  hello: "hello", challenge: "challenge", auth: "auth", ok: "ok", version: "version", proof: "proof",
  clientLabel: "worktrees-serve/v1/client\n", serverLabel: "worktrees-serve/v1/server\n",
  host: "127.0.0.1",
};
const HEX32 = /^[0-9a-f]{64}$/;
const hex32 = () => randomBytes(32).toString("hex");
/** HMAC-SHA256 keyed with the token's UTF-8 bytes, over label + cn + "\n" + sn, as lowercase hex. */
const spectag = (key, label, cn, sn) => createHmac("sha256", Buffer.from(key, "utf8")).update(`${label}${cn}\n${sn}`, "utf8").digest("hex");

const wireAll = read(SRC.wire);
const wire = wireAll.split("\n#[cfg(test)]")[0];
const policy = read(SRC.policy).split("\n#[cfg(test)]")[0];
const dispatch = read(SRC.dispatch).split("\n#[cfg(test)]")[0];
const conn = read(SRC.conn).split("\n#[cfg(test)]")[0];
const servefile = read(SRC.servefile).split("\n#[cfg(test)]")[0];

const K = {};
{
  const auth = jsonKeys(rustFn(wire, "auth_reply"));
  const okk = jsonKeys(rustFn(wire, "reply_ok"));
  const errk = jsonKeys(rustFn(wire, "reply_err"));
  const chan = jsonKeys(rustFn(wire, "channel_frame"));
  const evt = jsonKeys(rustFn(wire, "event_frame"));
  const req = rustFn(wire, "parse_request");
  const reqKeys = req ? [...req.matchAll(/o\.remove\("([^"]+)"\)/g)].map((x) => x[1]) : [];
  const prefix = /strip_prefix\("([^"]+)"\)/.exec(rustFn(wire, "channel_id") ?? "")?.[1];
  const clientLocal = /pub const CLIENT_LOCAL: &str = "([^"]+)";/.exec(wire)?.[1];
  // Handshake v1's client frames, as policy.rs reads them: `{<key>: <hex32>}`.
  const field = (fn) => /hex32_field\(frame, "([^"]+)"\)/.exec(rustFn(policy, fn) ?? "")?.[1];
  const helloKey = field("hello");
  const authKey = field("client_proof");
  const chall = jsonKeys(rustFn(wire, "challenge_frame"));
  const endk = jsonKeys(rustFn(wire, "end_frame"));
  // The first frame a sink writes, as the Rust test asserts it.
  const firstFrame = /assert_eq!\(seen\[0\], r#"(\{[^"]*"[^#]*)"#\)/.exec(wireAll)?.[1];
  const ff = firstFrame ? JSON.parse(firstFrame) : null;
  const sentFirst = /sink\.send\(b"([^"]*)"\)/.exec(wireAll)?.[1];

  const need = { auth, okk, errk, chan, evt, prefix, clientLocal, helloKey, authKey, chall, endk, ff };
  for (const [k, v] of Object.entries(need)) if (!v) fail(`wire: could not read ${k} out of the Rust — has a function been renamed? (this check would otherwise pass on nothing)`);
  if (auth && auth.length === 3 && /"ok":\s*true/.test(rustFn(wire, "auth_reply"))) {
    [K.authOk, K.authVersion, K.proof] = auth;
  } else fail(`wire: auth_reply is not {"ok": true, "<version>": …, "<proof>": …} (got ${JSON.stringify(auth)})`);
  if (chall?.length === 1) K.challenge = chall[0]; else fail(`wire: challenge_frame keys ${JSON.stringify(chall)}`);
  K.hello = helloKey;
  if (okk?.length === 2) [K.id, K.ok] = okk; else fail(`wire: reply_ok keys ${JSON.stringify(okk)}`);
  if (errk?.length === 2 && errk[0] === K.id) K.err = errk[1]; else fail(`wire: reply_err keys ${JSON.stringify(errk)}`);
  if (chan?.length === 3) [K.channel, K.seq, K.data] = chan; else fail(`wire: channel_frame keys ${JSON.stringify(chan)}`);
  if (evt?.length === 2) [K.event, K.payload] = evt; else fail(`wire: event_frame keys ${JSON.stringify(evt)}`);
  if (reqKeys.length === 3) [K.reqId, K.cmd, K.args] = reqKeys; else fail(`wire: parse_request reads ${JSON.stringify(reqKeys)}`);
  K.prefix = prefix;
  K.clientLocal = clientLocal;
  K.auth = authKey;
  if (ff && K.seq) {
    K.firstSeq = ff[K.seq];
    if (sentFirst != null && ff[K.data] !== b64(sentFirst)) fail(`wire: the Rust test's first frame data ${ff[K.data]} is not base64 of b"${sentFirst}" — is a ByteSink still base64?`);
  }
  K.byteKeys = [...dispatch.matchAll(/frames\.bytes\(a\.channel\("([^"]+)"\)/g)].map((x) => camel(x[1]));
  K.typedKeys = [...dispatch.matchAll(/frames\.typed::<[^>]+>\(a\.channel\("([^"]+)"\)/g)].map((x) => camel(x[1]));
  if (!K.byteKeys.length) fail("dispatch: no `frames.bytes(a.channel(…))` found — the byte-channel pin would be vacuous");
  if (!K.typedKeys.length) fail("dispatch: no `frames.typed::<…>(a.channel(…))` found — the typed-channel pin would be vacuous");
  K.authDeadline = Number(/AUTH_DEADLINE: std::time::Duration = std::time::Duration::from_secs\((\d+)\)/.exec(policy)?.[1]);
  K.clientLabel = rustStr(policy, "CLIENT_LABEL");
  K.serverLabel = rustStr(policy, "SERVER_LABEL");
  // The end frame: the channel frame's own keys, then one that is literally true.
  if (endk?.length === 3 && endk[0] === K.channel && endk[1] === K.seq && new RegExp(`"${endk[2]}":\\s*true`).test(rustFn(wire, "end_frame") ?? "")) K.end = endk[2];
  else fail(`wire: end_frame is not {"${K.channel}", "${K.seq}", "<end>": true} (got ${JSON.stringify(endk)})`);
  K.preauth = rustUsize(conn, "PREAUTH_BYTES");
  K.maxMessage = rustUsize(conn, "MAX_MESSAGE");
  if (!(K.preauth > 0) || !(K.maxMessage > 0)) fail(`conn: could not read PREAUTH_BYTES (${K.preauth}) or MAX_MESSAGE (${K.maxMessage})`);
}

const wsJs = await tsModule(SRC.ipcws);
check(!/^\s*import\s[^"']*["'][^"']+["']/m.test(wsJs), "ipcws.ts has no runtime import (a data: module could not resolve one — and this check loads it as one)");
const WS = await load(SRC.ipcws, wsJs);
{
  const W = WS.WIRE ?? {};
  const pairs = [
    ["hello", K.hello], ["challenge", K.challenge], ["auth", K.auth], ["proof", K.proof],
    ["clientLabel", K.clientLabel], ["serverLabel", K.serverLabel],
    ["version", K.authVersion], ["id", K.id], ["cmd", K.cmd], ["args", K.args],
    ["ok", K.ok], ["err", K.err], ["channel", K.channel], ["seq", K.seq], ["data", K.data], ["end", K.end],
    ["event", K.event], ["payload", K.payload], ["channelPrefix", K.prefix], ["firstSeq", K.firstSeq],
    ["clientLocal", K.clientLocal],
  ];
  const bad = pairs.filter(([k, v]) => W[k] !== v);
  for (const [k, v] of bad) fail(`WIRE.${k} is ${JSON.stringify(W[k])}, the Rust says ${JSON.stringify(v)}`);
  if (!bad.length) ok(`WIRE matches wire.rs/policy.rs — ${pairs.length} keys and labels (${pairs.map(([, v]) => JSON.stringify(v)).join(" ")})`);
  if (K.reqId !== K.id) fail(`wire: requests carry "${K.reqId}" and replies "${K.id}" — the client echoes one key`);
  check(K.authOk === K.ok, `the auth reply's success key is the reply's ("${K.authOk}")`);
  const bk = [...(WS.BYTE_CHANNEL_KEYS ?? [])].sort();
  check(same(bk, [...new Set(K.byteKeys)].sort()), `byte channels are exactly dispatch.rs's frames.bytes keys (${JSON.stringify(bk)} vs ${JSON.stringify([...new Set(K.byteKeys)])})`);
  check(K.typedKeys.every((k) => !bk.includes(k)), `no typed progress channel is decoded as bytes (${[...new Set(K.typedKeys)].join(", ")})`);
  check(K.authDeadline > 0 && WS.AUTH_WAIT_MS > K.authDeadline * 1000, `the client waits longer (${WS.AUTH_WAIT_MS}ms) than the server's AUTH_DEADLINE (${K.authDeadline}s), so a refusal reads as a refusal`);

  const spec = ["hello", "challenge", "auth", "ok", "version", "proof", "clientLabel", "serverLabel"];
  const sbad = spec.filter((k) => W[k] !== SPEC[k]);
  for (const k of sbad) fail(`WIRE.${k} is ${JSON.stringify(W[k])}, handshake v1 says ${JSON.stringify(SPEC[k])}`);
  if (!sbad.length) ok(`WIRE speaks handshake v1 — ${spec.length} keys and labels (${spec.map((k) => JSON.stringify(SPEC[k])).join(" ")})`);
  check(WS.SERVE_HOST === SPEC.host, `the client dials the literal ${SPEC.host}, never localhost, which resolves ::1 first (SERVE_HOST = ${JSON.stringify(WS.SERVE_HOST)})`);
  check(/StdListener::bind\(SocketAddr::from\(\(Ipv4Addr::LOCALHOST, port\)\)\)/.test(conn), "serve binds Ipv4Addr::LOCALHOST (conn.rs): the one address it listens on is the one the client dials");
  // The spec's rules as serve enforces them, which §2 holds the client to.
  const f32 = rustFn(policy, "hex32_field") ?? "";
  check(/if obj\.len\(\) != 1 \{\s*return None;/.test(f32), "serve refuses a handshake frame with more than ONE key (policy.rs hex32_field) — §2 holds the client's hello and auth to one key");
  const h32 = rustFn(policy, "is_hex32") ?? "";
  check(/s\.len\(\) == 64/.test(h32) && /\(b'a'\.\.=b'f'\)/.test(h32) && !/b'A'|ignore_ascii_case|to_ascii_lowercase/.test(h32),
    "serve's hex32 is 64 LOWERCASE hex and nothing else — upper case refused, not folded (policy.rs is_hex32); §2 holds the client to it");
  check(/\{x:02x\}/.test(rustFn(servefile, "hex32") ?? "") && /\{x:02x\}/.test(rustFn(policy, "server_proof") ?? ""),
    "serve writes its challenge and its proof as lowercase hex (servefile.rs hex32, policy.rs server_proof), which is all the client accepts");
  const mac = rustFn(policy, "mac") ?? "";
  const ups = [...mac.matchAll(/m\.update\((.+?)\);\s*$/gm)].map((x) => x[1]);
  check(/new_from_slice\(token\.as_bytes\(\)\)/.test(mac) && same(ups, ["label.as_bytes()", "cn.as_bytes()", 'b"\\n"', "sn.as_bytes()"]),
    `serve's HMAC is keyed with the token's bytes over label + cn + "\\n" + sn — the spec's formula, which §2 computes with node:crypto (${JSON.stringify(ups)})`);
  check(/mac\(token, CLIENT_LABEL,/.test(rustFn(policy, "client_proof_ok") ?? "") && /mac\(token, SERVER_LABEL,/.test(rustFn(policy, "server_proof") ?? ""),
    "serve checks the client's proof under the client label and proves itself under the server label (policy.rs)");
  const hs = rustFn(conn, "handshake_v1") ?? "";
  const at = (re) => hs.search(re);
  const steps = [/policy::hello\(/, /wire::challenge_frame\(/, /policy::client_proof\(/, /policy::client_proof_ok\(/, /wire::auth_reply\(&policy::server_proof\(/].map(at);
  check(steps.every((i, n) => i >= 0 && (n === 0 || i > steps[n - 1])),
    `serve's handshake is hello → challenge → proof → verified → ok+proof, proving itself only to a proven client (conn.rs handshake_v1; positions ${steps.join(",")})`);
  check(/let seq = self\.seq\.load\(Ordering::SeqCst\) \+ 1;\s*let _ = self\.outbox\.push\(end_frame\(self\.channel, seq\)\)/.test(wire) &&
    /end_frame\(self\.channel, self\.seq \+ 1\)/.test(wire),
    "a channel's end frame takes the seq one past its last data frame, bytes and typed alike (wire.rs Drop impls) — the client orders it like data");
  check(WS.MAX_MESSAGE === K.maxMessage, `the client's MAX_MESSAGE (${WS.MAX_MESSAGE}) is serve's (conn.rs, ${K.maxMessage}), so a request serve would end the connection over is refused before it is sent`);
}

// ── 2. the client, on a virtual clock ───────────────────────────────────────

const VERSION = "1.2.3";
// Shaped like serve's (servefile.rs: 32 CSPRNG bytes as hex), and random, so
// finding it in a frame can only mean the client put it there.
const TOKEN = randomBytes(32).toString("hex");
const PORT = 41237;

function clock() {
  let now = 0, next = 1;
  const timers = new Map();
  return {
    now: () => now,
    setTimeout(fn, ms) { const id = next++; timers.set(id, { at: now + ms, fn }); return id; },
    clearTimeout(id) { timers.delete(id); },
    advance(ms) {
      now += ms;
      for (const [id, t] of [...timers]) if (t.at <= now) { timers.delete(id); t.fn(); }
    },
    pending: () => timers.size,
  };
}

/** A call's outcome — or `{hung}`. A client that loses a call leaves its
 *  promise pending forever, and an awaited one would end this script with
 *  node's exit 13 and no failure printed: a hang has to be a RESULT. */
const HANG_MS = 1000;
const settle = (p) => Promise.race([
  p.then((v) => ({ ok: v }), (e) => ({ err: e })),
  new Promise((r) => setTimeout(() => r({ hung: true }), HANG_MS)),
]);
/** Wait, bounded, for what the client does after an await of its OWN:
 *  WebCrypto settles on node's thread pool, not on a tick. */
async function until(cond, ms = HANG_MS) {
  const end = Date.now() + ms;
  while (!cond()) { if (Date.now() > end) return false; await tick(); }
  return true;
}
const chanId = (s) => Number(String(s).slice(String(K.prefix).length));

/** Every frame any client in this run wrote; the token sweep in §3 reads it. */
const WIRE_LOG = [];
/** Anything the client threw OUT of a socket callback. In a browser that is an
 *  uncaught error nobody sees; here it would end the run with the failures it
 *  found unprinted. So it is recorded, and §2 ends by requiring none. */
const THROWN = [];
const cb = (fn) => { try { fn(); } catch (e) { THROWN.push(String(e?.stack ?? e).split("\n").slice(0, 2).join(" ")); } };
const tokenIn = (t) => [TOKEN, b64(TOKEN), Buffer.from(TOKEN, "utf8").toString("hex")].some((x) => t.includes(x));

/** WebCrypto as the page has it (`window.crypto`), wrapped only to SEE the
 *  server's proof go through `subtle.verify` and, with `gate`, to hold that
 *  check open while a test looks at what was sent in the meantime. */
function webcrypto({ gate = null, subtle = globalThis.crypto.subtle } = {}) {
  const seen = { verify: 0, verified: false };
  const s = subtle && {
    importKey: (...a) => subtle.importKey(...a),
    sign: (...a) => subtle.sign(...a),
    async verify(...a) {
      seen.verify++;
      const r = await subtle.verify(...a);
      if (gate) await gate;
      seen.verified = true;
      return r;
    },
  };
  return { crypto: { getRandomValues: (b) => globalThis.crypto.getRandomValues(b), subtle: s }, seen };
}

/** One client and the server side of its fake socket: frames go in through
 *  `recv`, what the client wrote comes out of `frames`. Requests are built
 *  from the RUST keys (K), the handshake from the spec (SPEC). */
function harness({ version = VERSION, gate = null, subtle, remedy, onError } = {}) {
  const socks = [];
  class FakeSock {
    constructor(url) { this.url = url; this.sent = []; this.closed = false; socks.push(this); }
    send(t) { if (this.closed) throw new Error("send on a closed socket"); this.sent.push(t); WIRE_LOG.push(t); }
    close() { this.closed = true; }
  }
  const errors = [];
  const warns = [];
  const clk = clock();
  const wc = webcrypto(subtle === undefined ? { gate } : { gate, subtle });
  const door = WS.connectSocket({
    port: PORT, token: TOKEN, version, crypto: wc.crypto,
    WebSocket: FakeSock, setTimeout: clk.setTimeout, clearTimeout: clk.clearTimeout,
    atob: globalThis.atob, onError: (m) => { errors.push(m); onError?.(m); }, warn: (m) => warns.push(m), remedy,
  });
  const sock = socks[0] ?? null;
  const frames = () => (sock?.sent ?? []).map((t) => JSON.parse(t));
  const h = {
    door, sock, socks, errors, warns, clk, seen: wc.seen, cn: null, sn: null,
    open: () => cb(() => sock?.onopen?.({})),
    recv: (obj) => cb(() => sock?.onmessage?.({ data: typeof obj === "string" ? obj : JSON.stringify(obj) })),
    drop: () => cb(() => sock?.onclose?.({})),
    frames,
    /** What the client wrote that is not the handshake: its requests. */
    requests: () => frames().filter((f) => K.cmd in f),
    reply: (id, v) => h.recv({ [K.id]: id, [K.ok]: v }),
    error: (id, e) => h.recv({ [K.id]: id, [K.err]: e }),
    chan: (ch, seq, data) => h.recv({ [K.channel]: ch, [K.seq]: seq, [K.data]: data }),
    /** A channel's last frame: no data, and the seq one past its last data frame. */
    end: (ch, seq) => h.recv({ [K.channel]: ch, [K.seq]: seq, [K.end]: true }),
    /** How many channels the client still holds a handler for. */
    openChannels: () => h.door.openChannels?.(),
    event: (name, payload) => h.recv({ [K.event]: name, [K.payload]: payload }),
    // serve's half of handshake v1, by the spec.
    hello: () => (h.cn = frames()[0]?.[SPEC.hello] ?? null),
    challenge: (sn = hex32()) => { h.sn = sn; h.recv({ [SPEC.challenge]: sn }); },
    /** The client's proof, once its own async sign has run (null if never). */
    async proof() {
      await until(() => frames().some((f) => SPEC.auth in f));
      return frames().find((f) => SPEC.auth in f)?.[SPEC.auth] ?? null;
    },
    /** serve's ok: its version, and its proof made with the token, unless
     *  `key`, `proof` or `version` say otherwise (`version: undefined` omits it). */
    welcome(o = {}) {
      const v = "version" in o ? o.version : VERSION;
      const proof = o.proof ?? spectag(o.key ?? TOKEN, SPEC.serverLabel, h.cn, h.sn);
      h.recv({ [SPEC.ok]: true, [SPEC.version]: v, [SPEC.proof]: proof });
    },
    /** Until the client's `subtle.verify` has settled, plus a turn for its continuation. */
    async verified() { const done = await until(() => wc.seen.verified); await tick(); return done; },
    async ready() { h.open(); h.hello(); h.challenge(); await h.proof(); h.welcome(); await h.verified(); },
  };
  return h;
}

// 2a. handshake v1, in order, and nothing before it.
{
  const h = harness();
  const heard = [];
  const early = settle(h.door.invoke("list_workspace"));
  await h.door.listen("places:changed", (e) => heard.push(e));
  check(h.sock !== null && h.sock.sent.length === 0, "a call (or a listen) before the socket opens writes nothing");
  check(h.sock?.url === `ws://${SPEC.host}:${PORT}/`, `the client dials the literal ${SPEC.host}, never localhost (${h.sock?.url})`);
  h.open();
  const f0 = h.frames();
  check(f0.length === 1 && same(Object.keys(f0[0]), [SPEC.hello]) && HEX32.test(f0[0][SPEC.hello]),
    `the first frame is exactly {"${SPEC.hello}": <32 random bytes, hex>} (${JSON.stringify(f0[0] ?? null).slice(0, 90)})`);
  h.hello();
  const mid = settle(h.door.invoke("get_settings"));
  h.challenge();
  const proof = await h.proof();
  const f1 = h.frames();
  check(f1.length === 2 && same(Object.keys(f1[1] ?? {}), [SPEC.auth]), `the challenge is answered with exactly {"${SPEC.auth}": <proof>} and nothing else (${f1.length} frames)`);
  check(proof === spectag(TOKEN, SPEC.clientLabel, h.cn, h.sn),
    "…whose proof is HMAC-SHA256(token, \"worktrees-serve/v1/client\\n\" + cn + \"\\n\" + sn), computed here with node:crypto from the spec");
  h.welcome();
  const verified = await h.verified();
  check(verified && h.seen.verify === 1, `(c) a server whose proof is right (computed here with node:crypto, from the spec) is checked with crypto.subtle.verify, once (${h.seen.verify} calls)`);
  const reqs = h.requests();
  check(reqs.length === 2 && reqs[0][K.cmd] === "list_workspace" && reqs[1][K.cmd] === "get_settings", "(c) …and accepted: the held calls go out, in call order");
  check(reqs.length === 2 && reqs[0][K.id] !== reqs[1][K.id], "each request has its own id");
  check(same(reqs[0]?.[K.args], {}), "a call with no args sends {} (wire.rs reads a missing args as {}, Tauri sends {})");
  h.reply(reqs[1]?.[K.id], { theme: "nord" });
  h.reply(reqs[0]?.[K.id], []);
  const [a, b] = await Promise.all([early, mid]);
  check(same(a, { ok: [] }) && same(b, { ok: { theme: "nord" } }), "…and each resolves with its own reply");
  h.event("places:changed", null);
  check(heard.length === 1, "a listen made before the handshake hears events once the server is proven");
  check(h.clk.pending() === 0, "the handshake timer is cleared once the server is proven");
  const h2 = harness();
  h2.open();
  check(h2.hello() !== null && h2.cn !== h.cn, "every connection says hello with a fresh nonce");
}

// 2b. (e) nothing but the handshake goes out until the server's proof verifies.
{
  let open;
  const gate = new Promise((r) => (open = r));
  const h = harness({ gate });
  const heard = [];
  await h.door.listen("sessions:busy", (e) => heard.push(e));
  const calls = [settle(h.door.invoke("list_workspace"))];
  h.open();
  h.hello();
  calls.push(settle(h.door.invoke("get_settings")));
  h.challenge();
  await h.proof();
  calls.push(settle(h.door.invoke("list_places", { repo: "/r" })));
  h.welcome();
  // serve adds a client to its broadcast set the moment it says ok, so an
  // event can be right behind the ok, inside the client's verify.
  h.event("sessions:busy", { busy: ["wt-a"], waiting: [] });
  calls.push(settle(h.door.invoke("term_write", { id: 1, data: "typed\r" })));
  await until(() => h.seen.verify > 0);
  await ticks();
  check(h.seen.verify === 1 && h.requests().length === 0,
    `(e) while the server's proof is being verified, nothing is sent: ${h.requests().length} of 4 queued requests went out (verify called ${h.seen.verify}×)`);
  check(heard.length === 0, "(e) …and an event right behind the ok is held, not delivered, until the proof verifies");
  open();
  await h.verified();
  const reqs = h.requests();
  check(same(reqs.map((r) => r[K.cmd]), ["list_workspace", "get_settings", "list_places", "term_write"]),
    `(e) once it verifies, every queued request goes out, in call order (${reqs.map((r) => r[K.cmd]).join(", ")})`);
  check(heard.length === 1 && heard[0].payload?.busy?.[0] === "wt-a", "(e) …and the held event is delivered");
}

// 2c. (a)(d) a server without the token is refused, and is sent nothing.
{
  const UNPROVEN = /did not prove it holds serve\.json's token/;
  const upToProof = async (h) => { h.open(); h.hello(); h.challenge(); return h.proof(); };
  const IMPOSTORS = [
    // The OWASP review's stand-in (owasp-serve/probes/impostor.py): it took the
    // port after serve died and answered the first frame with ok.
    ["(a)", "answers the hello with ok and no challenge (the review's stand-in)", (h) => { h.open(); h.hello(); h.recv({ [SPEC.ok]: true, [SPEC.version]: VERSION }); }],
    ["(a)", "challenges, then says ok with no proof", async (h) => { await upToProof(h); h.recv({ [SPEC.ok]: true, [SPEC.version]: VERSION }); }],
    ["(a)", "proves with a key that is not the token", async (h) => { await upToProof(h); h.welcome({ key: hex32() }); }],
    ["(a)", "reflects the client's own proof back", async (h) => { const p = await upToProof(h); h.welcome({ proof: p ?? hex32() }); }],
    ["(a)", "replays a good proof from another exchange", async (h) => { await upToProof(h); h.welcome({ proof: spectag(TOKEN, SPEC.serverLabel, hex32(), h.sn) }); }],
    ["(a)", "sends a proof that is not hex", async (h) => { await upToProof(h); h.welcome({ proof: "Z".repeat(64) }); }],
    ["(a)", "sends a challenge that is not 32 random bytes", (h) => { h.open(); h.hello(); h.challenge("abc"); }],
    ["(a)", "pushes an event before proving anything", (h) => { h.open(); h.hello(); h.event("places:changed", { forged: true }); }],
    // The spec has ONE spelling, and serve writes it (servefile.rs, policy.rs: `{x:02x}`).
    ["(a)", "sends its challenge in upper-case hex", (h) => { h.open(); h.hello(); h.challenge(hex32().toUpperCase()); }],
    ["(a)", "sends the right proof in upper-case hex", async (h) => { await upToProof(h); h.welcome({ proof: spectag(TOKEN, SPEC.serverLabel, h.cn, h.sn).toUpperCase() }); }],
    // A version is the server's word, and before the proof that word is anyone's.
    ["(d)", "claims another version, with no challenge", (h) => { h.open(); h.hello(); h.recv({ [SPEC.ok]: true, [SPEC.version]: "0.0.1" }); }],
    ["(d)", "claims another version behind a proof that fails", async (h) => { await upToProof(h); h.welcome({ version: "0.0.1", key: hex32() }); }],
  ];
  for (const [tag, label, act] of IMPOSTORS) {
    const h = harness();
    const heard = [];
    await h.door.listen("places:changed", (e) => heard.push(e));
    const p = settle(h.door.invoke("list_workspace"));
    const q = settle(h.door.invoke("term_write", { id: 1, data: "my-sudo-password\r" }));
    await act(h);
    await until(() => h.errors.length > 0 || h.requests().length > 0);
    await ticks();
    const [r, r2] = await Promise.all([p, q]);
    const said = String(r.err ?? JSON.stringify(r)).slice(0, 110);
    check(typeof r.err === "string" && UNPROVEN.test(r.err) && /stale/.test(r.err) && /impostor/.test(r.err) && !/version mismatch/.test(r.err),
      `${tag} a server that ${label} is refused as one that does not hold the token${tag === "(d)" ? ", NOT as a version mismatch" : ""} (${said})`);
    check(h.requests().length === 0 && !(h.sock?.sent ?? []).some(tokenIn),
      `${tag} …it was sent zero request frames and no token bytes (${h.requests().length} requests; frames: ${h.frames().map((f) => Object.keys(f).join("+")).join(", ")})`);
    check(!!h.sock?.closed && h.errors.length === 1 && r2.err === r.err && heard.length === 0,
      `${tag} …the socket is closed, the failure reported once, every waiting call refused, and nothing it sent delivered`);
  }
}

// 2d. (b) the token is never on the wire.
{
  const h = harness();
  await h.ready();
  settle(h.door.invoke("term_write", { id: 1, data: [104, 105] }));
  settle(h.door.invoke("get_settings"));
  const sent = h.sock?.sent ?? [];
  const leaked = sent.filter(tokenIn);
  check(sent.length >= 4 && leaked.length === 0,
    `(b) the token is in none of the ${sent.length} frames of a whole session, as text, base64 or hex${leaked.length ? ` — LEAKED in ${leaked[0].slice(0, 80)}` : ""}`);
}

// 2e. replies routed by id, out of order; err passed through unchanged.
{
  const h = harness();
  await h.ready();
  const p1 = settle(h.door.invoke("set_pin", { repo: "/r", slug: "s", on: true }));
  const p2 = settle(h.door.invoke("list_places", { repo: "/r" }));
  const p3 = settle(h.door.invoke("clone_project", { id: "j1", url: "nope", parent: "/tmp", name: null }));
  const p4 = settle(h.door.invoke("open_editor", { path: "/r/x", cmd: "code" }));
  const [r1, r2, r3, r4] = h.requests();
  check(same(r1?.[K.args], { repo: "/r", slug: "s", on: true }), "args go out exactly as the call site wrote them (camelCase, as Tauri sends)");
  const cloneErr = { kind: "auth", message: "Permission denied (publickey)", detail: null };
  h.error(r4?.[K.id], K.clientLocal);
  h.error(r3?.[K.id], cloneErr);
  h.reply(r2?.[K.id], [{ slug: "s" }]);
  h.error(r1?.[K.id], "no such place");
  const [o1, o2, o3, o4] = await Promise.all([p1, p2, p3, p4]);
  check(same(o2, { ok: [{ slug: "s" }] }), "a reply that arrives first resolves its own call");
  check(o1.err === "no such place", "a string err rejects with that string");
  check(typeof o3.err === "object" && same(o3.err, cloneErr), "clone_project's CloneError object rejects UNCHANGED — the dialog branches on its kind");
  check(o4.err === K.clientLocal, `a client-local command rejects with "${K.clientLocal}", unchanged`);
  check(h.warns.some((w) => w.includes("open_editor")), "…and the refusal is named for the developer (console)");
  check(h.errors.length === 0, "command errors are the caller's to show, not a transport failure");
}

// 2f. channels: seq order per channel, bytes vs typed, duplicates.
{
  const h = harness();
  await h.ready();
  const bytes = h.door.channel();
  const got = [];
  bytes.onmessage = (m) => got.push(m);
  const prog = h.door.channel();
  const progs = [];
  prog.onmessage = (m) => progs.push(m);
  settle(h.door.invoke("shell_open", { repo: "/r", slug: "s", index: 0, cols: 80, rows: 24, onBytes: bytes }));
  settle(h.door.invoke("clone_project", { id: "j", url: "u", parent: "/p", name: null, onProgress: prog }));
  const [open, clone] = h.requests();
  const bc = open?.[K.args].onBytes, pc = clone?.[K.args].onProgress;
  check(typeof bc === "string" && bc.startsWith(K.prefix) && typeof pc === "string" && pc.startsWith(K.prefix) && bc !== pc,
    `a Channel argument serialises as Tauri's "${K.prefix}<id>", one id per channel (${bc}, ${pc})`);
  const b = chanId(bc), p = chanId(pc);
  h.chan(b, K.firstSeq + 1, b64("world"));
  check(got.length === 0, "a frame that arrives AHEAD of its predecessor is held, not delivered");
  h.chan(p, K.firstSeq, { phase: "Receiving objects", percent: 10, detail: null });
  h.chan(b, K.firstSeq, b64("hello "));
  h.chan(b, K.firstSeq + 2, b64("!"));
  h.chan(b, K.firstSeq + 1, b64("DUPLICATE"));
  h.chan(p, K.firstSeq + 1, { phase: "Resolving deltas", percent: 100, detail: null });
  check(got.length === 3 && got.every((m) => m instanceof ArrayBuffer), "byte frames arrive as ArrayBuffers — what TerminalPane gets from Tauri");
  check(got.map(text).join("|") === "hello |world|!", `…in seq order, the reordered one released when its predecessor lands (${got.map(text).join("|")})`);
  check(progs.length === 2 && progs[0].percent === 10 && progs[1].percent === 100, "a typed progress channel delivers the JSON value itself, in its own seq order, unaffected by the other channel");
}

// 2g. the replay lands before the reply that answers it — and stays first.
{
  const h = harness();
  await h.ready();
  const order = [];
  const ch = h.door.channel();
  ch.onmessage = (m) => order.push(`bytes:${text(m)}`);
  const opened = settle(h.door.invoke("shell_open", { repo: "/r", slug: "s", index: 0, cols: 80, rows: 24, onBytes: ch }).then((v) => order.push(`reply:${v.replay}`)));
  const [req] = h.requests();
  const c = chanId(req?.[K.args].onBytes);
  // serve's one outbox: the replay is pushed through the sink BEFORE the reply.
  h.chan(c, K.firstSeq + 1, b64("live"));
  h.chan(c, K.firstSeq, b64("REPLAY"));
  h.reply(req?.[K.id], { gen: 1, replay: 6, replay_cols: null });
  await opened;
  check(same(order, ["bytes:REPLAY", "bytes:live", "reply:6"]), `the replay is the channel's first message even when a live frame overtakes it, and both precede the reply's continuation (${order.join(", ")})`);
}

// 2h. events: by name, unlisten.
{
  const h = harness();
  await h.ready();
  const busy = [], changed = [];
  const un = await h.door.listen("sessions:busy", (e) => busy.push(e));
  await h.door.listen("places:changed", (e) => changed.push(e));
  h.event("sessions:busy", { busy: ["wt-a"], waiting: [] });
  check(busy.length === 1 && busy[0].event === "sessions:busy" && same(busy[0].payload, { busy: ["wt-a"], waiting: [] }) && changed.length === 0,
    "an event frame reaches that event's listeners as {event, payload}, and only them");
  un();
  h.event("sessions:busy", { busy: [], waiting: [] });
  h.event("places:changed", null);
  check(busy.length === 1 && changed.length === 1, "unlisten stops delivery to that handler and no other");
}

// 2i. a version mismatch from a PROVEN server is refused, visibly.
for (const [label, v] of [["an older server", "0.0.1"], ["a server that sends no version", undefined]]) {
  const h = harness();
  const p = settle(h.door.invoke("list_workspace"));
  h.open(); h.hello(); h.challenge(); await h.proof();
  h.welcome({ version: v });
  await h.verified();
  const r = await p;
  const later = await settle(h.door.invoke("get_settings"));
  check(typeof r.err === "string" && /version mismatch/.test(r.err) && r.err.includes(VERSION) && (v === undefined || r.err.includes(v)),
    `${label}, proven: the waiting call is refused naming both versions (${r.err})`);
  check(h.requests().length === 0, `${label}: no request ever reaches it — only the handshake was written`);
  check(!!h.sock?.closed, `${label}: the client closes the socket`);
  check(h.errors.length === 1 && h.errors[0] === r.err, `${label}: reported once through onError → fail()`);
  check(later.err === r.err, `${label}: later calls fail the same way, at once`);
}

// 2j. refusals, a dead stack, a silent server — all surfaced, none swallowed.
{
  // serve answers anything but a hello by closing with nothing written, and a
  // serve older than handshake v1 reads a hello as exactly that.
  const h = harness();
  const p = settle(h.door.invoke("list_workspace"));
  h.open();
  h.drop();
  const r = await p;
  check(/older than handshake v1/.test(r.err ?? "") && h.errors.length === 1 && h.errors[0] === r.err, `a close at the hello says a serve older than handshake v1 does that (${r.err})`);
}
{
  const h = harness();
  const p = settle(h.door.invoke("list_workspace"));
  h.open(); h.hello(); h.challenge(); await h.proof();
  h.drop(); // serve's refusal of a wrong proof: the socket closes with nothing written
  const r = await p;
  check(/refused this client's proof/.test(r.err ?? "") && h.requests().length === 0 && h.errors.length === 1 && h.errors[0] === r.err,
    `a close after the client's proof is a REFUSAL of it, said so, once (${r.err})`);
}
{
  const h = harness();
  const p = settle(h.door.invoke("list_workspace"));
  h.drop(); // never opened
  const r = await p;
  check(/could not connect/.test(r.err ?? "") && h.errors.length === 1, `a socket that never opens says it could not connect (${r.err})`);
}
{
  const h = harness();
  const p = settle(h.door.invoke("list_workspace"));
  h.open(); h.hello(); h.challenge(); await h.proof();
  h.clk.advance(WS.AUTH_WAIT_MS - 1);
  check(h.errors.length === 0, "the handshake wait is not cut short");
  h.clk.advance(1);
  const r = await p;
  check(/did not answer the handshake/.test(r.err ?? "") && !!h.sock?.closed && h.errors.length === 1,
    `a server that goes quiet mid-handshake times out on the clock — the whole exchange counts — and is closed (${r.err})`);
}
{
  const h = harness();
  await h.ready();
  const p = settle(h.door.invoke("list_workspace"));
  h.drop();
  const r = await p;
  const later = await settle(h.door.invoke("get_settings"));
  check(/closed/.test(r.err ?? "") && later.err === r.err && h.errors.length === 1, `a socket that dies after the handshake fails its waiting call, every later one, and says so once (${r.err})`);
}
{
  const h = harness();
  await h.ready();
  const p = settle(h.door.invoke("list_workspace"));
  h.recv("{not json");
  const r = await p;
  check(/unreadable/.test(r.err ?? "") && h.errors.length === 1, "an unreadable frame ends the connection visibly rather than leaving a call to hang");
}
{
  // http://localhost is a secure context, a LAN address is not — and without
  // one there is no crypto.subtle, so no handshake: say so, dial nothing.
  let h = null, threw = null;
  try { h = harness({ subtle: null }); } catch (e) { threw = e; }
  const r = h ? await settle(h.door.invoke("list_workspace")) : {};
  check(h !== null && h.socks.length === 0 && /secure context/.test(r.err ?? "") && h.errors.length === 1 && h.errors[0] === r.err,
    `with no crypto.subtle the client opens no socket at all and says why (${threw ? `connectSocket THREW: ${threw}` : `${h.socks.length} sockets; ${r.err}`})`);
}

// 2j. `ready` — what the engine door waits on before it calls the engine up —
//     and the host's remedy, which replaces the dev stack's advice in the app.
{
  const h = harness();
  let r = "pending";
  h.door.ready.then(() => (r = "resolved"), (e) => (r = `rejected: ${e}`));
  h.open(); h.hello(); h.challenge(); await h.proof();
  await tick();
  check(r === "pending", `ready waits for the server's proof, not the socket's open or the client's own proof (${r})`);
  h.welcome(); await h.verified(); await tick();
  check(r === "resolved", `…and resolves once the proof has verified (${r})`);
  h.drop(); await tick();
  check(r === "resolved", "a socket that dies later does not un-resolve it — the door reports that through onError");
}
{
  const h = harness();
  let r = "pending";
  h.door.ready.then(() => (r = "resolved"), (e) => (r = `rejected: ${e}`));
  h.open(); h.hello(); h.challenge(); await h.proof();
  h.welcome({ key: "f".repeat(64) }); await h.verified(); await tick();
  check(r.startsWith("rejected: ") && r.slice(10) === h.errors[0], `a server that fails the proof rejects ready with the same failure onError got (${r.slice(0, 90)})`);
}
{
  const REMEDY = "Reconnect, and the app looks again.";
  const app = harness({ remedy: REMEDY });
  app.open(); app.hello(); app.challenge(); await app.proof();
  app.welcome({ key: "f".repeat(64) }); await app.verified();
  const stack = harness();
  stack.open(); stack.hello(); stack.challenge(); await stack.proof();
  stack.welcome({ key: "f".repeat(64) }); await stack.verified();
  check(app.errors[0]?.endsWith(REMEDY) && !/server-stack\.sh|restart the stack/i.test(app.errors[0]) && /server-stack\.sh/.test(stack.errors[0] ?? ""),
    `each host states its own remedy: the app's Reconnect, the dev stack's restart (app: …${(app.errors[0] ?? "").slice(-60)})`);
}

// 2k. the dev boot: one POST to the token endpoint, then the literal 127.0.0.1.
{
  const socks = [];
  class FakeSock { constructor(url) { this.url = url; this.sent = []; socks.push(this); } send(t) { this.sent.push(t); WIRE_LOG.push(t); } close() {} }
  const asked = [];
  await WS.bootSocket({
    version: VERSION, crypto: webcrypto().crypto, WebSocket: FakeSock, setTimeout: () => 0, clearTimeout: () => {}, atob: globalThis.atob,
    onError: () => {},
    fetch: async (u, init) => { asked.push([u, init]); return { ok: true, status: 200, text: async () => JSON.stringify({ port: 5151, token: TOKEN }) }; },
  });
  check(socks.length === 1 && socks[0].url === `ws://${SPEC.host}:5151/`, `the dev boot dials ws://${SPEC.host}:<port>/, never localhost (${socks[0]?.url})`);
  check(asked.length === 1 && asked[0][0] === WS.SERVE_ENDPOINT && asked[0][1].method === "POST" && !String(socks[0]?.url).includes(TOKEN),
    "…after one POST to the token endpoint, with nothing but the port in the URL");
}

// 2l. a channel's end releases its handler — what Tauri does when its Rust
//     `Channel` drops — and an `err` to the request that named it does too.
{
  const F = K.firstSeq;
  const shellOpen = (h, ch) => settle(h.door.invoke("shell_open", { repo: "/r", slug: "s", index: 0, cols: 80, rows: 24, onBytes: ch }));
  const notOpen = (h) => h.warns.filter((w) => /channel \d+/.test(w));
  {
    // A dock shell that exits: its bytes, its end, then `shell:exit` (serve
    // sends the end frame first, on the same queue).
    const h = harness();
    await h.ready();
    const got = [];
    const ch = h.door.channel();
    ch.onmessage = (m) => got.push(text(m));
    let atExit = null;
    await h.door.listen("shell:exit", () => (atExit = h.openChannels()));
    const opened = shellOpen(h, ch);
    const [req] = h.requests();
    const c = chanId(req?.[K.args].onBytes);
    h.reply(req?.[K.id], { gen: 1, replay: 0, replay_cols: null });
    await opened;
    check(h.openChannels() === 1, `an ok reply does not end the channel its request named — a shell's bytes keep coming after shell_open answers (${h.openChannels()} open)`);
    h.chan(c, F, b64("a"));
    h.chan(c, F + 1, b64("b"));
    h.end(c, F + 2);
    h.event("shell:exit", { repo: "/r", slug: "s", index: 0 });
    check(same(got, ["a", "b"]) && h.openChannels() === 0 && atExit === 0,
      `an end frame releases the channel after its last byte, before the shell:exit behind it (delivered ${JSON.stringify(got)}; ${h.openChannels()} open; ${atExit} at shell:exit)`);
    h.chan(c, F + 3, b64("late"));
    check(same(got, ["a", "b"]) && notOpen(h).length === 1, `a frame for an ended channel is not delivered, and is named for the developer (${JSON.stringify(got)}; ${JSON.stringify(notOpen(h))})`);
  }
  {
    // A tmux attach whose end overtakes its data (serve never does that; the
    // client orders by seq all the same, as Tauri's Channel orders by index).
    const h = harness();
    await h.ready();
    const got = [];
    const ch = h.door.channel();
    ch.onmessage = (m) => got.push(text(m));
    settle(h.door.invoke("term_open", { session: "wt-a", cols: 80, rows: 24, onBytes: ch }));
    const [req] = h.requests();
    const c = chanId(req?.[K.args].onBytes);
    h.reply(req?.[K.id], 7);
    h.end(c, F + 2);
    h.chan(c, F + 1, b64("y"));
    check(got.length === 0 && h.openChannels() === 1, `an end that arrives ahead of data is held: the channel stays open while a frame before it is missing (${h.openChannels()} open)`);
    h.chan(c, F, b64("x"));
    check(same(got, ["x", "y"]) && h.openChannels() === 0, `…and is released the moment the frames before it have been delivered (${JSON.stringify(got)}; ${h.openChannels()} open)`);
  }
  {
    // clone_project: progress, its end, then the reply (serve: end precedes it).
    const h = harness();
    await h.ready();
    const progs = [];
    const prog = h.door.channel();
    prog.onmessage = (m) => progs.push(m);
    let atReply = null;
    const done = settle(h.door.invoke("clone_project", { id: "j", url: "u", parent: "/p", name: null, onProgress: prog }).then((v) => ((atReply = h.openChannels()), v)));
    const [req] = h.requests();
    const p = chanId(req?.[K.args].onProgress);
    h.chan(p, F, { phase: "Receiving objects", percent: 40, detail: null });
    h.end(p, F + 1);
    h.reply(req?.[K.id], "/p/u");
    const r = await done;
    check(same(r, { ok: "/p/u" }) && progs.length === 1 && progs[0]?.percent === 40 && atReply === 0,
      `a progress channel ends before its operation's reply: the handler got its one value and NOT the end (${JSON.stringify(progs)}), and is gone when the call resolves (${atReply} open)`);
  }
  {
    // The err rule: a request refused on its arguments made no sink, so no
    // end frame comes — the err alone ends the channel it named.
    const h = harness();
    await h.ready();
    const ch = h.door.channel();
    ch.onmessage = () => {};
    const opened = shellOpen(h, ch);
    const [req] = h.requests();
    h.error(req?.[K.id], "invalid args `repo` for command `shell_open`: missing field");
    const r = await opened;
    check(typeof r.err === "string" && h.openChannels() === 0, `an err to the request that named a channel ends it, with no end frame (serve made no sink) — ${h.openChannels()} open`);
    // …and one with a sink: end frame, then the err — released once, quietly.
    const prog = h.door.channel();
    prog.onmessage = () => {};
    const cloned = settle(h.door.invoke("clone_project", { id: "j", url: "u", parent: "/p", name: null, onProgress: prog }));
    const req2 = h.requests()[1];
    h.end(chanId(req2?.[K.args].onProgress), F);
    h.error(req2?.[K.id], { kind: "auth", message: "denied", detail: null });
    const r2 = await cloned;
    check(typeof r2.err === "object" && h.openChannels() === 0 && notOpen(h).length === 0, `an end frame then the err: the channel is released once, with nothing to complain about (${JSON.stringify(notOpen(h))})`);
    // An err to a request that named no channel leaves the open ones alone.
    const live = h.door.channel();
    live.onmessage = () => {};
    const o = shellOpen(h, live);
    h.reply(h.requests()[2]?.[K.id], { gen: 1, replay: 0, replay_cols: null });
    await o;
    const g = settle(h.door.invoke("get_settings"));
    h.error(h.requests()[3]?.[K.id], "nope");
    await g;
    check(h.openChannels() === 1, `an err to a request that named no channel ends none (${h.openChannels()} open)`);
  }
  {
    // A closed connection can deliver nothing again: every handler goes.
    const h = harness();
    await h.ready();
    for (const k of [0, 1]) {
      const ch = h.door.channel();
      ch.onmessage = () => {};
      shellOpen(h, ch);
      h.reply(h.requests()[k]?.[K.id], { gen: 1, replay: 0, replay_cols: null });
    }
    await ticks();
    const before = h.openChannels();
    h.drop();
    check(before === 2 && h.openChannels() === 0, `a closed connection releases every channel it held (${before} → ${h.openChannels()})`);
  }
}

// 2m. what the client writes before it is in, by serve's rules: ONE key a
//     frame, lowercase hex, and all of it inside the pre-proof byte budget.
{
  const h = harness();
  settle(h.door.invoke("write_file", { path: "/r/f", content: "q".repeat(4096), expectedMtime: null })); // queued behind the handshake
  await h.ready();
  const all = h.sock?.sent ?? [];
  const hs = all.filter((t) => !(K.cmd in JSON.parse(t)));
  const vals = hs.map((t) => Object.values(JSON.parse(t)));
  check(hs.length === 2 && hs.every((t) => Object.keys(JSON.parse(t)).length === 1),
    `each handshake frame the client writes has exactly one key — serve refuses a frame with more (${hs.map((t) => Object.keys(JSON.parse(t)).join("+")).join(", ")})`);
  check(vals.length === 2 && vals.every((v) => v.length === 1 && HEX32.test(String(v[0]))), "…and its value is 64 LOWERCASE hex: serve refuses upper case rather than fold it");
  // A client frame is masked: 2 bytes of header, 4 of mask, 2 more past 125.
  const onWire = (t) => { const n = Buffer.byteLength(t); return n + 6 + (n > 125 ? 2 : 0) + (n > 65535 ? 6 : 0); };
  const pre = hs.reduce((a, t) => a + onWire(t), 0);
  check(pre <= K.preauth && all.length === 3,
    `the client's whole output before serve's proof verifies is ${pre} bytes on the wire, inside serve's PREAUTH_BYTES (${K.preauth}); the 4 KiB request queued behind it went only after (${all.length} frames)`);
}

// 2n. a request bigger than serve reads is refused HERE: serve would end the
//     whole connection over it, and every terminal on it with the one save.
{
  const h = harness();
  await h.ready();
  const sent = () => h.requests().length;
  const over = await settle(h.door.invoke("write_file", { path: "/r/big", content: "x".repeat(K.maxMessage), expectedMtime: null }));
  check(typeof over.err === "string" && over.err.includes(`${K.maxMessage / 2 ** 20} MiB`) && sent() === 0,
    `a request over serve's MAX_MESSAGE is refused without being sent, naming the limit (${String(over.err ?? JSON.stringify(over)).slice(0, 120)})`);
  // Counted in UTF-8 bytes, as serve counts them: 0.6× the limit in "é" is 1.2× on the wire.
  const wide = await settle(h.door.invoke("write_file", { path: "/r/wide", content: "é".repeat(Math.ceil(K.maxMessage * 0.6)), expectedMtime: null }));
  check(typeof wide.err === "string" && sent() === 0, "…counted in UTF-8 bytes, as serve counts them, not in string length (0.6× the limit in \"é\" is 1.2× on the wire)");
  const ch = h.door.channel();
  const withChan = await settle(h.door.invoke("shell_open", { repo: "/r", slug: "s", index: 0, cols: 80, rows: 24, onBytes: ch, pad: "x".repeat(K.maxMessage) }));
  check(typeof withChan.err === "string" && h.openChannels() === 0, `…and a channel it named is not held open for frames that cannot come (${h.openChannels()} open)`);
  const fits = settle(h.door.invoke("write_file", { path: "/r/ok", content: "x".repeat(K.maxMessage - 4096), expectedMtime: null }));
  check(sent() === 1 && !h.sock?.closed && h.errors.length === 0,
    "a request under the limit goes out, and a refused one was not a transport failure: the connection stays up, nothing reported");
  h.reply(h.requests()[0]?.[K.id], null);
  await fits;
}

check(THROWN.length === 0, `the client never threw out of a socket callback (${THROWN.length}${THROWN.length ? `: ${THROWN[0]}` : ""})`);

// ── 3. ipc.ts, and the termreplay contract over the socket ──────────────────

const ipcSrc = read(SRC.ipc);
for (const want of ['from "@tauri-apps/api/core"', 'from "@tauri-apps/api/event"']) {
  check(ipcSrc.includes(want), `ipc.ts imports ${want.slice(5)} (it is the door's Tauri half)`);
}
check(/^import \{ retrier, [^}]*\} from "\.\/reconnect";$/m.test(ipcSrc), "ipc.ts takes its retries from reconnect.ts — the module inlined below");
const unimported = (src) => src.split("\n").filter((l) => !/^\s*import\s/.test(l)).join("\n");
/** ipc.ts with its Tauri imports replaced by recording stubs, and its one
 *  relative import — reconnect.ts — inlined as the REAL source, exports
 *  dropped: a `data:` module cannot resolve `./reconnect`, and a stub would be
 *  a second answer to the rules reconnect-check.mjs holds. */
async function loadIpc() {
  const machine = unimported(read(SRC.reconnect)).replace(/^export (function|const|type) /gm, "$1 ");
  const js = (await transformWithEsbuild(
    // A fresh module per load (the comment changes the data: URL): each caller
    // gets its own door and its own stubs.
    `// ${Math.random()}\nconst { tauriInvoke, tauriListen, TauriChannel } = globalThis.__ipcStubs;\n${machine}\n${unimported(ipcSrc)}`,
    "ipc.ts", { loader: "ts", format: "esm" })).code;
  return load(SRC.ipc, js);
}
{
  const calls = [];
  class TauriChannel { constructor(m) { this.tauri = true; this.onmessage = m; } toJSON() { return "__CHANNEL__:t"; } }
  globalThis.__ipcStubs = {
    tauriInvoke: (...a) => { calls.push(["invoke", ...a]); return Promise.resolve("tauri"); },
    tauriListen: (...a) => { calls.push(["listen", ...a]); return Promise.resolve(() => {}); },
    TauriChannel,
  };
  const ipc = await loadIpc();
  const ch = new ipc.Channel();
  const args = { session: "wt-a", cols: 80, rows: 24, onBytes: ch };
  const v = await ipc.invoke("term_open", args);
  await ipc.invoke("list_workspace");
  const handler = () => {};
  await ipc.listen("places:changed", handler);
  check(ch instanceof TauriChannel, "under Tauri, `new Channel()` IS Tauri's Channel");
  check(v === "tauri" && calls[0][1] === "term_open" && calls[0][2] === args && calls[0].length === 3,
    "under Tauri, invoke hands Tauri the same command and the SAME args object, nothing added");
  check(calls[1][1] === "list_workspace" && calls[1][2] === undefined, "…and an arg-less call stays arg-less (Tauri's own default applies)");
  check(calls[2][1] === "places:changed" && calls[2][2] === handler, "under Tauri, listen hands Tauri the same event and handler");
  check(ipc.engineState().kind === "up" && ipc.engineState().gen === 0, "with no engine door, the engine reads up at gen 0 — the mock and plain Tauri are never gated");
}

// The engine door: the shipped app's (with Tauri as its local half) and the dev
// stack's (with none), over real ipcws clients on fake sockets, its retries on
// a virtual clock. reconnect.ts's own rules are reconnect-check.mjs's; here it
// is that the door feeds them what the shim said and does what they decide.
{
  globalThis.__ipcStubs = { tauriInvoke: () => Promise.reject("the Tauri stub must not be reached"), tauriListen: () => Promise.reject("no"), TauriChannel: class {} };
  const ipc = await loadIpc();
  const local = [];
  const tauri = {
    invoke: (cmd, args) => { local.push([cmd, args]); return Promise.resolve(`local:${cmd}`); },
    listen: () => Promise.reject("the local door has no events"),
    channel: () => { throw new Error("the local door makes no channels"); },
  };
  // What the next dials do: undefined (an honest serve, the app's), "unowned"
  // (honest, someone else's), "impostor" (fails the proof), "phase" (reports
  // the shim's phase first), "throw" (a string), or an object — the shim's
  // ConnectError — to reject with.
  const script = [];
  const dials = [];
  const dclk = clock();
  let seen = true;
  const env = { now: dclk.now, setTimeout: dclk.setTimeout, clearTimeout: dclk.clearTimeout, random: () => 0, visible: () => seen };
  const door = ipc.engineDoor(tauri, async (ask) => {
    const how = script.shift();
    const d = { reason: ask.reason, start: ask.start, manual: ask.manual, subs: [] };
    dials.push(d);
    if (how === "throw") throw "engine_connect: no serve binary";
    if (how && typeof how === "object") throw how;
    if (how === "phase") ask.onPhase("waiting-old");
    const h = harness({ onError: ask.onLost });
    d.h = h;
    // Every subscription this socket is asked for, by name.
    const listen = h.door.listen;
    h.door.listen = (name, fn) => { d.subs.push(name); return listen(name, fn); };
    void (async () => {
      h.open(); h.hello(); h.challenge(); await h.proof();
      h.welcome(how === "impostor" ? { key: "f".repeat(64) } : {}); await h.verified();
    })();
    return { sock: h.door, owned: how !== "unowned", started: false };
  }, env);
  ipc.openDoor(door);
  const st = () => ipc.engineState();
  const isUp = () => until(() => st().kind === "up");
  check(st().kind === "starting" && st().gen === 0, `an engine door starts out "starting" at gen 0, before anything has dialled — the gate holds the app back (${JSON.stringify(st())})`);
  const early = await settle(ipc.invoke("list_workspace"));
  check(typeof early.err === "string" && dials.length === 0, `a call before the first connect is refused, not queued and not dialled (${early.err})`);

  await settle(door.connect());
  await isUp();
  const h1 = dials[0]?.h;
  check(st().kind === "up" && st().gen === 1 && dials.length === 1 && dials[0].reason === null && dials[0].start && !dials[0].manual,
    `the launch dials once, with no reason, allowed to start, and the engine is up at gen 1 (${JSON.stringify(st())})`);

  const copied = (await settle(ipc.invoke("copy_text", { text: "x" }))).ok;
  const raised = (await settle(ipc.invoke("raise_window"))).ok;
  check(copied === "local:copy_text" && raised === "local:raise_window" && local.length === 2 && local[0][1].text === "x" && !h1?.requests().length,
    "client-local and shim commands go to the local door with their args, and not one byte of them to the socket");
  const listed = settle(ipc.invoke("list_workspace"));
  await tick();
  const req = h1?.requests().find((r) => r[K.cmd] === "list_workspace");
  h1?.reply(req?.[K.id], { projects: [] });
  check(same((await listed).ok, { projects: [] }) && local.length === 2, "every other command goes to the socket and is answered from it");
  const ch = ipc.Channel ? new ipc.Channel() : null;
  check(ch && typeof ch.toJSON === "function" && String(ch.toJSON()).startsWith(K.prefix), `a channel is the current socket's (${ch?.toJSON?.()})`);

  const heard = [];
  await settle(ipc.listen("places:changed", (e) => heard.push(["a", e.payload])));
  h1?.event("places:changed", 1);
  check(same(heard, [["a", 1]]), "a listener hears the engine's events");

  h1?.drop();
  await tick();
  const lost = st();
  check(lost.kind === "waiting" && lost.gen === 1 && /closed/.test(lost.reason) && lost.at === dclk.now() + 500,
    `a socket that dies leaves the engine waiting to retry in 0.5 s, keeping gen and naming why (${JSON.stringify(lost)})`);
  check(dials.length === 1, "…and nothing is dialled at once");
  const whileDown = await settle(ipc.invoke("get_settings"));
  check(whileDown.err === lost.reason, `while it waits, every engine call fails with that reason (${whileDown.err})`);
  const deadCh = new ipc.Channel();
  check(typeof deadCh.toJSON() === "string", "…and a channel can still be made — it just never hears anything");
  check((await settle(ipc.invoke("copy_text", { text: "y" }))).ok === "local:copy_text", "…while client-local commands still work: the window is still here");

  dclk.advance(500);
  check(dials.length === 2 && dials[1].start && !dials[1].manual && dials[1].reason === `automatic retry 1: ${lost.reason}`,
    `0.5 s later the door redials ON ITS OWN, telling the shim why and that it is a retry (${dials[1]?.reason?.slice(0, 60)})`);
  await isUp();
  const h2 = dials[1]?.h;
  check(st().kind === "up" && st().gen === 2, `…and the engine is up at gen 2 (${JSON.stringify(st())})`);
  await settle(ipc.listen("sessions:busy", (e) => heard.push(["b", e.payload])));
  h2?.event("places:changed", 2);
  h2?.event("sessions:busy", 3);
  check(same(heard.slice(1), [["a", 2], ["b", 3]]), `a listener from before the reconnect hears the NEW socket's events, beside one added after (${JSON.stringify(heard)})`);

  // The reconnect remounts the panes, and a tmux session that died while the
  // engine was down is refused by term_open with an {id, err}. That is the
  // pane's error, not the engine's: no retry, no dial (so no start), and the
  // socket stays up for every other pane.
  {
    const chans = h2?.openChannels();
    const opened = settle(ipc.invoke("term_open", { session: "wt-gone", cols: 80, rows: 24, onBytes: new ipc.Channel() }));
    await tick();
    const r = h2?.requests().find((q) => q[K.cmd] === "term_open");
    check(h2?.openChannels() === chans + 1, `a remounted pane's term_open goes out on the new socket, naming its channel (${h2?.openChannels()} open)`);
    h2?.error(r?.[K.id], "no tmux session named wt-gone");
    const res = await opened;
    await ticks();
    check(res.err === "no tmux session named wt-gone", `term_open's refusal reaches the pane in serve's words (${JSON.stringify(res)})`);
    check(st().kind === "up" && st().gen === 2 && dials.length === 2 && dclk.pending() === 0 && !h2?.sock.closed,
      `…and is not an engine failure: still up at gen 2, no dial, no retry timer, the socket open (${JSON.stringify(st())}; ${dials.length} dials, ${dclk.pending()} timers)`);
    const next = settle(ipc.invoke("list_workspace"));
    await tick();
    const r2 = h2?.requests().filter((q) => q[K.cmd] === "list_workspace").at(-1);
    h2?.reply(r2?.[K.id], { projects: [] });
    check(same((await next).ok, { projects: [] }) && h2?.openChannels() === chans,
      `the same socket answers the next call, and the refused attach's channel is let go (${h2?.openChannels()} open, ${chans} before)`);
  }

  // Two Reconnect presses are one dial, at once, cancelling the timer.
  h2?.drop(); await tick();
  const p1 = ipc.reconnectEngine(), p2 = ipc.reconnectEngine();
  check(dials.length === 3 && dials[2].manual && dials[2].start && dials[2].reason === st().reason && dclk.pending() === 0,
    `Reconnect now dials at once — manual, allowed to start, the timer gone (${dials.length - 2} dial(s), ${dclk.pending()} timer(s))`);
  await settle(Promise.all([p1, p2]));
  await isUp();
  check(dials.length === 3 && st().gen === 3, `two presses at once are one dial (${dials.length - 2}; gen ${st().gen})`);

  // Failures keep it retrying, in the shim's words, on the backoff.
  dials[2]?.h?.drop(); await tick();
  script.push("throw");
  dclk.advance(500); await ticks();
  check(st().kind === "waiting" && st().gen === 3 && st().reason === "engine_connect: no serve binary" && st().at === dclk.now() + 1000,
    `a dial that throws leaves the engine waiting, in the shim's words, 1 s further on (${JSON.stringify(st())})`);
  script.push({ message: "worktrees serve exited before it was ready (exit status: 1)", started: true, declined: false });
  dclk.advance(1000); await ticks();
  check(st().kind === "waiting" && st().reason === "worktrees serve exited before it was ready (exit status: 1)",
    `the shim's ConnectError is read by its fields, not stringified (${st().reason})`);
  script.push("impostor");
  dclk.advance(2000);
  await until(() => st().kind === "waiting" && /did not prove/.test(st().reason));
  check(st().kind === "waiting" && /did not prove/.test(st().reason), `a server that fails the proof is never driven, and the retries go on (${st().reason?.slice(0, 80)})`);
  script.push("phase");
  dclk.advance(4000);
  check(st().kind === "starting" && st().phase === "waiting-old" && !st().manual, `the shim's phase reaches the engine state while its dial is in flight (${JSON.stringify(st())})`);
  await isUp();
  check(st().kind === "up" && st().gen === 4, `…and the next good dial brings it back at gen 4 (${JSON.stringify(st())})`);

  // Five reconnects later: every event name subscribed ONCE per socket, and
  // every handler hears each event once — including a listen made, and an
  // unlisten made, while the engine was down.
  const before = dials.length;
  const counts = { a: 0, b: 0, c: 0, gone: 0, late: 0 };
  await settle(ipc.listen("places:changed", () => counts.c++));
  const unGone = (await settle(ipc.listen("sessions:busy", () => counts.gone++))).ok;
  for (let i = 0; i < 5; i++) {
    dials[dials.length - 1].h.drop(); await tick();
    if (i === 1) await settle(ipc.listen("shell:exit", () => counts.late++));
    if (i === 2) unGone?.();
    dclk.advance(500);
    await isUp();
  }
  const made = dials.slice(before);
  const names = ["places:changed", "sessions:busy"];
  const subsOk = made.every((d, i) => same([...d.subs].sort(), (i >= 1 ? [...names, "shell:exit"] : names).sort()));
  check(made.length === 5 && subsOk, `5 automatic reconnects: each new socket is subscribed to each event name exactly once (${made.map((d) => d.subs.length).join(",")} subscriptions)`);
  const last = made[made.length - 1].h;
  const tally = heard.length;
  last.event("places:changed", 9);
  last.event("sessions:busy", 9);
  last.event("shell:exit", 9);
  check(heard.length === tally + 2 && counts.c === 1 && counts.late === 1 && counts.gone === 0,
    `…and each handler hears one event once — the old ones, one listened while down, none unlistened while down (a/b +${heard.length - tally}, c ${counts.c}, late ${counts.late}, gone ${counts.gone})`);
  made[0].h.event("places:changed", 10);
  dials[0].h.event("places:changed", 10);
  check(counts.c === 1 && heard.length === tally + 2, "a dead socket's late event reaches nobody");
  check(st().gen === 9, `gen counted every connection (${st().gen})`);

  // A hidden window defers the retry until it is seen.
  seen = false;
  last.drop(); await tick();
  dclk.advance(500);
  check(st().kind === "hidden" && dials.length === before + 5, `a retry due while the window is hidden waits (${st().kind}, ${dials.length - before - 5} dial(s))`);
  seen = true;
  door.visibility();
  check(dials.length === before + 6 && !dials[dials.length - 1].manual, "being seen again dials at once");
  await isUp();

  // Someone else's serve is reconnected to, never restarted.
  dials[dials.length - 1].h.drop(); await tick();
  script.push("unowned");
  dclk.advance(500); await isUp();
  dials[dials.length - 1].h.drop(); await tick();
  script.push({ message: "nothing to reconnect to: no serve.json", started: false, declined: true });
  dclk.advance(500); await ticks();
  check(!dials[dials.length - 1].start && st().kind === "waiting" && st().notOurs, `after someone else's serve, the retry may not start one, and says whose it was (${JSON.stringify(st())})`);
  dclk.advance(1000); await isUp();

  // The crash-loop limit, through the door: the shim's `started` is counted.
  // A minute up first, so the ConnectError above that said `started` has aged
  // out of the window and the three below are the only starts in it.
  dclk.advance(61_000);
  dials[dials.length - 1].h.drop(); await tick();
  const exits = { message: "worktrees serve exited before it was ready (exit status: 1)", started: true, declined: false };
  script.push(exits, exits, exits);
  for (const ms of [500, 1000, 2000]) { dclk.advance(ms); await ticks(); }
  script.push({ message: "nothing to reconnect to: serve.json names pid 7, which is not running", started: false, declined: true });
  dclk.advance(4000); await ticks();
  check(!dials[dials.length - 1].start && st().kind === "down" && /3 times in the last 60 s/.test(st().reason) && dclk.pending() === 0,
    `three starts the shim reported, then a declined try: the retries STOP, with no timer left (${st().reason?.slice(0, 90)})`);
  await settle(ipc.reconnectEngine());
  await isUp();
  check(dials[dials.length - 1].manual && dials[dials.length - 1].start && st().kind === "up", "a press after the stop dials, may start, and brings it back");
}
{
  // The dev stack's door has no local half: a browser has no Tauri, so even a
  // client-local command goes to serve — which refuses it.
  globalThis.__ipcStubs = { tauriInvoke: () => Promise.reject("tauri must not be reached"), tauriListen: () => Promise.reject("no"), TauriChannel: class {} };
  const ipc = await loadIpc();
  let h = null;
  const dclk = clock();
  const door = ipc.engineDoor(null, async ({ onLost }) => {
    h = harness({ onError: onLost });
    void (async () => { h.open(); h.hello(); h.challenge(); await h.proof(); h.welcome(); await h.verified(); })();
    return { sock: h.door, owned: false, started: false };
  }, { now: dclk.now, setTimeout: dclk.setTimeout, clearTimeout: dclk.clearTimeout, random: () => 0, visible: () => true });
  await settle(door.connect());
  await until(() => ipc.engineState().kind === "up");
  const p = settle(door.invoke("copy_text", { text: "x" }));
  await tick();
  const req = h?.requests().find((r) => r[K.cmd] === "copy_text");
  h?.error(req?.[K.id], K.clientLocal);
  check(!!req && (await p).err === K.clientLocal, "with no local half (the dev stack), a client-local command goes to serve and comes back refused");
}

// The full chain: TerminalPane → ipc.ts → ipcws.ts → the fake socket.
{
  const QUERY = "\x1b[>c", REPLY = "\x1b[>0;276;0c";
  const raw = read(SRC.pane);
  const body = raw.split("\n").filter((l) => !/^\s*import\s/.test(l)).join("\n").replace(/^export (function|const|type|class) /gm, "$1 ");
  const js = (await transformWithEsbuild(`
export function build(env: any) {
  const {
    useCallback, useEffect, useRef, useState,
    Terminal, FitAddon, SearchAddon, UnicodeGraphemesAddon, Channel, invoke,
    FindBar, findColors, window, document, getComputedStyle, performance,
    requestAnimationFrame, cancelAnimationFrame, setTimeout, clearTimeout, TextEncoder, h, F,
  } = env;
${body}
  return { useTerm, shellTransport };
}`, "transport-pane.tsx", { loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F" })).code;
  const { build } = await load(SRC.pane, js);

  async function run(liveFirst) {
    const h = harness();
    globalThis.__ipcStubs = { tauriInvoke: () => Promise.reject("tauri must not be reached"), tauriListen: () => Promise.reject("no"), TauriChannel: class {} };
    const ipc = await loadIpc();
    ipc.openDoor(h.door);
    await h.ready();
    let term = null;
    class TerminalStub {
      constructor(o) { this.options = { ...o }; this.unicode = { activeVersion: "" }; this.cols = 100; this.rows = 40; this.element = {}; this.queue = []; this.listener = null; term = this; }
      loadAddon(a) { a.activate?.(this); } open() {} writeln() {} focus() {} dispose() {}
      resize(c, r) { this.cols = c; this.rows = r; }
      onData(fn) { this.listener = fn; }
      write(bytes, cb) { this.queue.push({ bytes, cb }); }
      parseNext() { const w = this.queue.shift(); if (!w) throw new Error("nothing queued"); if (new TextDecoder().decode(w.bytes).includes(QUERY)) this.listener?.(REPLY); w.cb?.(); }
    }
    const effects = [];
    const env = {
      useRef: (i) => ({ current: i }), useState: (i) => [typeof i === "function" ? i() : i, () => {}],
      useCallback: (f) => f, useEffect: (cb) => { effects.push(cb); },
      Terminal: TerminalStub, FitAddon: class { activate() {} dispose() {} fit() {} },
      SearchAddon: class { activate() {} dispose() {} onDidChangeResults() { return { dispose() {} }; } clearDecorations() {} findNext() {} findPrevious() {} },
      UnicodeGraphemesAddon: class { activate() {} dispose() {} },
      Channel: ipc.Channel, invoke: ipc.invoke,
      FindBar: () => null, findColors: () => ({}), h: () => null, F: null,
      window: { addEventListener() {}, removeEventListener() {} },
      document: { addEventListener() {}, removeEventListener() {}, documentElement: {}, visibilityState: "visible", hasFocus: () => true },
      getComputedStyle: () => ({ getPropertyValue: () => "" }), performance: { now: () => 0 },
      requestAnimationFrame: (cb) => setTimeout(cb, 0), cancelAnimationFrame: (id) => clearTimeout(id),
      setTimeout, clearTimeout, TextEncoder,
    };
    const prevRO = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class { observe() {} disconnect() {} };
    const { useTerm, shellTransport } = build(env);
    const api = useTerm(() => shellTransport("/r", "s", 0), "shell|/r|s|0", 0, 0, true);
    api.hostRef.current = { clientWidth: 800, clientHeight: 600 };
    const cleanups = effects.map((cb) => cb()).filter((c) => typeof c === "function");
    await ticks();
    const open = h.requests().find((r) => r[K.cmd] === "shell_open");
    if (!open) { fail("TerminalPane's shellTransport never sent shell_open over the socket"); return; }
    const c = chanId(open[K.args].onBytes);
    const RING = `~/x » vim notes.md\r\n${QUERY}\r\n~/x » `;
    if (liveFirst) h.chan(c, K.firstSeq + 1, b64(QUERY));
    h.chan(c, K.firstSeq, b64(RING));
    h.reply(open[K.id], { gen: 1, replay: RING.length, replay_cols: null });
    if (!liveFirst) h.chan(c, K.firstSeq + 1, b64(QUERY));
    await ticks();
    const writes = () => h.requests().filter((r) => r[K.cmd] === "shell_write").map((r) => new TextDecoder().decode(new Uint8Array(r[K.args].data)));
    term.parseNext();
    const afterReplay = writes().length;
    term.parseNext();
    const all = writes();
    cleanups.forEach((f) => f());
    globalThis.ResizeObserver = prevRO;
    const how = liveFirst ? "a live frame overtaking the replay on the wire" : "in order";
    check(afterReplay === 0, `over the socket (${how}): the replay's query is NOT answered (${afterReplay} shell_write)`);
    check(all.length === 1 && all[0] === REPLY, `over the socket (${how}): the live query behind it IS answered, down shell_write (${JSON.stringify(all)})`);
  }
  await run(false);
  await run(true);
}

// The token never went out, from any client this run made.
{
  const leaked = WIRE_LOG.filter(tokenIn);
  check(WIRE_LOG.length > 50 && leaked.length === 0, `(b) none of the ${WIRE_LOG.length} frames written by every client in this run carries the token${leaked.length ? ` — ${leaked.length} DID, e.g. ${leaked[0].slice(0, 80)}` : ""}`);
}

// ── 4. the edges ────────────────────────────────────────────────────────────

// One door.
{
  const offenders = [];
  let doorUsers = 0;
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) { if (e.name !== "mock") walk(p); continue; }
      if (!/\.(ts|tsx)$/.test(e.name) || p === SRC.ipcInTree) continue;
      const s = read(p);
      // Code only: a template-string fixture or a comment may name the module.
      const code = s.split("\n").filter((l) => /^\s*(import|export)\b/.test(l) || /\bimport\(/.test(l));
      if (code.some((l) => /["']@tauri-apps\/api\/(core|event)["']/.test(l))) offenders.push(path.relative(SRC.appsrc, p));
      if (code.some((l) => /from "\.\/ipc"/.test(l))) doorUsers++;
    }
  };
  walk(SRC.appsrc);
  check(offenders.length === 0, `nothing under app/src but ipc.ts (and mock/) imports @tauri-apps/api/core or /event${offenders.length ? `: ${offenders.join(", ")}` : ""}`);
  check(doorUsers >= 25, `the call sites go through the door (${doorUsers} files import ./ipc)`);
}

// The dev token endpoint's client is reachable only behind DEV && VITE_SERVE;
// the app's boot takes the token from the shim's engine_connect and nowhere else.
{
  const main = read(SRC.main);
  const lines = main.split("\n");
  const guardOf = (at) => { for (let i = at; i >= 0; i--) if (/^\s*(\} else )?if \(/.test(lines[i])) return lines[i]; return null; };
  const bootAt = lines.findIndex((l) => /\{ bootSocket \} = await import\("\.\/ipcws"\)/.test(l));
  const bootGuard = bootAt < 0 ? null : guardOf(bootAt);
  check(bootAt >= 0 && /^\s*if \(import\.meta\.env\.DEV && import\.meta\.env\.VITE_SERVE\) \{\s*$/.test(bootGuard ?? ""),
    `main.tsx reaches bootSocket (the dev token endpoint) only under import.meta.env.DEV && VITE_SERVE (${bootGuard?.trim() ?? "no guard found"})`);
  const uses = [...main.split("\n").map((l) => l.replace(/\/\/.*$/, "")).join("\n").matchAll(/bootSocket/g)].length;
  check(uses === 2, `bootSocket is named twice in main.tsx — the guarded import and its one call — and nowhere else (${uses})`);
  const appAt = lines.findIndex((l) => /\{ connectSocket \} = await import\("\.\/ipcws"\)/.test(l));
  const appGuard = appAt < 0 ? null : guardOf(appAt);
  check(appAt >= 0 && /\} else if \(!import\.meta\.env\.VITE_MOCK && "__TAURI_INTERNALS__" in window\) \{/.test(appGuard ?? ""),
    `the app's boot connects with connectSocket inside Tauri, and not in the mock (${appGuard?.trim() ?? "no guard found"})`);
  const appBlock = appAt < 0 ? "" : lines.slice(appAt, lines.findIndex((l, i) => i > appAt && /^\s*gated = true;/.test(l))).join("\n");
  check(/const ep = await tauriDoor\.invoke<\{ port: number; token: string; owned: boolean; started: boolean \}>\("engine_connect", \{ reason, start, manual, onPhase: onPhaseCh \}\);/.test(appBlock) &&
    /connectSocket\(\{ \.\.\.socketDeps\(onLost, RECONNECT\), port: ep\.port, token: ep\.token \}\)/.test(appBlock) &&
    /return \{ sock, owned: ep\.owned, started: ep\.started \};/.test(appBlock) &&
    /engineDoor\(\s*tauriDoor,/.test(appBlock) && !/localStorage|sessionStorage|location|console\./.test(appBlock),
    "the app takes port and token from engine_connect's answer alone, hands them straight to connectSocket, and stores and logs neither; the retry policy's start/manual go in, the shim's owned/started come back");
  check((appBlock.match(/engine_connect/g) ?? []).length === 1 && /\{ reason, start, manual, onLost, onPhase \}/.test(appBlock),
    "engine_connect is called once, from the one dial, with exactly the try the retry policy made");
  check(/document\.addEventListener\("visibilitychange", \(\) => door\.visibility\(\)\);/.test(appBlock) && /retryEnv\(\)/.test(appBlock) &&
    /visible: \(\) => document\.visibilityState !== "hidden"/.test(main) && /now: \(\) => performance\.now\(\)/.test(main),
    "the app's retries run on performance.now() and hear every visibilitychange, so a hidden window defers them");
  check(/engineDoor\(\s*null,/.test(main), "the dev stack's engine door has no local half");
  const statics = [];
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) { walk(p); continue; }
      if (!/\.(ts|tsx)$/.test(e.name)) continue;
      if (/^\s*import\s(?!type\b)[^;]*from\s+"\.\/ipcws"/m.test(read(p))) statics.push(path.relative(SRC.appsrc, p));
    }
  };
  walk(SRC.appsrc);
  check(statics.length === 0, `no file imports ipcws.ts statically — main.tsx's dynamic imports decide what a build carries${statics.length ? `: ${statics.join(", ")}` : ""}`);
  check(/version: __WT_VERSION__/.test(main), "every socket the page opens hands the client __WT_VERSION__ as this frontend's version");
}

// The version source: one, the workspace Cargo.toml, on both ends.
{
  const vite = read(SRC.vite);
  check(/__WT_VERSION__: JSON\.stringify\(workspaceVersion\(\)\)/.test(vite) && /\.\.\/Cargo\.toml/.test(vite) && /workspace\\\.package/.test(vite),
    "vite.config.ts defines __WT_VERSION__ from ../Cargo.toml's [workspace.package]");
  check(/^version\.workspace = true$/m.test(read(SRC.cliToml)), "worktrees-cli inherits the workspace version, so serve's auth reply carries the same source");
  check(/^\[workspace\.package\]\s*\nversion = "[^"]+"/m.test(read(SRC.wsToml)), "the workspace Cargo.toml has the [workspace.package] version the config reads");
  check(/declare const __WT_VERSION__: string;/.test(read(SRC.envd)), "__WT_VERSION__ is declared for tsc");
}

// The dev server's token endpoint: same-origin or nothing.
{
  const js = (await transformWithEsbuild(read(SRC.door), "vite.serve-door.ts", { loader: "ts", format: "esm" })).code;
  const D = await load(SRC.door, js);
  const P = 1430;
  const good = { method: "POST", peer: "127.0.0.1", localPort: P, headers: { host: `localhost:${P}`, origin: `http://localhost:${P}`, "sec-fetch-site": "same-origin" } };
  const withH = (h, o = {}) => ({ ...good, ...o, headers: { ...good.headers, ...h } });
  check(D.sameOriginAsk(good), "the dev page's own POST is answered");
  check(D.sameOriginAsk(withH({ host: `127.0.0.1:${P}`, origin: `http://127.0.0.1:${P}` })), "…by either loopback spelling of itself");
  const refused = [
    ["a GET (a <script src>, an <img>, a navigation)", withH({}, { method: "GET" })],
    ["another localhost PORT's page (same-site, not same-origin)", withH({ origin: "http://localhost:3000", "sec-fetch-site": "same-site" })],
    ["a cross-site page", withH({ origin: "https://evil.example", "sec-fetch-site": "cross-site" })],
    ["a same-origin claim with a foreign Origin", withH({ origin: "http://localhost:3000" })],
    ["no Sec-Fetch-Site at all", withH({ "sec-fetch-site": undefined })],
    ["no Origin", withH({ origin: undefined })],
    ["a rebinding Host", withH({ host: `evil.example:${P}`, origin: `http://evil.example:${P}` })],
    ["a Host on another port", withH({ host: "localhost:1420", origin: "http://localhost:1420" })],
    ["a non-loopback peer", withH({}, { peer: "192.168.1.20" })],
  ];
  const leaks = refused.filter(([, r]) => D.sameOriginAsk(r));
  check(leaks.length === 0, `the token endpoint refuses ${refused.length} cross-origin shapes${leaks.length ? ` — LEAKS to: ${leaks.map(([l]) => l).join("; ")}` : ""}`);
  const endpoint = /const SERVE_ENDPOINT = "([^"]+)"/.exec(read(SRC.door))?.[1];
  check(endpoint === WS.SERVE_ENDPOINT, `the plugin's endpoint and the client's agree (${endpoint} / ${WS.SERVE_ENDPOINT})`);

  // The middleware itself: CORS headers set upstream are stripped, a refusal is
  // a bare 403, and the answer is {port, token} with no-store.
  let mw = null;
  D.serveDoor().configureServer({ config: { server: { host: undefined } }, middlewares: { use: (p, f) => { if (p === endpoint) mw = f; } } });
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "transport-check-"));
  const sf = path.join(tmp, "serve.json");
  fs.writeFileSync(sf, JSON.stringify({ port: 41237, token: TOKEN, pid: 1, version: VERSION }));
  const call = (ask) => {
    const headers = new Map([["access-control-allow-origin", "http://localhost:3000"], ["access-control-allow-credentials", "true"]]);
    const res = { statusCode: 0, body: "", setHeader: (k, v) => headers.set(k.toLowerCase(), v), getHeaderNames: () => [...headers.keys()], removeHeader: (k) => headers.delete(k), end: (b = "") => { res.body = b; } };
    mw({ method: ask.method, headers: ask.headers, socket: { remoteAddress: ask.peer, localPort: ask.localPort } }, res);
    return { ...res, headers };
  };
  const prev = process.env.WT_SERVE_JSON;
  process.env.WT_SERVE_JSON = sf;
  const yes = call(good);
  const no = call(withH({ origin: "http://localhost:3000", "sec-fetch-site": "same-site" }));
  if (prev === undefined) delete process.env.WT_SERVE_JSON; else process.env.WT_SERVE_JSON = prev;
  fs.rmSync(tmp, { recursive: true, force: true });
  check(mw !== null, "the plugin mounts a middleware on its endpoint");
  check(yes.statusCode === 200 && same(JSON.parse(yes.body || "null"), { port: 41237, token: TOKEN }) && yes.headers.get("cache-control") === "no-store",
    "the dev page gets {port, token} — no pid, no version — with Cache-Control: no-store");
  check(![...yes.headers.keys()].some((k) => k.startsWith("access-control-")), "every Access-Control-* header vite's cors middleware set is stripped, so no other origin can read even a leaked answer");
  check(no.statusCode === 403 && no.body === "" && !no.body.includes(TOKEN), "a cross-origin ask gets a bare 403");
  let threw = false;
  try { D.serveDoor().configureServer({ config: { server: { host: "0.0.0.0" } }, middlewares: { use() {} } }); } catch { threw = true; }
  check(threw, "the plugin refuses to start on a dev server listening beyond loopback");
}

// The build: when one exists, the socket client IS in it — the shipped app is
// a client of its host's serve — and the dev token endpoint's client is NOT.
{
  const assets = path.join(SRC.dist, "assets");
  const client = ["did not answer the handshake", "refusing to drive it", "did not prove it holds"];
  const devOnly = [WS.SERVE_ENDPOINT, "the dev server could not say"];
  // A marker the source no longer says would be absent from every bundle.
  const wsText = read(SRC.ipcws);
  const missing = [...client, ...devOnly].filter((m) => !wsText.includes(m));
  check(missing.length === 0, `every bundle marker is in ipcws.ts, so finding (or not finding) it in a build means something (${missing.join(", ") || "all present"})`);
  if (!fs.existsSync(assets)) {
    console.log("ok — app/dist absent — the bundle assertions are skipped on this run (build with `pnpm build` to exercise them)");
  } else {
    const js = fs.readdirSync(assets).filter((f) => f.endsWith(".js")).map((f) => [f, read(path.join(assets, f))]);
    const absent = client.filter((m) => !js.some(([, s]) => s.includes(m)));
    check(absent.length === 0, `the production bundle carries the socket client — the app is serve's client (${absent.length ? `MISSING: ${absent.join(", ")}` : `${client.length} markers present`})`);
    const leaked = [];
    for (const [f, s] of js) for (const m of devOnly) if (s.includes(m)) leaked.push(`${f}: ${m}`);
    check(leaked.length === 0, `the production bundle carries no dev token endpoint client (${leaked.length ? leaked.join("; ") : `${devOnly.length} markers absent`})`);
    const stale = fs.statSync(SRC.ipcws).mtimeMs > fs.statSync(assets).mtimeMs || fs.statSync(SRC.main).mtimeMs > fs.statSync(assets).mtimeMs;
    if (stale) console.log("note — app/dist is older than ipcws.ts or main.tsx; rebuild before trusting the bundle assertions");
  }
}

console.log(failed ? `\n${failed} failure(s)` : "\nall good");
process.exit(failed ? 1 : 0);
