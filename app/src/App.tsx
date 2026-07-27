import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { TerminalPane } from "./TerminalPane";
import { SettingsSheet } from "./SettingsSheet";
import { applySettings, clampNav, DEFAULTS, loadSettings, saveSettings, type Settings, type UpdateInfo } from "./settings";
import logoUrl from "./assets/logo.png";
import "./tokens.css";
import "./App.css";

type Declared = {
  lifecycle?: string;
  pinned?: boolean;
  note?: string;
  last_opened_epoch?: number;
} | null;

type Place = {
  slug: string;
  path: string;
  is_main: boolean;
  registered: boolean;
  branch: string | null;
  detached: boolean | null;
  dirty: boolean | null;
  dirty_files?: number | null;
  ahead: number | null;
  behind: number | null;
  tmux_session: { name: string; up: boolean };
  last_commit_epoch?: number | null;
  claude_session_present: boolean;
  declared: Declared;
  lifecycle_effective: string;
};
type Snapshot = { repo: string; prefix: string; places: Place[] };
type ProjectView = { root: string; ok: boolean; error: string | null; snapshot: Snapshot | null };
type Workspace = { projects: ProjectView[] };
type CmdResult = { ok: boolean; code: number; output: string; slug?: string | null };
type Lens = "places" | "recent" | "attention";

// ⌘1..N nav targets, in the nav's displayed top-to-bottom order: Home (clear
// selection → briefing), then the rail's lens entries. Module scope = stable.
const NAV_CHORDS: { home?: boolean; lens?: Lens }[] = [
  { home: true }, { lens: "places" }, { lens: "recent" }, { lens: "attention" },
];

const LIVE_TIERS = ["pinned", "active", "idle"] as const;
const DORMANT_TIERS = ["saved", "closed", "archived", "abandoned"] as const;
const GROUP_LABEL: Record<string, string> = {
  pinned: "Pinned", active: "Active", idle: "Idle",
  saved: "Saved", closed: "Closed", archived: "Archived", abandoned: "Abandoned",
};
const SETTABLE = [
  { label: "Close", value: "closed" },
  { label: "Save", value: "saved" },
  { label: "Archive", value: "archived" },
  { label: "Abandon", value: "abandoned" },
];
const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;
const bucketOf = (p: Place) => (p.declared?.pinned ? "pinned" : p.lifecycle_effective);
const hasAttention = (p: Place) => !!p.dirty || !!p.ahead || !!p.behind;

// nav icons — inline SVG so they inherit currentColor per theme
const FolderIcon = () => (
  <svg className="picon-svg" width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor"
    strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="M1.75 4.25c0-.83.67-1.5 1.5-1.5h2.9l1.6 1.7h5c.83 0 1.5.67 1.5 1.5v6c0 .83-.67 1.5-1.5 1.5H3.25c-.83 0-1.5-.67-1.5-1.5v-7.7Z" />
  </svg>
);
const HomeIcon = () => (
  <svg className="picon-svg" width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor"
    strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="M2.5 6.7 8 2.2l5.5 4.5V13a1 1 0 0 1-1 1H9.8v-3.8H6.2V14H3.5a1 1 0 0 1-1-1V6.7Z" />
  </svg>
);

function ago(epoch?: number): string {
  if (!epoch) return "";
  const s = Math.floor(Date.now() / 1000) - epoch;
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

// fixed-order signal glyphs; geometry (3-col row grid) guarantees no collision.
function glyphs(p: Place) {
  const g: { cls: string; text: string; title: string }[] = [];
  if (p.dirty) g.push({ cls: "g-dirty", text: `●${p.dirty_files ?? ""}`, title: `${p.dirty_files ?? "dirty"} uncommitted` });
  if (p.ahead) g.push({ cls: "g-ahead", text: `↑${p.ahead}`, title: `${p.ahead} ahead` });
  if (p.behind) g.push({ cls: "g-behind", text: `↓${p.behind}`, title: `${p.behind} behind` });
  if (p.detached) g.push({ cls: "g-det", text: "⑂", title: "detached HEAD" });
  const MAX = 4;
  if (g.length > MAX) return [...g.slice(0, MAX), { cls: "g-more", text: `+${g.length - MAX}`, title: "more" }];
  return g;
}

// Right-click context menu shell: fixed at the cursor, clamped to the viewport,
// Esc / click-away / right-click-away all close. Top-level (NOT nested in App)
// so its position state survives App re-renders.
function CtxMenu({ x, y, onClose, children }: { x: number; y: number; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState({ left: x, top: y });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - r.width - 8)),
      top: Math.max(4, Math.min(y, window.innerHeight - r.height - 8)),
    });
  }, [x, y]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <>
      <div className="menu-catch" onClick={onClose} onContextMenu={(e) => { e.preventDefault(); onClose(); }} />
      <div ref={ref} className="ctxmenu" style={pos} onContextMenu={(e) => e.preventDefault()}>
        {children}
      </div>
    </>
  );
}

type Ctx =
  | { kind: "place"; x: number; y: number; repo: string; slug: string }
  | { kind: "project"; x: number; y: number; root: string };

// Single-quote for pasting into a shell (the main session name carries parens).
const shq = (s: string) => `'${s.replace(/'/g, "'\\''")}'`;

// "v0.2.1" vs "0.2.0" — numeric per-component version compare (a > b).
const vparts = (s: string) => s.replace(/^v/, "").split(".").map((n) => parseInt(n, 10) || 0);
const vnewer = (a: string, b: string) => {
  const x = vparts(a), y = vparts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  }
  return false;
};

// Sections of the embedded CHANGELOG strictly newer than `seen`, up to `current`.
function changelogBetween(changelog: string, seen: string, current: string): string {
  const sections = changelog.split(/^## /m).slice(1);
  const out: string[] = [];
  for (const sec of sections) {
    const m = sec.match(/^\[([0-9][^\]]*)\]/);
    if (!m) continue;
    const v = m[1];
    if (vnewer(v, seen) && !vnewer(v, current)) out.push("## " + sec.trimEnd());
  }
  return out.join("\n\n");
}

// keepachangelog markdown → typed sections, so "What's new" renders formatted
// notes instead of raw markdown. Hard-wrapped bullet continuations are unwrapped.
type NotesGroup = { name: string; items: string[] };
type NotesSection = { version: string; date: string; groups: NotesGroup[] };

