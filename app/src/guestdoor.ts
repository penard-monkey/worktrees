// The GUEST connection (group chat §7): a small socket of its own, beside the
// window's engine door, to a host this machine is a guest of — David's, from
// Bret's window. It carries the guest rows and nothing else.
//
// Why a separate connection and not the door: a window talks to ONE engine,
// chosen at startup (`ipc.ts`), and Bret keeps his own workspace on it. A
// guest host is never that engine (`HostStore::choose` refuses it, and the
// Hosts panel never offers it), so its socket lives here, OUTSIDE ipc.ts's
// door and its route table. Nothing that calls `invoke` can reach it: a stray
// `invoke("agent_send", …)` goes to Bret's own engine, and the only commands
// this file sends are `GUEST_COMMANDS`, which dispatch.rs's `GUEST_ROWS` are
// (transport-check.mjs holds both, and that only guest modules import this).
// David's host refuses every other row to a guest anyway; this is the client
// half of that line, so the UI never asks.
//
// The socket is ipcws.ts's `connectSocket`, unchanged, over the shim's pinned
// pipe (`ipcremote.ts`): its deps are per call, and the shim keeps one pipe
// per HOST (`remote.rs`), so this one and a remote engine's live side by side.
//
// A lost connection is retried on its own, on a slow backoff, while the
// window is visible; meanwhile the state is `offline` with the host's words,
// and the views keep the last page they read (§6: "David's host is offline").
// Nothing is queued while the app is closed: every (re)connect re-reads
// `shares_list` (the guest store does), which is where a closed app's news is.
//
// A loss the HOST explains as this device's end is not offline: the host
// closing with `revoked`, `removed` or `disabled` (conn.rs's `Clients::hear`),
// or TLS refusing this device's key on a redial (pipe.rs's `upgrade_error`).
// Then the state is `removed`, no timer redials (every try would be refused
// the same way), and only a person's Retry dials again: a disabled device can
// be turned back on. Until v0.52.0's three-VM run a removed guest read
// "offline" and redialled forever.
//
// No runtime imports: transport-check loads this file as a `data:` module.
import type { EventCallback, UnlistenFn } from "./ipc";
import type { HostSummary } from "./ipcremote";
import type { SocketDoor } from "./ipcws";

/** dispatch.rs's `GUEST_ROWS`: every command a guest may make, and the ONLY
 *  ones this connection sends. Held against the Rust by transport-check. */
export const GUEST_COMMANDS = [
  "shares_list",
  "share_join",
  "guest_places",
  "share_request",
  "thread_read",
  "thread_post",
  "shared_turns",
  "shared_tool_call",
  "shared_image",
  "shared_call_image",
] as const;
export type GuestCommand = (typeof GUEST_COMMANDS)[number];

/** The one event a guest listens for (`sharethread::EVENT`). */
export const SHARES_CHANGED = "shares:changed";

/** `shares:changed`'s payload: ids only, never text or names. Re-read. */
export type SharesChanged = { what: string; share?: string; seq?: number; to?: string[] };

export type GuestState =
  | { kind: "connecting" }
  | { kind: "up" }
  | { kind: "offline"; why: string; retryAt: number | null }
  | { kind: "removed"; why: string; by: RemovedBy };

/** How the host ended this device: its close reason, or `refused` for a key
 *  TLS turned away (revoked, disabled or expired there: TLS does not say). */
export type RemovedBy = "revoked" | "removed" | "disabled" | "refused";

/** pipe.rs's `HOST_CLOSED`: how the pipe's words for a host's close begin,
 *  its reason after them. ipcws.ts has the same constant; this file may not
 *  import it (transport-check holds it to pipe.rs). */
export const HOST_CLOSED_BY = "the host closed the connection: ";
/** pipe.rs's words when TLS refuses this device's key. */
export const KEY_REFUSED = "the host refused this device's key";

/** The host ended this device (`RemovedBy`), or null for any other loss: a
 *  host that quit, no route, `unreadable` (the host's device file, which a
 *  fix there brings back). Pure, for the check. */
export function removedBy(why: string): RemovedBy | null {
  const m = new RegExp(`${HOST_CLOSED_BY}(revoked|removed|disabled)(?![\\w-])`).exec(why);
  if (m) return m[1] as RemovedBy;
  return why.includes(KEY_REFUSED) ? "refused" : null;
}

/** A host this machine is a guest of (`HostSummary.guest`). */
export const isGuestHost = (h: Pick<HostSummary, "guest">): boolean => h.guest === true;

/** One socket to the guest host, and the shim's words if it ended. */
export type Dialled = { sock: SocketDoor; closedWhy(): string | null; close(): void };

/** How a connection is opened: the app's pinned pipe, or the harness's fake.
 *  `onLost` fires once, when that socket ends. */
export type GuestDial = (onLost: (msg: string) => void) => Promise<Dialled>;

