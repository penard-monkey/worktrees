// The remote door's socket: `ipcws.ts`, UNCHANGED, running over a structural
// `Sock` built from the shim's three pipe commands (Phase 2 §4.4).
//
// A page's own WebSocket cannot pin a certificate or present a client key, and
// a key the page could use is a key an XSS in the page could use. So the socket
// to another host lives in the shim, in Rust (`app/src-tauri/src/remote.rs`,
// over `crates/worktrees-remote`), and this file makes it look like the
// browser's WebSocket to `connectSocket`:
//
//   remote_connect {hostId, onFrame}  → {conn, token, host}; then onFrame gets
//                                       {frame: text}… and, last, {closed: why}
//   remote_send {conn, text}          one frame out
//   remote_close {conn}
//
// The shim is a PIPE: it parses no frame. The protocol — handshake v1, the
// held frames, `declare`, channels, events — is ipcws.ts's, its one
// implementation. `token` is the session's handshake-v1 key: the TLS
// exporter under EXPORTER-worktrees-remote-v1, which only the shim's TLS
// session can use (a departure from §4.5's first frame, decided by (main) so
// that ipcws.ts stays the protocol's one client). The page never sees the
// device key or the host's pin.
//
// Two rules this file keeps, and transport-check.mjs drives:
// - Frames go out IN ORDER. Each `remote_send` is its own Tauri invoke, and
//   async commands run concurrently in the shim, so two sends in flight could
//   reach the socket swapped — the 26-keystroke reordering serve's per-stream
//   lanes exist for. So a send waits for the one before it.
// - A frame that arrives before ipcws has wired `onmessage` is held, and so is
//   the close: nothing is dropped on the way in.
//
// No runtime imports (transport-check loads this as a `data:` module).
import type { Channel, Door } from "./ipc";
import type { Sock } from "./ipcws";

/** What the shim tells the page about a host: never its pin, never a key. */
export type HostSummary = { id: string; name: string; addr: string; device_id: string; expiry: unknown; paired_at: number };

/** One message on `remote_connect`'s channel. */
export type PipeMsg = { frame: string } | { closed: string };

export type Connected = { conn: number; token: string; host: HostSummary };

/** The shim's three pipe commands. */
export type RemoteShim = {
  connect(hostId: string, onFrame: (m: PipeMsg) => void): Promise<Connected>;
  send(conn: number, text: string): Promise<void>;
  close(conn: number): Promise<void>;
};

/** The pipe commands, through a door's Tauri half. */
export function shimOf(door: Pick<Door, "invoke" | "channel">): RemoteShim {
  return {
    connect: (hostId, onFrame) => {
      const ch: Channel<PipeMsg> = door.channel<PipeMsg>(onFrame);
      return door.invoke<Connected>("remote_connect", { hostId, onFrame: ch });
    },
    send: (conn, text) => door.invoke<void>("remote_send", { conn, text }),
    close: (conn) => door.invoke<void>("remote_close", { conn }),
  };
}

/** An open pipe: the shim's answer, a `WebSocket` constructor for
 *  `connectSocket` that yields the ONE socket over it, and why it ended. */
export type OpenPipe = {
  connected: Connected;
  WebSocket: new (url: string) => Sock;
  /** The shim's words for the end (a revoke says so), once it has ended. */
  closedWhy(): string | null;
};

type Timers = { setTimeout: (fn: () => void, ms: number) => unknown };

/** Open the pipe to `hostId`. Rejects with the shim's words if it cannot. */
export async function openPipe(shim: RemoteShim, hostId: string, timers: Timers): Promise<OpenPipe> {
  const early: PipeMsg[] = [];
  let sock: PipeSock | null = null;
  let why: string | null = null;
  const take = (m: PipeMsg) => {
    if (sock && sock.live()) sock.take(m);
    else early.push(m);
  };
  const connected = await shim.connect(hostId, take);
  const conn = connected.conn;

  class PipeSock implements Sock {
    onopen: Sock["onopen"] = null;
    onmessage: Sock["onmessage"] = null;
    onclose: Sock["onclose"] = null;
    onerror: Sock["onerror"] = null;
    private opened = false;
    private done = false;
    /** The send in flight, which the next one waits for. */
    private chain: Promise<unknown> = Promise.resolve();

    constructor(_url: string) {
      if (sock) throw new Error("a remote pipe carries one socket");
      sock = this;
      // ipcws wires its handlers right after constructing; open after that.
      timers.setTimeout(() => this.open(), 0);
    }

    live(): boolean {
      return this.opened && !this.done;
    }

    private open() {
      if (this.done) return;
      this.opened = true;
      this.onopen?.call(this as unknown as WebSocket, new Event("open"));
      for (const m of early.splice(0)) this.take(m);
    }

    take(m: PipeMsg) {
      if (this.done) return;
      if ("frame" in m) this.onmessage?.call(this as unknown as WebSocket, { data: m.frame } as MessageEvent);
      else {
        why = m.closed;
        this.end();
      }
    }

    private end() {
      if (this.done) return;
      this.done = true;
      this.onclose?.call(this as unknown as WebSocket, { reason: why ?? "" } as CloseEvent);
    }

    send(text: string) {
      if (!this.live()) throw new Error("send on a remote pipe that is not open");
      this.chain = this.chain
        .then(() => shim.send(conn, text))
        .catch((e: unknown) => {
          // The pipe is gone under us: say so once, through onclose.
          if (why === null) why = String(e);
          this.end();
        });
    }

    close() {
      if (this.done) return;
      this.done = true;
      // After whatever is still going out; nothing more is delivered.
      this.chain = this.chain.then(() => shim.close(conn)).catch(() => {});
    }
  }

  return { connected, WebSocket: PipeSock as unknown as new (url: string) => Sock, closedWhy: () => why };
}
