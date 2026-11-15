// Where the guest connections come from (guest.ts's `GuestSource`), built
// once by main.tsx: in the app, the shim's host list and a pinned pipe per
// guest host (ipcremote.ts) under ipcws.ts's `connectSocket`; in the design
// harness, the mock's own answers to the guest rows.
//
// The pipe commands go through `tauriDoor` — the shim's, the Tauri half of
// every door — never through `invoke`, the window's engine door, so nothing
// here can reach the window's own engine and nothing the engine door routes
// can reach this socket.
import { tauriDoor } from "./ipc";
import type { Door, EventCallback } from "./ipc";
import type { HostSummary } from "./ipcremote";
import type { SocketDoor } from "./ipcws";
import type { GuestDial } from "./guestdoor";
import type { GuestSource } from "./guest";

function env(): GuestSource["env"] {
  return {
    now: () => performance.now(),
    setTimeout: (fn, ms) => window.setTimeout(fn, ms),
    clearTimeout: (h) => window.clearTimeout(h as number),
    visible: () => document.visibilityState !== "hidden",
  };
}

function storage(): GuestSource["storage"] {
  try {
    const ls = window.localStorage;
    return { get: (k) => ls.getItem(k), set: (k, v) => ls.setItem(k, v) };
  } catch {
    return null;
  }
}

async function forgetOf(door: Pick<Door, "invoke">, hostId: string): Promise<void> {
  await door.invoke("remote_forget", { hostId });
}

async function hostsOf(door: Pick<Door, "invoke">): Promise<HostSummary[]> {
  const h = await door.invoke<{ hosts: HostSummary[] }>("remote_hosts");
  return h.hosts;
}

/** The app's: one pinned pipe per guest host, the handshake ipcws.ts's. */
export function appGuestSource(): GuestSource {
  const dial = (host: HostSummary): GuestDial => async (onLost) => {
    const { connectSocket } = await import("./ipcws");
    const { openPipe, shimOf } = await import("./ipcremote");
    const shim = shimOf(tauriDoor);
    const pipe = await openPipe(shim, host.id, { setTimeout: (fn, ms) => window.setTimeout(fn, ms) });
    const sock = connectSocket({
      version: __WT_VERSION__,
      crypto: window.crypto,
      WebSocket: pipe.WebSocket,
      setTimeout: (fn, ms) => window.setTimeout(fn, ms),
      clearTimeout: (h) => window.clearTimeout(h as number),
      atob: (s) => window.atob(s),
      onError: onLost,
      warn: (m) => console.warn(`[guest ${host.name}] ${m}`),
      origin: window.location.origin,
      remedy: `${host.name} may be asleep or its app closed; this window tries again on its own.`,
      port: 0,
      token: pipe.connected.token,
      remoteHost: pipe.remoteHost,
    });
    return { sock, closedWhy: pipe.closedWhy, close: () => { void shim.close(pipe.connected.conn).catch(() => {}); } };
  };
  return { hosts: () => hostsOf(tauriDoor), dial, forget: (id) => forgetOf(tauriDoor, id), env: env(), storage: storage() };
}

/** The harness's: the mock answers the guest rows itself (install.ts),
 *  told which host each call is for, and emits `shares:changed` on its own
 *  event bus. `?guest=offline` makes the first connect fail. */
export function mockGuestSource(): GuestSource {
  // `__mock.guestDrop()` ends the open connection and refuses the next dials
  // until `__mock.guestBack()`: David's app quit, then came back (§6).
  const w = window as unknown as { __guestLost?: (m: string) => void; __guestDown?: boolean | "revoked" };
  const dial = (host: HostSummary): GuestDial => async (onLost) => {
    const offline = new URLSearchParams(window.location.search).get("guest") === "offline";
    if (w.__guestDown === "revoked") throw `${host.name} (${host.addr}): the secure connection failed: the host refused this device's key (AccessDenied): it was revoked, disabled or expired there, or never enrolled`;
    if (offline || w.__guestDown) throw `${host.name} (${host.addr}): connection refused`;
    w.__guestLost = onLost;
    // A real socket's listeners die with it; the mock's bus outlives every
    // "socket", so each one lets go of its own on close.
    const uns: Promise<() => void>[] = [];
    const sock: SocketDoor = {
      ready: Promise.resolve(),
      invoke: <T>(cmd: string, args?: Record<string, unknown>) => tauriDoor.invoke<T>(cmd, { ...args, __guestHost: host.id }),
      listen: <T>(event: string, h: EventCallback<T>) => { const u = tauriDoor.listen<T>(event, h); uns.push(u); return u; },
      channel: (m) => tauriDoor.channel(m),
      openChannels: () => 0,
      declare: () => {},
    };
    return { sock, closedWhy: () => null, close: () => { for (const u of uns.splice(0)) void u.then((f) => f()); } };
  };
  return { hosts: () => hostsOf(tauriDoor), dial, forget: (id) => forgetOf(tauriDoor, id), env: env(), storage: storage() };
}
