// The WebSocket half of the door (`ipc.ts`): `worktrees serve`'s protocol,
// client side. DEV-ONLY in this lane — `main.tsx` loads it under
// `import.meta.env.DEV && VITE_SERVE`, which is dead code in a build, so this
// file is not in a production bundle (transport-check.mjs greps app/dist).
//
// The protocol is crates/worktrees-cli/src/serve/wire.rs + policy.rs:
//
//   → {"auth": "<token>"}                    first frame, nothing before it
//   ← {"ok": true, "version": "x.y.z"}       or a close with NOTHING said
//   → {id, cmd, args}                        ← {id, ok} | {id, err}
//   ← {channel, seq, data}                   a Channel argument's stream
//   ← {event, payload}                       Host::emit, to every socket
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
  auth: "auth",
  version: "version",
  id: "id",
  cmd: "cmd",
  args: "args",
  ok: "ok",
  err: "err",
  channel: "channel",
  seq: "seq",
  data: "data",
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

/** How long connect + auth may take before the client gives up. Longer than
 *  the server's own `policy::AUTH_DEADLINE` (3s), so a server that is there and
 *  refusing is reported as a refusal, not as a timeout. */
export const AUTH_WAIT_MS = 5000;

/** The browser's WebSocket, as far as this client uses it. */
export type Sock = Pick<WebSocket, "send" | "close" | "onopen" | "onmessage" | "onclose" | "onerror">;

export type SocketDeps = {
  url: string;
  token: string;
  /** This frontend's own version (`__WT_VERSION__`). The auth reply must carry
   *  the same string, or the client refuses to drive the server. */
  version: string;
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
};

type Pending = { cmd: string; resolve: (v: unknown) => void; reject: (e: unknown) => void };

