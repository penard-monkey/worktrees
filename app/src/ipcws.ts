// The WebSocket half of the door (`ipc.ts`): `worktrees serve`'s protocol,
// client side. The shipped app is a client of its host's serve, so this file
// SHIPS (`connectSocket`, dialled with the port and token the shim's
// `engine_connect` answers). Only `bootSocket` — the dev stack's way to learn
// them, from vite's token endpoint — is dev-only: `main.tsx` reaches it under
// `import.meta.env.DEV && VITE_SERVE`, which is dead code in a build, and
// transport-check.mjs greps app/dist to prove the endpoint is not in it.
//
// The protocol is crates/worktrees-cli/src/serve/wire.rs + policy.rs, and its
// handshake is v1 (the fix for OWASP H1, spec by (main), 2026-10-18):
//
//   → {"hello": cn}                          cn: 32 random bytes, hex
//   ← {"challenge": sn}                      sn: the same, chosen by serve
//   → {"auth": HMAC(token, client|cn|sn)}    or serve closes with NOTHING said
//   ← {"ok": true, "version", "proof": HMAC(token, server|cn|sn)}
//   → {id, cmd, args}                        ← {id, ok} | {id, err}
//   ← {channel, seq, data}                   a Channel argument's stream
//   ← {channel, seq, end: true}              its last frame: the handler goes
//   ← {event, payload}                       Host::emit, to every socket
//
// The token is the HMAC KEY and never goes on the wire. The client sends
// nothing but the handshake until serve's proof verifies: before then the
// server is whoever holds the port — after a crash, possibly not serve — and
// its version claim is not believed either.
//
// A channel ends the way Tauri's does when its Rust `Channel` drops: serve
// sends `{channel, seq, end: true}` after the last data frame (a detach,
// term_close, a shell exiting, a progress callback's operation returning), and
// the client drops the handler and whatever it closes over. An `err` reply to
// the request that named a channel ends it too — serve sends an end frame
// first only if it had made a sink.
//
// `err` is passed through UNCHANGED: a string for 142 commands, the CloneError
// object for `clone_project`, whose dialog branches on its `kind` — exactly
// what Tauri's `invoke` rejects with today. A transport failure (refused
// handshake, version mismatch, a close) rejects every pending and later call
// with a string, and is reported once through `onError` → `fail()`.
//
// No runtime imports, on purpose: transport-check.mjs evaluates this file as a
// `data:` URL module under stubs and a virtual clock, and a `data:` module has
// no base to resolve `./x` against (the zoom-check trap, AGENTS.md).
import type { Channel, Door, Event, EventCallback, InvokeArgs, UnlistenFn } from "./ipc";

/** Every key and constant the socket speaks. transport-check.mjs pins each one
 *  against wire.rs / policy.rs / dispatch.rs, so a rename on the Rust side
 *  fails the check rather than a browser. */
export const WIRE = {
  hello: "hello",
  challenge: "challenge",
  auth: "auth",
  proof: "proof",
  clientLabel: "worktrees-serve/v1/client\n",
  serverLabel: "worktrees-serve/v1/server\n",
  version: "version",
  id: "id",
  cmd: "cmd",
  args: "args",
  ok: "ok",
  err: "err",
  channel: "channel",
  seq: "seq",
  data: "data",
  /** `{channel, seq, end: true}`: the channel's last frame, `seq` one past its last data. */
  end: "end",
  event: "event",
  payload: "payload",
  channelPrefix: "__CHANNEL__:",
  /** A channel's first frame is `seq: 1` (FrameSink's `fetch_add(1) + 1`). */
  firstSeq: 1,
  clientLocal: "client-local",
} as const;

/** The Channel arguments that carry a ByteSink — base64 `data`, delivered as an
 *  ArrayBuffer, which is what TerminalPane gets from Tauri. Every other channel
 *  is a typed progress callback whose `data` is the JSON value itself. The
 *  frame cannot say which (a JSON value may be a string), so the ARG KEY does,
 *  as it does in dispatch.rs: `frames.bytes(a.channel("on_bytes"))`. */
