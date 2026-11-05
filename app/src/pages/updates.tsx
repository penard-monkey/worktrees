// Settings → Updates: this app and its CLI, the agent CLIs with the restarts
// their updates call for (Advanced), and installing another release (Expert).
//
// The RUN state (logs, "Updating…") lives in the sheet, not the page
// (`useUpdateRuns`, called by SettingsSheet): a page unmounts when you click
// to another one, and an Update button that came back enabled mid-install
// would run the whole installer again.
import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check as checkAppUpdate } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { invoke, notGranted } from "../ipc";
import { copyToClipboard } from "../clipboard";
import type { HarnessManual, UpdateInfo } from "../settings";
import { harnessPaneAction } from "../updatepoll";
import {
  forceConfirm, forceLabel, inactiveConfirm, inactiveLabel, outsideLine, restartPlan, resultLine, showForce, showInactive,
  summaryLine, tallyLine, type HarnessRestarted, type HarnessSessions,
} from "../harnessrestart";
import { useArmed } from "../useArmed";
import { Page, Sec, useSheet } from "./kit";

type CmdResult = { ok: boolean; code: number; output: string; slug?: string | null; warnings?: string[] };
type Release = { tag: string; published: string };

export const RELEASES_URL = "https://github.com/penard-monkey/worktrees/releases";

/** Compare "vX.Y.Z" tags numerically; an unparseable side sorts as equal. */
function cmpVer(a: string, b: string): number {
  const p = (t: string) => t.replace(/^v/, "").split(/[.-]/).slice(0, 3).map(Number);
  const [x, y] = [p(a), p(b)];
  if (x.length < 3 || y.length < 3 || [...x, ...y].some(Number.isNaN)) return 0;
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i];
  return 0;
}

