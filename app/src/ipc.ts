// The frontend's ONE door to the backend: `invoke`, `Channel`, `listen`.
//
// Every app command goes through here; nothing else in `app/src` imports
// `@tauri-apps/api/core` or `/event` (transport-check.mjs holds that line, the
// mock excepted). Plugins — opener, dialog, updater, process, `api/window` —
// are NOT app commands and keep importing their packages directly: they act on
// the machine the window runs on, whichever engine is driving it.
//
// Three doors, chosen once by `main.tsx` before anything that talks to the
// backend is imported:
//
// - The ENGINE door (`engineDoor`), which is the shipped app's. The engine is
//   this host's `worktrees serve` (one engine per host): every command goes to
//   it over an authenticated WebSocket (`ipcws.ts`), except the ones that act
//   on the machine the window runs on — serve's `ClientLocal` rows plus the
//   shim's own two — which go to Tauri IPC. ONE table decides
//   (`CLIENT_LOCAL`, `SHIM_COMMANDS`), and engine-route-check.mjs holds it
//   against dispatch.rs and the shim's `generate_handler!` both ways. The dev
//   stack (`pnpm dev:serve`, a browser) uses the same door with no local half.
// - Tauri IPC alone (`tauriDoor`): the default, which is what the mock harness
//   answers — it sits below `@tauri-apps/api/core` as a fake
//   `__TAURI_INTERNALS__`, so it works untouched.
//
// Why a module and not a WS-backed `__TAURI_INTERNALS__` (plan §III.1): inside
// a real Tauri webview that object also carries every plugin call, so a socket
// installed there would have to intercept and forward plugin traffic through an
// undocumented, per-version surface. Here plugins never pass through the door.
import { Channel as TauriChannel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { Event, EventCallback, UnlistenFn } from "@tauri-apps/api/event";
import { retrier, type Outcome, type RetryEnv, type RetryView, type Try } from "./reconnect";

export type { Event, EventCallback, UnlistenFn } from "@tauri-apps/api/event";

export type InvokeArgs = Record<string, unknown>;

/** An ordered stream the backend pushes into: a terminal's bytes (an
 *  ArrayBuffer per message) or a progress callback's values. Structural, so
 *  Tauri's class and the socket's both satisfy it. */
export interface Channel<T> {
  onmessage: (response: T) => void;
  toJSON(): string;
}

/** What a backend must provide. */
export type Door = {
  invoke<T>(cmd: string, args?: InvokeArgs): Promise<T>;
  listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn>;
  channel<T>(onmessage?: (response: T) => void): Channel<T>;
};

export const tauriDoor: Door = {
  invoke: (cmd, args) => tauriInvoke(cmd, args),
  listen: (event, handler) => tauriListen(event, handler),
  channel: (onmessage) => new TauriChannel(onmessage),
};

let door: Door = tauriDoor;

/** Replace the Tauri door. Called once, by `main.tsx`, before anything that
 *  talks to the backend is imported. */
export function openDoor(d: Door): void {
  door = d;
}

export function invoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  return door.invoke<T>(cmd, args);
}

export function listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  return door.listen<T>(event, handler);
}

/** `new Channel<T>()`, as the call sites wrote it against Tauri. The door
 *  decides whose channel it is — Tauri's, or the socket's — which is why this
 *  is a constructor function returning the door's object rather than a class. */
export const Channel = function <T>(onmessage?: (response: T) => void): Channel<T> {
  return door.channel<T>(onmessage);
} as unknown as new <T>(onmessage?: (response: T) => void) => Channel<T>;

// ── the engine door ─────────────────────────────────────────────────────────

/** serve's `ClientLocal` rows (crates/worktrees-cli/src/serve/dispatch.rs):
 *  each acts on the machine the window runs on — its clipboard, its editor, its
 *  terminal emulator, its page zoom, its own app bundle — so serve refuses it
 *  and the shipped app runs it in its own process, over Tauri IPC. */
export const CLIENT_LOCAL: readonly string[] = [
  "copy_text",
  "install_app_version",
  "open_editor",
  "detect_editors",
  "open_terminal",
  "set_zoom",
];

/** The shim's own commands: not serve rows at all, and never to become ones.
 *  `engine_connect` finds or starts serve and is the ONLY way its token reaches
 *  the page; `raise_window` brings the window forward for `app:open-doc`. */
