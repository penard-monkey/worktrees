import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** `cross_project_status` (lib.rs): the user's level and every registered
 *  project. Machine-level — nothing here is about the project in focus. */
export type CrossProjectStatus = {
  level: "off" | "read" | "full";
  config_path: string;
  projects: { root: string; name: string; private: boolean }[];
};

const LEVELS: { id: CrossProjectStatus["level"]; label: string; hint: string }[] = [
  { id: "off", label: "Off", hint: "A session sees only its own project's places." },
  { id: "read", label: "Read", hint: "A session can list your other projects' places and check on their agents (list_projects, place_status, wait). It cannot change anything there." },
  { id: "full", label: "Full", hint: "As Read, and paths are included. Acting on other projects arrives in a later release; removing a worktree in another project is never allowed." },
];

/** Settings → Agent guidance → Other projects (cross-project proposal P1b).
 *  The user's `cross_project` setting and each project's `private` flag. Both
 *  are the user's alone: a repo cannot set either, and no agent can. */
export function CrossProjectSection({ status, onStatus, onReport, offerPending = false, onSilenceOffer, "data-focus": focusId }: {
  status: CrossProjectStatus | null;
  /** Every read or write goes back to App: the offer and the nav drag follow it. */
  onStatus: (s: CrossProjectStatus) => void;
  onReport: (text: string) => void;
  offerPending?: boolean;
  onSilenceOffer?: () => void;
  "data-focus"?: string;
}) {
  const [busy, setBusy] = useState(false);
  // Re-read on open: `worktrees projects` may have changed the list since app start.
  useEffect(() => {
    invoke<CrossProjectStatus>("cross_project_status").then(onStatus).catch((e) => onReport(`cross_project_status: ${String(e)}`));
  }, []);
  const run = async (cmd: string, args: Record<string, unknown>) => {
    setBusy(true);
    try { onStatus(await invoke<CrossProjectStatus>(cmd, args)); }
    catch (e) { onReport(`${cmd}: ${String(e)}`); }
    finally { setBusy(false); }
  };
  const level = status?.level ?? "off";
  return <section className="setting" data-focus={focusId} data-testid="cross-project">
    <label>Other projects</label>
    <div className="hint">Let an agent in one project see the places in your other projects, addressed as <code>project:place</code>. Off by default.</div>
    {!status ? <div className="hint">Checking…</div> : <>
      <div className="seg" role="radiogroup" aria-label="Cross-project reach">
        {LEVELS.map((l) => (
          <button key={l.id} role="radio" aria-checked={level === l.id} className={level === l.id ? "on" : ""} disabled={busy}
            data-level={l.id} onClick={() => level !== l.id && run("set_cross_project", { level: l.id })}>{l.label}</button>
        ))}
      </div>
      <div className="hint">{LEVELS.find((l) => l.id === level)?.hint}</div>
      <div className="hint" data-testid="cross-project-restart">Restart sessions to apply: a running agent keeps the reach it started with.</div>
      {status.projects.length === 0
        ? <div className="hint">No registered projects yet.</div>
        : <div className="cross-project-rows">
          <div className="hint">A private project is out of reach both ways: other projects see only its name, and its own agents reach no other project.</div>
          {status.projects.map((p) => (
            <label key={p.root} className="tier-toggle setting-check" data-project={p.name}>
              <input type="checkbox" checked={p.private} disabled={busy}
                onChange={(e) => run("set_project_private", { root: p.root, private: e.currentTarget.checked })} />
              <span><b>{p.name}</b> is private</span>
            </label>
          ))}
          <div className="hint">Names come from <code>worktrees projects</code>, which can also rename them.</div>
        </div>}
      <div className="hint">Settings file: {status.config_path}</div>
      {offerPending && onSilenceOffer && <div className="ver-actions"><button className="mcp-dismiss" onClick={onSilenceOffer}>Stop suggesting this</button></div>}
    </>}
  </section>;
}
