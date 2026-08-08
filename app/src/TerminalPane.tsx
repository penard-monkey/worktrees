import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Channel, invoke } from "@tauri-apps/api/core";
import "@xterm/xterm/css/xterm.css";

// Two kinds of embedded terminal, one renderer.
//
//   TerminalPane — the place's canonical tmux session. Rust ATTACHES; tmux owns
//     the shell, the panes and the scrollback, and unmounting detaches. This is
//     where Claude runs, so it survives quitting the app.
//   ShellPane    — a dock scratch shell. Rust OWNS the PTY (no tmux), so
//     unmounting must DETACH, never kill: a tab flip or ⌘J can't be allowed to
//     take down a running build. The backend replays a ring buffer on re-attach.
//
// Font comes from the independent --term-* CSS vars (Settings), so UI zoom never
// disturbs the grid. Colors come from the active [data-theme]'s --term-*/--ansi-*
// vars (tokens.css) so the terminal repaints with the rest of the app on a theme
// switch.
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

// ── cursor blink, gated on the window ──────────────────────────────────────
// A blinking cursor is a style recalc + paint twice a second, per mounted
// terminal, forever — and xterm 5.5 has no "stop blinking when idle" (that
// landed upstream in 7.0). xterm's own gate is the `.xterm-focus` class, which
// is useless here: `useTerm` force-focuses the pane on mount and on every
// re-entry, and element focus does NOT drop when the OS window deactivates. So
// the blink ran whenever the app was open, whether or not anyone was there.
//
// Every live terminal registers here and follows the window instead. Listeners
// are module-scope and deliberately never removed — they outlive any single
// pane and cost one function call per window event.
const liveTerms = new Set<Terminal>();
const blinkWanted = () => document.visibilityState !== "hidden" && document.hasFocus();
const applyBlink = () => {
  const on = blinkWanted();
  liveTerms.forEach((t) => { t.options.cursorBlink = on; });
};
window.addEventListener("focus", applyBlink);
window.addEventListener("blur", applyBlink);
document.addEventListener("visibilitychange", applyBlink);

/** How one pane talks to its backend. `close` is the unmount path and means
 * "stop streaming" for BOTH kinds — detach the tmux client, or drop the sink on
 * an owned shell. Neither ends the thing on the other side. */
type Transport = {
  open(cols: number, rows: number, onBytes: Channel<ArrayBuffer>): Promise<void>;
  write(data: number[]): void;
  resize(cols: number, rows: number): void;
  close(): void;
};

const tmuxTransport = (session: string): Transport => {
  let id: number | null = null;
  return {
    async open(cols, rows, onBytes) {
      id = await invoke<number>("term_open", { session, cols, rows, onBytes });
    },
    write: (data) => { if (id != null) invoke("term_write", { id, data }); },
    resize: (cols, rows) => { if (id != null) invoke("term_resize", { id, cols, rows }); },
    close: () => { if (id != null) invoke("term_close", { id }); id = null; },
  };
};

const shellTransport = (repo: string, slug: string, index: number): Transport => {
  // The attach generation from shell_open. Detach presents it so a STALE
  // detach (StrictMode: unmount №1 resolving after mount №2 attached) is a
  // backend no-op instead of clearing the new attach's stream.
  let gen: number | null = null;
  return {
    async open(cols, rows, onBytes) {
      gen = await invoke<number>("shell_open", { repo, slug, index, cols, rows, onBytes });
    },
    write: (data) => { invoke("shell_write", { repo, slug, index, data }); },
    resize: (cols, rows) => { invoke("shell_resize", { repo, slug, index, cols, rows }); },
    close: () => { if (gen != null) invoke("shell_detach", { repo, slug, index, gen }); gen = null; },
  };
};

/** The xterm instance + wiring. `key` re-creates everything when it changes. */
function useTerm(makeTransport: () => Transport, key: string, termVersion: number, focusToken: number) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const txRef = useRef<Transport | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;

    const { family, size, theme } = termOptions();
    const term = new Terminal({ fontFamily: family, fontSize: size, cursorBlink: blinkWanted(), theme });
    liveTerms.add(term);
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    const safeFit = () => { try { fit.fit(); } catch { /* renderer not measured yet */ } };
    safeFit();
    termRef.current = term;
    fitRef.current = fit;

    const tx = makeTransport();
    txRef.current = tx;

    const onBytes = new Channel<ArrayBuffer>();
    onBytes.onmessage = (msg) => term.write(new Uint8Array(msg));

    (async () => {
      try {
        await tx.open(term.cols, term.rows, onBytes);
        if (disposed) {
          tx.close();
          return;
        }
        term.onData((data) => tx.write(Array.from(new TextEncoder().encode(data))));
        term.focus();
      } catch (e) {
        term.writeln(`\r\n\x1b[31m[worktrees] ${e}\x1b[0m\r\n`);
      }
    })();

    const ro = new ResizeObserver(() => {
      try {
        fit.fit();
      } catch {
        /* host detached mid-resize */
      }
      tx.resize(term.cols, term.rows);
    });
    ro.observe(host);

    return () => {
      disposed = true;
      ro.disconnect();
      tx.close(); // detach, not kill — for either backend
      liveTerms.delete(term);
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      txRef.current = null;
    };
    // makeTransport is re-created every render; `key` is the real identity
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

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
    txRef.current?.resize(term.cols, term.rows);
  }, [termVersion]);

  return hostRef;
}

export function TerminalPane({ session, termVersion = 0, focusToken = 0 }: { session: string; termVersion?: number; focusToken?: number }) {
  const hostRef = useTerm(() => tmuxTransport(session), session, termVersion, focusToken);
  return <div ref={hostRef} className="term-host" />;
}

export function ShellPane({ repo, slug, index, termVersion = 0, focusToken = 0 }: {
  repo: string; slug: string; index: number; termVersion?: number; focusToken?: number;
}) {
  const hostRef = useTerm(
    () => shellTransport(repo, slug, index),
    `${repo}|${slug}|${index}`,
    termVersion,
    focusToken,
  );
  return <div ref={hostRef} className="term-host" />;
}
