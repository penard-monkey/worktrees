import { useEffect, useRef, useState } from "react";
import { useEscape } from "./useEscape";
import { McpSection, type McpStatus } from "./McpPanel";
import { CodexMcpSection, type CodexMcpStatus } from "./CodexMcpPanel";
import { GuidanceSection, type GuidanceStatus } from "./GuidancePanel";
import { CrossProjectSection, type CrossProjectStatus } from "./CrossProjectPanel";
import { QuotaSection } from "./QuotaPanel";
import { UserSkillsSection, type UserSkill } from "./AgentSetup";
import * as Icons from "./icons";
import { invoke } from "@tauri-apps/api/core";
import { copyToClipboard } from "./clipboard";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { check as checkAppUpdate } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import ProfilesPanel from "./ProfilesPanel";
import PiPanel from "./PiPanel";
import type { PiMcpStatus } from "./PiMcpPanel";
import type { Settings, ThemeId, ThemeSetting, UpdateInfo } from "./settings";
import { clampNav, clampRem, clampTerm, clampZoom, THEMES, ZOOM_STEPS } from "./settings";
import { clampSteps, doneBounds, DONE_FIRST_SECS, DONE_HORIZONS, DONE_STEPS_MAX, DONE_STEPS_MIN, fmtSecs, snapHorizon } from "./afterglow";
import { humanSize } from "./filekind";
import { HARNESSES, HARNESS_LABEL } from "./harness";

type CmdResult = { ok: boolean; code: number; output: string; slug?: string | null; warnings?: string[] };
type AiConfig = { ai_cmd: string; ai_resume_arg: string; path: string; exists: boolean };
/** `term_history_info` — what the saved-scrollback tree currently costs. */
type TermHistoryInfo = { dir: string; bytes: number; tabs: number };
type Release = { tag: string; published: string };

export const RELEASES_URL = "https://github.com/penard-monkey/worktrees/releases";

/** The by-hand route for the same install — for when the buttons fail, or the
 *  app is too broken to press them. The version MUST ride in
 *  WORKTREES_INSTALL_VERSION on the `bash` side of the pipe: install.sh resolves
 *  latest otherwise, whatever tag the script's own URL names. */
function ManualInstall({ tag, cliDir }: { tag: string; cliDir: string | null }) {
  const [copied, setCopied] = useState("");
  const script = `curl -fsSL https://raw.githubusercontent.com/penard-monkey/worktrees/${tag}/install.sh`;
  const dir = cliDir ? ` WORKTREES_INSTALL_DIR="${cliDir}"` : "";
  const rows = [
    { id: "cli", label: "CLI only", cmd: `${script} | WORKTREES_INSTALL_VERSION=${tag}${dir} WORKTREES_INSTALL_APP=0 bash` },
    { id: "both", label: "App + CLI (quit worktrees first)", cmd: `${script} | WORKTREES_INSTALL_VERSION=${tag}${dir} WORKTREES_INSTALL_APP=1 bash` },
  ];
  const copy = (id: string, cmd: string) =>
    copyToClipboard(cmd).then(() => { setCopied(id); setTimeout(() => setCopied(""), 2000); }).catch(() => {});
  return (
    <details className="manual-install">
      <summary>Install {tag} manually</summary>
      {rows.map((r) => (
        <div key={r.id} className="manual-row">
          <div className="manual-head">
            <span className="sub">{r.label}</span>
            <button className="ctrl sm" onClick={() => copy(r.id, r.cmd)}>{copied === r.id ? "Copied" : "Copy"}</button>
          </div>
          <pre className="update-log">{r.cmd}</pre>
        </div>
      ))}
      <div className="hint">
        Run in any terminal. The app goes to /Applications (or ~/Applications) and is checksum-verified. Or download{" "}
        <code>worktrees-app-&lt;arch&gt;.app.tar.gz</code> from the{" "}
        <a href="#" onClick={(e) => { e.preventDefault(); openUrl(`${RELEASES_URL}/tag/${tag}`).catch(() => {}); }}>{tag} release page</a>, unpack it into
        /Applications and run <code>xattr -cr /Applications/worktrees.app</code>.
      </div>
    </details>
  );
}

/** Compare "vX.Y.Z" tags numerically; an unparseable side sorts as equal. */
function cmpVer(a: string, b: string): number {
  const p = (t: string) => t.replace(/^v/, "").split(/[.-]/).slice(0, 3).map(Number);
  const [x, y] = [p(a), p(b)];
  if (x.length < 3 || y.length < 3 || [...x, ...y].some(Number.isNaN)) return 0;
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
  return 0;
}

function agentSetupLabel(state: string | null): string {
  if (!state) return "Checking…";
  return ({
    installed: "Connected",
    "read-only": "Connected (read only)",
    elsewhere: "Connected in this project",
    absent: "Not connected",
    stale: "Needs repair",
    foreign: "Name in use",
    "cli-missing": "Worktrees CLI missing",
    disabled: "Disabled in pi",
    unreadable: "Config unreadable",
    "pi-missing": "pi not installed",
    "not-applicable": "Unavailable",
  } as Record<string, string>)[state] ?? state;
}

// Sheet categories. One is shown at a time (see .settings-split) — the flat
// 12-section pile made every setting equally hard to find. Purely presentational:
// no section's markup or onChange contract changed when they were bucketed.
const CATS = [
  { id: "appearance", label: "Appearance" },
  { id: "terminal", label: "Terminal" },
  { id: "navigation", label: "Navigation" },
  { id: "commands", label: "Commands" },
  { id: "ai", label: "AI profiles" },
  // Its own category rather than a block inside Commands: it starts as one
  // section and already carries status, scope, the two repair paths, the
  // hand-run command and an uninstall — and it is the surface a dismissed Home
  // card sends people to, so it has to be findable by name.
  { id: "claude", label: "Claude" },
  { id: "codex", label: "Codex" },
  { id: "pi", label: "pi" },
  // Its own category (decision Q8): it is about every agent, not one harness,
  // and not about AI profiles, which are per-profile and Claude-only.
  { id: "guidance", label: "Agent guidance" },
  { id: "behavior", label: "Behavior" },
  { id: "updates", label: "Updates" },
  { id: "data", label: "Data & Logs" },
  { id: "shortcuts", label: "Shortcuts" },
  // Last, behind a divider: not a setting. It is the only category that shows
  // what the app has RECORDED rather than what it will do.
  { id: "usage", label: "Usage" },
] as const;
/** One row of `detect_editors` (lib.rs `editors::Editor`). */
type DetectedEditor = { label: string; cmd: string; kind: "app" | "cli" };
/** The select's Custom… value — not a command anyone could store. */
const EDITOR_CUSTOM = "\u0000custom";

export type CatId = (typeof CATS)[number]["id"];

// ── Settings → Usage ────────────────────────────────────────────────────────
// What this Mac's copy of the app has recorded about itself: where the
// foreground hours went, and which controls got clicked. It exists to find the
// DEAD ones — a row of zeros is the finding, not a gap in the data.
//
// Nothing here identifies anything. An event is a control key (a constant in
// the source), a surface (a fixed enum) and a timestamp; `usage.ts` decides
// what may become a key and `valid_token` in lib.rs refuses to store the rest.
// The file never leaves the machine.

type UiUsage = {
  days: string[];
  acts: { key: string; total: number; per_day: number[] }[];
  dwell: { surface: string; ms_total: number; per_day: number[] }[];
  file: string;
};

const USAGE_RANGES = [7, 14, 30] as const;
/** Past this the table stops being readable; the tail folds into one line. */
const USAGE_ROWS = 40;

