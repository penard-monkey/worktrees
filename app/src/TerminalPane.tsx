import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Channel, invoke } from "@tauri-apps/api/core";
import "@xterm/xterm/css/xterm.css";

// Embeds a live tmux session. Rust attaches (never owns a shell); this component
// renders the byte stream and forwards keystrokes + resizes. Font comes from the
// independent --term-* CSS vars (Settings), so UI zoom never disturbs the grid.
function termFont() {
  const cs = getComputedStyle(document.documentElement);
  const family = cs.getPropertyValue("--term-family").trim() || 'Menlo, Monaco, monospace';
  const size = parseInt(cs.getPropertyValue("--term-size"), 10) || 13;
  const bg = cs.getPropertyValue("--bg-abyss").trim() || "#0f0f16";
  return { family, size, bg };
}

export function TerminalPane({ session, termVersion = 0 }: { session: string; termVersion?: number }) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const idRef = useRef<number | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;

    const { family, size, bg } = termFont();
    const term = new Terminal({
      fontFamily: family,
      fontSize: size,
      cursorBlink: true,
      theme: { background: bg },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    const safeFit = () => { try { fit.fit(); } catch { /* renderer not measured yet */ } };
    safeFit();
    termRef.current = term;
    fitRef.current = fit;

    const onBytes = new Channel<ArrayBuffer>();
    onBytes.onmessage = (msg) => term.write(new Uint8Array(msg));

    (async () => {
      try {
        const id = await invoke<number>("term_open", { session, cols: term.cols, rows: term.rows, onBytes });
        if (disposed) {
          await invoke("term_close", { id });
          return;
        }
        idRef.current = id;
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
      if (idRef.current != null) invoke("term_resize", { id: idRef.current, cols: term.cols, rows: term.rows });
    });
    ro.observe(host);

    return () => {
      disposed = true;
      ro.disconnect();
      if (idRef.current != null) invoke("term_close", { id: idRef.current }); // detach, not kill
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      idRef.current = null;
    };
  }, [session]);

  // live re-fit when Settings change the terminal font
  useEffect(() => {
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    const { family, size } = termFont();
    term.options.fontFamily = family;
    term.options.fontSize = size;
    try {
      fit.fit();
    } catch {
      /* ignore */
    }
    if (idRef.current != null) invoke("term_resize", { id: idRef.current, cols: term.cols, rows: term.rows });
  }, [termVersion]);

  return <div ref={hostRef} className="term-host" />;
}