function toArrayBuffer(b64: string, atob: (s: string) => string): ArrayBuffer {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

/** A door whose every call fails with `msg` — what the dev boot hands out when
 *  there is no server to connect to at all. */
export function deadDoor(msg: string): Door {
  let nextChannel = 1;
  return {
    invoke: () => Promise.reject(msg),
    listen: () => Promise.resolve(() => {}),
    channel: <T>(onmessage?: (response: T) => void) => new WsChannel<T>(nextChannel++, onmessage),
  };
}

/** Open a socket to `worktrees serve` and return the door over it. Calls made
 *  before the handshake completes wait for it; nothing but the auth frame is
 *  ever written before the server has answered it. */
export function connectSocket(d: SocketDeps): Door {
  const warn = d.warn ?? (() => {});
  type State = "connecting" | "authing" | "ready" | "closed";
  let state: State = "connecting";
  let failure = "";
  let nextId = 1;
  let nextChannel = 1;
  let nextEvent = 1;
  const queued: string[] = [];
  const pending = new Map<number, Pending>();
  const streams = new Map<number, Stream>();
  const listeners = new Map<string, Set<EventCallback<unknown>>>();

  const ws = new d.WebSocket(d.url);
  const timer = d.setTimeout(() => {
    shut(`worktrees serve did not answer the handshake within ${AUTH_WAIT_MS / 1000}s (${d.url})`);
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
    const waiting = [...pending.values()];
    pending.clear();
    for (const p of waiting) p.reject(msg);
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

  function onChannel(id: number, seq: number, data: unknown) {
    const st = streams.get(id);
    if (!st) {
      warn(`worktrees serve: a frame for channel ${id}, which this client never sent`);
      return;
    }
    if (seq < st.next || st.held.has(seq)) return; // a duplicate
    if (seq !== st.next) {
      st.held.set(seq, data);
      return;
    }
    deliver(st, data);
    st.next++;
    while (st.held.has(st.next)) {
      const held = st.held.get(st.next);
      st.held.delete(st.next);
      deliver(st, held);
      st.next++;
    }
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
      if (f[WIRE.err] === WIRE.clientLocal) warn(`${p.cmd} is client-local: it acts on the machine the window runs on, and the socket refuses it`);
      p.reject(f[WIRE.err]);
    } else {
      p.resolve(f[WIRE.ok]);
    }
  }

  ws.onopen = () => {
    if (state !== "connecting") return;
    state = "authing";
    ws.send(JSON.stringify({ [WIRE.auth]: d.token }));
  };

  ws.onmessage = (ev) => {
    if (state === "closed") return;
    if (typeof ev.data !== "string") {
      shut("worktrees serve sent a binary frame; the protocol is text only");
      return;
    }
    let f: Record<string, unknown>;
    try {
      const v: unknown = JSON.parse(ev.data);
      if (!v || typeof v !== "object" || Array.isArray(v)) throw new Error("not an object");
      f = v as Record<string, unknown>;
    } catch (e) {
      shut(`worktrees serve sent an unreadable frame: ${String(e)}`);
      return;
    }
    if (state === "authing") {
      if (f[WIRE.ok] !== true) {
        shut(`worktrees serve answered the handshake with ${ev.data.slice(0, 80)}`);
        return;
      }
      const theirs = f[WIRE.version];
      if (theirs !== d.version) {
        shut(
          `version mismatch: worktrees serve is ${typeof theirs === "string" ? `v${theirs}` : "of an unknown version"}, ` +
            `this frontend is v${d.version} — refusing to drive it. Rebuild (cargo build --release -p worktrees-cli) and restart the stack.`,
        );
        return;
      }
      state = "ready";
      d.clearTimeout(timer);
      for (const text of queued.splice(0)) ws.send(text);
      return;
    }
    if (state !== "ready") return;
    if (WIRE.channel in f) onChannel(Number(f[WIRE.channel]), Number(f[WIRE.seq]), f[WIRE.data]);
    else if (WIRE.event in f) {
      const name = String(f[WIRE.event]);
      for (const h of [...(listeners.get(name) ?? [])]) h({ event: name, id: nextEvent++, payload: f[WIRE.payload] } as Event<unknown>);
    } else if (WIRE.id in f) onReply(f);
    else warn(`worktrees serve: an unrecognised frame ${ev.data.slice(0, 80)}`);
  };

  ws.onclose = () => {
    if (state === "closed") return;
    // serve refuses a foreign Origin (or Host) by closing during the upgrade
    // with nothing said, so a browser sees exactly what it sees for a dead
    // port. Name both; the stack's serve.log says which.
    if (state === "connecting")
      shut(
        `could not connect to worktrees serve at ${d.url} — it is not running, or it refused ` +
          `${d.origin ? `this page's Origin (${d.origin})` : "this page's Origin"}: a dev stack needs --allow-origin for it`,
      );
    else if (state === "authing")
      shut("worktrees serve refused the connection — it closes with nothing said (wrong token, an Origin it was not started with, or a stale serve.json)");
    else shut("the connection to worktrees serve closed");
  };
  // A failed socket fires `error` and then `close`; `close` says what happened.
  ws.onerror = () => {};

  function invoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
    if (state === "closed") return Promise.reject(failure);
    const a = (args ?? {}) as Record<string, unknown>;
    // Channels are bound where they are SENT: the key decides bytes vs typed.
    for (const [key, v] of Object.entries(a)) {
      if (v instanceof WsChannel && !streams.has(v.id)) {
        streams.set(v.id, { ch: v, bytes: BYTE_CHANNEL_KEYS.includes(key), next: WIRE.firstSeq, held: new Map() });
      }
    }
    const id = nextId++;
    const text = JSON.stringify({ [WIRE.id]: id, [WIRE.cmd]: cmd, [WIRE.args]: a });
    return new Promise<T>((resolve, reject) => {
      pending.set(id, { cmd, resolve: resolve as (v: unknown) => void, reject });
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
    invoke,
    listen,
    channel: <T>(onmessage?: (response: T) => void) => new WsChannel<T>(nextChannel++, onmessage),
  };
}

/** Where the dev server hands the page `{port, token}` (`vite.serve-door.ts`).
 *  A POST, so the browser sends `Origin`; the token comes back in the BODY —
 *  never in a URL, never in storage. */
export const SERVE_ENDPOINT = "/__worktrees/serve";

export type BootDeps = Omit<SocketDeps, "url" | "token"> & {
  fetch: (url: string, init: { method: string; cache: "no-store"; credentials: "same-origin" }) => Promise<{ ok: boolean; status: number; text(): Promise<string> }>;
};

/** The dev boot: ask the dev server where the stack is, then connect. The
 *  token lives only in this closure and in the socket's first frame. */
export async function bootSocket(d: BootDeps): Promise<Door> {
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
  return connectSocket({ ...d, url: `ws://127.0.0.1:${port}/`, token });
}
