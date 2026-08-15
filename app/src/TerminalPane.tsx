import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Channel, invoke } from "@tauri-apps/api/core";
import { FindBar, findColors } from "./Find";
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
  const searchRef = useRef<SearchAddon | null>(null);
  // Bumped whenever the terminal below is re-created. Refs cannot wake an
  // effect, so anything that has to re-subscribe to THIS xterm (the find bar's
  // results event) depends on this instead.
  const [epoch, setEpoch] = useState(0);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let disposed = false;

    const { family, size, theme } = termOptions();
    // allowProposedApi: the search addon paints its match highlights through
    // `registerDecoration`, which xterm 5.5 still gates behind this flag — it
    // throws without it, and only once you actually search, so the terminal
    // looks fine right up until ⌘F.
    const term = new Terminal({
      fontFamily: family, fontSize: size, cursorBlink: blinkWanted(), theme,
      allowProposedApi: true,
    });
    liveTerms.add(term);
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    // ⌘F. Loaded AFTER open(): the addon subscribes to the render service, and
    // activating it against a terminal that has not been opened yet leaves the
    // viewport syncing against dimensions that do not exist. `highlightLimit`
    // caps how many matches get a decoration; past it the addon reports
    // resultIndex -1 rather than lying about where you are.
    const search = new SearchAddon({ highlightLimit: 2000 });
    term.loadAddon(search);
    const safeFit = () => { try { fit.fit(); } catch { /* renderer not measured yet */ } };
    safeFit();
    termRef.current = term;
    fitRef.current = fit;
    searchRef.current = search;
    setEpoch((v) => v + 1);

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
      term.dispose(); // disposes the loaded addons with it
      termRef.current = null;
      fitRef.current = null;
      searchRef.current = null;
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

  return { hostRef, termRef, searchRef, epoch };
}

/** What ⌘F can and cannot see here. xterm only holds what the app RECEIVED:
 *  attaching to tmux replays the visible screen, not tmux's scrollback, so a
 *  fresh attach starts with almost nothing to search. Said out loud in the
 *  field's tooltip rather than left to be discovered. */
const TERM_HINT =
  "Searches this terminal's output since it was attached (not tmux's own history)";

/** Run a search-addon call without letting it take the pane down with it. The
 *  addon throws for reasons that have nothing to do with the terminal being
 *  usable (a proposed-API gate, a dispose landing mid-search), and an
 *  unhandled throw inside an effect unmounts the whole surface — a blank
 *  terminal because a search failed. Nothing is swallowed: it goes to the app
 *  log the same way `fail()` does on the frontend. */
function guard<T>(what: string, fn: () => T): T | undefined {
  try {
    return fn();
  } catch (e) {
    invoke("log_event", { level: "error", msg: `terminal find (${what}): ${e}` }).catch(() => {});
    return undefined;
  }
}

export type TermFindProps = {
  /** the find bar is showing on THIS pane — App keeps it exclusive */
  findOpen?: boolean;
  /** bumps on every ⌘F, so a second press re-selects the field */
  findToken?: number;
  onFindClose?: () => void;
};

/** The rendered pane: xterm plus its find bar. Both kinds of terminal share it,
 *  so find behaves identically in the main pane and in a dock shell tab. */