function parseNotes(md: string): NotesSection[] {
  const sections: NotesSection[] = [];
  let sec: NotesSection | null = null;
  let group: NotesGroup | null = null;
  for (const line of md.split("\n")) {
    const h2 = line.match(/^## \[([^\]]+)\](?:\s*-\s*(.*))?/);
    if (h2) {
      sec = { version: h2[1], date: (h2[2] ?? "").trim(), groups: [] };
      sections.push(sec);
      group = null;
      continue;
    }
    const h3 = line.match(/^### (.+)/);
    if (h3 && sec) {
      group = { name: h3[1].trim(), items: [] };
      sec.groups.push(group);
      continue;
    }
    const li = line.match(/^[-*] (.+)/);
    if (li && group) { group.items.push(li[1].trim()); continue; }
    if (line.trim() && group && group.items.length) {
      group.items[group.items.length - 1] += " " + line.trim();
    }
  }
  return sections.filter((s) => s.groups.some((g) => g.items.length));
}

// `code spans` → <code>; the only inline markup the changelog uses.
function renderInline(s: string): React.ReactNode[] {
  return s.split(/`([^`]+)`/g).map((part, i) => (i % 2 ? <code key={i}>{part}</code> : part));
}

function ReleaseNotes({ notes }: { notes: string }) {
  const sections = parseNotes(notes);
  if (!sections.length) return <pre className="notes">{notes}</pre>;
  return (
    <div className="relnotes">
      {sections.map((sec) => (
        <section key={sec.version}>
          <div className="rel-head">
            {sections.length > 1 && <b className="rel-v">v{sec.version}</b>}
            {sec.date && <span className="rel-date">{sec.date}</span>}
          </div>
          {sec.groups.map((g) => (
            <div key={g.name} className="rel-group">
              <span className={`rel-tag rel-${g.name.toLowerCase()}`}>{g.name}</span>
              <ul>
                {g.items.map((it, i) => <li key={i}>{renderInline(it)}</li>)}
              </ul>
            </div>
          ))}
        </section>
      ))}
    </div>
  );
}

// New-worktree form. Module scope + OWN draft state: components defined inside
// App get a fresh identity every render, which remounts their DOM and drops
// input focus per keystroke — the form must live outside that churn.
function NewPlaceForm({ project, initialBase, onCreate, onCancel }: {
  project: string;
  initialBase: string;
  onCreate: (branch: string, name: string, base: string) => void;
  onCancel: () => void;
}) {
  const [branch, setBranch] = useState("");
  const [name, setName] = useState("");
  const [base, setBase] = useState(initialBase);
  const submit = () => { if (branch.trim()) onCreate(branch.trim(), name.trim(), base.trim()); };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") submit();
    if (e.key === "Escape") onCancel();
  };
  return (
    <div className="newform nav-newform">
      <div className="newform-h">
        New worktree · <b>{basename(project)}</b>
        <button className="mini" title="cancel (Esc)" onClick={onCancel}>✕</button>
      </div>
      <input placeholder="branch (e.g. feat/x)" value={branch} autoFocus
        onChange={(e) => setBranch(e.currentTarget.value)} onKeyDown={onKey} />
      <input placeholder="name (optional)" value={name}
        onChange={(e) => setName(e.currentTarget.value)} onKeyDown={onKey} />
      <input placeholder="base (default: main)" value={base}
        onChange={(e) => setBase(e.currentTarget.value)} onKeyDown={onKey} />
      <button onClick={submit} disabled={!branch.trim()}>Create</button>
    </div>
  );
}

function App() {
  const [ws, setWs] = useState<Workspace | null>(null);
  const [err, setErr] = useState("");
  const [sel, setSel] = useState<{ repo: string; slug: string } | null>(null);
  const [filter, setFilter] = useState("");
  const [lens, setLens] = useState<Lens>("places");
  const [newFor, setNewFor] = useState<string | null>(null);
  const [newBase, setNewBase] = useState("");
  const [ctx, setCtx] = useState<Ctx | null>(null);
  const [switchTo, setSwitchTo] = useState("");
  const [confirmRm, setConfirmRm] = useState<string | null>(null);
  const [menu, setMenu] = useState<"life" | "more" | null>(null);
  const [groupOpen, setGroupOpen] = useState<Record<string, boolean>>({});
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [settings, setSettings] = useState<Settings>(DEFAULTS);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [termVersion, setTermVersion] = useState(0);
  const [termFocus, setTermFocus] = useState(0);
  const [upd, setUpd] = useState<UpdateInfo | null>(null);
  const [sortOpen, setSortOpen] = useState(false);
  const [drag, setDrag] = useState<{ repo: string; slug: string } | null>(null);
  const [dragOver, setDragOver] = useState<string | null>(null);
  const [whatsNew, setWhatsNew] = useState<{ version: string; notes: string; manual?: boolean } | null>(null);
  // Badge = actionable updates. CLI via the pinned-tag installer; the app via
  // tauri-plugin-updater (signed bundles) — both one click in Settings now.
  const cliStale = !!(upd?.latest && upd.cli_version && vnewer(upd.latest, upd.cli_version));
  const cliMissing = !!(upd?.latest && !upd.cli_version);
  const appStale = !!(upd?.latest && vnewer(upd.latest, upd.app_version));
  const updateAvail = cliStale || cliMissing || appStale;
  const searchRef = useRef<HTMLInputElement | null>(null);

  // every surfaced error also lands in the app log (Settings → Logs)
  const fail = useCallback((e: unknown) => {
    const m = String(e);
    setErr(m);
    invoke("log_event", { level: "error", msg: m }).catch(() => {});
  }, []);

  const refresh = useCallback(async () => {
    try {
      setErr("");
      setWs(await invoke<Workspace>("list_workspace"));
    } catch (e) {
      fail(e);
    }
  }, []);
  useEffect(() => { refresh(); }, [refresh]);

  // live refresh: backend emits "places:changed" (poll/fs-watch) → re-pull
  useEffect(() => {
    const un = listen("places:changed", () => refresh());
    return () => { un.then((f) => f()).catch(() => {}); };
  }, [refresh]);

  // Claude working state — pushed by the backend poll thread from the
  // ~/.claude/sessions/<pid>.json probes (see lib.rs claude_activity). Keyed by
  // WORKTREE PATH (probe cwd == place.path), not the tmux session name. Two sets:
  // busy → green blinking dot, waiting → static amber "needs input". Pure event
  // state: no extra invokes.
  const [busyPaths, setBusyPaths] = useState<Set<string>>(new Set());
  const [waitingPaths, setWaitingPaths] = useState<Set<string>>(new Set());
  useEffect(() => {
    const un = listen<{ busy: string[]; waiting: string[] }>("sessions:busy", (e) => {
      setBusyPaths(new Set(e.payload.busy));
      setWaitingPaths(new Set(e.payload.waiting));
    });
    return () => { un.then((f) => f()).catch(() => {}); };
  }, []);
  const activityOf = (p: Place): "busy" | "waiting" | "" =>
    busyPaths.has(p.path) ? "busy" : waitingPaths.has(p.path) ? "waiting" : "";

  // uncaught frontend errors → app log (the "it just didn't respond" killers)
  useEffect(() => {
    const onErr = (e: ErrorEvent) =>
      invoke("log_event", { level: "error", msg: `window: ${e.message} @ ${e.filename}:${e.lineno}` }).catch(() => {});
    const onRej = (e: PromiseRejectionEvent) =>
      invoke("log_event", { level: "error", msg: `unhandledrejection: ${String(e.reason)}` }).catch(() => {});
    window.addEventListener("error", onErr);
    window.addEventListener("unhandledrejection", onRej);
    return () => {
      window.removeEventListener("error", onErr);
      window.removeEventListener("unhandledrejection", onRej);
    };
  }, []);

  // update check: once shortly after launch (delayed off the startup path);
  // manual re-check from Settings. Offline → latest stays null, no nagging.
  const checkUpdate = useCallback(async () => {
    try { setUpd(await invoke<UpdateInfo>("check_update")); } catch { /* offline */ }
  }, []);
  // Gate on the setting AND keep it in the deps: a timer scheduled pre-hydration
  // (under DEFAULTS, update_auto_check=true) is CANCELLED by this effect's cleanup
  // when hydration lands with the toggle off — re-running the flag INSIDE the
  // callback would already have fired. Manual "Check for updates" is unaffected.
  useEffect(() => {
    if (!settings.update_auto_check) return;
    const t = setTimeout(checkUpdate, 3000);
    return () => clearTimeout(t);
  }, [checkUpdate, settings.update_auto_check]);

  // Push the auto-fetch cadence to the backend watcher (which owns the pass loop
  // but can't read the opaque settings blob). Fires post-hydration and on every
  // change of the setting; before hydration `settings` still holds DEFAULTS
  // (fetch_interval_min=0 → off), so an early push is a harmless no-op. Idempotent
  // (a plain store), so re-syncing the same value costs nothing.
  useEffect(() => {
    invoke("set_fetch_interval", { mins: settings.fetch_interval_min }).catch(() => {});
  }, [settings.fetch_interval_min]);

  // hydrate persisted settings BEFORE first meaningful paint. A pre-hydration
  // interaction (⌘B at launch) must neither be visually reverted nor let its
  // debounced save write a DEFAULTS-seeded object over the on-disk settings —
  // so: record early patches, merge them over the loaded state, and gate every
  // save on hydration having landed.
  const hydrated = useRef(false);
  const preHydration = useRef<Partial<Settings>>({});
  useLayoutEffect(() => {
    (async () => {
      const s = await loadSettings();
      const merged = { ...s, ...preHydration.current };
      hydrated.current = true;
      applySettings(merged);
      setSettings(merged);
      if (Object.keys(preHydration.current).length > 0) saveSettings(merged);
      setLens(merged.lens);
      setCollapsed(merged.collapsed ?? {});
      // release notes: embedded CHANGELOG vs last-seen version. Fresh install
      // (no last_seen) records silently; a version CHANGE shows What's-new.
      try {
        const ci = await invoke<{ version: string; changelog: string }>("get_changelog");
        if (!merged.last_seen_version) {
          const next = { ...merged, last_seen_version: ci.version };
          setSettings(next);
          saveSettings(next);
        } else if (merged.last_seen_version !== ci.version) {
          const notes = changelogBetween(ci.changelog, merged.last_seen_version, ci.version);
          if (notes) setWhatsNew({ version: ci.version, notes });
          else {
            const next = { ...merged, last_seen_version: ci.version };
            setSettings(next);
            saveSettings(next);
          }
        }
      } catch { /* harness / older backend */ }
    })();
  }, []);

  const updateSettings = (patch: Partial<Settings>) => {
    setSettings((prev) => {
      // resizing a hidden nav is dead UI — bring it back for live preview
      const auto = patch.nav_width !== undefined && prev.nav_collapsed ? { nav_collapsed: false } : null;
      const next = { ...prev, ...patch, ...auto };
      applySettings(next);
      if (hydrated.current) saveSettings(next);
      else Object.assign(preHydration.current, patch, auto);
      return next;
    });
    // theme changes the terminal colors too (xterm reads CSS vars once per version)
    if (patch.term_family !== undefined || patch.term_size !== undefined || patch.theme !== undefined ||
        patch.theme_light !== undefined || patch.theme_dark !== undefined)
      setTermVersion((v) => v + 1);
  };

  // theme "system": re-apply when macOS appearance flips
  useEffect(() => {
    if (settings.theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const onFlip = () => {
      applySettings(settings);
      setTermVersion((v) => v + 1);
    };
    mq.addEventListener("change", onFlip);
    return () => mq.removeEventListener("change", onFlip);
  }, [settings]);

  const selected: Place | null =
    (sel && ws?.projects.find((p) => p.root === sel.repo)?.snapshot?.places.find((pl) => pl.slug === sel.slug)) || null;

  // ctx target derived live from ws (a refresh while the menu is open must not go stale)
  const ctxPlace: Place | null =
    ctx?.kind === "place"
      ? ws?.projects.find((v) => v.root === ctx.repo)?.snapshot?.places.find((pl) => pl.slug === ctx.slug) ?? null
      : null;

  // if a refresh removes the ctx target, finalize the close (else a stale ctx
  // resurrects the menu — with its armed remove — when the target reappears)
  useEffect(() => {
    if (!ctx || !ws) return;
    // BOTH branches must clear the arm (closeCtx's contract, inlined here to keep
    // this effect's deps stable): a vanished target must never leave confirmRm
    // armed — for the project branch that would resurrect a one-click remove.
    if (ctx.kind === "place" && !ctxPlace) { setCtx(null); setConfirmRm(null); }
    if (ctx.kind === "project" && !ws.projects.some((v) => v.root === ctx.root)) { setCtx(null); setConfirmRm(null); }
  }, [ctx, ctxPlace, ws]);

  const mutate = async (p: Promise<unknown>) => {
    try { await p; await refresh(); } catch (e) { fail(e); }
  };
  // Returns the op's CmdResult (so callers can read result.slug), or null when
  // the invoke itself threw. On a non-ok result the error banner is set here.
  const runCmd = async (name: string, args: Record<string, unknown>): Promise<CmdResult | null> => {
    try {
      setErr("");
      const r = await invoke<CmdResult>(name, args);
      if (!r.ok) setErr(r.output || `${name} failed (exit ${r.code})`);
      await refresh();
      return r;
    } catch (e) {
      fail(e);
      return null;
    }
  };

  const addProject = async () => {
    try {
      const dir = await open({ directory: true, title: "Add a git project" });
      if (typeof dir === "string") { setErr(""); setWs(await invoke<Workspace>("add_project", { dir })); }
    } catch (e) { fail(e); }
  };
  const removeProject = async (root: string) => {
    try {
      setWs(await invoke<Workspace>("remove_project", { root }));
      if (sel?.repo === root) setSel(null);
    } catch (e) { fail(e); }
  };
  // Two-click arm for the project ctx-menu item (shares confirmRm with the
  // place-remove; key namespaced "proj|" so it never collides).
  const removeProjectCtx = (root: string) => {
    const key = `proj|${root}`;
    if (confirmRm !== key) { setConfirmRm(key); return; } // arm; menu stays open
    closeCtx();
    removeProject(root);
  };
  // Two-click arm for the project-header ✕ (key namespaced "hdr|"). Unlike the
  // popover surfaces there's no dismissal path to clear the arm, so it also
  // auto-disarms after a few seconds (effect below) and on selection change.
  const removeProjectHdr = (root: string) => {
    const key = `hdr|${root}`;
    if (confirmRm !== key) { setConfirmRm(key); return; }
    setConfirmRm(null);
    removeProject(root);
  };
  // Auto-disarm the header-✕ confirm (no popover-close to clear it otherwise).
  useEffect(() => {
    if (!confirmRm?.startsWith("hdr|")) return;
    const t = setTimeout(() => setConfirmRm(null), 4000);
    return () => clearTimeout(t);
  }, [confirmRm]);
  // Clear any header-✕ arm when the selection changes (a switch of focus means
  // the destructive intent is stale). Popover arms are cleared by their own
  // close paths; this covers the header ✕ which has none.
  useEffect(() => {
    setConfirmRm((c) => (c?.startsWith("hdr|") ? null : c));
  }, [sel]);

  // Every ctx-menu dismissal goes through here: an armed confirmRm must NEVER
  // survive the menu (it would leak into the topbar ⋯ popover as one-click remove).
  const closeCtx = () => {
    setCtx(null);
    setConfirmRm(null);
  };

  // Same guarantee for the topbar Lifecycle/⋯ popover: dismissing it must also
  // disarm the ⋯ "Remove worktree…" confirm — otherwise a stray arm survives and
  // reopening ⋯ shows a pre-armed one-click destructive remove.
  const closeMenu = () => {
    setMenu(null);
    setConfirmRm(null);
  };

  // THE primary verb: inhabit a place — stamp recency, ensure its session, select it.
  // `fresh` skips the AI auto-resume. Explicit opts.fresh (right-click override)
  // wins; otherwise the ai_auto_resume setting decides — OFF → default opens are
  // fresh. (Backend no-ops resume unless the AI command is claude anyway.)
  const enterPlace = (repo: string, p: Place, opts?: { fresh?: boolean }) => {
    setSel({ repo, slug: p.slug });
    setMenu(null);
    closeCtx();
    setTermFocus((v) => v + 1); // hand the keyboard back to the terminal
    const fresh = opts?.fresh ?? !settings.ai_auto_resume;
    (async () => {
      invoke("touch_place", { repo, slug: p.slug }).catch(() => {}); // fire-and-forget recency stamp
      await runCmd("open_place", { repo, slug: p.slug, fresh });
    })();
  };

  // ── context-menu verbs ──
  const closeSession = (repo: string, slug: string) => {
    closeCtx();
    runCmd("close_place", { repo, slug });
  };
  const copyText = (text: string) => {
    closeCtx();
    closeMenu(); // also dismiss the topbar ⋯ popover (its "Copy path" routes here)
    if (!navigator.clipboard) { fail("clipboard unavailable"); return; }
    navigator.clipboard.writeText(text).catch(fail);
  };
  const openOnRemote = async (repo: string, slug: string) => {
    closeCtx();
    try {
      const url = await invoke<string | null>("github_url", { repo, slug });
      if (url) await openUrl(url);
      else setErr("No origin remote for this project.");
    } catch (e) { fail(e); }
  };
  const revealPlace = (path: string) => {
    closeCtx();
    revealItemInDir(path).catch((e) => fail(e));
  };
  // Settings → "Release notes": the What's-new sheet again, with the FULL
  // released history (every section ≤ the running version), on top of Settings.
  const showReleaseNotes = async () => {
    try {
      const ci = await invoke<{ version: string; changelog: string }>("get_changelog");
      const notes = changelogBetween(ci.changelog, "0", ci.version);
      setWhatsNew({ version: ci.version, notes: notes || ci.changelog, manual: true });
    } catch (e) { fail(e); }
  };
  const editIn = (path: string) => {
    closeCtx();
    invoke("open_editor", { path, cmd: settings.editor_cmd }).catch((e) => fail(e));
  };
  // Settings → Data → "Reset to defaults". Preserve last_seen_version so the
  // What's-new sheet doesn't re-fire, push through the same apply/persist/refit
  // path as every other change, and reset the React state that MIRRORS settings
  // (lens + per-project collapsed) so the UI doesn't show stale nav state.
  const onReset = () => {
    const next = { ...DEFAULTS, last_seen_version: settings.last_seen_version };
    updateSettings(next);
    setLens(next.lens);
    setCollapsed(next.collapsed);
  };
  const editNote = (repo: string, p: Place) => {
    closeCtx();
    setSel({ repo, slug: p.slug });
    setTimeout(() => document.querySelector<HTMLInputElement>(".note-strip")?.focus(), 60);
  };
  // the form lives in the nav — a ctx-menu path must first bring the nav back
  const openNewForm = (root: string, base: string) => {
    setNewFor(root);
    setNewBase(base);
    if (settings.nav_collapsed) toggleNav();
  };
  const newFromBranch = (root: string, base: string) => {
    closeCtx();
    openNewForm(root, base);
  };
  // Unarmed click arms (menu stays open, showing the two danger buttons below).
  const armRemovePlaceCtx = (repo: string, slug: string) => {
    setConfirmRm(`ctx|${repo}|${slug}`); // namespaced: never matches the topbar's key
  };
  // Armed confirm. `delBranch` picks the button: false = remove only, true =
  // remove + branch. With force:false the core uses `git branch -d`, so only a
  // MERGED branch is deleted; an unmerged one degrades to a warning while the
  // remove still succeeds — del_branch is safe by construction.
  const confirmRemovePlaceCtx = async (repo: string, slug: string, delBranch: boolean) => {
    closeCtx();
    if ((await runCmd("remove_place", { repo, slug, del_branch: delBranch, force: false }))?.ok) {
      if (sel?.repo === repo && sel?.slug === slug) setSel(null);
    }
  };
  const placeCtx = (e: React.MouseEvent, repo: string, p: Place) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu(null);
    setConfirmRm(null);
    setCtx({ kind: "place", x: e.clientX, y: e.clientY, repo, slug: p.slug });
  };
  const projectCtx = (e: React.MouseEvent, root: string) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu(null);
    setConfirmRm(null);
    setCtx({ kind: "project", x: e.clientX, y: e.clientY, root });
  };

  // ── nav sorting (Settings-persisted; Manual = drag) ──
  const recencyOf = (p: Place) => p.declared?.last_opened_epoch ?? p.last_commit_epoch ?? 0;
  const sortPlaces = (repo: string, arr: Place[]): Place[] => {
    const out = [...arr];
    if (settings.sort_mode === "manual") {
      const order = settings.manual_order[repo] ?? [];
      const idx = (p: Place) => { const i = order.indexOf(p.slug); return i < 0 ? order.length : i; };
      out.sort((a, b) => idx(a) - idx(b));
      return out;
    }
    if (settings.sort_mode === "alpha") out.sort((a, b) => a.slug.localeCompare(b.slug));
    else out.sort((a, b) => recencyOf(b) - recencyOf(a));
    const flip = settings.sort_mode === "alpha" ? settings.sort_dir === "desc" : settings.sort_dir === "asc";
    if (flip) out.reverse();
    return out;
  };
  const dropOnRow = (repo: string, targetSlug: string) => {
    if (!drag || drag.repo !== repo || drag.slug === targetSlug) { setDrag(null); setDragOver(null); return; }
    const pv = ws?.projects.find((v) => v.root === repo);
    const slugs = (pv?.snapshot?.places ?? []).filter((p) => !p.is_main).map((p) => p.slug);
    const existing = settings.manual_order[repo] ?? [];
    const order = [...existing.filter((x) => slugs.includes(x)), ...slugs.filter((x) => !existing.includes(x))];
    const from = order.indexOf(drag.slug);
    if (from >= 0) order.splice(from, 1);
    const to = order.indexOf(targetSlug);
    order.splice(to < 0 ? order.length : to, 0, drag.slug);
    updateSettings({ manual_order: { ...settings.manual_order, [repo]: order } });
    setDrag(null);
    setDragOver(null);
  };

  const createPlace = async (repo: string, branch: string, name: string, base: string) => {
    if (!branch) return;
    const r = await runCmd("new_place", { repo, branch, base: base || null, name: name || null });
    if (r?.ok) {
      // Select core's ACTUAL final slug (origin/ stripped, holder-reuse applied);
      // fall back to the old derivation only if the backend didn't report one.
      setSel({ repo, slug: r.slug ?? (name || branch).replace(/\//g, "-") });
      setNewFor(null);
      setNewBase("");
    }
  };
  const doSwitch = async () => {
    if (!sel) return;
    const b = switchTo.trim();
    if (!b) return;
    if ((await runCmd("switch_place", { repo: sel.repo, slug: sel.slug, branch: b, base: null }))?.ok) setSwitchTo("");
  };
  // Topbar ⋯ remove — same armed two-button pair as the ctx menu (arm key
  // `repo|slug`). Unarmed click arms; the armed state renders "Confirm remove"
  // (del_branch:false) + "Confirm remove + branch" (del_branch:true).
  const armRemove = () => {
    if (!sel) return;
    setConfirmRm(`${sel.repo}|${sel.slug}`);
  };
  const confirmRemove = async (delBranch: boolean) => {
    if (!sel) return;
    closeMenu();
    if ((await runCmd("remove_place", { repo: sel.repo, slug: sel.slug, del_branch: delBranch, force: false }))?.ok) setSel(null);
  };

  const toggleProject = (root: string) => {
    setCollapsed((c) => {
      const next = { ...c, [root]: !c[root] };
      updateSettings({ collapsed: next });
      return next;
    });
  };
  const isOpen = (key: string, def: boolean) => groupOpen[key] ?? def;
  const toggleGroup = (key: string, def: boolean) =>
    setGroupOpen((g) => ({ ...g, [key]: !(g[key] ?? def) }));

  // rail-only mode: hide the nav column, keep the rail (persisted in ui-state)
  const toggleNav = useCallback(() => {
    setSettings((prev) => {
      const next = { ...prev, nav_collapsed: !prev.nav_collapsed };
      applySettings(next);
      if (hydrated.current) saveSettings(next);
      else preHydration.current.nav_collapsed = next.nav_collapsed;
      return next;
    });
  }, []);
  // keyboard lens select (⌘2..N): unlike changeLens, re-selecting the active lens
  // must NOT toggle/collapse the nav — a keyboard chord always REVEALS the view.
  const selectLens = useCallback((l: Lens) => {
    setLens(l);
    setSettings((prev) => {
      if (prev.lens === l && !prev.nav_collapsed) return prev;
      const next = { ...prev, lens: l, nav_collapsed: false };
      applySettings(next);
      if (hydrated.current) saveSettings(next);
      else Object.assign(preHydration.current, { lens: l, nav_collapsed: false });
      return next;
    });
  }, []);
  // ⌘-digit / ⌘E read live state (selection, editor cmd) — hold it in a ref so
  // the keydown listener stays stable (registered once, no per-render churn).
  const keyRef = useRef({ selectLens, selectedPath: null as string | null, editorCmd: settings.editor_cmd });
  keyRef.current = { selectLens, selectedPath: selected?.path ?? null, editorCmd: settings.editor_cmd };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.repeat || e.shiftKey || e.altKey) return;
      const k = e.key.toLowerCase();
      if (k === "b") {
        // Ctrl+B is the tmux prefix — let the embedded terminal keep it; ⌘B still toggles
        if (e.ctrlKey && !e.metaKey && e.target instanceof Element && e.target.closest(".term-host")) return;
        e.preventDefault();
        toggleNav();
      } else if (e.metaKey && e.key === ",") {
        // ⌘, opens Settings (macOS convention) — a meta chord is safe past the
        // term-host (its passthrough concerns are ctrl-only). Esc already closes.
        e.preventDefault();
        setSettingsOpen((v) => !v);
      } else if (e.metaKey && e.key >= "1" && e.key <= "9") {
        // ⌘1..N — jump to a nav view (Home / lenses). Always REVEALS (never
        // toggles). Meta-only chord is safe past the term-host.
        const idx = e.key.charCodeAt(0) - 49; // '1' → 0
        const target = NAV_CHORDS[idx];
        if (!target) return;
        e.preventDefault();
        if (target.home) { setSel(null); setMenu(null); closeCtx(); if (settings.nav_collapsed) toggleNav(); }
        else if (target.lens) keyRef.current.selectLens(target.lens);
      } else if (e.metaKey && k === "e") {
        // ⌘E — open the selected place in the editor (no-op when nothing selected).
        e.preventDefault();
        const path = keyRef.current.selectedPath;
        if (path) invoke("open_editor", { path, cmd: keyRef.current.editorCmd }).catch((err) => fail(err));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleNav, fail, settings.nav_collapsed]);

  // lens click: collapsed → expand into that lens; active lens again → collapse (VS Code style)
  const changeLens = (l: Lens) => {
    if (settings.nav_collapsed) { setLens(l); updateSettings({ lens: l, nav_collapsed: false }); return; }
    if (l === lens) { toggleNav(); return; }
    setLens(l);
    updateSettings({ lens: l });
  };

  const q = filter.trim().toLowerCase();
  const matchPlace = (p: Place) =>
    !q ||
    p.slug.toLowerCase().includes(q) ||
    (p.branch ?? "").toLowerCase().includes(q) ||
    (p.declared?.note ?? "").toLowerCase().includes(q);

  // workspace-wide stats for the Briefing cockpit
  const allPlaces = useMemo(
    () => (ws?.projects ?? []).flatMap((pv) => (pv.snapshot?.places ?? []).map((p) => ({ pv, p }))),
    [ws],
  );
  const stats = useMemo(() => {
    let live = 0, dirty = 0;
    for (const { p } of allPlaces) { if (p.tmux_session.up) live++; if (p.dirty) dirty++; }
    return { live, dirty };
  }, [allPlaces]);
  const resume = useMemo(
    () => allPlaces
      .filter(({ p }) => !p.is_main)
      .sort((a, b) => (b.p.declared?.last_opened_epoch ?? 0) - (a.p.declared?.last_opened_epoch ?? 0))
      .slice(0, 6),
    [allPlaces],
  );

  // Restore last place on launch (opt-in). SELECTION-ONLY: this calls setSel and
  // nothing else — NO enterPlace/touch_place/open_place (those would auto-resume a
  // Claude session on every reboot). TerminalPane attaches on its own if the tmux
  // session is up; otherwise the place view shows its normal "Enter ▸ to start".
  // Target = the most recently opened place across all projects (max
  // last_opened_epoch, main excluded) — same derivation as the Resume list, so
  // resume[0]. Fires exactly once, and only after BOTH settings hydration and the
  // first workspace load have landed, only if restore_last is on, and only if the
  // user hasn't already selected something.
  const restoredOnce = useRef(false);
  useEffect(() => {
    if (restoredOnce.current) return;
    if (!hydrated.current || !ws) return; // wait for both hydration + first ws load
    restoredOnce.current = true; // launch moment passed — one-shot even with the
    // toggle off, so enabling it later can't yank the selection mid-session
    if (!settings.restore_last) return;
    if (sel) return; // user already clicked — don't override their choice
    const target = resume[0];
    if (target) setSel({ repo: target.pv.root, slug: target.p.slug });
  }, [ws, settings.restore_last, resume, sel]);

  // ── nav resizer (drag the nav's right edge) ──
  const onResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startW = settings.nav_width;
    const move = (ev: MouseEvent) => updateSettings({ nav_width: clampNav(startW + (ev.clientX - startX)) });
    const up = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", up); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  // ── row ──
  const PlaceRow = ({ repo, p, showProject }: { repo: string; p: Place; showProject?: boolean }) => {
    const divergent = !p.is_main && !p.detached && p.branch && p.branch !== p.slug;
    return (
      <li
        className={
          "row" +
          (sel?.repo === repo && sel?.slug === p.slug ? " sel" : "") +
          (dragOver === p.slug && drag?.repo === repo ? " dragover" : "")
        }
        onClick={() => enterPlace(repo, p)}
        onContextMenu={(e) => placeCtx(e, repo, p)}
        draggable={settings.sort_mode === "manual" && !p.is_main}
        onDragStart={(e) => { e.dataTransfer.effectAllowed = "move"; setDrag({ repo, slug: p.slug }); }}
        onDragOver={(e) => { if (drag && drag.repo === repo && drag.slug !== p.slug && !p.is_main) { e.preventDefault(); setDragOver(p.slug); } }}
        onDragLeave={() => { if (dragOver === p.slug) setDragOver(null); }}
        onDrop={(e) => { e.preventDefault(); dropOnRow(repo, p.slug); }}
        onDragEnd={() => { setDrag(null); setDragOver(null); }}
        title={p.slug}
      >
        <span
          className={"status-dot" + (activityOf(p) === "busy" ? " busy" : activityOf(p) === "waiting" ? " waiting" : "")}
          title={activityOf(p) === "busy" ? "Claude working" : activityOf(p) === "waiting" ? "Claude needs input" : undefined}
        />
        <span className="row-id">
          <span className="row-name">
            {p.is_main ? "◆ " : p.declared?.pinned ? "★ " : ""}
            {p.slug}
            {showProject ? <span className="row-proj">{basename(repo)}</span> : null}
          </span>
          {divergent ? <span className="row-branch">↗ {p.branch}</span> : null}
        </span>
        <span className="glyphs">
          {glyphs(p).map((g, i) => (
            <span key={i} className={"g " + g.cls} title={g.title}>{g.text}</span>
          ))}
          <span className="row-age">{ago(recencyOf(p))}</span>
        </span>
      </li>
    );
  };

  const GroupHeader = ({ gkey, label, count, open, onToggle }: { gkey: string; label: string; count: number; open: boolean; onToggle: () => void }) => (
    <div className="group-h" key={gkey} onClick={onToggle}>
      <span className="caret">{open ? "▾" : "▸"}</span>
      {label}
      <span className="count">{count}</span>
    </div>
  );

  const ProjectNode = ({ pv }: { pv: ProjectView }) => {
    const open = !collapsed[pv.root];
    const places = (pv.snapshot?.places ?? []).filter(matchPlace);
    const main = places.find((p) => p.is_main) ?? null;
    // Rollup: a working child dominates a waiting one — busy wins, else waiting.
    const rollup = places.some((p) => activityOf(p) === "busy")
      ? "busy"
      : places.some((p) => activityOf(p) === "waiting")
        ? "waiting"
        : "";
    const buckets: Record<string, Place[]> = {};
    for (const p of places) { if (p.is_main) continue; (buckets[bucketOf(p)] ??= []).push(p); }
    for (const k of Object.keys(buckets)) buckets[k] = sortPlaces(pv.root, buckets[k]);
    const hiddenTiers = new Set(settings.hidden_tiers);
    const dormant = DORMANT_TIERS.flatMap((t) => buckets[t] ?? []);

    return (
      <div className="project">
        <div className="project-h" onContextMenu={(e) => projectCtx(e, pv.root)}>
          <span className="caret" onClick={() => toggleProject(pv.root)}>{open ? "▾" : "▸"}</span>
          {pv.ok
            ? <span className={"picon" + (rollup ? " " + rollup : "")} title={rollup === "busy" ? "a session is working" : rollup === "waiting" ? "a session needs input" : undefined}><FolderIcon /></span>
            : <span className="rollup broken" title="repo gone">⊘</span>}
          <span className="pname" title={pv.root} onClick={() => toggleProject(pv.root)}>{basename(pv.root)}</span>
          {pv.ok ? <span className="pcount">{places.length}</span> : <span className="pgone">repo gone</span>}
          <button className="mini" title="new worktree" onClick={() => { setNewFor(newFor === pv.root ? null : pv.root); setNewBase(""); }}>＋</button>
          <button
            className={"mini" + (confirmRm === `hdr|${pv.root}` ? " armed" : "")}
            title={confirmRm === `hdr|${pv.root}` ? "click again to remove from workspace" : "remove project"}
            onClick={() => removeProjectHdr(pv.root)}
          >
            {confirmRm === `hdr|${pv.root}` ? "remove?" : "✕"}
          </button>
        </div>

        {open && pv.ok && (
          <div className="kids">
            {main && <ul className="places"><PlaceRow repo={pv.root} p={main} /></ul>}
            {LIVE_TIERS.filter((g) => buckets[g]?.length && !hiddenTiers.has(g)).map((g) => {
              const key = `${pv.root}|${g}`;
              const opened = isOpen(key, g !== "idle"); // idle collapsed by default
              return (
                <div className="group" key={key}>
                  <GroupHeader gkey={key} label={GROUP_LABEL[g]} count={buckets[g].length} open={opened} onToggle={() => toggleGroup(key, g !== "idle")} />
                  {opened && <ul className="places">{buckets[g].map((p) => <PlaceRow key={p.slug} repo={pv.root} p={p} />)}</ul>}
                </div>
              );
            })}
            {dormant.length > 0 && !hiddenTiers.has("dormant") && (() => {
              const key = `${pv.root}|dormant`;
              const opened = isOpen(key, false);
              return (
                <div className="group dormant" key={key}>
                  <div className="group-h dormant-h" onClick={() => toggleGroup(key, false)}>
                    <span className="caret">{opened ? "▾" : "▸"}</span>
                    Dormant<span className="count">{dormant.length}</span>
                  </div>
                  {opened && (
                    <div className="kids-d">
                      {DORMANT_TIERS.filter((t) => buckets[t]?.length).map((t) => (
                        <div className="subgroup" key={t}>
                          <div className="subdiv">{GROUP_LABEL[t]}</div>
                          <ul className="places">{buckets[t].map((p) => <PlaceRow key={p.slug} repo={pv.root} p={p} />)}</ul>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              );
            })()}
          </div>
        )}
      </div>
    );
  };

  // flat lens (recent / attention) across all projects
  const FlatLens = ({ items }: { items: { pv: ProjectView; p: Place }[] }) => (
    <ul className="places flat">
      {items.length === 0 && <li className="flat-empty">Nothing here.</li>}
      {items.map(({ pv, p }) => <PlaceRow key={pv.root + p.slug} repo={pv.root} p={p} showProject />)}
    </ul>
  );

  const recentItems = useMemo(
    () => allPlaces.filter(({ p }) => matchPlace(p) && !p.is_main)
      .sort((a, b) => (b.p.declared?.last_opened_epoch ?? 0) - (a.p.declared?.last_opened_epoch ?? 0)),
    [allPlaces, q],
  );
  const attentionItems = useMemo(
    () => allPlaces.filter(({ p }) => matchPlace(p) && hasAttention(p)),
    [allPlaces, q],
  );

  const RAIL = [
    { key: "places" as Lens, icon: "▤", title: "Places — the full tree" },
    { key: "recent" as Lens, icon: "◷", title: "Recent — resurface dormant places" },
    { key: "attention" as Lens, icon: "⚠", title: "Attention — dirty / ahead-behind / broken" },
  ];

  return (
    <div
      className="app"
      style={{ gridTemplateColumns: settings.nav_collapsed ? `var(--rail-w) 1fr` : `var(--rail-w) ${settings.nav_width}px 1fr` }}
    >
      {/* ── activity rail ── */}
      <nav className="rail">
        {RAIL.map((r) => (
          <button
            key={r.key}
            className={"rail-icon" + (lens === r.key ? (settings.nav_collapsed ? " remembered" : " active") : "")}
            title={r.title}
            onClick={() => changeLens(r.key)}
          >
            {r.icon}
          </button>
        ))}
        <div className="rail-spacer" />
        <button className="rail-icon" title={settings.nav_collapsed ? `show nav — ${lens} (⌘B)` : "hide nav (⌘B)"} onClick={toggleNav}>
          {settings.nav_collapsed ? "»" : "«"}
        </button>
        <button className="rail-icon" title="add project" onClick={addProject}>＋</button>
        <button className={"rail-icon" + (updateAvail ? " upd" : "")} title={updateAvail ? "settings — update available" : "settings (⌘,)"} onClick={() => setSettingsOpen(true)}>⚙</button>
      </nav>

      {/* ── nav (kept mounted while collapsed so form drafts / scroll survive ⌘B) ── */}
      <aside className={"nav" + (settings.nav_collapsed ? " hidden" : "")}>
        <button className={"home-item" + (sel ? "" : " on")} onClick={() => { setSel(null); setMenu(null); closeCtx(); }}>
          <HomeIcon /> Home
        </button>
        <div className="nav-head">
          <span className="nav-title">{lens === "places" ? "PLACES" : lens === "recent" ? "RECENT" : "ATTENTION"}</span>
          <div className="menu-wrap">
            <button className={"icon-btn" + (settings.sort_mode !== "recent" ? " on" : "")} title="sort places" onClick={() => setSortOpen(!sortOpen)}>⇅</button>
            {sortOpen && (
              <div className="popover sortpop">
                <div className="pop-hint">sort places</div>
                {([["recent", "Last used"], ["alpha", "A–Z"], ["manual", "Manual (drag rows)"]] as const).map(([m, label]) => (
                  <button key={m} className="pop-item"
                    onClick={() => updateSettings({ sort_mode: m, sort_dir: m === "alpha" ? "asc" : "desc" })}>
                    <span className="check">{settings.sort_mode === m ? "✓" : ""}</span>{label}
                  </button>
                ))}
                <div className="ctx-sep" />
                <button className="pop-item" disabled={settings.sort_mode === "manual"}
                  onClick={() => updateSettings({ sort_dir: settings.sort_dir === "desc" ? "asc" : "desc" })}>
                  <span className="check" />{settings.sort_dir === "desc" ? "↓ descending" : "↑ ascending"}
                </button>
              </div>
            )}
          </div>
          <button className="icon-btn" title="focus search" onClick={() => searchRef.current?.focus()}>⌕</button>
        </div>
        <input ref={searchRef} className="search" placeholder="filter places…" value={filter} onChange={(e) => setFilter(e.currentTarget.value)} />
        {newFor && (
          <NewPlaceForm
            key={newFor + "|" + newBase}
            project={newFor}
            initialBase={newBase}
            onCreate={(b, n, ba) => createPlace(newFor, b, n, ba)}
            onCancel={() => { setNewFor(null); setNewBase(""); }}
          />
        )}
        <div className="nav-scroll">
          {ws && ws.projects.length === 0 && <div className="empty small">No projects yet.<br />Click ＋ to add one.</div>}
          {lens === "places" && ws?.projects.map((pv) => <ProjectNode key={pv.root} pv={pv} />)}
          {lens === "recent" && <FlatLens items={recentItems} />}
          {lens === "attention" && (
            <>
              <FlatLens items={attentionItems} />
              {ws?.projects.filter((pv) => !pv.ok).map((pv) => (
                <div className="project broken-flat" key={pv.root}><span className="rollup broken">⊘</span> {basename(pv.root)} <span className="pgone">repo gone</span></div>
              ))}
            </>
          )}
        </div>
        <button className="add-footer" onClick={addProject}>＋ Add project</button>
        <div className="nav-resizer" onMouseDown={onResize} />
      </aside>

      {/* ── main ── */}
      <main className="main">
        {selected && sel ? (
          <>
            <header className="topbar">
              <div className="identity">
                <b className="slug">{selected.is_main ? "◆ " : ""}{selected.slug}</b>
                {selected.branch && (
                  <span className={"branch" + (!selected.is_main && selected.branch !== selected.slug ? " hi" : "")}>
                    {!selected.is_main && selected.branch !== selected.slug ? "↗ " : ""}{selected.branch}
                  </span>
                )}
                <span className="status-cluster">
                  {selected.tmux_session.up && <span className="s ok" title="tmux live"><span className="status-dot on" /> live</span>}
                  {selected.dirty && <span className="s dirty">● {selected.dirty_files ?? ""}</span>}
                  {(selected.ahead || selected.behind) && <span className="s ab">↑{selected.ahead ?? 0} ↓{selected.behind ?? 0}</span>}
                  <span className={"life " + selected.lifecycle_effective}>{selected.lifecycle_effective}</span>
                </span>
              </div>

              <div className="controls">
                {selected.tmux_session.up ? (
                  <>
                    <span className="live-badge" title="session live"><span className="status-dot on" /> live</span>
                    <button className="ctrl" title="end the tmux session — the worktree stays"
                      onClick={() => runCmd("close_place", { repo: sel.repo, slug: sel.slug })}>Close</button>
                  </>
                ) : (
                  <button className="enter-btn" onClick={() => enterPlace(sel.repo, selected)}>Enter ▸</button>
                )}
                <button className={"icon-btn" + (selected.declared?.pinned ? " on" : "")} title={selected.declared?.pinned ? "unpin" : "pin"}
                  onClick={() => mutate(invoke("set_pin", { repo: sel.repo, slug: sel.slug, on: !selected.declared?.pinned }))}>★</button>

                <div className="menu-wrap">
                  <button className="ctrl" onClick={() => (menu === "life" ? closeMenu() : (setConfirmRm(null), setMenu("life")))}>Lifecycle ▾</button>
                  {menu === "life" && (
                    <div className="popover right">
                      <div className="pop-hint">active / idle are derived</div>
                      {SETTABLE.map((s) => (
                        <button key={s.value} className="pop-item" onClick={() => { mutate(invoke("set_lifecycle", { repo: sel.repo, slug: sel.slug, label: s.value })); closeMenu(); }}>
                          <span className="check">{selected.declared?.lifecycle === s.value ? "✓" : ""}</span>{s.label}
                        </button>
                      ))}
                    </div>
                  )}
                </div>

                {!selected.is_main && (
                  <div className="menu-wrap">
                    <button className="ctrl" onClick={() => (menu === "more" ? closeMenu() : (setConfirmRm(null), setMenu("more")))}>⋯</button>
                    {menu === "more" && (
                      <div className="popover right">
                        <button className="pop-item" onClick={() => copyText(selected.path)}>Copy path</button>
                        {confirmRm === `${sel.repo}|${sel.slug}` ? (
                          <>
                            <button className="pop-item danger armed" onClick={() => confirmRemove(false)}>Confirm remove</button>
                            <button className="pop-item danger armed" onClick={() => confirmRemove(true)}>Confirm remove + branch</button>
                          </>
                        ) : (
                          <button className="pop-item danger" onClick={armRemove}>Remove worktree…</button>
                        )}
                      </div>
                    )}
                  </div>
                )}
              </div>
            </header>

            <input className="note-strip" placeholder="note…" defaultValue={selected.declared?.note ?? ""}
              key={sel.repo + sel.slug + (selected.declared?.note ?? "")}
              onBlur={(e) => mutate(invoke("set_note", { repo: sel.repo, slug: sel.slug, note: e.currentTarget.value }))} />

            {selected.tmux_session.up ? (
              <TerminalPane key={selected.tmux_session.name} session={selected.tmux_session.name} termVersion={termVersion} focusToken={termFocus} />
            ) : (
              <div className="term-empty">
                <div className="term-empty-card">
                  <div className="te-title">No live session for <b>{selected.slug}</b></div>
                  <button className="enter-btn big" onClick={() => enterPlace(sel.repo, selected)}>Enter ▸ to start</button>
                </div>
              </div>
            )}

            <footer className="statusbar">
              <div className="switch-wrap">
                {!selected.is_main && (
                  <>
                    <span className="sb-label">⎇</span>
                    <input className="switchto" placeholder="switch branch…" value={switchTo}
                      onChange={(e) => setSwitchTo(e.currentTarget.value)}
                      onKeyDown={(e) => e.key === "Enter" && doSwitch()} />
                    <button className="ctrl sm" onClick={doSwitch} disabled={!switchTo.trim()}>Switch</button>
                  </>
                )}
              </div>
              <div className="sb-facts">
                {selected.tmux_session.up ? <>tmux <span className="ok">●</span> up · {selected.tmux_session.name}</> : <>tmux <span className="off">○</span> down</>}
                {selected.claude_session_present ? " · pane0 claude" : ""}
              </div>
            </footer>
          </>
        ) : (
          <div className="briefing">
            <div className="home-hero">
              <img className="home-logo" src={logoUrl} alt="worktrees logo" />
              <div className="home-id">
                <h1>worktrees</h1>
                <div className="home-tag">a place for every work stream</div>
              </div>
            </div>
            <button className="enter-btn big home-open" onClick={addProject}>＋ Open a project</button>
            <div className="chips">
              <span className="chip"><span className="dot" style={{ background: "var(--ok)" }} /> {stats.live} live</span>
              <span className="chip"><span className="dot" style={{ background: "var(--dirty)" }} /> {stats.dirty} dirty</span>
            </div>
            <div className="resume-h">RESUME WHERE YOU LEFT OFF</div>
            <div className="resume">
              {resume.length === 0 && <div className="empty small">No places yet — ＋ open a project.</div>}
              {resume.map(({ pv, p }) => (
                <div className="resume-row" key={pv.root + p.slug} onClick={() => enterPlace(pv.root, p)} onContextMenu={(e) => placeCtx(e, pv.root, p)}>
                  <span
                    className={"status-dot" + (activityOf(p) === "busy" ? " busy" : activityOf(p) === "waiting" ? " waiting" : "")}
                    title={activityOf(p) === "busy" ? "Claude working" : activityOf(p) === "waiting" ? "Claude needs input" : undefined}
                  />
                  <span className="rr-name">{p.declared?.pinned ? "★ " : ""}{p.slug}</span>
                  <span className="rr-proj">{basename(pv.root)}</span>
                  <span className="rr-life">{p.lifecycle_effective}</span>
                  <span className="rr-age">{ago(p.declared?.last_opened_epoch)}</span>
                  <button className="enter-btn sm">Enter ▸</button>
                </div>
              ))}
            </div>
          </div>
        )}
      </main>

      {/* error surface lives OUTSIDE the nav — must stay visible in rail-only mode */}
      {err && <div className="err err-float" title="dismiss" onClick={() => setErr("")}>{err}</div>}

      {sortOpen && <div className="menu-catch" onClick={() => setSortOpen(false)} />}

      <SettingsSheet open={settingsOpen} settings={settings} onChange={updateSettings} onClose={() => setSettingsOpen(false)}
        update={upd} cliStale={cliStale} cliMissing={cliMissing} appStale={appStale} onCheckUpdate={checkUpdate}
        onShowNotes={showReleaseNotes} onReset={onReset} />

      {/* after SettingsSheet: opened from Settings, the notes stack ON TOP of it */}
      {whatsNew && (
        <div className="scrim" onClick={() => { updateSettings({ last_seen_version: whatsNew.version }); setWhatsNew(null); }}>
          <aside className="settings-sheet whatsnew" onClick={(e) => e.stopPropagation()}>
            <header className="settings-h">
              <b>{whatsNew.manual ? "Release notes" : `What's new — v${whatsNew.version}`}</b>
              <button className="icon-btn" title="close"
                onClick={() => { updateSettings({ last_seen_version: whatsNew.version }); setWhatsNew(null); }}>✕</button>
            </header>
            <div className="settings-body">
              <ReleaseNotes notes={whatsNew.notes} />
            </div>
          </aside>
        </div>
      )}
      {menu && <div className="menu-catch" onClick={closeMenu} />}

      {/* ── right-click: place ── */}
      {ctx?.kind === "place" && ctxPlace && (
        <CtxMenu x={ctx.x} y={ctx.y} onClose={closeCtx}>
          <div className="pop-hint">{ctxPlace.is_main ? "◆ main" : ctxPlace.slug}</div>
          <button className="pop-item" onClick={() => enterPlace(ctx.repo, ctxPlace)}>Enter ▸</button>
          {!ctxPlace.tmux_session.up && ctxPlace.claude_session_present && (
            settings.ai_auto_resume ? (
              <button className="pop-item" onClick={() => enterPlace(ctx.repo, ctxPlace, { fresh: true })}>Open fresh (skip resume)</button>
            ) : (
              <button className="pop-item" onClick={() => enterPlace(ctx.repo, ctxPlace, { fresh: false })}>Open with resume</button>
            )
          )}
          {ctxPlace.tmux_session.up && (
            <>
              <button className="pop-item" onClick={() => closeSession(ctx.repo, ctxPlace.slug)}>Close session</button>
              <button className="pop-item" onClick={() => copyText(`tmux attach -t ${shq(ctxPlace.tmux_session.name)}`)}>Copy attach command</button>
              {settings.terminal_cmd.trim() && (
                <button className="pop-item" onClick={() => {
                  const session = ctxPlace.tmux_session.name;
                  closeCtx();
                  invoke("open_terminal", { cmd: settings.terminal_cmd, session }).catch((e) => fail(e));
                }}>Open in terminal app</button>
              )}
            </>
          )}
          <div className="ctx-sep" />
          {!ctxPlace.is_main && (
            <>
              <button className="pop-item" onClick={() => { closeCtx(); mutate(invoke("set_pin", { repo: ctx.repo, slug: ctxPlace.slug, on: !ctxPlace.declared?.pinned })); }}>
                {ctxPlace.declared?.pinned ? "★ Unpin" : "☆ Pin"}
              </button>
              <div className="pop-hint">lifecycle</div>
              <div className="ctx-life">
                {SETTABLE.map((s) => (
                  <button key={s.value} className={ctxPlace.declared?.lifecycle === s.value ? "on" : ""}
                    onClick={() => { closeCtx(); mutate(invoke("set_lifecycle", { repo: ctx.repo, slug: ctxPlace.slug, label: s.value })); }}>
                    {s.label}
                  </button>
                ))}
              </div>
              <button className="pop-item" onClick={() => editNote(ctx.repo, ctxPlace)}>Edit note…</button>
              <div className="ctx-sep" />
              {ctxPlace.branch && (
                <button className="pop-item" onClick={() => newFromBranch(ctx.repo, ctxPlace.branch!)}>New worktree off {ctxPlace.branch}…</button>
              )}
            </>
          )}
          <button className="pop-item" onClick={() => openOnRemote(ctx.repo, ctxPlace.slug)}>Open on GitHub</button>
          <button className="pop-item" onClick={() => revealPlace(ctxPlace.path)}>Reveal in Finder</button>
          <button className="pop-item" onClick={() => editIn(ctxPlace.path)}>Open in editor</button>
          <button className="pop-item" onClick={() => copyText(ctxPlace.path)}>Copy path</button>
          {ctxPlace.branch && <button className="pop-item" onClick={() => copyText(ctxPlace.branch!)}>Copy branch</button>}
          {!ctxPlace.is_main && (
            <>
              <div className="ctx-sep" />
              {confirmRm === `ctx|${ctx.repo}|${ctxPlace.slug}` ? (
                <>
                  <button className="pop-item danger armed" onClick={() => confirmRemovePlaceCtx(ctx.repo, ctxPlace.slug, false)}>
                    Confirm remove
                  </button>
                  <button className="pop-item danger armed" onClick={() => confirmRemovePlaceCtx(ctx.repo, ctxPlace.slug, true)}>
                    Confirm remove + branch
                  </button>
                </>
              ) : (
                <button className="pop-item danger" onClick={() => armRemovePlaceCtx(ctx.repo, ctxPlace.slug)}>
                  Remove worktree…
                </button>
              )}
            </>
          )}
        </CtxMenu>
      )}

      {/* ── right-click: project ── */}
      {ctx?.kind === "project" && (() => {
        const pv = ws?.projects.find((v) => v.root === ctx.root);
        const main = pv?.snapshot?.places.find((p) => p.is_main) ?? null;
        return (
          <CtxMenu x={ctx.x} y={ctx.y} onClose={closeCtx}>
            <div className="pop-hint">{basename(ctx.root)}</div>
            <button className="pop-item" onClick={() => { closeCtx(); openNewForm(ctx.root, ""); }}>New worktree…</button>
            {main && <button className="pop-item" onClick={() => enterPlace(ctx.root, main)}>Enter main ▸</button>}
            {pv?.ok && (
              <button className="pop-item" onClick={() => { closeCtx(); mutate(invoke("fetch_origin", { root: ctx.root })); }}>Fetch origin</button>
            )}
            <div className="ctx-sep" />
            <button className="pop-item" onClick={() => copyText(ctx.root)}>Copy path</button>
            <button className="pop-item" onClick={() => revealPlace(ctx.root)}>Reveal in Finder</button>
            <button className="pop-item" onClick={() => editIn(ctx.root)}>Open in editor</button>
            <div className="ctx-sep" />
            <button
              className={"pop-item danger" + (confirmRm === `proj|${ctx.root}` ? " armed" : "")}
              onClick={() => removeProjectCtx(ctx.root)}
            >
              {confirmRm === `proj|${ctx.root}` ? "Confirm remove?" : "Remove from workspace"}
            </button>
          </CtxMenu>
        );
      })()}
    </div>
  );
}

export default App;