/** Every update the sheet can be running, and the logs they write. */
export function useUpdateRuns({ open, update, onCheckUpdate }: {
  open: boolean;
  update: UpdateInfo | null;
  onCheckUpdate: () => Promise<unknown> | void;
}) {
  const [updating, setUpdating] = useState(false);
  const [updateLog, setUpdateLog] = useState("");
  const [harnessLogs, setHarnessLogs] = useState<Record<string, { running: boolean; output: string }>>({});
  const [checkState, setCheckState] = useState<"" | "checking" | "done">("");
  const [appUpdating, setAppUpdating] = useState(false);
  const [releases, setReleases] = useState<Release[] | null>(null);
  const [releasesErr, setReleasesErr] = useState("");
  const [pickTag, setPickTag] = useState("");
  const [pinning, setPinning] = useState(false);
  const pickArm = useArmed<"install">();
  const { disarm } = pickArm;
  useEffect(() => { if (!open) disarm(); }, [open, disarm]);

  const doHarnessUpdate = async (tool: string, command: string) => {
    if (harnessLogs[tool]?.running) return;
    setHarnessLogs((prev) => ({ ...prev, [tool]: { running: true, output: `$ ${command}\n` } }));
    try {
      const r = await invoke<CmdResult>("update_harness", { tool });
      setHarnessLogs((prev) => ({ ...prev, [tool]: {
        running: false, output: `$ ${command}\n${r.output}\n${r.ok ? "✓ done" : `✗ failed (exit ${r.code})`}`,
      } }));
    } catch (e) {
      setHarnessLogs((prev) => ({ ...prev, [tool]: { running: false, output: `$ ${command}\n✗ ${String(e)}` } }));
    } finally {
      await onCheckUpdate();
    }
  };
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
  // app self-update: signed bundle via tauri-plugin-updater (latest.json
  // endpoint on the release). Verify → download → swap → relaunch.
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
  const installVersion = async () => {
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
  const doInstallVersion = () => {
    if (!pickTag || pinning || updating || appUpdating) return;
    pickArm.confirm("install", () => void installVersion());
  };
  const choose = (tag: string) => { setPickTag(tag); pickArm.disarm(); };
  return {
    updating, updateLog, harnessLogs, checkState, appUpdating, releases, releasesErr, pickTag, pinning,
    pickArmed: pickArm.armed === "install", doHarnessUpdate, doCheck, doUpdate, doAppUpdate, loadReleases, doInstallVersion, choose,
  };
}
export type UpdateRuns = ReturnType<typeof useUpdateRuns>;

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

/** The host's by-hand steps for an agent CLI it won't update itself. Each
 *  command is fixed on the host; this copies it and never runs it. Open by
 *  default only while a newer version is out. */
function HarnessSteps({ tool, manual, open }: { tool: string; manual: HarnessManual; open: boolean }) {
  const [copied, setCopied] = useState(-1);
  const copy = (i: number, cmd: string) =>
    copyToClipboard(cmd).then(() => { setCopied(i); setTimeout(() => setCopied(-1), 2000); }).catch(() => {});
  return (
    <details className="manual-install harness-steps" open={open} data-testid={`harness-steps-${tool}`}>
      <summary>How to update</summary>
      <ol>
        {manual.steps.map((s, i) => (
          <li key={i} className="manual-row">
            <div className="manual-head">
              <span className="sub">{s.text}</span>
              {s.command && (
                <button className="ctrl sm" onClick={() => copy(i, s.command!)}>{copied === i ? "Copied" : "Copy"}</button>
              )}
            </div>
            {s.command && <pre className="update-log">{s.command}</pre>}
          </li>
        ))}
      </ol>
    </details>
  );
}

/** Under a tool's Agent CLIs row: the sessions in worktrees places that run an
 *  older build than the one installed, and the two restarts the owner asked
 *  for (2026-11-03) — the inactive ones, or all of them, each a two-click
 *  confirm (docs/proposals/harness-restart.md). Shown only while some are
 *  older. Re-read when the installed version moves (after an Update) and
 *  after a restart; `harness_sessions` is live on the host, not cached. */
function HarnessRestartPanel({ tool, label, installed }: { tool: "claude" | "codex"; label: string; installed: string | null }) {
  const [list, setList] = useState<HarnessSessions | null>(null);
  const arm = useArmed<"inactive" | "force">();
  const { armed } = arm;
  const [running, setRunning] = useState<"inactive" | "force" | null>(null);
  const [done, setDone] = useState<HarnessRestarted | null>(null);
  const [err, setErr] = useState("");
  const load = useCallback(async () => {
    try {
      setList(await invoke<HarnessSessions>("harness_sessions", { tool }));
      setErr("");
    } catch (e) {
      setList(null);
      setErr(String(e));
    }
  }, [tool]);
  // A remote window paired without admin cannot restart: it says why
  // instead of offering buttons the host would refuse.
  const denied = notGranted("restart_harness_sessions");
  useEffect(() => { void load(); }, [load, installed]);
  const plan = restartPlan(list);
  const run = async (which: "inactive" | "force") => {
    setRunning(which);
    setDone(null);
    setErr("");
    try {
      setDone(await invoke<HarnessRestarted>("restart_harness_sessions", { tool, force: which === "force" }));
    } catch (e) {
      setErr(String(e));
    } finally {
      setRunning(null);
      await load();
    }
  };
  const go = (which: "inactive" | "force") => {
    if (running) return;
    arm.confirm(which, () => void run(which));
  };
  const summary = summaryLine(label, list);
  const outside = outsideLine(label, list);
  if (!summary && !outside && !done && !err) return null;
  return (
    <div className="harness-restart" data-testid={`harness-restart-${tool}`}>
      {summary && <div className="harness-restart-summary">{summary}</div>}
      {denied && (showInactive(plan) || showForce(plan)) && <div className="harness-update-meta">{denied}</div>}
      {!denied && (showInactive(plan) || showForce(plan)) && (
        <div className="harness-restart-actions">
          {showInactive(plan) && (
            <button className={`ctrl sm${armed === "inactive" ? " armed" : ""}`} data-testid={`restart-inactive-${tool}`}
              disabled={!!running} onClick={() => go("inactive")}>
              {running === "inactive" ? "Restarting…" : armed === "inactive" ? "Confirm restart" : inactiveLabel(plan)}
            </button>
          )}
          {showForce(plan) && (
            <button className={`ctrl sm restart-force${armed === "force" ? " armed" : ""}`} data-testid={`restart-force-${tool}`}
              disabled={!!running} onClick={() => go("force")}>
              {running === "force" ? "Restarting…" : armed === "force" ? "Confirm force restart" : forceLabel(plan)}
            </button>
          )}
        </div>
      )}
      {armed && (
        <div className={`harness-restart-confirm${armed === "force" ? " force" : ""}`} data-testid={`restart-confirm-${tool}`}>
          {armed === "force" ? forceConfirm(label, plan) : inactiveConfirm(label, plan)} Click again to confirm.
        </div>
      )}
      {plan.unresumable > 0 && (
        <div className="harness-update-meta">
          {plan.unresumable === 1 ? "1 of them" : `${plan.unresumable} of them`} can't be resumed exactly, so Worktrees leaves {plan.unresumable === 1 ? "it" : "them"} running.
        </div>
      )}
      {outside && <div className="harness-update-meta" data-testid={`restart-outside-${tool}`}>{outside}</div>}
      {done && (
        <div className="harness-restart-results" data-testid={`restart-results-${tool}`}>
          <div className="harness-restart-tally">{tallyLine(done)}</div>
          {done.results.map((r) => (
            <div key={`${r.project}|${r.session}`} className={`harness-restart-result ${r.outcome}`}>{resultLine(r)}</div>
          ))}
        </div>
      )}
      {err && <div className="harness-update-meta" data-testid={`restart-error-${tool}`}>✗ {err}</div>}
    </div>
  );
}

export function UpdatesPage() {
  const { settings, onChange, update, cliStale, cliMissing, appStale, onShowNotes, runs } = useSheet();
  const actionable = cliStale || cliMissing;
  const {
    updating, updateLog, harnessLogs, checkState, appUpdating, releases, releasesErr, pickTag, pinning, pickArmed,
    doHarnessUpdate, doCheck, doUpdate, doAppUpdate, loadReleases, doInstallVersion, choose,
  } = runs;
  return <Page page="updates">
    <Sec id="version">
      <section className="setting">
        <label>Version{actionable ? <span className="upd-tag">{cliMissing ? "cli not installed" : "update available"}</span> : null}</label>
        <label className="tier-toggle setting-check">
          <input type="checkbox" checked={settings.update_auto_check}
            onChange={(e) => onChange({ update_auto_check: e.currentTarget.checked })} />
          Check for updates automatically
        </label>
        <div className="hint">Hourly while this window is open. “Check for updates” below always works regardless of this setting.</div>
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
            {update?.latest || update?.harnesses?.some((h) => h.latest_version)
              ? actionable || appStale || update?.harnesses?.some((h) => h.update_available)
                ? "updates available"
                : "✓ up to date"
              : "✗ couldn't reach the release feeds (offline?)"}
          </div>
        )}
        {updateLog && <pre className="update-log">{updateLog}</pre>}
      </section>
    </Sec>

    <Sec id="agent-clis">
      <section className="setting" data-testid="harness-updates">
        <label>Agent CLIs{update?.harnesses?.some((h) => h.update_available) ? <span className="upd-tag">update available</span> : null}</label>
        <div className="hint">Check for updates includes these tools. A running agent keeps the version it started with: after an update, restart the Claude and Codex sessions listed under each tool, or restart the others yourself.</div>
        <div className="harness-updates">
          {(update?.harnesses ?? [
            { tool: "claude" }, { tool: "codex" }, { tool: "kimi" }, { tool: "pi" },
          ]).map((h) => {
            const known = "install_method" in h;
            const method = known ? h.install_method : "";
            const action = known ? harnessPaneAction(h) : "unknown";
            const canUpdate = action === "update";
            const name = h.tool === "pi" ? "Pi" : h.tool[0].toUpperCase() + h.tool.slice(1);
            const log = harnessLogs[h.tool];
            return <div className="harness-update" key={h.tool} data-testid={`harness-update-${h.tool}`}>
              <div className="harness-update-main">
                <b>{name}</b>
                <span className="harness-update-versions">
                  {known && h.installed_version ? h.installed_version : "not installed"}
                  {known && h.latest_version ? ` → ${h.latest_version}` : " · latest unknown"}
                </span>
                {canUpdate && <button className="ctrl sm" data-testid={`update-${h.tool}`}
                  disabled={!!log?.running} onClick={() => void doHarnessUpdate(h.tool, "update_command" in h ? h.update_command! : "")}>
                  {log?.running ? "Updating…" : "Update"}
                </button>}
              </div>
              {known && h.installed_version && <div className="harness-update-meta">
                {h.manual ? h.manual.why
                  : method === "unknown" || method === "legacy-python" ? "Worktrees can't update this install itself."
                  : method === "not-installed" ? "Not installed"
                  : `Installed with ${method}${h.update_command ? ` · ${h.update_command}` : ""}`}
              </div>}
              {known && h.installed_version && h.manual && (
                <HarnessSteps tool={h.tool} manual={h.manual} open={h.update_available} />
              )}
              {log?.output && <pre className="update-log" data-testid={`update-output-${h.tool}`}>{log.output}</pre>}
              {known && (h.tool === "claude" || h.tool === "codex") && (
                <HarnessRestartPanel tool={h.tool} label={name} installed={h.installed_version ?? null} />
              )}
            </div>;
          })}
        </div>
      </section>
    </Sec>

    <Sec id="install-version">
      <section className="setting">
        <label>Install another version</label>
        {releases === null ? (
          <div className="ver-actions">
            <button className="ctrl sm" onClick={loadReleases}>Choose a version…</button>
          </div>
        ) : (
          <div className="ver-actions">
            <select value={pickTag} disabled={pinning} aria-label="Version to install" onChange={(e) => choose(e.currentTarget.value)}>
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
              className={"ctrl sm" + (pickArmed ? " armed" : "")}
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
    </Sec>
  </Page>;
}