function TermSurface({ makeTransport, tkey, termVersion, focusToken, findOpen = false, findToken = 0, onFindClose }: {
  makeTransport: () => Transport; tkey: string; termVersion: number; focusToken: number;
} & TermFindProps) {
  const { hostRef, termRef, searchRef, epoch } = useTerm(makeTransport, tkey, termVersion, focusToken);
  const [query, setQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [res, setRes] = useState({ index: 0, count: 0 });

  // Search options, colours read live so they always match the current theme.
  // Held in a REF as well: the effects below must fire on what actually changed
  // (the query, the case flag, the theme) and not merely because this function
  // has a new identity — every `findNext` moves the selection on, so an effect
  // that re-runs for no reason silently advances the user a match.
  const opts = useCallback(() => {
    const c = findColors();
    return {
      caseSensitive,
      decorations: {
        matchBackground: c.hit,
        matchOverviewRuler: c.hit,
        activeMatchBackground: c.on,
        activeMatchColorOverviewRuler: c.on,
      },
    };
  }, [caseSensitive]);
  const optsRef = useRef(opts);
  optsRef.current = opts;

  useEffect(() => {
    const s = searchRef.current;
    if (!s) return;
    // resultIndex is -1 when the match count blew past the highlight limit.
    const d = s.onDidChangeResults(({ resultIndex, resultCount }) =>
      setRes({ index: resultIndex < 0 ? 0 : resultIndex + 1, count: resultCount }));
    return () => d.dispose();
  }, [searchRef, epoch]);

  // Live search as the query changes. `incremental` keeps the selection anchored
  // on the hit you are already on while you keep typing, instead of jumping.
  useEffect(() => {
    const s = searchRef.current;
    if (!s) return;
    if (!findOpen || !query) {
      guard("clear", () => s.clearDecorations());
      setRes({ index: 0, count: 0 });
      return;
    }
    guard("type", () => s.findNext(query, { incremental: true, ...optsRef.current() }));
  }, [searchRef, findOpen, query, caseSensitive, epoch]);

  // Repaint the matches on a theme switch. Handing `findNext` the new colours is
  // NOT enough: the addon re-highlights only when the TERM or
  // case/regex/wholeWord changed — `_didOptionsChange` never looks at
  // `decorations` — so every non-active match would keep the old theme's hex on
  // the new theme's background. `clearDecorations()` drops the cached term,
  // which forces the next search to lay them all down again. It also makes
  // `_findNextAndSelect` measure from the selection's START rather than its
  // end, so the user stays on the match they were on. Skipped on the first run:
  // nothing is painted yet.
  const themeSeen = useRef(false);
  useEffect(() => {
    if (!themeSeen.current) { themeSeen.current = true; return; }
    const s = searchRef.current;
    if (!s || !findOpen || !query) return;
    guard("theme", () => { s.clearDecorations(); s.findNext(query, optsRef.current()); });
    // Only the theme belongs here; the effect above owns query/case changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [termVersion]);

  const next = useCallback(() => {
    if (query) guard("next", () => searchRef.current?.findNext(query, optsRef.current()));
  }, [searchRef, query]);
  const prev = useCallback(() => {
    if (query) guard("prev", () => searchRef.current?.findPrevious(query, optsRef.current()));
  }, [searchRef, query]);
  const close = useCallback(() => {
    guard("clear", () => searchRef.current?.clearDecorations());
    onFindClose?.();
    termRef.current?.focus();
  }, [searchRef, termRef, onFindClose]);

  return (
    <div className="term-wrap">
      <div ref={hostRef} className="term-host" />
      {findOpen && (
        <FindBar
          query={query} onQuery={setQuery}
          // index 0 with matches present means the addon gave up placing you
          // (more matches than `highlightLimit`) — `capped` renders that as
          // "2000+" instead of a nonsensical "0/2000".
          index={res.index} count={res.count} capped={res.count > 0 && res.index === 0}
          caseSensitive={caseSensitive} onCaseSensitive={setCaseSensitive}
          onNext={next} onPrev={prev} onClose={close}
          focusToken={findToken} hint={TERM_HINT}
        />
      )}
    </div>
  );
}

export function TerminalPane({ session, termVersion = 0, focusToken = 0, ...find }: {
  session: string; termVersion?: number; focusToken?: number;
} & TermFindProps) {
  return (
    <TermSurface makeTransport={() => tmuxTransport(session)} tkey={session}
      termVersion={termVersion} focusToken={focusToken} {...find} />
  );
}

export function ShellPane({ repo, slug, index, termVersion = 0, focusToken = 0, ...find }: {
  repo: string; slug: string; index: number; termVersion?: number; focusToken?: number;
} & TermFindProps) {
  return (
    <TermSurface makeTransport={() => shellTransport(repo, slug, index)}
      tkey={`${repo}|${slug}|${index}`}
      termVersion={termVersion} focusToken={focusToken} {...find} />
  );
}