/** ms → the shortest true reading: `4h 12m`, `12m`, `48s`. */
function dur(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 90) return `${s}s`;
  const m = Math.round(s / 60);
  if (m < 90) return `${m}m`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** The five-step ramp, as a share of `--accent` mixed into the panel.
 *
 *  Thresholds are FIXED (1 / 3 / 7 / 16) once a day in the range has 16+
 *  actions, so the colour of a cell means the same thing week to week. Below
 *  that the same five steps are spread over the observed maximum instead —
 *  otherwise a light week paints entirely in the first bucket and the map says
 *  nothing at all. Sequential single hue, so the only thing the colour encodes
 *  is "more". */
function usageBuckets(max: number): number[] {
  if (max >= 16) return [1, 3, 7, 16];
  const q = (f: number) => Math.max(1, Math.ceil(max * f));
  const steps = [1, q(0.25), q(0.5), q(0.75)];
  // dedupe upward so two buckets can never share a threshold
  return steps.map((v, i) => (i === 0 ? v : Math.max(v, steps[i - 1] + 1)));
}
const USAGE_MIX = [0, 25, 45, 70, 100];
function usageLevel(n: number, cuts: number[]): number {
  let lvl = 0;
  for (const c of cuts) if (n >= c) lvl++;
  return lvl;
}
const usageCell = (lvl: number) => `color-mix(in srgb, var(--accent) ${USAGE_MIX[lvl]}%, var(--bg-elev))`;