export const BYTE_CHANNEL_KEYS: readonly string[] = ["onBytes"];

/** How long connect + the whole handshake may take before the client gives
 *  up. Longer than the server's own `policy::AUTH_DEADLINE` (3s), so a server
 *  that is there and refusing is reported as a refusal, not as a timeout. */
export const AUTH_WAIT_MS = 5000;

/** The largest message serve reads from an authenticated client (conn.rs
 *  `MAX_MESSAGE`), counted in UTF-8 bytes. A larger one ENDS the connection,
 *  every terminal on it included, so a call that would send one is refused
 *  here instead and the connection stays up. */
export const MAX_MESSAGE = 16 << 20;

/** serve binds this and nothing else (conn.rs). Dialled as the LITERAL: a
 *  second process can bind `[::1]:<port>` beside serve's socket, and
 *  `localhost` resolves ::1 first (the OWASP review measured it). */
export const SERVE_HOST = "127.0.0.1";

const HEX32 = /^[0-9a-f]{64}$/;
const hex = (b: ArrayBuffer | Uint8Array) => Array.from(new Uint8Array(b), (x) => x.toString(16).padStart(2, "0")).join("");
function unhex(s: string): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.slice(2 * i, 2 * i + 2), 16);
  return out;
}

/** The browser's WebSocket, as far as this client uses it. */
export type Sock = Pick<WebSocket, "send" | "close" | "onopen" | "onmessage" | "onclose" | "onerror">;

export type SocketDeps = {
  port: number;
  /** serve.json's token: the HMAC key, never a frame. */
  token: string;
  /** This frontend's own version (`__WT_VERSION__`). A PROVEN server's ok must
   *  carry the same string, or the client refuses to drive it. */
  version: string;
  /** `window.crypto`. `subtle` exists only in a secure context. */
  crypto: Pick<Crypto, "getRandomValues"> & { subtle?: SubtleCrypto | null };
  WebSocket: new (url: string) => Sock;
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
  atob: (b64: string) => string;
  /** A transport failure, once per connection. `ipc.transportError`. */
  onError: (msg: string) => void;
  /** Developer-facing notes (a client-local refusal, a stray frame). */
  warn?: (msg: string) => void;
  /** This page's origin, for the one failure the browser cannot diagnose. */
  origin?: string;
  /** What the person can DO about a server this client will not drive — said
   *  at the end of every such failure. The dev stack's default restarts the
   *  stack; the app passes its Reconnect, which finds, starts or replaces
   *  serve as needed. A remedy is a claim, so each host states its own. */
  remedy?: string;
};

class WsChannel<T> implements Channel<T> {
  onmessage: (response: T) => void;
  constructor(readonly id: number, onmessage?: (response: T) => void) {
    this.onmessage = onmessage ?? (() => {});
  }
  // Tauri 2's own spelling, which `wire::channel_id` parses.
  toJSON(): string {
    return WIRE.channelPrefix + this.id;
  }
}

type Stream = {
  ch: WsChannel<unknown>;
  bytes: boolean;
  next: number;
  /** Frames that arrived ahead of `next`, by seq. */
  held: Map<number, unknown>;
  /** The end frame's seq, once it has arrived: the channel is over when `next` reaches it. */
  end: number | null;
};

type Pending = {
  cmd: string;
  resolve: (v: unknown) => void;
  reject: (e: unknown) => void;
  /** The channels this request named: an `err` to it ends them. */
  channels: number[];
};

/** The door over a socket, plus what the checks and the drive look at. */
export type SocketDoor = Door & {
  /** Settles once: resolved when the server has PROVEN it holds the token and
   *  runs this frontend's version, rejected with the failure (the same string
   *  `onError` gets) if the connection ends first. */
  ready: Promise<void>;
  /** How many channels this connection still holds a handler for. */
  openChannels(): number;
};

