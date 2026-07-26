import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Channel, invoke } from "@tauri-apps/api/core";
import "@xterm/xterm/css/xterm.css";

// Embeds a live tmux session. Rust attaches (never owns a shell); this component
// renders the byte stream and forwards keystrokes + resizes. Font comes from the
// independent --term-* CSS vars (Settings), so UI zoom never disturbs the grid.
// Colors come from the active [data-theme]'s --term-*/--ansi-* vars (tokens.css)
// so the terminal repaints with the rest of the app on a theme switch.
const ANSI = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow",
  "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
] as const;

function termOptions() {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
  const family = v("--term-family", "Menlo, Monaco, monospace");
  const size = parseInt(cs.getPropertyValue("--term-size"), 10) || 13;
  const bg = v("--term-bg", "#0f0f16");
  const theme: Record<string, string> = {
    background: bg,
    foreground: v("--term-fg", "#c0caf5"),
    cursor: v("--term-cursor", "#c0caf5"),
    cursorAccent: bg,
    selectionBackground: v("--term-sel", "rgba(122, 162, 247, 0.3)"),
  };
  ANSI.forEach((name, i) => (theme[name] = v(`--ansi-${i}`, theme.foreground)));
  return { family, size, theme };
}

export function TerminalPane({ session, termVersion = 0, focusToken = 0 }: { session: string; termVersion?: number; focusToken?: number }) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const idRef = useRef<number | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;

    const { family, size, theme } = termOptions();
    const term = new Terminal({
      fontFamily: family,
      fontSize: size,
      cursorBlink: true,
      theme,
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

  // Re-grab keyboard focus when the user re-enters the place (clicking any
  // chrome — rows, pin, popovers — moves focus there and nothing else returns
  // it; xterm only self-focuses on a click inside its own canvas).
  useEffect(() => {
    termRef.current?.focus();
  }, [focusToken]);

  // live re-fit when Settings change the terminal font or theme
  useEffect(() => {
    const term = termRef.current;
    const fit = fitRef.current;
    if (!term || !fit) return;
    const { family, size, theme } = termOptions();
    term.options.fontFamily = family;
    term.options.fontSize = size;
    term.options.theme = theme;
    try {
      fit.fit();
    } catch {
      /* ignore */
    }
    if (idRef.current != null) invoke("term_resize", { id: idRef.current, cols: term.cols, rows: term.rows });
  }, [termVersion]);

  return <div ref={hostRef} className="term-host" />;
}