export const SHIM_COMMANDS: readonly string[] = ["engine_connect", "raise_window"];

const LOCAL = new Set([...CLIENT_LOCAL, ...SHIM_COMMANDS]);

/** Where the page stands with the engine. `gen` counts successful connections:
 *  0 means none yet (the app has not mounted), and every bump is a new socket
 *  that terminals re-attach through (their React keys carry it).
 *
 *  - `starting`: one try in flight. `phase` is the shim's news about a long
 *    one ("waiting-old": a serve still stopping; "starting": spawning one);
 *    `reason` is why the engine is not up (null at launch).
 *  - `waiting`: the next automatic try is at `at` (`performance.now()`'s
 *    clock). `notOurs`: the serve that went was started outside the app, so
 *    the retries only reconnect.
 *  - `hidden`: a try fell due while the window was hidden, and waits to be seen.
 *  - `down`: the retries have STOPPED (the crash-loop limit); only Reconnect
 *    now goes on. */
export type EngineState =
  | { kind: "starting"; gen: number; phase: string | null; manual: boolean; reason: string | null }
  | { kind: "up"; gen: number }
  | { kind: "waiting"; gen: number; reason: string; at: number; notOurs: boolean }
  | { kind: "hidden"; gen: number; reason: string; notOurs: boolean }
  | { kind: "down"; gen: number; reason: string };

// "up" at gen 0 is what every door but the engine door reports: the mock and
// plain Tauri have no engine to lose, so nothing gates on them.
let engine: EngineState = { kind: "up", gen: 0 };
const engineSubs = new Set<() => void>();

function setEngine(s: EngineState) {
  engine = s;
  for (const f of [...engineSubs]) f();
}

/** The current engine state (`useSyncExternalStore`'s snapshot). */
export function engineState(): EngineState {
  return engine;
}

/** Subscribe to engine state changes. Returns the unsubscribe. */
export function onEngineState(f: () => void): () => void {
  engineSubs.add(f);
  return () => {
    engineSubs.delete(f);
  };
}

let reconnector: (() => Promise<void>) | null = null;

/** Reconnect now: cancel the pending retry and try at once — allowed to start
 *  a serve, because a person asked. A no-op where there is no engine door. */
export function reconnectEngine(): Promise<void> {
  return reconnector ? reconnector() : Promise.resolve();
}

/** A socket door, as `ipcws.ts` makes one: `ready` settles once the handshake
 *  has proven the server (or failed), and `declare` tells serve which events
 *  this page listens for (sent once the proof passes). */
export type EngineSocket = Door & { ready: Promise<void>; declare(names: readonly string[]): void };

/** One dial, as the engine door asks for it: the try's `reason` (null at
 *  launch) and its `start` permission and `manual` mark for the shim, `onLost`
 *  for the socket to report that it died — refused, mismatched, or closed —
 *  once, and `onPhase` for the shim's news about a long try. */
export type DialAsk = {
  reason: string | null;
  start: boolean;
  manual: boolean;
  onLost: (msg: string) => void;
  onPhase: (phase: string) => void;
};

/** What a dial found: the socket, and the shim's two facts — is this serve the
 *  app's (`owned`), and did this dial start it (`started`). A dial that throws
 *  rejects with the shim's `ConnectError` ({message, started, declined}), or
 *  with anything else, which reads as a plain failure. */
export type Dialled = { sock: EngineSocket; owned: boolean; started: boolean };

export type Dial = (ask: DialAsk) => Promise<Dialled>;

/** Events that are one act each rather than a state, kept for a moment when
 *  they arrive with nobody listening and handed to the first listener.
 *  `app:open-doc` is a `worktrees show`: serve holds one until a connection
 *  DECLARES it listens for it, which this door does only while App's listener
 *  is there (§9.3) — so serve does the waiting, and what is kept here is what
 *  slips between a declaration and a listener going: StrictMode's throwaway
 *  listener lets go a microtask late, and the frame that says so goes on the
 *  next macrotask. A state event (`sessions:busy`, `places:changed`) is never
 *  kept: a late one would be a stale set, and the next tick sends a fresh one
 *  anyway. */
