import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Channel, invoke } from "@tauri-apps/api/core";
import "@xterm/xterm/css/xterm.css";

// Embeds a live tmux session. The Rust side attaches (never owns a shell); this
// component just renders the byte stream and forwards keystrokes + resizes.
export function TerminalPane({ session }: { session: string }) {
  const hostRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;
    let termId: number | null = null;

    const term = new Terminal({
      fontFamily: "Menlo, Monaco, monospace",
      fontSize: 13,
      cursorBlink: true,
      theme: { background: "#16161e" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();

    // Raw PTY bytes from Rust (InvokeResponseBody::Raw) → xterm.
    const onBytes = new Channel<ArrayBuffer>();
    onBytes.onmessage = (msg) => term.write(new Uint8Array(msg));

    (async () => {
      try {
        const id = await invoke<number>("term_open", {
          session,
          cols: term.cols,
          rows: term.rows,
          onBytes,
        });
        if (disposed) {
          await invoke("term_close", { id });
          return;
        }
        termId = id;
        term.onData((data) => {
          invoke("term_write", { id, data: Array.from(new TextEncoder().encode(data)) });
        });
        term.focus();
      } catch (e) {
        term.writeln(`\r\n\x1b[31m[worktrees] attach failed: ${e}\x1b[0m\r\n`);
      }
    })();

    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
      } catch {
        /* host detached mid-resize */
      }
      if (termId != null) {
        invoke("term_resize", { id: termId, cols: term.cols, rows: term.rows });
      }
    });
    ro.observe(host);

    return () => {
      disposed = true;
      ro.disconnect();
      if (termId != null) invoke("term_close", { id: termId }); // detach, not kill
      term.dispose();
    };
  }, [session]);

  return <div ref={hostRef} className="term-host" />;
}