function UsagePanel() {
  const [days, setDays] = useState<number>(14);
  const [data, setData] = useState<UiUsage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [clearArmed, setClearArmed] = useState(false);

  // On open and on range change only — an interval here would make the panel
  // measure itself.
  const load = async (n: number) => {
    try {
      // The backend has no timezone database, and the heatmap's columns are the
      // user's days, not UTC's. `getTimezoneOffset` is minutes WEST of UTC, so
      // the sign flips on the way out.
      const tzOffsetMin = -new Date().getTimezoneOffset();
      setData(await invoke<UiUsage>("ui_usage", { days: n, tzOffsetMin }));
      setErr(null);
    } catch (e) {
      setErr(String(e));
    }
  };
  useEffect(() => { void load(days); }, [days]);

  const doClear = async () => {
    if (!clearArmed) { setClearArmed(true); return; } // arm; second click confirms
    setClearArmed(false);
    try {
      await invoke("ui_events_clear");
      await load(days);
    } catch (e) {
      const m = `clear usage events failed: ${String(e)}`;
      setErr(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };
  const reveal = async () => {
    try {
      if (data?.file) await revealItemInDir(data.file);
    } catch (e) {
      const m = `reveal usage file failed: ${String(e)}`;
      setErr(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };

  const dwellTotal = (data?.dwell ?? []).reduce((a, d) => a + d.ms_total, 0);
  const max = Math.max(0, ...(data?.acts ?? []).flatMap((a) => a.per_day));
  const cuts = usageBuckets(max);
  const rows = (data?.acts ?? []).slice(0, USAGE_ROWS);
  const hidden = (data?.acts ?? []).length - rows.length;
  const empty = !!data && data.acts.length === 0 && data.dwell.length === 0;

  return (
    <>
      <section className="setting">
        <label>Usage</label>
        <div className="um-head">
          <span className="hint">last {days} days · stays on this Mac</span>
          <select value={days} onChange={(e) => setDays(+e.currentTarget.value)}>
            {USAGE_RANGES.map((n) => <option key={n} value={n}>{n} days</option>)}
          </select>
        </div>
      </section>

      {empty && (
        <section className="setting">
          <div className="hint">Nothing recorded yet — use the app for a day and come back.</div>
        </section>
      )}

      {!!data && data.dwell.length > 0 && (
        <section className="setting">
          <label className="sub">Foreground time by surface</label>
          <div className="um-bars">
            {data.dwell.map((d) => {
              const pct = dwellTotal ? Math.round((d.ms_total / dwellTotal) * 100) : 0;
              return (
                <div className="um-bar-row" key={d.surface}>
                  <span className="um-bar-label">{d.surface}</span>
                  <span className="um-bar-track" title={`${d.surface} · ${dur(d.ms_total)}`}>
                    <span className="um-bar-fill" style={{ width: `${pct}%` }} />
                  </span>
                  <span className="um-bar-val">{dur(d.ms_total)}</span>
                </div>
              );
            })}
          </div>
        </section>
      )}

      {!!data && data.acts.length > 0 && (
        <section className="setting">
          <label className="sub">Actions per day</label>
          {/* Its own scroller: a month of columns is wider than the modal, and
              the body must never scroll sideways as a whole. */}
          <div className="um-grid-scroll">
            <div className="um-grid">
              <div className="um-row um-head-row">
                <span className="um-key" />
                {data.days.map((d) => <span className="um-col" key={d} title={d}>{d.slice(8)}</span>)}
                <span className="um-total">total</span>
              </div>
              {rows.map((a) => (
                <div className={"um-row" + (a.total === 0 ? " um-dead" : "")} key={a.key}>
                  <span className="um-key" title={a.key}>{a.key}</span>
                  {a.per_day.map((n, i) => (
                    <span
                      key={i}
                      className="um-cell"
                      style={{ background: usageCell(usageLevel(n, cuts)) }}
                      title={`${a.key} · ${data.days[i]} · ${n}`}
                    />
                  ))}
                  <span className="um-total">{a.total}</span>
                </div>
              ))}
            </div>
          </div>
          {hidden > 0 && <div className="hint">… {hidden} more</div>}
          <div className="um-legend">
            fewer
            {USAGE_MIX.map((_, i) => <span className="um-cell" key={i} style={{ background: usageCell(i) }} />)}
            more
          </div>
        </section>
      )}

      <section className="setting">
        <div className="ver-rows">
          <div className="ver-row"><span className="ver-path" title={data?.file ?? ""}>{data?.file || "…"}</span></div>
        </div>
        <div className="hint">Control names and surfaces only — never a place name, a path, or anything you typed. Never sent anywhere.</div>
        <div className="ver-actions">
          <button className="ctrl sm" onClick={reveal}>Reveal file</button>
          <button className={"ctrl sm danger" + (clearArmed ? " armed" : "")} onClick={doClear}>
            {clearArmed ? "Clear — sure?" : "Clear"}
          </button>
        </div>
        {err && <pre className="update-log">{err}</pre>}
      </section>
    </>
  );
}

// A centered modal (the file keeps its name; the component is still what the
// app calls Settings). Presentational: App owns the Settings state and does the
// apply-live + persist + terminal-refit on each change. Esc / scrim closes.
// The Version section owns its own update-run state (log/progress) locally.
export function SettingsSheet({
  open,
  at,
  settings,
  onChange,
  onClose,
  update,
  cliStale,
  cliMissing,
  appStale,
  onCheckUpdate,
  onCatShown,
  onShowNotes,
  onReset,
  repo,
  onReport,
  mcpStatus,
  onMcpChanged,
  mcpOfferPending,
  onSilenceMcpOffer,
  onCodexMcpChanged,
  codexMcpOfferPending,
  onSilenceCodexMcpOffer,
  onPiMcpChanged,
  piMcpOfferPending,
  onSilencePiMcpOffer,
  userSkills,
  onUserSkillsChanged,
  skillsOfferPending,
  onSilenceSkillsOffer,
  guidance,
  onGuidanceChanged,
  guidanceOfferPending,
  onSilenceGuidanceOffer,
  guidanceChangeOfferPending,
  onSilenceGuidanceChangeOffer,
  crossProject,
  onCrossProjectChanged,
  crossProjectOfferPending,
  onSilenceCrossProjectOffer,
}: {
  open: boolean;
  /// Where the sheet was asked to open, when the caller had somewhere in mind —
  /// an offer's deep link (`offers.ts`). `null` is the ordinary ⌘, open.
  at: { cat: CatId; focus?: string } | null;
  settings: Settings;
  onChange: (patch: Partial<Settings>) => void;
  onClose: () => void;
  update: UpdateInfo | null;
  cliStale: boolean;
  cliMissing: boolean;
  appStale: boolean;
  onCheckUpdate: () => Promise<unknown> | void;
  /** The category on screen, whichever way it got there (opened at it, or
   *  clicked to). App uses it to retire the update bubble once Updates has
   *  been seen — `at` alone only knows where the sheet was OPENED. */
  onCatShown?: (cat: CatId) => void;
  onShowNotes: () => void;
  onReset: () => void;
  /// The project in focus — AI profiles can be bound per repo, so the panel
  /// needs to know which one to report as effective. Empty is fine (global view).
  repo: string;
  onReport: (msg: string) => void;
  /// Claude MCP wiring — fetched by App (Home shows a card from the same
  /// status), so this panel renders rather than re-probes.
  mcpStatus: McpStatus | null;
  onMcpChanged: (s: McpStatus) => void;
  /// Is the MCP server still an open suggestion? The Claude panel carries the
  /// only on-demand way to end it — see `McpSection`.
  mcpOfferPending: boolean;
  onSilenceMcpOffer: () => void;
  /// Codex's twins of the two above (offers.ts `codex-mcp` / `codex-skills`).
  /// The Codex MCP panel still probes for itself; it reports what it reads so
  /// App's offer tracks it.
  onCodexMcpChanged: (s: CodexMcpStatus) => void;
  codexMcpOfferPending: boolean;
  onSilenceCodexMcpOffer: () => void;
  /// pi's twins (offers.ts `pi-mcp`); Settings → pi's section probes for itself.
  onPiMcpChanged: (s: PiMcpStatus) => void;
  piMcpOfferPending: boolean;
  onSilencePiMcpOffer: () => void;
  userSkills: UserSkill[] | null;
  onUserSkillsChanged: (s: UserSkill[]) => void;
  skillsOfferPending: boolean;
  onSilenceSkillsOffer: () => void;
  /// Agent guidance (offers.ts `agent-guidance`): App's machine-level probe;
  /// the section re-reads on open and reports back so the offer follows it.
  guidance: GuidanceStatus | null;
  onGuidanceChanged: (s: GuidanceStatus) => void;
  guidanceOfferPending: boolean;
  onSilenceGuidanceOffer: () => void;
  /// `agent-guidance-changed`: the user's edited skill and a default that moved.
  guidanceChangeOfferPending: boolean;
  onSilenceGuidanceChangeOffer: () => void;
  /// Cross-project reach (offers.ts `cross-project`): App's machine-level
  /// probe; the section re-reads on open and reports back.
  crossProject: CrossProjectStatus | null;
  onCrossProjectChanged: (s: CrossProjectStatus) => void;
  crossProjectOfferPending: boolean;
  onSilenceCrossProjectOffer: () => void;
}) {
  // Selected category — local, deliberately NOT persisted: the sheet opens on
  // Appearance so "where was I" never depends on last session.
  //
  // An explicit `at` overrides that, and does NOT break the rule: what the rule
  // forbids is IMPLICIT restoration of wherever you happened to be last time.
  // A caller naming its destination is the opposite — it is the whole point of
  // an offer's deep link, which would otherwise drop you on Appearance and make
  // you hunt for the thing it just offered.
  const [cat, setCat] = useState<CatId>("appearance");
  // Reset on CLOSE too: the open effect below it reads `cat` in the same commit
  // as this one sets it, so a sheet reopened at Appearance would otherwise
  // report last visit's category once — and "Updates, seen" retires a bubble.
  useEffect(() => { setCat(open ? at?.cat ?? "appearance" : "appearance"); }, [open, at]);
  useEffect(() => { if (open) onCatShown?.(cat); }, [open, cat, onCatShown]);
  /** Installed editors (lib.rs `detect_editors`); null until the Commands
   *  page has asked. Probed on entry, like the MCP states below. */
  const [editors, setEditors] = useState<DetectedEditor[] | null>(null);
  /** "Custom…" was CHOSEN. Without it, picking Custom while the stored command
   *  matches a detected entry would snap straight back to that entry. */
  const [editorCustom, setEditorCustom] = useState(false);
  const [codexMcpStatus, setCodexMcpStatus] = useState<CodexMcpStatus | null>(null);
  const [piMcpStatus, setPiMcpStatus] = useState<PiMcpStatus | null>(null);
  useEffect(() => {
    if (!open || cat !== "commands") return;
    let alive = true;
    invoke<CodexMcpStatus>("codex_mcp_status")
      .then((status) => { if (alive) setCodexMcpStatus(status); })
      .catch((e) => { if (alive) onReport(String(e)); });
    invoke<DetectedEditor[]>("detect_editors")
      .then((list) => { if (alive) setEditors(list); })
      .catch((e) => { if (alive) onReport(String(e)); });
    invoke<PiMcpStatus>("pi_mcp_status")
      .then((status) => { if (alive) setPiMcpStatus(status); })
      .catch((e) => { if (alive) onReport(String(e)); });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- probe on entry, not every App render
  }, [open, cat]);

  // …and then say which section it meant. A transient class rather than focus:
  // the sheet body scrolls, and `autoFocus` inside a scrolling box is what
  // pushed a header 34px out of view once already. `scrollIntoView` with
  // `block: "nearest"` moves nothing when the section is on screen.
  const bodyRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open || !at?.focus) return;
    // After the category switch has painted — the section does not exist until
    // `cat` renders it.
    const id = requestAnimationFrame(() => {
      const el = bodyRef.current?.querySelector<HTMLElement>(`[data-focus="${at.focus}"]`);
      if (!el) return;
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
  }, [open, at, cat]);

  const [updating, setUpdating] = useState(false);
  const [updateLog, setUpdateLog] = useState("");
  const [checkState, setCheckState] = useState<"" | "checking" | "done">("");
  const doCheck = async () => {
    setCheckState("checking");
    try { await onCheckUpdate(); } finally { setCheckState("done"); }
  };
  const doUpdate = async () => {
    if (!update?.latest || updating) return;
    setUpdating(true);
    setUpdateLog(`$ install.sh @ ${update.latest}\n`);
    try {
      const r = await invoke<CmdResult>("update_cli", { tag: update.latest });
      setUpdateLog((l) => l + r.output + (r.ok ? "\n✓ done" : `\n✗ failed (exit ${r.code})`));
    } catch (e) {
      setUpdateLog((l) => l + `\n✗ ${String(e)}`);
    } finally {
      // versions re-read BEFORE re-enabling the button — a stale-enabled button
      // in the re-check window would re-run the whole installer on a click
      await onCheckUpdate();
      setUpdating(false);
    }
  };
  const actionable = cliStale || cliMissing;

  // app self-update: signed bundle via tauri-plugin-updater (latest.json
  // endpoint on the release). Verify → download → swap → relaunch.
  const [appUpdating, setAppUpdating] = useState(false);
  const doAppUpdate = async () => {
    if (appUpdating) return;
    setAppUpdating(true);
    setUpdateLog("checking for a signed app update…\n");
    try {
      const up = await checkAppUpdate();
      if (!up) {
        setUpdateLog((l) => l + "no newer signed build published.");
        return;
      }
      setUpdateLog((l) => l + `downloading app ${up.version}…\n`);
      await up.downloadAndInstall();
      setUpdateLog((l) => l + "installed — relaunching…");
      await relaunch();
    } catch (e) {
      const m = `app update failed: ${String(e)}`;
      setUpdateLog((l) => l + `\n✗ ${m}`);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    } finally {
      setAppUpdating(false);
    }
  };

  // Install a specific release — the escape hatch for a release that broke
  // something but left this sheet usable. CLI first (they share
  // ~/.config/worktrees, so they should move together), then the app, then
  // relaunch. The backend re-checks the tag against the published list.
  const [releases, setReleases] = useState<Release[] | null>(null);
  const [releasesErr, setReleasesErr] = useState("");
  const [pickTag, setPickTag] = useState("");
  const [pickArmed, setPickArmed] = useState(false);
  const [pinning, setPinning] = useState(false);
  useEffect(() => { if (!open) setPickArmed(false); }, [open]);
  const loadReleases = async () => {
    setReleasesErr("");
    try {
      const rs = await invoke<Release[]>("list_releases");
      setReleases(rs);
      const cur = update?.app_version ? `v${update.app_version}` : "";
      setPickTag(rs.find((r) => r.tag !== cur && cmpVer(r.tag, cur) < 0)?.tag ?? rs.find((r) => r.tag !== cur)?.tag ?? "");
    } catch (e) {
      setReleasesErr(String(e));
    }
  };
  const doInstallVersion = async () => {
    if (!pickTag || pinning || updating || appUpdating) return;
    if (!pickArmed) { setPickArmed(true); return; } // arm; second click confirms
    setPickArmed(false);
    setPinning(true);
    const tag = pickTag;
    setUpdateLog(`installing ${tag}…\n`);
    try {
      if (update?.cli_version) {
        setUpdateLog((l) => l + `$ install.sh @ ${tag}\n`);
        const r = await invoke<CmdResult>("update_cli", { tag });
        setUpdateLog((l) => l + r.output + "\n");
        if (!r.ok) {
          setUpdateLog((l) => l + `✗ CLI install failed (exit ${r.code}) — app left as is.`);
          return;
        }
      }
      setUpdateLog((l) => l + `downloading app ${tag}…\n`);
      await invoke<string>("install_app_version", { tag });
      setUpdateLog((l) => l + "installed — relaunching…");
      await relaunch();
    } catch (e) {
      const m = `install ${tag} failed: ${String(e)}`;
      setUpdateLog((l) => l + `\n✗ ${m}`);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    } finally {
      await onCheckUpdate();
      setPinning(false);
    }
  };

  // logs (app.log — backend op results, frontend errors, panics)
  const [logPath, setLogPath] = useState("");
  const [logTail, setLogTail] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    invoke<{ dir: string; file: string }>("log_info").then((i) => setLogPath(i.file)).catch(() => {});
  }, [open]);
  // reveal (not open-path): opener:default only grants open-url +
  // reveal-item-in-dir — openPath was silently rejected by the capability
  // system. Reveal also highlights app.log in Finder. Failures are NEVER
  // swallowed: they land in the tail area AND the log itself.
  const openLogsDir = async () => {
    try {
      const i = await invoke<{ dir: string; file: string }>("log_info");
      await revealItemInDir(i.file);
    } catch (e) {
      const m = `open logs folder failed: ${String(e)}`;
      setLogTail(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };
  const viewLogTail = async () => {
    try {
      setLogTail((await invoke<string>("log_tail", { lines: 200 })) || "(empty)");
    } catch (e) {
      setLogTail(String(e));
    }
  };

  // Copy diagnostics: backend assembles the plaintext block (app/cli/PATH/git/
  // tmux/core-config/log-tail); we append the frontend-known UI bits, then write
  // to the clipboard. SettingsSheet has no fail() — errors surface in the logTail
  // area (like openLogsDir) AND land in app.log via log_event.
  const [diagCopied, setDiagCopied] = useState(false);
  const copyDiagnostics = async () => {
    try {
      // `gh`: the Pull requests switch reaches the backend, so "off stops
      // every gh call" holds for this button too (`gh auth status` is a
      // network check, ~0.5s).
      const back = await invoke<string>("diagnostics", { gh: settings.pull_requests });
      const ui =
        `\nUI settings\n-----------\n` +
        `theme   : ${settings.theme}\n` +
        `density : ${settings.density}\n` +
        `ui_rem  : ${settings.ui_rem}px\n` +
        `app_zoom: ${Math.round(clampZoom(settings.app_zoom) * 100)}%\n` +
        `term    : ${settings.term_size}px\n`;
      await copyToClipboard(back + ui);
      setDiagCopied(true);
      setTimeout(() => setDiagCopied(false), 2000);
    } catch (e) {
      const m = `copy diagnostics failed: ${String(e)}`;
      setLogTail(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };

  // AI command config (read-only, Phase 1). Fetched when the sheet opens, like
  // settings_info. Shared with the CLI (~/.config/worktrees/config).
  const [aiConfig, setAiConfig] = useState<AiConfig | null>(null);
  useEffect(() => {
    if (!open) return;
    invoke<AiConfig>("get_ai_config").then(setAiConfig).catch(() => {});
  }, [open]);
  // Reveal the config file — but revealItemInDir REJECTS a non-existent path
  // (the plugin canonicalizes). When exists:false, reveal the PARENT dir (the
  // config dir, which may also not exist). If the parent reveal also fails,
  // surface it in the logTail error area + app.log. Never open-path (the
  // capability doesn't grant it — it rejects silently).
  const revealAiConfig = async () => {
    if (!aiConfig) return;
    try {
      if (aiConfig.exists) {
        await revealItemInDir(aiConfig.path);
      } else {
        const parent = aiConfig.path.replace(/\/[^/]*$/, "") || "/";
        await revealItemInDir(parent);
      }
    } catch (e) {
      // Same error area as Feature 2 (the Logs-section logTail pane) + app.log.
      const m = `reveal AI config failed: ${String(e)}`;
      setLogTail(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };

  // Data section: settings file path (for reveal) + two-click "reset to defaults".
  const [settingsPath, setSettingsPath] = useState("");
  const [dataErr, setDataErr] = useState<string | null>(null);
  const [resetArmed, setResetArmed] = useState(false);
  const [histArmed, setHistArmed] = useState(false);
  const [termHist, setTermHist] = useState<TermHistoryInfo | null>(null);
  useEffect(() => {
    if (!open) return;
    setResetArmed(false); // re-arm each time the sheet opens
    setHistArmed(false);
    invoke<{ dir: string; file: string }>("settings_info").then((i) => setSettingsPath(i.file)).catch(() => {});
    invoke<TermHistoryInfo>("term_history_info").then(setTermHist).catch(() => {});
  }, [open]);
  // reveal (not open-path): opener:default grants reveal-item-in-dir only.
  // Failures are NEVER swallowed — they surface here AND land in app.log.
  const revealSettings = async () => {
    try {
      const i = await invoke<{ dir: string; file: string }>("settings_info");
      await revealItemInDir(i.file);
    } catch (e) {
      const m = `reveal settings file failed: ${String(e)}`;
      setDataErr(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };
  const doReset = () => {
    if (!resetArmed) { setResetArmed(true); return; } // arm; second click confirms
    setResetArmed(false);
    onReset();
  };
  // Saved terminal history — the one store here that holds user CONTENT rather
  // than preferences, so it gets its own size readout and its own two-click
  // clear rather than riding on "reset to defaults".
  const doClearHist = async () => {
    if (!histArmed) { setHistArmed(true); return; }
    setHistArmed(false);
    try {
      await invoke("term_history_clear");
      setTermHist(await invoke<TermHistoryInfo>("term_history_info"));
    } catch (e) {
      const m = `clear terminal history failed: ${String(e)}`;
      setDataErr(m);
      invoke("log_event", { level: "error", msg: m }).catch(() => {});
    }
  };
  // Escape belongs to whatever is ON TOP. What's new opens FROM here and
  // stacks over it; the Escape stack orders by activation, so the notes take
  // the press and Settings stays put until they are gone.
  useEscape(onClose, open);

  if (!open) return null;

  return (
    <div className="modal-scrim" onClick={onClose}>
      {/* A centered modal, not the right-hand sheet it started as: nine
          categories in a 560px side panel left every pane cramped, and the
          terminal keeps painting underneath either way. The inner layout
          (`settings-h` / `settings-split` / `settings-cats` / `settings-body`)
          is unchanged and still shared with the sheets that stayed sheets. */}
      <aside className="modal settings-modal" onClick={(e) => e.stopPropagation()}>
        <header className="settings-h">
          <b>Settings</b>
          <span className="modal-keys">
            <kbd className="kbd">esc</kbd>
            <kbd className="kbd">⌘,</kbd>
          </span>
          <button className="icon-btn" title="close (Esc)" onClick={onClose}><Icons.X /></button>
        </header>

        <div className="settings-split">
        <nav className="settings-cats">
          {CATS.map((c) => (
            <button
              key={c.id}
              className={"settings-cat" + (cat === c.id ? " on" : "") + (c.id === "usage" ? " sep" : "")}
              onClick={() => setCat(c.id)}
            >
              {c.label}
              {c.id === "updates" && actionable ? <span className="upd-tag">upd</span> : null}
              {/* `stale` only, still. An OFFER already has its mark — the dock
                  rail's sparkles button, which opens the list — so badging this
                  category too would say the same thing twice, and the panel it
                  points at can end the suggestion, which is what stops a
                  standing mark being a chore. A BROKEN server is a different
                  claim and earns the mark. */}
              {c.id === "claude" && mcpStatus?.state === "stale" ? <span className="upd-tag warn">!</span> : null}
            </button>
          ))}
        </nav>

        <div className="settings-body" ref={bodyRef}>
          {cat === "terminal" && <>
          <section className="setting">
            <label>Terminal font</label>
            <input
              type="text" value={settings.term_family}
              onChange={(e) => onChange({ term_family: e.currentTarget.value })}
            />
            <label className="sub">Terminal size <span className="val">{settings.term_size}px</span></label>
            <input
              type="range" min={10} max={24} step={1} value={settings.term_size}
              onChange={(e) => onChange({ term_size: clampTerm(+e.currentTarget.value) })}
            />
          </section>

          <section className="setting">
            <label>What a shell tab remembers</label>
            <label className="tier-toggle setting-check">
              <input
                type="checkbox"
                checked={settings.term_persist_scrollback}
                onChange={(e) => onChange({ term_persist_scrollback: e.currentTarget.checked })}
              />
              Keep each tab's output between restarts
            </label>
            <div className="hint">
              A dock shell dies with the app, so its tab used to reopen blank. Its last
              256K of output is saved and replayed instead. This is the only thing that
              writes a terminal's output to disk — whatever a command printed, secrets
              included — under Application Support. Clear it under Data &amp; Logs.
            </div>
            <label className="tier-toggle setting-check">
              <input
                type="checkbox"
                checked={settings.term_per_tab_history}
                onChange={(e) => onChange({ term_per_tab_history: e.currentTarget.checked })}
              />
              Give each tab its own command history
            </label>
            <div className="hint">
              Arrow-up recalls what this tab ran, rather than the one history every
              terminal on the Mac shares. zsh and bash only; your own rc files are read
              as usual and never modified. Turn it off if a shell setup of yours depends
              on $ZDOTDIR.
            </div>
          </section>
          </>}

          {cat === "ai" && <ProfilesPanel key={repo || "none"} repo={repo} onReport={onReport} />}

          {cat === "claude" && <>
          <McpSection status={mcpStatus} repo={repo} offerPending={mcpOfferPending}
            onSilenceOffer={onSilenceMcpOffer} onChanged={onMcpChanged} onReport={onReport} />
          </>}

          {cat === "codex" && <>
          <section className="setting">
            <label>Permissions</label>
            <div className="seg seg-plain" data-testid="codex-permissions">
              {([
                ["ask", "Ask"],
                ["auto-review", "Auto-review"],
                ["full", "Full access"],
              ] as const).map(([mode, label]) => (
                <button key={mode} className={settings.codex_permissions === mode ? "on" : ""}
                  onClick={() => onChange({ codex_permissions: mode })}>
                  {label}
                </button>
              ))}
            </div>
            {/* All three lines, always: the choice is only honest if you can read
                what you are NOT picking. The current one is bold. */}
            {([
              ["ask", "Ask", "Codex asks before running commands."],
              ["auto-review", "Auto-review", "a reviewer approves or refuses each request; commands run sandboxed to this worktree plus the repo's git data (hooks and config included, so commits work) and the network."],
              ["full", "Full access", "no prompts and no sandbox."],
            ] as const).map(([mode, label, line]) => (
              <div className="hint" key={mode}>
                {settings.codex_permissions === mode ? <b>{label}:</b> : `${label}:`} {line}
              </div>
            ))}
            <div className="hint">Applies to the next Codex launch; a session already running keeps what it started with.</div>
          </section>
          {/* ORDER IS LOAD-BEARING: both sections below are deep-link targets
              (offers.ts), and the focus effect scrolls once, on the next frame.
              The MCP panel and its migration table fill in ASYNCHRONOUSLY, so
              anything under them is pushed off screen after the scroll landed
              — measured: "Your skills" ended at y=1258 in an 897px window.
              Skills renders from App's state at once, so it goes above. */}
          <UserSkillsSection data-focus="codex-skills" skills={userSkills} onChanged={onUserSkillsChanged}
            offerPending={skillsOfferPending} onSilenceOffer={onSilenceSkillsOffer} onReport={onReport} />
          <CodexMcpSection data-focus="codex-mcp" onReport={onReport} onStatus={onCodexMcpChanged}
            offerPending={codexMcpOfferPending} onSilenceOffer={onSilenceCodexMcpOffer} />
          </>}

          {cat === "pi" && <PiPanel settings={settings} repo={repo || null} onChange={onChange} onReport={onReport}
            onMcpStatus={(s) => { setPiMcpStatus(s); onPiMcpChanged(s); }}
            mcpOfferPending={piMcpOfferPending} onSilenceMcpOffer={onSilencePiMcpOffer} />}

          {cat === "guidance" && <>
          <GuidanceSection data-focus="agent-guidance" status={guidance} onStatus={onGuidanceChanged} onReport={onReport}
            offerPending={guidanceOfferPending} onSilenceOffer={onSilenceGuidanceOffer}
            changeOfferPending={guidanceChangeOfferPending} onSilenceChangeOffer={onSilenceGuidanceChangeOffer} />
          <CrossProjectSection data-focus="cross-project" status={crossProject} onStatus={onCrossProjectChanged} onReport={onReport}
            offerPending={crossProjectOfferPending} onSilenceOffer={onSilenceCrossProjectOffer} />
          </>}

          {cat === "commands" && <>
          <section className="setting">
            <label>Default agent</label>
            <div className="seg">
              {HARNESSES.map((provider) => (
                <button key={provider} className={settings.default_provider === provider ? "on" : ""}
                  onClick={() => onChange({ default_provider: provider })}>
                  {HARNESS_LABEL[provider]}
                </button>
              ))}
            </div>
            <div className="hint">Preselected for new worktrees when more than one agent is installed. Entering a place keeps its running agent; this default applies when none is running. Switching agents closes the current session first. pi also needs a default model (Settings → pi), or it asks for one each time.</div>
          </section>
          <section className="setting">
            <label>Worktrees tools for agents</label>
            <div className="hint">Every agent can be connected at the same time. Their MCP setup is independent of the default agent.</div>
            <div className="hint">Claude: {agentSetupLabel(mcpStatus?.state ?? null)} · Codex: {agentSetupLabel(codexMcpStatus?.state ?? null)} · pi: {agentSetupLabel(piMcpStatus?.state ?? null)}</div>
            <div className="ver-actions">
              <button className="ctrl sm" onClick={() => setCat("claude")}>Configure Claude</button>
              <button className="ctrl sm" onClick={() => setCat("codex")}>Configure Codex</button>
              <button className="ctrl sm" onClick={() => setCat("pi")}>Configure pi</button>
            </div>
          </section>
          <section className="setting">
            <label>Commands</label>
            <label className="sub">Editor</label>
            {(() => {
              // `editor_cmd` stays the ONE stored value: a pick writes its
              // command there, and a stored command that matches no detected
              // entry simply reads as Custom.
              const known = editors?.some((e) => e.cmd === settings.editor_cmd) ?? false;
              const custom = editors === null || editors.length === 0 || editorCustom || !known;
              return (
                <>
                  {editors !== null && editors.length > 0 && (
                    <select
                      value={custom ? EDITOR_CUSTOM : settings.editor_cmd}
                      onChange={(e) => {
                        const v = e.currentTarget.value;
                        if (v === EDITOR_CUSTOM) { setEditorCustom(true); return; }
                        setEditorCustom(false);
                        onChange({ editor_cmd: v });
                      }}
                    >
                      {editors.map((e) => <option key={e.cmd} value={e.cmd}>{e.label}</option>)}
                      <option value={EDITOR_CUSTOM}>Custom…</option>
                    </select>
                  )}
                  {custom && (
                    <input
                      type="text" value={settings.editor_cmd}
                      aria-label="Editor command"
                      // Typing IS choosing Custom. Without this, a field shown
                      // only because the stored value matched nothing unmounts
                      // — focused, mid-word — the keystroke its text first
                      // equals a detected entry's command.
                      onChange={(e) => { setEditorCustom(true); onChange({ editor_cmd: e.currentTarget.value }); }}
                    />
                  )}
                </>
              );
            })()}
            <div className="hint">
              Used by right-click “Open in editor” and ⌘E.
              {editors !== null && editors.length === 0 && " No known editor was found in Applications or on your PATH."}
              {" "}A custom command takes quoted args (e.g. code, cursor, open -a "Visual Studio Code"); the file is added as the last argument.
            </div>
            <label className="sub">Terminal command</label>
            <input
              type="text" value={settings.terminal_cmd}
              onChange={(e) => onChange({ terminal_cmd: e.currentTarget.value })}
            />
            <div className="hint">Right-click “Open in terminal app”. {"{session}"} is the tmux session name, already shell-quoted (e.g. ghostty -e tmux attach -t {"{session}"}). Leave empty to hide the item.</div>
            <label className="tier-toggle setting-check">
              <input
                type="checkbox"
                checked={settings.ai_auto_resume}
                onChange={(e) => onChange({ ai_auto_resume: e.currentTarget.checked })}
              />
              Resume agent conversation on open
            </label>
            <div className="hint">Single-click Enter resumes the selected agent's last conversation when available.</div>
            <div className="ver-rows">
              <div className="ver-row">
                AI command: <b>{aiConfig?.ai_cmd ?? "…"}</b>
                {aiConfig && <> · resume: <b>{aiConfig.ai_resume_arg}</b></>}
              </div>
            </div>
            <div className="ver-actions">
              <button className="ctrl sm" onClick={revealAiConfig} disabled={!aiConfig}>Reveal config file</button>
            </div>
            <div className="hint">The CLI uses this command when you omit --ai. The app uses Default agent above. Shell env vars (WORKTREES_AI_CMD) don't reach the GUI.</div>
          </section>
          </>}

          {cat === "appearance" && <>
          {/* Overall size first: it is the knob that moves EVERYTHING, and the
              two below it are then a preference within it. The slider walks
              ZOOM_STEPS by index rather than taking a percentage directly, so
              it can only ever land on a legal step (browser zoom is
              multiplicative — a linear range gives useless 1% moves at 3×). */}
          <section className="setting">
            <label>Overall size <span className="val">{Math.round(clampZoom(settings.app_zoom) * 100)}%</span></label>
            <input
              type="range" min={0} max={ZOOM_STEPS.length - 1} step={1}
              value={Math.max(0, ZOOM_STEPS.indexOf(clampZoom(settings.app_zoom) as (typeof ZOOM_STEPS)[number]))}
              onChange={(e) => onChange({ app_zoom: ZOOM_STEPS[+e.currentTarget.value] })}
            />
            <div className="hint">⌘+ / ⌘− / ⌘0 anywhere in the app. Scales the whole window — chrome, icons, files and the terminal, which re-fits its tmux pane to the new size. (⌘⌥+ / ⌘⌥− is the markdown reader's own size.)</div>
          </section>

          <section className="setting">
            <label>UI font size <span className="val">{settings.ui_rem}px</span></label>
            <input
              type="range" min={13} max={22} step={1} value={settings.ui_rem}
              onChange={(e) => onChange({ ui_rem: clampRem(+e.currentTarget.value) })}
            />
            <div className="preview">The quick brown fox jumps</div>
            <div className="hint">The chrome's base size, before Overall size multiplies it. Does not touch the terminal — that has its own size below.</div>
          </section>

          <section className="setting">
            <label>Theme</label>
            <select value={settings.theme} onChange={(e) => onChange({ theme: e.currentTarget.value as ThemeSetting })}>
              <option value="system">System (match macOS)</option>
              {THEMES.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.label} ({t.appearance})
                </option>
              ))}
            </select>
            {settings.theme === "system" && (
              <>
                <label className="sub">Light ↔ dark pair</label>
                <div className="row2">
                  <select value={settings.theme_light} onChange={(e) => onChange({ theme_light: e.currentTarget.value as ThemeId })}>
                    {THEMES.filter((t) => t.appearance === "light").map((t) => (
                      <option key={t.id} value={t.id}>{t.label}</option>
                    ))}
                  </select>
                  <span className="times">↔</span>
                  <select value={settings.theme_dark} onChange={(e) => onChange({ theme_dark: e.currentTarget.value as ThemeId })}>
                    {THEMES.filter((t) => t.appearance === "dark").map((t) => (
                      <option key={t.id} value={t.id}>{t.label}</option>
                    ))}
                  </select>
                </div>
                <div className="hint">Follows macOS appearance: this light theme by day, this dark theme in dark mode.</div>
              </>
            )}
          </section>

          <section className="setting">
            <label>Density</label>
            <div className="seg">
              {(["comfortable", "compact"] as const).map((d) => (
                <button
                  key={d}
                  className={settings.density === d ? "on" : ""}
                  onClick={() => onChange({ density: d })}
                >
                  {d}
                </button>
              ))}
            </div>
          </section>

          {/* The Claude plan bars used to be sidebar furniture, which stopped
              meaning "on screen" once the sidebar started auto-hiding. Three
              hosts now, and a real off switch — "off" stops the poll itself. */}
          <section className="setting">
            <label>Usage meter</label>
            <div className="seg seg-plain">
              {([
                ["strip", "Strip"],
                ["footer", "Footer"],
                ["rail", "Rail"],
                ["off", "Off"],
              ] as const).map(([v, label]) => (
                <button
                  key={v}
                  className={settings.usage_place === v ? "on" : ""}
                  onClick={() => onChange({ usage_place: v })}
                >
                  {label}
                </button>
              ))}
            </div>
            <div className="hint">
              Where account plan limits live. Strip is a row across the bottom of the window, so it
              stays put through ⌘B and is the only one that also shows on Home. Footer puts them
              under the terminal, on a place only. Rail is a tile beside the ＋ and ⚙ icons. Hover
              any of them for the full windows and how long until each resets.
            </div>
            <label className="usage-setting-toggle"><input type="checkbox" checked={settings.usage_claude}
              onChange={e => onChange({ usage_claude: e.target.checked })} /> Claude plan usage</label>
            <label className="usage-setting-toggle"><input type="checkbox" checked={settings.usage_codex}
              onChange={e => onChange({ usage_codex: e.target.checked })} /> Codex plan usage</label>
            <div className="hint">Account limits use your existing CLI sign-ins. Turning a provider off stops its usage checks.</div>
          </section>
          </>}

          {cat === "behavior" && <>
          <section className="setting">
            <label>Startup</label>
            <label className="tier-toggle setting-check">
              <input
                type="checkbox"
                checked={settings.restore_last}
                onChange={(e) => onChange({ restore_last: e.currentTarget.checked })}
              />
              Restore last place on launch
            </label>
            <div className="hint">Reopens the most recently used place, ready but not started — press Enter to attach.</div>
            <label className="tier-toggle setting-check">
              <input
                type="checkbox"
                checked={settings.restore_window}
                onChange={(e) => onChange({ restore_window: e.currentTarget.checked })}
              />
              Restore window on launch
            </label>
            <div className="hint">
              Reopens the window the way you left it — size, position, and full screen — after a
              quit or an update. Takes effect the next time the app starts.
            </div>
          </section>

          <section className="setting">
            <label>Git</label>
            <label className="sub">Auto-fetch origin</label>
            <select
              value={settings.fetch_interval_min}
              onChange={(e) => onChange({ fetch_interval_min: +e.currentTarget.value })}
            >
              <option value={0}>Off</option>
              <option value={5}>Every 5 min</option>
              <option value={15}>Every 15 min</option>
              <option value={60}>Every 60 min</option>
            </select>
            <div className="hint">Keeps the divergence counts fresh. Fetches each project's origin in the background.</div>
          </section>

          <section className="setting">
            <label>Pull requests</label>
            <label className="tier-toggle setting-check">
              <input
                type="checkbox"
                checked={settings.pull_requests}
                onChange={(e) => onChange({ pull_requests: e.currentTarget.checked })}
              />
              Show pull requests for GitHub projects
            </label>
            <div className="hint">
              A chip in a place's header for its branch's PR, a Pull requests tab in the dock, and a
              count on its icon. Read through the GitHub CLI (<code>gh</code>) for the selected
              project only, while the window is visible. Off stops every <code>gh</code> call.
            </div>
          </section>

          <QuotaSection data-focus="plan-limits" onReport={onReport} />
          </>}

          {cat === "updates" && <>
          <section className="setting">
            <label>Version{actionable ? <span className="upd-tag">{cliMissing ? "cli not installed" : "update available"}</span> : null}</label>
            <label className="tier-toggle setting-check">
              <input
                type="checkbox"
                checked={settings.update_auto_check}
                onChange={(e) => onChange({ update_auto_check: e.currentTarget.checked })}
              />
              Check for updates automatically
            </label>
            <div className="hint">Hourly while the window is open. “Check for updates” below always works regardless of this setting.</div>
            <div className="ver-rows">
              <div className="ver-row">app <b>{update?.app_version ?? "…"}</b></div>
              <div className="ver-row">
                cli{" "}
                {update?.cli_version ? (
                  <><b>{update.cli_version}</b> <span className="ver-path" title={update.cli_path ?? ""}>{update.cli_path}</span></>
                ) : (
                  <i>not installed</i>
                )}
              </div>
              <div className="ver-row">latest {update?.latest ? <b>{update.latest}</b> : <i>unknown (offline?)</i>}</div>
            </div>
            <div className="ver-actions">
              <button className="ctrl sm" disabled={checkState === "checking"} onClick={doCheck}>
                {checkState === "checking" ? "Checking…" : "Check for updates"}
              </button>
              <button className="ctrl sm" onClick={onShowNotes}>Release notes</button>
              {actionable && update?.latest && (
                <button className="ctrl sm" disabled={updating || appUpdating} onClick={doUpdate}>
                  {updating ? "Updating…" : `${cliMissing ? "Install" : "Update"} CLI → ${update.latest}`}
                </button>
              )}
              {appStale && update?.latest && (
                <button className="ctrl sm" disabled={appUpdating || updating} onClick={doAppUpdate}>
                  {appUpdating ? "Updating app…" : `Update app → ${update.latest}`}
                </button>
              )}
            </div>
            {checkState === "done" && (
              <div className="hint">
                {update?.latest
                  ? actionable || appStale
                    ? `updates available (latest ${update.latest})`
                    : `✓ up to date (latest ${update.latest})`
                  : "✗ couldn't reach the release feed (offline?)"}
              </div>
            )}
            {updateLog && <pre className="update-log">{updateLog}</pre>}
          </section>

          <section className="setting">
            <label>Install another version</label>
            {releases === null ? (
              <div className="ver-actions">
                <button className="ctrl sm" onClick={loadReleases}>Choose a version…</button>
              </div>
            ) : (
              <div className="ver-actions">
                <select
                  value={pickTag}
                  disabled={pinning}
                  onChange={(e) => { setPickTag(e.currentTarget.value); setPickArmed(false); }}
                >
                  {releases.map((r) => {
                    const cur = r.tag === `v${update?.app_version}`;
                    return (
                      <option key={r.tag} value={r.tag} disabled={cur}>
                        {r.tag} · {r.published}{cur ? " (current)" : ""}
                      </option>
                    );
                  })}
                </select>
                <button
                  className="ctrl sm"
                  disabled={!pickTag || pinning || updating || appUpdating}
                  onClick={doInstallVersion}
                >
                  {pinning
                    ? "Installing…"
                    : pickArmed
                      ? `Confirm: install ${pickTag}`
                      : `${cmpVer(pickTag, `v${update?.app_version ?? ""}`) < 0 ? "Roll back" : "Install"} → ${pickTag}`}
                </button>
              </div>
            )}
            {releasesErr && <div className="hint">✗ {releasesErr}</div>}
            <div className="hint">
              For when a release breaks something. Installs the app{update?.cli_version ? " and the CLI" : ""} at
              that version and relaunches; “Update app” brings you back to the latest. Your places and settings are kept.
            </div>
            <ManualInstall
              tag={pickTag || update?.latest || "vX.Y.Z"}
              cliDir={update?.cli_path ? update.cli_path.replace(/\/[^/]*$/, "") : null}
            />
          </section>
          </>}

          {cat === "navigation" && <>
          {/* One control for what used to be two booleans and a rail button.
              The pair it writes is `nav_pinned` + `nav_hover_reveal`, and only
              the middle state uses both — "Hidden" is unpinned with the pointer
              trigger off, which is why it still opens on ⌘B. */}
          <section className="setting">
            <label>Sidebar</label>
            <div className="seg seg-plain">
              {([
                ["pinned", "Pinned", { nav_pinned: true }],
                ["auto", "Auto-hide", { nav_pinned: false, nav_hover_reveal: true }],
                ["hidden", "Hidden", { nav_pinned: false, nav_hover_reveal: false }],
              ] as const).map(([mode, label, patch]) => {
                const cur = settings.nav_pinned ? "pinned" : settings.nav_hover_reveal ? "auto" : "hidden";
                return (
                  <button key={mode} className={cur === mode ? "on" : ""} onClick={() => onChange(patch)}>
                    {label}
                  </button>
                );
              })}
            </div>
            <div className="hint">
              ⌘B and the Places icon switch between pinned and hidden. With auto-hide, the sidebar
              slides over the terminal when the pointer reaches the rail or on ⌘B, and closes on Esc
              or when you open a place.
            </div>
          </section>

          {/* One control for BOTH halves of the shell. See `places_side` in
              settings.ts for why it is a mirror rather than a side per panel:
              the nav and the dock are not interchangeable hosts, so the thing
              that can move is which edge each of them owns. */}
          <section className="setting">
            <label>Sides</label>
            <div className="seg seg-plain">
              {([
                ["left", "Places left"],
                ["right", "Places right"],
              ] as const).map(([side, label]) => (
                <button
                  key={side}
                  className={settings.places_side === side ? "on" : ""}
                  onClick={() => onChange({ places_side: side })}
                >
                  {label}
                </button>
              ))}
            </div>
            <div className="hint">
              Which window edge each half of the app owns. The Places rail and sidebar take the side
              you pick; the Files / Terminal / Docs rail and panel take the other one. Widths, ⌘B and
              ⌘J are unchanged — only the sides move, and each resizer moves with its panel.
            </div>
          </section>

          <section className="setting">
            <label>Nav width <span className="val">{settings.nav_width}px</span></label>
            <input
              type="range" min={220} max={460} step={10} value={settings.nav_width}
              onChange={(e) => onChange({ nav_width: clampNav(+e.currentTarget.value) })}
            />
          </section>

          {/* The purple dot's decay. Horizon walks DONE_HORIZONS by index for
              the same reason Overall size walks ZOOM_STEPS: a linear range over
              1h..7d spends most of its travel on moves nobody can see. The hint
              prints the boundaries from `doneBounds` itself rather than a
              hand-written example, so it cannot go stale when either knob or
              the geometry changes. */}
          <section className="setting">
            <label>Afterglow</label>
            <label className="sub">Fade out over <span className="val">{fmtSecs(snapHorizon(settings.done_horizon_secs))}</span></label>
            <input
              type="range" min={0} max={DONE_HORIZONS.length - 1} step={1}
              value={Math.max(0, DONE_HORIZONS.indexOf(snapHorizon(settings.done_horizon_secs) as (typeof DONE_HORIZONS)[number]))}
              onChange={(e) => onChange({ done_horizon_secs: DONE_HORIZONS[+e.currentTarget.value] })}
            />
            <label className="sub">Steps <span className="val">{clampSteps(settings.done_steps)}</span></label>
            <input
              type="range" min={DONE_STEPS_MIN} max={DONE_STEPS_MAX} step={1} value={clampSteps(settings.done_steps)}
              onChange={(e) => onChange({ done_steps: clampSteps(+e.currentTarget.value) })}
            />
            <div className="hint">
              The purple dot marks a place where Claude finished, fading in steps:{" "}
              {doneBounds(settings.done_horizon_secs, settings.done_steps).map(fmtSecs).join(" · ")}.
              The first step is always {fmtSecs(DONE_FIRST_SECS)} and is the one that lights the project folder.
              A finish you have not looked at yet holds the first step, however old it is, until you select the place.
            </div>
          </section>

          <section className="setting">
            <label>Nav tiers</label>
            <div className="tier-toggles">
              {(["active", "idle", "dormant"] as const).map((t) => (
                <label key={t} className="tier-toggle">
                  <input
                    type="checkbox"
                    checked={!settings.hidden_tiers.includes(t)}
                    onChange={(e) => {
                      const show = e.currentTarget.checked;
                      onChange({
                        hidden_tiers: show
                          ? settings.hidden_tiers.filter((x) => x !== t)
                          : [...settings.hidden_tiers, t],
                      });
                    }}
                  />
                  {t}
                </label>
              ))}
            </div>
            <div className="hint">Pinned and (main) always show. Sort order lives in the nav header.</div>
          </section>

          <section className="setting">
            <label>Tree guide lines</label>
            <label className="tier-toggle setting-check">
              <input
                type="checkbox"
                checked={settings.nav_guides}
                onChange={(e) => onChange({ nav_guides: e.currentTarget.checked })}
              />
              Draw the vertical guides
            </label>
            <div className="hint">The 1px rails that connect a project to its places. Off leaves the indentation alone.</div>
          </section>
          </>}

          {cat === "data" && <>
          <section className="setting">
            <label>Logs</label>
            <div className="ver-rows">
              <div className="ver-row"><span className="ver-path" title={logPath}>{logPath || "…"}</span></div>
            </div>
            <div className="ver-actions">
              <button className="ctrl sm" onClick={openLogsDir}>Open folder</button>
              <button className="ctrl sm" onClick={viewLogTail}>{logTail ? "Refresh tail" : "View tail"}</button>
              <button className="ctrl sm" onClick={copyDiagnostics}>{diagCopied ? "Copied ✓" : "Copy diagnostics"}</button>
            </div>
            {logTail && <pre className="update-log log-tail">{logTail}</pre>}
          </section>

          <section className="setting">
            <label>Data</label>
            <div className="ver-rows">
              <div className="ver-row"><span className="ver-path" title={settingsPath}>{settingsPath || "…"}</span></div>
            </div>
            <div className="ver-actions">
              <button className="ctrl sm" onClick={revealSettings}>Reveal settings file</button>
              <button className={"ctrl sm danger" + (resetArmed ? " armed" : "")} onClick={doReset}>
                {resetArmed ? "Confirm reset?" : "Reset to defaults"}
              </button>
            </div>
            {resetArmed && <div className="hint">Restores every setting to its default (theme, fonts, nav, sort, tiers).</div>}
            <div className="ver-rows">
              <div className="ver-row">
                <span className="ver-path" title={termHist?.dir}>
                  {termHist ? `Saved terminal history — ${termHist.tabs} tab(s), ${humanSize(termHist.bytes)}` : "…"}
                </span>
              </div>
            </div>
            <div className="ver-actions">
              <button className={"ctrl sm danger" + (histArmed ? " armed" : "")} onClick={doClearHist}>
                {histArmed ? "Confirm clear?" : "Clear terminal history"}
              </button>
            </div>
            {histArmed && <div className="hint">Forgets every tab's saved output and command history. Terminals already on screen keep what they are showing.</div>}
            {dataErr && <pre className="update-log">{dataErr}</pre>}
          </section>
          </>}


          {cat === "usage" && <UsagePanel />}

          {cat === "shortcuts" && <>
          <section className="setting">
            <label>Shortcuts</label>
            <div className="shortcuts">
              {[
                ["⌘B", "Sidebar — reveal, then pin (unpin when pinned)"],
                ["⌘J", "Toggle the dock (Files / Terminal)"],
                ["⌘K", "Quick switcher"],
                ["⌘,", "Open Settings"],
                ["⌘1", "Home (briefing)"],
                ["⌘2", "Places — reveal the sidebar and focus the filter"],
                ["⌘E", "Open selection in editor"],
                ["⌘F", "Find — in the terminal, or in the open file"],
                ["⌘S", "Save the markdown file you are editing in Source view"],
                ["⌘B / ⌘I", "Bold / italic — while the markdown editor has focus (⌘B is the sidebar everywhere else)"],
                ["⌘T", "New terminal in the dock"],
                ["⌘N", "New worktree — in the current project, or pick one"],
                ["⌘⇧E", "Read the dock's file over the whole pane"],
                ["⌘+ / ⌘−", "Overall size — the whole window, terminal included"],
                ["⌘0", "Overall size back to 100%"],
                ["⌘⌥+ / ⌘⌥−", "Reading size of a rendered markdown file"],
                ["Esc", "Close sheets, menus & the revealed sidebar"],
              ].map(([key, desc]) => (
                <div className="shortcut-row" key={key}>
                  <kbd>{key}</kbd>
                  <span>{desc}</span>
                </div>
              ))}
            </div>
          </section>
          </>}
        </div>
        </div>
      </aside>
    </div>
  );
}
