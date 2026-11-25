// Settings → Agent policy → Codex folder trust: the repos whose Codex lanes
// start trusted. Codex asks "Trust this folder?" in every repo it has not been
// told about, and saves the answer in ~/.codex/config.toml. Worktrees never
// writes that file: an allowed repo instead gets a per-run `-c` on each launch
// (core `trust::codex_launch_trust`), so nothing is saved, a plain `codex` run
// outside worktrees is unaffected, and a revoke applies at the next launch.
//
// The allowance is `[trust] codex` in ~/.config/worktrees/config.toml — the
// user's act, from here or `worktrees trust codex`. Off for every repo by
// default, and the write is console-only: no agent and no paired device sets it.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "./ipc";
import { useArmed } from "./useArmed";

export type CodexLaunchTrust = {
  config: string | null;
  source: "allowance" | "unspellable" | "own";
  allowed: boolean;
  repo_root: string | null;
  reason: string | null;
};

export type CodexTrustStatus = {
  projects: { root: string; name: string; allowed: boolean }[];
  /** Allowed roots no registered project has any more (the console's alone). */
  stale: string[];
  repo_root: string | null;
  repo_allowed: boolean;
  launch: CodexLaunchTrust | null;
  /** Codex's own `projects` already trusts the repo in focus. */
  own_trusted: boolean | null;
};

/** What decided the trust of a Codex lane in the repo in focus, in words. */
export function codexTrustText(t: CodexLaunchTrust, ownTrusted: boolean | null): string {
  switch (t.source) {
    case "allowance": return "Allowed below: Codex lanes here start trusted, for that launch only.";
    case "unspellable": return `Allowed, but not trusted: ${t.reason ?? "this path cannot be passed to Codex safely"}.`;
    default: return ownTrusted
      ? "Codex's own config already trusts this repo; nothing is passed."
      : "Codex asks \"Trust this folder?\" in this repo, and a launch is refused until you allow it below or answer it once in Codex.";
  }
}

/** `codex_status` for `repo`, and the allowance write. */
export function useCodexTrustStatus(repo: string | null, onReport: (msg: string) => void) {
  const [status, setStatus] = useState<CodexTrustStatus | null>(null);
  const reload = useCallback(() => {
    invoke<CodexTrustStatus>("codex_status", { repo }).then(setStatus).catch((e) => onReport(`codex trust: ${String(e)}`));
  }, [repo, onReport]);
  useEffect(() => { reload(); }, [reload]);
  const allow = async (root: string, on: boolean) => {
    try {
      await invoke("set_codex_allowed", { repo: root, allow: on });
      reload();
    } catch (e) { onReport(`codex allowance: ${String(e)}`); }
  };
  return { status, reload, allow };
}

/** One toggle per registered project, all off until the user turns one on. A
 *  stale entry (its project is gone) keeps a two-click Revoke so it can never
 *  become permanent. */
export function CodexTrustSection({ status, allow }: {
  status: CodexTrustStatus | null;
  allow: (root: string, on: boolean) => void;
}) {
  const arm = useArmed<string>();
  return (
    <section className="setting" data-testid="codex-trust">
      <label>Codex folder trust</label>
      <div className="hint">Codex asks to trust every repo it has not seen, and a lane cannot answer for you. Turning a repo on makes its Codex lanes start trusted: Codex then reads the repo's <code>AGENTS.md</code> and <code>.codex/</code> config and runs without its folder prompt — so code the repo ships runs inside Codex with no question. Off for every repo until you turn it on.</div>
      {status?.launch && <div className="hint" data-testid="codex-trust-source"><b>This repo:</b> {codexTrustText(status.launch, status.own_trusted)}</div>}
      {!status ? <div className="hint">reading…</div> : status.projects.length === 0
        ? <div className="hint">No registered projects yet.</div>
        : <div className="cross-project-rows">
          {status.projects.map((p) => (
            <label key={p.root} className="tier-toggle setting-check" data-project={p.name}>
              <input type="checkbox" data-testid="codex-allow" checked={p.allowed}
                onChange={(e) => allow(p.root, e.currentTarget.checked)} />
              <span><b>{p.name}</b> — start Codex lanes trusted</span>
            </label>
          ))}
        </div>}
      {status && status.stale.length > 0 && <>
        <label className="sub">Allowed, no longer registered</label>
        <div className="ver-rows">{status.stale.map((r) => (
          <div className="ver-row" key={r}><code>{r}</code>
            <button className={"ctrl sm danger" + (arm.armed === r ? " armed" : "")} data-testid="codex-revoke"
              onClick={() => arm.confirm(r, () => allow(r, false))}>{arm.armed === r ? "Revoke — sure?" : "Revoke"}</button></div>
        ))}</div>
      </>}
      <div className="hint">Kept in ~/.config/worktrees/config.toml as [trust] codex. It trusts each launch only and is never written into Codex's own <code>~/.codex/config.toml</code>, so it does not reach a Codex you run by hand; turning a repo off applies at its next launch. Also: <code>worktrees trust codex [--revoke]</code>, and <code>worktrees doctor --codex</code> says what a launch would do. Only you can change this: no agent and no paired device can.</div>
    </section>
  );
}
