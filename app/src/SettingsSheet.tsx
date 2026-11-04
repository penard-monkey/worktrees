// Settings: a centered modal (the file keeps its name; the component is still
// what the app calls Settings). Since the regroup (docs/proposals/
// settings-reorg.md) it is a SHELL: the header with the mode switch, the
// grouped page list, and the page on screen. Every page is a file under
// `pages/`; what is on which page, at which tier, is `settingsMap.ts`.
//
// Presentational, as before: App owns the Settings state and does the
// apply-live + persist + terminal-refit on each change. Esc / scrim closes.
import { useCallback, useEffect, useMemo, useRef, useState, type ComponentType } from "react";
import { useEscape } from "./useEscape";
import * as Icons from "./icons";
import { invoke, notGranted, remoteHost, windowOnHost } from "./ipc";
import { dismissCat, useRemote } from "./remoteStore";
import { DEFAULTS } from "./settings";
import { HARNESSES, type Harness } from "./harness";
import {
  GROUPS, MODES, MODE_LABEL, PAGES, focusSection, isMode, pageListed, resolveAt, sectionWhy, SECTION,
  type HostKey, type Mode, type PageId, type VisCtx,
} from "./settingsMap";
import { SheetContext, type HostSnap, type SheetApi, type SheetProps } from "./pages/kit";
import { useUpdateRuns } from "./pages/updates";
import { GeneralPage } from "./pages/general";
import { AppearancePage } from "./pages/appearance";
import { LayoutPage } from "./pages/layout";
import { TerminalPage } from "./pages/terminal";
import { ReferencePage } from "./pages/reference";
import { AgentsPage } from "./pages/agents";
import { ClaudePage } from "./pages/claude";
import { CodexPage } from "./pages/codex";
import { PiPage } from "./pages/pi";
import { KimiPage } from "./pages/kimi";
import { ProfilesPage } from "./pages/profiles";
import { GuidancePage } from "./pages/guidance";
import { PolicyPage } from "./pages/policy";
import { ProjectsPage } from "./pages/projects";
import { RemotePage } from "./pages/remote";
import { UpdatesPage } from "./pages/updates";
import { DataPage } from "./pages/data";
import { ShortcutsPage } from "./pages/shortcuts";
import type { QuotaSettings } from "./QuotaPanel";
import type { SlotsSettings } from "./SlotsPanel";
import type { PiStatus } from "./ModelPicker";
import type { KimiStatus } from "./KimiPanel";
import type { ProfilesInfo } from "./ProfilesPanel";
import type { CodexPermissions } from "./settings";

export { RELEASES_URL } from "./pages/updates";
export type { CatId } from "./settingsMap";

/** Every page's component. `settingsmap-check.mjs` holds this against PAGES. */
const PAGE_VIEW: Record<PageId, ComponentType> = {
  general: GeneralPage,
  appearance: AppearancePage,
  layout: LayoutPage,
  terminal: TerminalPage,
  reference: ReferencePage,
  agents: AgentsPage,
  claude: ClaudePage,
  codex: CodexPage,
  pi: PiPage,
  kimi: KimiPage,
  profiles: ProfilesPage,
  guidance: GuidancePage,
  policy: PolicyPage,
  projects: ProjectsPage,
  remote: RemotePage,
  updates: UpdatesPage,
  data: DataPage,
  shortcuts: ShortcutsPage,
};

/** Where the sheet opens when nobody named a page. */
const HOME: PageId = "general";

/** The host's values the mode rules need (§4.2 rules 2 and 3) and the pointer
 *  lines show, read when the sheet opens and on every page change: cheap
 *  getters, and pi's and kimi's status only where that CLI is installed. A
 *  row this window may not ask for is skipped, never refused on screen. */