const ONE_SHOT: ReadonlySet<string> = new Set(["app:open-doc"]);
/** How long an unheard one-shot event waits. It bridges "authenticated" to
 *  "App is listening", which is seconds; the request it carries was already up
 *  to 30 s old (`inbox::MAX_AGE_SECS`), and a document opening a minute after
 *  it was asked for would be a surprise, not a service. */
export const ONE_SHOT_TTL_MS = 10_000;
const ONE_SHOT_MAX = 16;

/** A channel for a door with no socket: nothing will ever arrive on it, and the
 *  call it is handed to fails anyway. */
function deadChannel<T>(onmessage?: (response: T) => void): Channel<T> {
  return { onmessage: onmessage ?? (() => {}), toJSON: () => "__CHANNEL__:0" };
}

/** The shim's rejection, or anything a dial threw, as an outcome's fields. */
function dialFailure(e: unknown): { reason: string; started: boolean; declined: boolean } {
  if (typeof e === "object" && e !== null && typeof (e as { message?: unknown }).message === "string") {
    const f = e as { message: string; started?: unknown; declined?: unknown };
    return { reason: f.message, started: f.started === true, declined: f.declined === true };
  }
  return { reason: String(e), started: false, declined: false };
}

/** The engine door's handle on its retries: `connect` is the launch's try,
 *  `visibility` is called on every `visibilitychange`. */
export type EngineDoor = Door & { connect(): Promise<void>; visibility(): void };

/** The shipped app's door: `local` (Tauri's, or null in a browser, where every
 *  command goes to the socket and serve refuses the client-local ones) for the
 *  names in `CLIENT_LOCAL` and `SHIM_COMMANDS`, and the engine socket that
 *  `dial` opens for everything else.
 *
 *  It owns its event listeners rather than lending them to one socket, so a
 *  reconnect re-subscribes every one of them on the new socket — once per
 *  socket, whatever happened in between — and a component that listened
 *  before the engine died hears it after. It tells each socket which of them
 *  have a live handler (`declare`, §9.3) — once per macrotask, and afresh on
 *  every reconnect — because serve sends a one-shot event only to a socket
 *  that said it listens for it. Channels come from the socket
 *  current when they are made; a terminal makes a new one when it re-attaches.
 *  When the socket dies the door retries on its own (`reconnect.ts` holds when,
 *  and whether a try may start a serve); `reconnectEngine()` tries at once.
 *  `env.now` also ages the one-shot events kept for a late listener. */
