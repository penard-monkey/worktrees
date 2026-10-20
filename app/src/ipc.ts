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
 *  that terminals re-attach through (their React keys carry it). */
export type EngineState =
  | { kind: "starting"; gen: number }
  | { kind: "up"; gen: number }
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

let reconnector: ((reason: string | null) => Promise<void>) | null = null;

/** Find or start the engine again and re-attach to it — the "Reconnect"
 *  button. A no-op where there is no engine door. */
export function reconnectEngine(): Promise<void> {
  return reconnector ? reconnector(engine.kind === "down" ? engine.reason : null) : Promise.resolve();
}

/** A socket door, as `ipcws.ts` makes one: `ready` settles once the handshake
 *  has proven the server (or failed). */
export type EngineSocket = Door & { ready: Promise<void> };

/** Open a socket to the engine. `reason` is why this is a reconnect (null on
 *  the first), for the shim's log; `onLost` is how the socket reports that it
 *  died — refused, mismatched, or closed — once. */
export type Dial = (reason: string | null, onLost: (msg: string) => void) => Promise<EngineSocket>;

/** A channel for a door with no socket: nothing will ever arrive on it, and the
 *  call it is handed to fails anyway. */
function deadChannel<T>(onmessage?: (response: T) => void): Channel<T> {
  return { onmessage: onmessage ?? (() => {}), toJSON: () => "__CHANNEL__:0" };
}

/** The shipped app's door: `local` (Tauri's, or null in a browser, where every
 *  command goes to the socket and serve refuses the client-local ones) for the
 *  names in `CLIENT_LOCAL` and `SHIM_COMMANDS`, and the engine socket that
 *  `dial` opens for everything else.
 *
 *  It owns its event listeners rather than lending them to one socket, so a
 *  reconnect re-subscribes every one of them on the new socket and a component
 *  that listened before the engine died hears it after. Channels come from the
 *  socket current when they are made; a terminal makes a new one when it
 *  re-attaches. Nothing is retried on its own: a lost engine says so
 *  (`engineState`), and `reconnectEngine()` is the one way back. */
export function engineDoor(local: Door | null, dial: Dial): Door & { connect(reason: string | null): Promise<void> } {
  // Nothing reaches the page until the first connect succeeds: the gate in
  // main.tsx mounts the app at gen 1, never against a dead engine.
  setEngine({ kind: "starting", gen: 0 });
  let sock: EngineSocket | null = null;
  let failure = "the engine is not connected yet";
  let attempt: Promise<void> | null = null;
  const handlers = new Map<string, Set<EventCallback<unknown>>>();

  const deliver = (name: string) => (e: Event<unknown>) => {
    for (const h of [...(handlers.get(name) ?? [])]) h(e);
  };
  const subscribe = (s: EngineSocket, name: string) => {
    void s.listen(name, deliver(name));
  };

  function connect(reason: string | null): Promise<void> {
    if (attempt) return attempt;
    attempt = (async () => {
      const gen = engine.gen;
      sock = null;
      setEngine({ kind: "starting", gen });
      // `onLost` can fire before `dial` has returned the socket it belongs
      // to; the holder lets it name which socket it is about.
      const holder: { s: EngineSocket | null } = { s: null };
      try {
        holder.s = await dial(reason, (msg) => {
          if (sock === null || sock !== holder.s) return; // not live yet: the catch below reports it
          sock = null;
          failure = msg;
          setEngine({ kind: "down", gen: engine.gen, reason: msg });
        });
        await holder.s.ready;
        for (const name of handlers.keys()) subscribe(holder.s, name);
        sock = holder.s;
        setEngine({ kind: "up", gen: gen + 1 });
      } catch (e) {
        failure = String(e);
        setEngine({ kind: "down", gen, reason: failure });
      } finally {
        attempt = null;
      }
    })();
    return attempt;
  }
  reconnector = connect;

  return {
    connect,
    invoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
      if (local && LOCAL.has(cmd)) return local.invoke<T>(cmd, args);
      if (sock) return sock.invoke<T>(cmd, args);
      return Promise.reject(failure);
    },
    listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
      let set = handlers.get(event);
      if (!set) {
        handlers.set(event, (set = new Set()));
        if (sock) subscribe(sock, event);
      }
      const h = handler as EventCallback<unknown>;
      set.add(h);
      return Promise.resolve(() => {
        handlers.get(event)?.delete(h);
      });
    },
    channel<T>(onmessage?: (response: T) => void): Channel<T> {
      return sock ? sock.channel<T>(onmessage) : deadChannel<T>(onmessage);
    },
  };
}
