// The frontend's ONE door to the backend: `invoke`, `Channel`, `listen`.
//
// Every app command goes through here; nothing else in `app/src` imports
// `@tauri-apps/api/core` or `/event` (transport-check.mjs holds that line, the
// mock excepted). Plugins — opener, dialog, updater, process, `api/window` —
// are NOT app commands and keep importing their packages directly: they act on
// the machine the window runs on, whichever engine is driving it.
//
// Two implementations:
//
// - Tauri IPC, the default and the only one in a build. The door's functions
//   ARE `@tauri-apps/api/core`'s `invoke` / `Channel` and `api/event`'s
//   `listen`, called with the same arguments, so the mock harness — which sits
//   below `@tauri-apps/api/core` as a fake `__TAURI_INTERNALS__` — works
//   untouched.
// - A WebSocket to `worktrees serve` (`ipcws.ts`), installed by `main.tsx` in
//   the dev stack only (`pnpm dev:serve`). The next lane makes the shipped app a
//   client; this door is where it switches.
//
// Why a module and not a WS-backed `__TAURI_INTERNALS__` (plan §III.1): inside
// a real Tauri webview that object also carries every plugin call, so a socket
// installed there would have to intercept and forward plugin traffic through an
// undocumented, per-version surface. Here plugins never pass through the door.
import { Channel as TauriChannel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { EventCallback, UnlistenFn } from "@tauri-apps/api/event";

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

const tauri: Door = {
  invoke: (cmd, args) => tauriInvoke(cmd, args),
  listen: (event, handler) => tauriListen(event, handler),
  channel: (onmessage) => new TauriChannel(onmessage),
};

let door: Door = tauri;

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

// ── transport failures → fail() ─────────────────────────────────────────────
// A refused handshake happens before App has mounted, and `fail()` is App's
// own state, so failures wait here for the first subscriber. Tauri's door never
// reports one: a dead IPC is a dead window.
let sink: ((msg: string) => void) | null = null;
const early: string[] = [];

export function transportError(msg: string): void {
  if (sink) sink(msg);
  else early.push(msg);
}

/** App registers `fail`. Returns the unsubscribe. */
export function onTransportError(f: (msg: string) => void): () => void {
  sink = f;
  for (const m of early.splice(0)) f(m);
  return () => {
    if (sink === f) sink = null;
  };
}