export type GuestEnv = {
  now: () => number;
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (h: unknown) => void;
  visible: () => boolean;
};

/** Seconds between tries after a loss: slow, since nothing here is urgent and
 *  a host that is down (its app quit) stays down for a while. */
export const RETRY_MS: readonly number[] = [3_000, 10_000, 30_000, 60_000];

export type GuestLink = {
  host: HostSummary;
  /** A guest row. Any other name is refused here, without a frame. */
  call<T>(cmd: GuestCommand, args?: Record<string, unknown>): Promise<T>;
  /** Hear `shares:changed`, and every (re)connect as `{what: "connected"}`. */
  onChanged(f: (p: SharesChanged) => void): () => void;
  state(): GuestState;
  onState(f: () => void): () => void;
  /** Try now: the window came back, or (`manual`) a person pressed Retry.
   *  A removed link dials only on a person's Retry. */
  retry(manual?: boolean): void;
  close(): void;
};

/** Why `cmd` is not sent on a guest connection, or null when it is a guest
 *  row. The words a stray call gets. */
export function notAGuestRow(cmd: string): string | null {
  return (GUEST_COMMANDS as readonly string[]).includes(cmd) ? null : `${cmd} is not a guest row: a guest connection sends only ${GUEST_COMMANDS.join(", ")}`;
}

export function guestLink(host: HostSummary, dial: GuestDial, env: GuestEnv): GuestLink {
  let st: GuestState = { kind: "connecting" };
  let sock: SocketDoor | null = null;
  let open: Dialled | null = null;
  let closed = false;
  let attempt = 0;
  let timer: unknown = null;
  let gen = 0;
  const stateSubs = new Set<() => void>();
  const changeSubs = new Set<(p: SharesChanged) => void>();
  const set = (s: GuestState) => {
    st = s;
    for (const f of [...stateSubs]) f();
  };
  const tell = (p: SharesChanged) => {
    for (const f of [...changeSubs]) f(p);
  };

  const schedule = (why: string) => {
    if (closed) return;
    const by = removedBy(why);
    if (by) {
      set({ kind: "removed", why, by });
      return;
    }
    const ms = RETRY_MS[Math.min(attempt, RETRY_MS.length - 1)];
    attempt += 1;
    set({ kind: "offline", why, retryAt: env.now() + ms });
    timer = env.setTimeout(() => {
      timer = null;
      // A hidden window does not dial; it tries when it is seen (`retry`).
      if (env.visible()) void connect();
      else set({ kind: "offline", why, retryAt: null });
    }, ms);
  };

  async function connect() {
    if (closed) return;
    if (timer !== null) { env.clearTimeout(timer); timer = null; }
    const mine = ++gen;
    sock = null;
    open?.close();
    open = null;
    set({ kind: "connecting" });
    let d: Dialled | null = null;
    const lost = (msg: string) => {
      if (mine !== gen || closed) return;
      const why = d?.closedWhy();
      sock = null;
      open?.close();
      open = null;
      schedule(why && !msg.includes(why) ? `${msg} (${why})` : msg);
    };
    try {
      d = await dial(lost);
      if (mine !== gen || closed) { d.close(); return; }
      open = d;
      const s = d.sock;
      // Wired before the proof: a push right behind the host's ok is handed
      // on as the proof passes, ahead of anything awaiting `ready`.
      void s.listen(SHARES_CHANGED, ((e) => tell((e.payload ?? {}) as SharesChanged)) as EventCallback<unknown>);
      s.declare([SHARES_CHANGED]);
      await s.ready;
      if (mine !== gen || closed) return;
      sock = s;
      attempt = 0;
      set({ kind: "up" });
      tell({ what: "connected" });
    } catch (e) {
      if (mine !== gen || closed) return;
      sock = null;
      schedule(typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
    }
  }

  void connect();

  return {
    host,
    call<T>(cmd: GuestCommand, args?: Record<string, unknown>): Promise<T> {
      const refused = notAGuestRow(cmd);
      if (refused) return Promise.reject(refused);
      if (!sock) return Promise.reject(st.kind === "offline" || st.kind === "removed" ? st.why : `${host.name} is not connected yet`);
      return sock.invoke<T>(cmd, args);
    },
    onChanged(f) {
      changeSubs.add(f);
      return () => { changeSubs.delete(f); };
    },
    state: () => st,
    onState(f) {
      stateSubs.add(f);
      return () => { stateSubs.delete(f); };
    },
    retry(manual = false) {
      if (closed || st.kind === "up" || st.kind === "connecting") return;
      if (st.kind === "removed" && !manual) return;
      void connect();
    },
    close() {
      closed = true;
      gen += 1;
      if (timer !== null) env.clearTimeout(timer);
      sock = null;
      open?.close();
      open = null;
    },
  };
}

/** The unlisten a door hands back, for a caller that keeps one. */
export type { UnlistenFn };