function useHostSnap(open: boolean, cat: PageId, installed: VisCtx["installed"]): HostSnap {
  const [snap, setSnap] = useState<HostSnap>({});
  useEffect(() => {
    if (!open) return;
    let alive = true;
    const read = <T,>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> =>
      notGranted(cmd) ? Promise.resolve(undefined) : invoke<T>(cmd, args).catch(() => undefined);
    void Promise.all([
      read<CodexPermissions>("get_codex_permissions"),
      read<number>("get_fetch_interval"),
      read<HostSnap["termHistory"]>("get_term_history_opts"),
      read<QuotaSettings>("quota_settings"),
      read<SlotsSettings>("slots_settings"),
      installed.pi === false ? undefined : read<PiStatus>("pi_status", { repo: null }),
      installed.kimi === false ? undefined : read<KimiStatus>("kimi_status", { repo: null }),
      read<ProfilesInfo | null>("profiles_info", { repo: null }),
    ]).then(([codexPermissions, fetchInterval, termHistory, quota, slots, pi, kimi, profiles]) => {
      if (!alive) return;
      setSnap({
        codexPermissions, fetchInterval, termHistory, quota, slots, pi, kimi,
        profiles: profiles ? { count: profiles.profiles.length, hasDefault: !!profiles.default_id } : undefined,
      });
    });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- re-read per page, not per App render
  }, [open, cat, installed.pi, installed.kimi]);
  return snap;
}