function toArrayBuffer(b64: string, atob: (s: string) => string): ArrayBuffer {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

/** A door whose every call fails with `msg` — what the dev boot hands out when
 *  there is no server to connect to at all. */
export function deadDoor(msg: string): SocketDoor {
  let nextChannel = 1;
  const ready = Promise.reject(msg);
  ready.catch(() => {}); // a caller that never awaits it is not an unhandled rejection
  return {
    ready,
    invoke: () => Promise.reject(msg),
    listen: () => Promise.resolve(() => {}),
    channel: <T>(onmessage?: (response: T) => void) => new WsChannel<T>(nextChannel++, onmessage),
    openChannels: () => 0,
  };
}

/** Open a socket to `worktrees serve` and return the door over it. Calls made
 *  before the handshake completes wait for it; nothing but the handshake is
 *  ever written before the server has PROVEN it holds the token. */
export function connectSocket(d: SocketDeps): SocketDoor {
  const warn = d.warn ?? (() => {});
  const restart = d.remedy ?? "Restart the stack (app/scripts/server-stack.sh): serve picks a new port and rewrites serve.json.";
  const rebuild = d.remedy ?? "Rebuild (cargo build --release -p worktrees-cli) and restart the stack.";
  const addr = `${SERVE_HOST}:${d.port}`;
  const url = `ws://${addr}/`;
  const maybeSubtle = d.crypto?.subtle;
  if (!maybeSubtle) {
    const msg =
      "this page is not a secure context, so it has no crypto.subtle, and worktrees serve's handshake needs it — " +
      "open the dev page at http://localhost or http://127.0.0.1";
    d.onError(msg);
    return deadDoor(msg);
  }
  const subtle: SubtleCrypto = maybeSubtle;
  // hello → (challenge) → proving → (ok) → verifying → ready.
  type State = "connecting" | "hello" | "proving" | "verifying" | "ready" | "closed";
  let state: State = "connecting";
  let failure = "";
  let nextId = 1;
  let nextChannel = 1;
  let nextEvent = 1;
  const queued: string[] = [];
  let markReady: () => void = () => {};
  let markFailed: (msg: string) => void = () => {};
  const ready = new Promise<void>((res, rej) => {
    markReady = res;
    markFailed = rej;
  });
  ready.catch(() => {}); // reported through onError; awaiting it is optional
  /** Frames behind serve's ok, held while its proof is checked. */
  const inbox: unknown[] = [];
  const pending = new Map<number, Pending>();
  const streams = new Map<number, Stream>();
  const listeners = new Map<string, Set<EventCallback<unknown>>>();

  const enc = new TextEncoder();
  const key = subtle.importKey("raw", enc.encode(d.token), { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"]);
  key.catch(() => {}); // awaited below, where a failure is reported
  const cn = hex(d.crypto.getRandomValues(new Uint8Array(32)));
  let sn = "";
  let proofSent = false;
  const tagged = (label: string) => enc.encode(label + cn + "\n" + sn);

  /** Whoever answers on serve.json's port is serve only once it proves it.
   *  Not the user's doing, and nothing of theirs reached it. */
  const unproven = (why: string) =>
    `the server on ${addr} did not prove it holds serve.json's token (${why}), so it is not the worktrees serve ` +
    `that wrote serve.json: the port is stale (serve exited and another process has it) or an impostor is on it. ` +
    `No request was sent to it. ${restart}`;

  const ws = new d.WebSocket(url);
  const timer = d.setTimeout(() => {
    shut(`worktrees serve did not answer the handshake within ${AUTH_WAIT_MS / 1000}s (${url})`);
  }, AUTH_WAIT_MS);

  /** End the connection for good: every waiting call rejects with `msg`, and so
   *  does every later one. Reported once. */
  function shut(msg: string) {
    if (state === "closed") return;
    state = "closed";
    failure = msg;
    d.clearTimeout(timer);
    try {
      ws.close(); // a CONNECTING socket may be closed too; it just fails
    } catch {
      /* already closing */
    }
    queued.length = 0;
    inbox.length = 0;
    streams.clear(); // nothing can arrive on them now
    const waiting = [...pending.values()];
    pending.clear();
    for (const p of waiting) p.reject(msg);
    markFailed(msg); // no-op once ready has resolved
    d.onError(msg);
  }

  function deliver(st: Stream, data: unknown) {
    const value = st.bytes ? toArrayBuffer(String(data), d.atob) : data;
    try {
      st.ch.onmessage(value);
    } catch (e) {
      // A throwing handler must not stall the frames held behind it; rethrown
      // where an uncaught error belongs, as Tauri's own callback would be.
      queueMicrotask(() => {
        throw e;
      });
    }
  }

  /** A channel is over: drop its handler and everything it closes over, as
   *  Tauri's `Channel` unregisters its callback when the Rust side drops. */
  function release(id: number) {
    streams.delete(id);
  }

  function onChannel(id: number, seq: number, f: Record<string, unknown>) {
    const st = streams.get(id);
    if (!st) {
      warn(`worktrees serve: a frame for channel ${id}, which is not open here (never sent, or already ended)`);
      return;
    }
    if (seq < st.next || st.held.has(seq) || seq === st.end) return; // a duplicate
    if (f[WIRE.end] === true) {
      // Ordered like data: it ends the channel once every frame before it is in.
      st.end = seq;
    } else if (seq !== st.next) {
      st.held.set(seq, f[WIRE.data]);
      return;
    } else {
      deliver(st, f[WIRE.data]);
      st.next++;
      while (st.held.has(st.next)) {
        const held = st.held.get(st.next);
        st.held.delete(st.next);
        deliver(st, held);
        st.next++;
      }
    }
    if (st.next === st.end) release(id);
  }

  function onReply(f: Record<string, unknown>) {
    const id = f[WIRE.id];
    const p = typeof id === "number" ? pending.get(id) : undefined;
    if (!p) {
      // `id: null` is the server saying it could not read a request at all —
      // a client bug, and the call it belonged to would otherwise hang.
      if (WIRE.err in f) shut(`worktrees serve could not read a request: ${JSON.stringify(f[WIRE.err])}`);
      else warn(`worktrees serve: a reply to unknown request ${JSON.stringify(id)}`);
      return;
    }
    pending.delete(id as number);
    if (WIRE.err in f) {
      for (const c of p.channels) release(c);
      if (f[WIRE.err] === WIRE.clientLocal) warn(`${p.cmd} is client-local: it acts on the machine the window runs on, and the socket refuses it`);
      p.reject(f[WIRE.err]);
    } else {
      p.resolve(f[WIRE.ok]);
    }
  }

  /** serve's challenge: answer it with this client's proof. */
  async function onChallenge(f: Record<string, unknown>) {
    const s = f[WIRE.challenge];
    if (typeof s !== "string" || !HEX32.test(s)) {
      shut(unproven(WIRE.challenge in f ? "its challenge is not 32 random bytes in hex" : `it answered the hello with ${JSON.stringify(Object.keys(f))}, not a challenge`));
      return;
    }
    sn = s;
    state = "proving";
    const proof = hex(await subtle.sign("HMAC", await key, tagged(WIRE.clientLabel)));
    if (state !== "proving") return; // shut while signing
    proofSent = true;
    ws.send(JSON.stringify({ [WIRE.auth]: proof }));
  }

  /** serve's ok: believed — version included — only once its proof verifies. */
  async function onWelcome(f: Record<string, unknown>) {
    const proof = f[WIRE.proof];
    if (!proofSent) return shut(unproven("it answered before this client had sent its proof"));
    if (f[WIRE.ok] !== true) return shut(unproven(`it answered the proof with ${JSON.stringify(Object.keys(f))}, not ok`));
    if (typeof proof !== "string") return shut(unproven("its ok carries no proof"));
    if (!HEX32.test(proof)) return shut(unproven("its proof is not an HMAC-SHA256 in hex"));
    state = "verifying";
    // Constant-time by specification; never `===` on the hex.
    const good = await subtle.verify("HMAC", await key, unhex(proof), tagged(WIRE.serverLabel));
    if (state !== "verifying") return; // shut while verifying
    if (!good) return shut(unproven("its proof does not verify"));
    const theirs = f[WIRE.version];
    if (theirs !== d.version) {
      return shut(
        `version mismatch: worktrees serve is ${typeof theirs === "string" ? `v${theirs}` : "of an unknown version"}, ` +
          `this frontend is v${d.version} — refusing to drive it. ${rebuild}`,
      );
    }
    state = "ready";
    d.clearTimeout(timer);
    markReady();
    for (const text of queued.splice(0)) ws.send(text);
    for (const raw of inbox.splice(0)) handle(raw);
  }

  function handle(raw: unknown) {
    if (state === "closed") return;
    const proven = state === "ready";
    if (typeof raw !== "string") {
      shut(proven ? "worktrees serve sent a binary frame; the protocol is text only" : unproven("it sent a binary frame"));
      return;
    }
    let f: Record<string, unknown>;
    try {
      const v: unknown = JSON.parse(raw);
      if (!v || typeof v !== "object" || Array.isArray(v)) throw new Error("not an object");
      f = v as Record<string, unknown>;
    } catch (e) {
      shut(proven ? `worktrees serve sent an unreadable frame: ${String(e)}` : unproven("it sent an unreadable frame"));
      return;
    }
    if (state === "hello" || state === "proving") {
      // Async (WebCrypto), and each sets the next state before its first
      // await, so a frame behind this one meets the right state.
      (state === "hello" ? onChallenge(f) : onWelcome(f)).catch((e) => shut(`the handshake failed in this client: ${String(e)}`));
      return;
    }
    if (WIRE.channel in f) onChannel(Number(f[WIRE.channel]), Number(f[WIRE.seq]), f);
    else if (WIRE.event in f) {
      const name = String(f[WIRE.event]);
      for (const h of [...(listeners.get(name) ?? [])]) h({ event: name, id: nextEvent++, payload: f[WIRE.payload] } as Event<unknown>);
    } else if (WIRE.id in f) onReply(f);
    else warn(`worktrees serve: an unrecognised frame ${raw.slice(0, 80)}`);
  }

  ws.onopen = () => {
    if (state !== "connecting") return;
    state = "hello";
    ws.send(JSON.stringify({ [WIRE.hello]: cn }));
  };

  ws.onmessage = (ev) => {
    if (state === "closed") return;
    if (state === "verifying") inbox.push(ev.data);
    else handle(ev.data);
  };

  ws.onclose = () => {
    if (state === "closed") return;
    // serve refuses a foreign Origin (or Host) by closing during the upgrade
    // with nothing said, so a browser sees exactly what it sees for a dead
    // port. Name both; the stack's serve.log says which.
    if (state === "connecting")
      shut(
        `could not connect to worktrees serve at ${url} — it is not running, or it refused ` +
          `${d.origin ? `this page's Origin (${d.origin})` : "this page's Origin"}` +
          (d.remedy ? `. ${d.remedy}` : ": a dev stack needs --allow-origin for it"),
      );
    else if (state === "hello")
      shut(
        "worktrees serve closed the connection at this client's hello, with nothing said — a serve older than handshake v1 " +
          `reads a hello as a bad token. ${rebuild}`,
      );
    else if (state === "proving")
      shut(
        `the server on ${addr} refused this client's proof, closing with nothing said: it does not hold the token ` +
          `serve.json gave this page (serve.json is stale, or the port now belongs to another process). No request was sent. ${restart}`,
      );
    else if (state === "verifying") shut("the connection to worktrees serve closed during the handshake; no request was sent");
    else shut(d.remedy ? `the connection to worktrees serve closed. ${d.remedy}` : "the connection to worktrees serve closed");
  };
  // A failed socket fires `error` and then `close`; `close` says what happened.
  ws.onerror = () => {};

  /** `text`'s size in UTF-8 bytes if it is over MAX_MESSAGE, else 0. A UTF-16
   *  unit is 1–3 bytes, so only a string within 3× of the limit is encoded. */
  function oversize(text: string): number {
    if (text.length * 3 <= MAX_MESSAGE) return 0;
    const n = text.length > MAX_MESSAGE ? text.length : enc.encode(text).length;
    return n > MAX_MESSAGE ? n : 0;
  }

  function invoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
    if (state === "closed") return Promise.reject(failure);
    const a = (args ?? {}) as Record<string, unknown>;
    const id = nextId++;
    const text = JSON.stringify({ [WIRE.id]: id, [WIRE.cmd]: cmd, [WIRE.args]: a });
    const over = oversize(text);
    if (over) {
      const mib = (n: number) => `${Math.round((n / 2 ** 20) * 10) / 10} MiB`;
      return Promise.reject(
        `${cmd}: this request is over ${mib(over)}, and worktrees serve reads at most ${mib(MAX_MESSAGE)} in one message — ` +
          "it would end the connection, every terminal on it too, rather than read it. Not sent.",
      );
    }
    // Channels are bound where they are SENT: the key decides bytes vs typed.
    const channels: number[] = [];
    for (const [key, v] of Object.entries(a)) {
      if (!(v instanceof WsChannel)) continue;
      channels.push(v.id);
      if (!streams.has(v.id)) {
        streams.set(v.id, { ch: v, bytes: BYTE_CHANNEL_KEYS.includes(key), next: WIRE.firstSeq, held: new Map(), end: null });
      }
    }
    return new Promise<T>((resolve, reject) => {
      pending.set(id, { cmd, resolve: resolve as (v: unknown) => void, reject, channels });
      if (state === "ready") ws.send(text);
      else queued.push(text);
    });
  }

  function listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
    let set = listeners.get(event);
    if (!set) listeners.set(event, (set = new Set()));
    const h = handler as EventCallback<unknown>;
    set.add(h);
    return Promise.resolve(() => {
      listeners.get(event)?.delete(h);
    });
  }

  return {
    ready,
    invoke,
    listen,
    channel: <T>(onmessage?: (response: T) => void) => new WsChannel<T>(nextChannel++, onmessage),
    openChannels: () => streams.size,
  };
}

/** Where the dev server hands the page `{port, token}` (`vite.serve-door.ts`).
 *  A POST, so the browser sends `Origin`; the token comes back in the BODY —
 *  never in a URL, never in storage. */
export const SERVE_ENDPOINT = "/__worktrees/serve";

export type BootDeps = Omit<SocketDeps, "port" | "token"> & {
  fetch: (url: string, init: { method: string; cache: "no-store"; credentials: "same-origin" }) => Promise<{ ok: boolean; status: number; text(): Promise<string> }>;
};

/** The dev boot: ask the dev server where the stack is, then connect. The
 *  token lives only in this closure and the HMAC key made from it. */
export async function bootSocket(d: BootDeps): Promise<SocketDoor> {
  let port: number;
  let token: string;
  try {
    const r = await d.fetch(SERVE_ENDPOINT, { method: "POST", cache: "no-store", credentials: "same-origin" });
    const body = await r.text();
    if (!r.ok) throw new Error(body || `HTTP ${r.status}`);
    const j = JSON.parse(body) as { port?: unknown; token?: unknown };
    if (typeof j.port !== "number" || typeof j.token !== "string") throw new Error("no port/token in the answer");
    port = j.port;
    token = j.token;
  } catch (e) {
    const msg = `the dev server could not say where worktrees serve is: ${e instanceof Error ? e.message : String(e)}`;
    d.onError(msg);
    return deadDoor(msg);
  }
  return connectSocket({ ...d, port, token });
}