export function engineDoor(local: Door | null, dial: Dial, env: RetryEnv): EngineDoor {
  // Nothing reaches the page until the first connect succeeds: the gate in
  // main.tsx mounts the app at gen 1, never against a dead engine.
  setEngine({ kind: "starting", gen: 0, phase: null, manual: false, reason: null });
  let sock: EngineSocket | null = null;
  let gen = 0;
  let failure = "the engine is not connected yet";
  const handlers = new Map<string, Set<EventCallback<unknown>>>();
  // The socket being connected or in use, and the names this door listens to
  // on it. Wired from the moment `dial` returns it, before its handshake is
  // through: a frame serve sends right behind its ok is handed on as the proof
  // passes, synchronously, ahead of anything awaiting `ready`.
  let wired: { s: EngineSocket; names: Set<string> } | null = null;
  // One-shot events nobody was listening for yet (`ONE_SHOT`).
  const parked: { name: string; e: Event<unknown>; at: number }[] = [];
  // What the socket being connected or in use was last told this door listens
  // for (§9.3): serve sends a one-shot event only to a connection that
  // declared it, and holds a `worktrees show` until one has. The set is the
  // names with a LIVE handler — not ONE_SHOT's, which are wired for parking
  // whether or not anyone listens — so a show waits in serve until App
  // listens. At most one frame per macrotask, so StrictMode's listen,
  // unlisten, listen is one frame or none. Every new socket is told afresh;
  // it starts out having declared nothing, so an empty set needs no frame.
  let told: { s: EngineSocket; names: string } | null = null;
  let telling = false;
  const tell = () => {
    telling = false;
    const s = wired?.s;
    if (!s) return;
    const names = [...handlers].filter(([, hs]) => hs.size > 0).map(([name]) => name).sort();
    const key = names.join("\n");
    if (told?.s === s ? told.names === key : names.length === 0) return;
    told = { s, names: key };
    s.declare(names);
  };
  const retell = () => {
    if (telling) return;
    telling = true;
    setTimeout(tell, 0);
  };

  const deliver = (name: string) => (e: Event<unknown>) => {
    const hs = handlers.get(name);
    if (hs?.size) {
      for (const h of [...hs]) h(e);
    } else if (ONE_SHOT.has(name)) {
      parked.push({ name, e, at: env.now() });
      if (parked.length > ONE_SHOT_MAX) parked.shift();
    }
  };
  const wire = (name: string) => {
    if (!wired || wired.names.has(name)) return;
    wired.names.add(name);
    void wired.s.listen(name, deliver(name));
  };
  // Handed on a macrotask after a listener arrives, not a microtask: React's
  // StrictMode listens, lets go and listens again, and App lets go a microtask
  // late (`un.then(f => f())`), so by now only the listener that stays is here.
  const flush = (name: string) =>
    setTimeout(() => {
      const hs = handlers.get(name);
      if (!hs?.size) return;
      const due = parked.filter((p) => p.name === name);
      parked.splice(0, parked.length, ...parked.filter((p) => p.name !== name));
      for (const p of due) {
        if (env.now() - p.at > ONE_SHOT_TTL_MS) continue;
        for (const h of [...hs]) h(p.e);
      }
    }, 0);

  /** One try: dial, wire, prove, go live. */
  async function attempt(t: Try, onPhase: (p: string) => void): Promise<Outcome> {
    sock = null;
    wired = null;
    // `onLost` can fire before `dial` has returned the socket it belongs to;
    // the holder lets it name which socket it is about.
    const holder: { s: EngineSocket | null } = { s: null };
    let started = false;
    try {
      const d = await dial({
        reason: t.reason === null ? null : t.manual ? t.reason : `automatic retry ${t.attempt}: ${t.reason}`,
        start: t.start,
        manual: t.manual,
        onPhase,
        onLost: (msg) => {
          // A dead socket is listened on no more: a name first listened to
          // while the engine is down is wired by the next try, to its socket.
          if (wired?.s === holder.s) wired = null;
          if (sock === null || sock !== holder.s) return; // not live yet: the catch below reports it
          sock = null;
          failure = msg;
          retries.lost(msg);
        },
      });
      started = d.started;
      holder.s = d.sock;
      wired = { s: d.sock, names: new Set() };
      for (const name of [...ONE_SHOT, ...handlers.keys()]) wire(name);
      tell(); // held by the socket until serve's proof passes, then sent first
      await d.sock.ready;
      sock = d.sock;
      gen += 1;
      return { ok: true, owned: d.owned, started };
    } catch (e) {
      wired = null;
      const f = dialFailure(e);
      failure = f.reason;
      return { ok: false, reason: f.reason, started: started || f.started, declined: f.declined };
    }
  }

  const retries = retrier(env, attempt, (v: RetryView) => {
    if (v.kind === "up") setEngine({ kind: "up", gen });
    else if (v.kind === "trying") setEngine({ kind: "starting", gen, phase: v.phase, manual: v.manual, reason: v.reason });
    else if (v.kind === "waiting") setEngine({ kind: "waiting", gen, reason: v.reason, at: v.at, notOurs: v.notOurs });
    else if (v.kind === "hidden") setEngine({ kind: "hidden", gen, reason: v.reason, notOurs: v.notOurs });
    else setEngine({ kind: "down", gen, reason: v.reason });
  });
  reconnector = retries.now;

  return {
    connect: retries.boot,
    visibility: retries.visibility,
    invoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
      if (local && LOCAL.has(cmd)) return local.invoke<T>(cmd, args);
      if (sock) return sock.invoke<T>(cmd, args);
      return Promise.reject(failure);
    },
    listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
      let set = handlers.get(event);
      if (!set) {
        handlers.set(event, (set = new Set()));
        wire(event);
      }
      const h = handler as EventCallback<unknown>;
      set.add(h);
      retell();
      if (parked.some((p) => p.name === event)) flush(event);
      return Promise.resolve(() => {
        handlers.get(event)?.delete(h);
        retell();
      });
    },
    channel<T>(onmessage?: (response: T) => void): Channel<T> {
      return sock ? sock.channel<T>(onmessage) : deadChannel<T>(onmessage);
    },
  };
}