export function SettingsSheet(props: SheetProps) {
  const {
    open, at, settings, onChange, onClose, update, appStale, cliStale, cliMissing, onCatShown,
    mcpStatus, guidance, crossProject, planning,
  } = props;
  // The page on screen — local, deliberately NOT persisted: the sheet opens
  // on General so "where was I" never depends on last session. An explicit
  // `at` overrides that, and does not break the rule: a caller naming its
  // destination is the whole point of an offer's deep link.
  const [cat, setCat] = useState<PageId>(HOME);
  /** The focus the sheet was last sent to, resolved (a band in a section
   *  included); null for an ordinary open. */
  const [focus, setFocus] = useState<string | null>(null);
  // Reset on CLOSE too: the open effect below reads `cat` in the same commit
  // as this one sets it, so a sheet reopened at General would otherwise
  // report last visit's page once — and "Updates, seen" retires a bubble.
  useEffect(() => {
    if (!open || !at) { setCat(HOME); setFocus(null); return; }
    const r = resolveAt(at);
    setCat(r.cat);
    setFocus(r.focus ?? null);
  }, [open, at]);
  useEffect(() => { if (open) onCatShown?.(cat); }, [open, cat, onCatShown]);

  const mode: Mode = isMode(settings.settings_mode) ? settings.settings_mode : "expert";
  const setMode = useCallback((m: Mode) => onChange({ settings_mode: m }), [onChange]);

  // Which harness CLIs the host has (rule 4): the update check reads each
  // one's installed version. Unknown until it has answered, which draws them.
  const installed = useMemo(() => {
    const out: Partial<Record<Harness, boolean>> = {};
    for (const h of update?.harnesses ?? []) {
      if (HARNESSES.includes(h.tool)) out[h.tool] = !!h.installed_version;
    }
    return out;
  }, [update?.harnesses]);

  const snap = useHostSnap(open, cat, installed);
  const remote = useRemote();

  // Problems a page has told the sheet about (rule 3), beside the ones it
  // can see for itself below.
  const [reported, setReported] = useState<Record<string, boolean>>({});
  const reportProblem = useCallback((id: string, on: boolean) => {
    setReported((r) => (!!r[id] === on ? r : { ...r, [id]: on }));
  }, []);
  const problems = useMemo(() => {
    const p = new Set<string>(Object.keys(reported).filter((k) => reported[k]));
    if (mcpStatus?.state === "stale") p.add("mcp-server");
    if (guidance?.skill_edit?.invalid) p.add("guidance-skill");
    if (guidance?.error) p.add("agent-guidance");
    if ((snap.quota?.problems.length ?? 0) > 0) p.add("plan-limits");
    if ((snap.slots?.problems.length ?? 0) > 0) p.add("heavy-jobs");
    if (snap.pi?.preflight.problem) p.add("pi-install");
    const st = remote.status?.state;
    if (st === "unreadable" || st === "error" || st === "paused" || remote.alerts.some((a) => a.cat === "remote")) p.add("remote-access");
    if (remote.alerts.some((a) => a.cat === "devices")) p.add("devices");
    return p;
  }, [reported, mcpStatus?.state, guidance?.skill_edit?.invalid, guidance?.error, snap, remote.status?.state, remote.alerts]);

  // Rule 2's host half: which host values differ from what ships.
  const host = useMemo(() => {
    const h: Partial<Record<HostKey, boolean>> = {};
    if (snap.codexPermissions) h.codex_access = snap.codexPermissions !== "auto-review";
    if (snap.fetchInterval !== undefined) h.fetch_interval = snap.fetchInterval !== 0;
    if (snap.termHistory) h.term_history = !snap.termHistory.persist_scrollback || !snap.termHistory.per_tab_history;
    if (snap.quota) { h.quota_gate = !snap.quota.gate; h.quota_pct = snap.quota.weekly_warn_pct !== null; }
    if (snap.slots) h.slots = snap.slots.slots.some((s) => s.explicit);
    if (snap.pi) { h.pi_trust = snap.pi.trust !== "never"; h.pi_allowed = snap.pi.allowed.length > 0; }
    if (snap.kimi) h.kimi_allowed = snap.kimi.allowed.length > 0;
    if (snap.profiles) h.profiles = snap.profiles.count > 0;
    if (guidance) {
      h.main_guard = guidance.settings.guard;
      h.guidance_enabled = !guidance.settings.enabled;
      h.guidance_skill = !!guidance.skill_edit;
    }
    if (crossProject) {
      h.cross_project = crossProject.level !== "off";
      h.private_projects = crossProject.projects.some((p) => p.private);
    }
    if (planning) h.planning_default = planning.default !== "unset";
    if (remote.status) h.remote_access = remote.status.state !== "off";
    if (remote.devices) h.devices = remote.devices.some((d) => d.revoked_at == null);
    return h;
  }, [snap, guidance, crossProject, planning, remote.status, remote.devices]);

  // Sections drawn in this visit (§4.3's deep-link reveal, and so a value set
  // back to its default does not make its section vanish under the pointer).
  // Cleared on a page change, a mode change and a close; the deep link's own
  // section is kept for the whole visit.
  const [seen, setSeen] = useState<ReadonlySet<string>>(new Set());
  useEffect(() => { setSeen(new Set()); }, [cat, mode, open]);
  const linked = focus ? focusSection(focus) : null;
  const revealed = useMemo(() => {
    const r = new Set(seen);
    if (linked) r.add(linked);
    return r;
  }, [seen, linked]);

  const vis: VisCtx = useMemo(() => ({
    mode, settings, defaults: DEFAULTS, host, problems, installed, revealed, onHost: windowOnHost(),
  }), [mode, settings, host, problems, installed, revealed]);

  // Remember what this visit drew above the mode, so it stays put.
  useEffect(() => {
    const now = Object.values(SECTION).filter((s) => s.page === cat && sectionWhy(s, vis) && sectionWhy(s, vis) !== "mode").map((s) => s.id);
    if (now.some((id) => !seen.has(id))) setSeen((prev) => new Set([...prev, ...now]));
  }, [cat, vis, seen]);

  // …and then say which section it meant. A transient class rather than focus:
  // the sheet body scrolls, and `autoFocus` inside a scrolling box is what
  // pushed a header 34px out of view once already. `scrollIntoView` with
  // `block: "nearest"` moves nothing when the section is on screen.
  const bodyRef = useRef<HTMLDivElement | null>(null);
  /** Bumped by every in-sheet move, so a second click on the same pointer
   *  flashes its section again. */
  const [flash, setFlash] = useState(0);
  const goto = useCallback((c: PageId, f?: string) => {
    setCat(c);
    setFocus(f ?? null);
    setFlash((n) => n + 1);
  }, []);
  useEffect(() => {
    if (!open || !focus) return;
    // After the page switch has painted — the section does not exist until
    // `cat` renders it. Sheet-wide: the focus names a section, wherever it is.
    const id = requestAnimationFrame(() => {
      const el = bodyRef.current?.querySelector<HTMLElement>(`[data-focus="${focus}"]`);
      if (!el) return;
      // A collapsed record (Usage) opens for the link that named it.
      const details = el.querySelector("details");
      if (details && !details.open && focus === "usage") details.open = true;
      el.scrollIntoView({ block: "nearest" });
      el.classList.add("focus-flash");
      // Removed on animation end, so a re-open re-triggers it: the class must be
      // gone before it can be added again. The TIMER is not belt-and-braces —
      // under `prefers-reduced-motion` the rule is `animation: none`, so
      // `animationend` never fires at all and the outline would stand until the
      // section unmounted. A mark that never leaves is not a mark.
      const done = () => { clearTimeout(timer); el.classList.remove("focus-flash"); };
      const timer = setTimeout(done, 1600);
      el.addEventListener("animationend", done, { once: true });
    });
    return () => cancelAnimationFrame(id);
  }, [open, at, cat, focus, flash]);

  const runs = useUpdateRuns({ open, update, onCheckUpdate: props.onCheckUpdate });

  // Escape belongs to whatever is ON TOP. What's new opens FROM here and
  // stacks over it; the Escape stack orders by activation, so the notes take
  // the press and Settings stays put until they are gone.
  useEscape(onClose, open);

  // Remote access and devices: the alerts' badge, retired once looked at.
  const remoteBadge = remote.alerts.filter((a) => a.cat === "remote" || a.cat === "devices").length;
  useEffect(() => {
    if (open && cat === "remote") { dismissCat("remote"); dismissCat("devices"); }
  }, [open, cat, remote.alerts]);

  const api: SheetApi = {
    ...props, mode, setMode, goto, snap, why: (id) => (SECTION[id] ? sectionWhy(SECTION[id], vis) : null),
    reportProblem, host: remoteHost(), vis, runs, focus,
  };

  if (!open) return null;

  const updatesBadge = cliStale || cliMissing || appStale || !!update?.harnesses?.some((h) => h.update_available);
  const View = PAGE_VIEW[cat];
  return (
    <div className="modal-scrim" onClick={onClose}>
      {/* A centered modal, not the right-hand sheet it started as: nine
          categories in a 560px side panel left every pane cramped, and the
          terminal keeps painting underneath either way. */}
      <aside className="modal settings-modal" onClick={(e) => e.stopPropagation()}>
        <header className="settings-h">
          <b>Settings</b>
          {/* The mode (§4.6): what the sheet DRAWS, never what anything does.
              A segmented control, not a popover — no new Escape surface. */}
          <div className="seg seg-plain settings-mode" role="radiogroup" aria-label="Settings mode" data-testid="settings-mode">
            {MODES.map((m) => (
              <button key={m} role="radio" aria-checked={mode === m} className={mode === m ? "on" : ""}
                data-mode={m} onClick={() => setMode(m)}>{MODE_LABEL[m]}</button>
            ))}
          </div>
          <span className="modal-keys">
            <kbd className="kbd">esc</kbd>
            <kbd className="kbd">⌘,</kbd>
          </span>
          <button className="icon-btn" title="close (Esc)" onClick={onClose}><Icons.X /></button>
        </header>

        <SheetContext.Provider value={api}>
        <div className="settings-split">
        <nav className="settings-cats" aria-label="Settings pages">
          {GROUPS.map((g) => {
            const pages = PAGES.filter((p) => p.group === g.id && (p.id === cat || pageListed(p.id, vis)));
            return (
              <div className="settings-group" key={g.id} data-group={g.id}>
                <div className="settings-group-h">{g.label}</div>
                {pages.map((c) => (
                  <button
                    key={c.id}
                    className={"settings-cat" + (cat === c.id ? " on" : "")}
                    data-cat={c.id}
                    onClick={() => goto(c.id)}
                  >
                    {c.label}
                    {c.id === "updates" && updatesBadge ? <span className="upd-tag">upd</span> : null}
                    {/* `stale` only, still. An OFFER already has its mark — the
                        dock rail's sparkles button — and a BROKEN server is a
                        different claim, which earns the mark. */}
                    {c.id === "claude" && mcpStatus?.state === "stale" ? <span className="upd-tag warn">!</span> : null}
                    {/* §5.3: an alert raises a badge on Remote until the person looks. */}
                    {c.id === "remote" && remoteBadge > 0 ? <span className="upd-tag warn" data-testid="badge-remote">{remoteBadge}</span> : null}
                  </button>
                ))}
              </div>
            );
          })}
        </nav>

        <div className="settings-body" ref={bodyRef} data-page={cat}>
          <View key={cat} />
        </div>
        </div>
        </SheetContext.Provider>
      </aside>
    </div>
  );
}
