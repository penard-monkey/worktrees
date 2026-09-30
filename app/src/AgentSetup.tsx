import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as Icons from "./icons";
import { projectTodos, todoCount } from "./projectTodos";

// Agent instructions for every agent: AGENTS.md is the real file, CLAUDE.md a
// one-line `@AGENTS.md` stub, and repo skills linked into `.agents/skills`
// where Codex looks. The app half of `worktrees agent-setup`; the rules live in
// `worktrees_core::agentfiles`, and nothing here re-derives them — every
// verdict shown is a field the backend computed.

/** `agentfiles::DirKind`, kebab-cased by serde. */
export type AgentDirKind = "claude-only" | "agents-only" | "stub" | "linked" | "diverged";
export type AgentDirState = { dir: string; kind: AgentDirKind };
export type AgentSkillState = { name: string; kind: "missing" | "present" };
/** `agentfiles::Report` — judged against the DEFAULT-BRANCH ref, not a checkout. */
export type AgentReport = {
  reference: string;
  dirs: AgentDirState[];
  skills: AgentSkillState[];
  fixable: boolean;
  conflicts: boolean;
  /** The fix branch exists: a previous fix awaits its merge. The base ref does
   *  not move until then, so `fixable` stays true — this is what zeroes the
   *  project's to-do count instead (`projectTodos.ts`). */
  pending: string | null;
  /** The pending branch is on origin — so a PR can exist. False with `pending`
   *  set means it was committed locally and never pushed (no origin, a failed
   *  push): there is nothing to wait for. */
  pending_on_origin: boolean;
};
export type UserSkill = { name: string; status: "linked" | "missing" | "conflict" };
export type AgentSetupStatus = { repo: AgentReport; user_skills: UserSkill[] };
export type AgentFixOutcome = {
  branch: string;
  base: string;
  commit: string | null;
  pushed: boolean;
  pr_url: string | null;
  left_alone: string[];
  notes: string[];
};

const missingUser = (skills: UserSkill[]) => skills.filter((u) => u.status === "missing");

const dirLabel = (d: string) => (d === "" ? "(root)" : d);

function dirLine(k: AgentDirKind): { sev: "info" | "warn" | "error"; glyph: string; text: string } {
  switch (k) {
    case "claude-only": return { sev: "warn", glyph: "▸", text: "CLAUDE.md only — Fix moves it to AGENTS.md and leaves a stub" };
    case "agents-only": return { sev: "warn", glyph: "▸", text: "AGENTS.md only — Claude reads nothing here; Fix adds the stub" };
    case "stub": return { sev: "info", glyph: "✓", text: "ok — CLAUDE.md imports AGENTS.md" };
    case "linked": return { sev: "info", glyph: "✓", text: "ok — one file links to the other" };
    case "diverged": return { sev: "error", glyph: "✗", text: "CLAUDE.md and AGENTS.md differ — merge by hand" };
  }
}


// How long the Fix button stays armed. Short on purpose: an arm that outlives
// the reader's attention is a one-click push.
const ARM_MS = 4000;

/** Everything the Fix needs, owned ONCE per open ProjectSheet. Two surfaces
 *  press it — the To do row at the top of the sheet and the Agent setup
 *  section — and they must share one status, one arm and one in-flight flag:
 *  two copies would let the section re-offer a push the To do row just made,
 *  in the window before its re-read lands. */
export type AgentSetupCtl = {
  status: AgentSetupStatus | null;
  loadErr: string | null;
  running: boolean;
  armed: boolean;
  outcome: AgentFixOutcome | null;
  log: string;
  load: () => Promise<void>;
  /** First call arms, second call (within ARM_MS) pushes. */
  fix: () => Promise<void>;
};

/** `onLoaded` gets EVERY successful read, not only the one after a Fix: the
 *  sheet's read is fresher than App's 5-minute sweep, and the header badge,
 *  the menu and the sheet's own To do list must agree the moment it lands —
 *  the doctor side already hands its report up the same way (`onReport`). */
export function useAgentSetup(root: string, open: boolean, onLoaded: (root: string, s: AgentSetupStatus) => void, onError: (msg: string) => void): AgentSetupCtl {
  const [status, setStatus] = useState<AgentSetupStatus | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [armed, setArmed] = useState(false);
  const [outcome, setOutcome] = useState<AgentFixOutcome | null>(null);
  const [log, setLog] = useState("");
  // Through a ref: App passes a fresh closure every render, and `load` keyed on
  // it would re-run the open effect (and re-probe) on every render it causes.
  const cb = useRef(onLoaded);
  cb.current = onLoaded;

  const load = useCallback(async () => {
    setArmed(false);
    try {
      const s = await invoke<AgentSetupStatus | null>("agent_setup_status", { repo: root });
      setStatus(s ?? null);
      setLoadErr(null);
      if (s) cb.current(root, s);
    } catch (e) {
      // Shown in the section AND logged — a section that silently rendered
      // nothing would read as "nothing to set up".
      setLoadErr(String(e));
      onError(`agent setup ${root}: ${String(e)}`);
    }
  }, [root, onError]);

  useEffect(() => {
    if (!open) return;
    setOutcome(null);
    setLog("");
    load();
  }, [open, load]);

  // The arm expires: a Fix armed and forgotten must not fire on a stray click.
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(false), ARM_MS);
    return () => clearTimeout(t);
  }, [armed]);

  const fix = async () => {
    if (running) return;
    if (!armed) { setArmed(true); return; }
    setArmed(false);
    setRunning(true);
    setOutcome(null);
    setLog("");
    try {
      const o = await invoke<AgentFixOutcome | null>("agent_setup_fix", { repo: root });
      setOutcome(o ?? null);
    } catch (e) {
      setLog(`✗ ${String(e)}`);
      onError(`agent_setup_fix ${root}: ${String(e)}`);
    } finally {
      // Re-read BEFORE the button comes back (ProjectSheet's `run` hazard): a
      // stale `fixable` in the gap would offer the same push twice. The re-read
      // hands itself up through `onLoaded`, like every other read.
      await load();
      setRunning(false);
    }
  };

  return { status, loadErr, running, armed, outcome, log, load, fix };
}

/** The Fix button, shared by the To do row and the section so the two can
 *  never drift in what they say the second click does. */
export function AgentFixButton({ ctl, disabled = false, onPress }: { ctl: AgentSetupCtl; disabled?: boolean; onPress?: () => void }) {
  const repo = ctl.status?.repo;
  return (
    <button
      className={"ctrl sm" + (ctl.armed ? " danger armed" : "")}
      data-testid="agent-setup-fix"
      disabled={disabled || ctl.running || !repo?.fixable || !!repo?.pending}
      title={ctl.armed ? "click again to push the branch and open a PR" : `commit the fix on 'agent-instructions' off ${repo?.reference ?? "the default branch"}, push it and open a PR`}
      onClick={() => { onPress?.(); ctl.fix(); }}
    >
      {ctl.running ? "Pushing…" : ctl.armed ? "Push branch + open PR" : "Fix…"}
    </button>
  );
}

export function openPr(url: string, onError: (msg: string) => void) {
  openUrl(url).catch((e) => onError(`open ${url}: ${String(e)}`));
}

/** The ProjectSheet's "Agent setup" section. Module scope with props
 *  (CLAUDE.md): defined inside a parent it would remount every render. */
export function AgentSetupSection({ ctl, onError }: { ctl: AgentSetupCtl; onError: (msg: string) => void }) {
  const { status, loadErr, outcome, log } = ctl;
  const repo = status?.repo;
  const missingSkills = repo?.skills.filter((k) => k.kind === "missing") ?? [];
  // The same number the To do list and the header show — derived, never
  // recounted here, so the three cannot drift.
  const todo = todoCount(projectTodos(null, status));

  return (
    <section className="setting" data-testid="agent-setup">
      <label>
        Agent setup
        {repo?.pending
          ? <span className="upd-tag">{repo.pending_on_origin ? "PR waiting" : "not pushed"}</span>
          : todo > 0 ? <span className="upd-tag warn">{todo} to fix</span> : null}
        {repo?.conflicts ? <span className="upd-tag warn">merge by hand</span> : null}
      </label>
      {loadErr ? (
        <pre className="update-log">{loadErr}</pre>
      ) : !repo ? (
        <div className="hint">Checking…</div>
      ) : repo.dirs.length === 0 && repo.skills.length === 0 ? (
        <div className="hint">No CLAUDE.md, AGENTS.md or repo skills on {repo.reference}.</div>
      ) : (
        <div className="dx-list">
          {repo.dirs.map((d) => {
            const l = dirLine(d.kind);
            return (
              <div className="dx-row" key={"d|" + d.dir}>
                <span className={"dx-sev " + l.sev}>{l.glyph}</span>
                <span className="dx-msg"><b className="dx-place">{dirLabel(d.dir)}</b> {l.text}</span>
              </div>
            );
          })}
          {missingSkills.map((k) => (
            <div className="dx-row" key={"s|" + k.name}>
              <span className="dx-sev warn">▸</span>
              <span className="dx-msg">
                skill <b className="dx-place">{k.name}</b> — Codex can't see it; Fix links it into .agents/skills
              </span>
            </div>
          ))}
        </div>
      )}
      <div className="ver-actions">
        <AgentFixButton ctl={ctl} />
        <button className="ctrl sm" disabled={ctl.running} onClick={ctl.load}>Re-check</button>
      </div>
      {repo?.pending && !outcome && (repo.pending_on_origin ? (
        <div className="hint">
          A fix is waiting on branch <code>{repo.pending}</code>: merge its PR to finish. If that PR was
          closed, delete the branch (locally and on origin) to fix again.
        </div>
      ) : (
        <div className="hint">
          Branch <code>{repo.pending}</code> was committed but never pushed, so there is no PR to merge.
          Push it and open one yourself, or delete it (<code>git branch -D {repo.pending}</code>) to fix again.
        </div>
      ))}
      <div className="hint">
        AGENTS.md becomes the one instruction file — Codex and other agents read it directly — and
        CLAUDE.md a one-line <code>@AGENTS.md</code> import, so Claude reads the same words. Judged
        against <b>{repo?.reference ?? "the default branch"}</b>, not this checkout. Fix commits that on a
        branch, pushes it and opens a PR; nothing lands until you merge it.
      </div>
      {outcome && (
        <>
          {outcome.notes.length > 0 && <pre className="update-log">{outcome.notes.join("\n")}</pre>}
          {outcome.pr_url && (
            <div className="ver-actions">
              <button className="ctrl sm" onClick={() => openPr(outcome.pr_url!, onError)}>
                Open PR <Icons.ExternalLink size={12} />
              </button>
            </div>
          )}
        </>
      )}
      {/* The user's OWN skills used to be linked from here. They are
          machine-wide — the same answer under every project — so they live in
          Settings → Codex, which is also where the after-update offer about
          them points (offers.ts, `codex-skills`). */}
      <div className="hint">Your own skills in ~/.claude/skills are linked for Codex in Settings → Codex.</div>
      {log && <pre className="update-log">{log}</pre>}
    </section>
  );
}

/** Settings → Codex → "Your skills": link ~/.claude/skills into ~/.agents/skills.
 *  Machine-level, so it needs no project — it reads `agent_user_skills`, which
 *  is why the `codex-skills` offer can land here on a fresh install with no
 *  project at all (a suggestion's surface may not add preconditions). */
export function UserSkillsSection({ skills, onChanged, offerPending, onSilenceOffer, onReport, "data-focus": focusId }: {
  /** The deep-link target, named at the CALL site (SettingsSheet's Codex
   *  category), where offers-check can see it beside the category it opens. */
  "data-focus": string;
  /** App's probe (`agent_user_skills`); null = not read yet or failed. */
  skills: UserSkill[] | null;
  /** The re-read after a link — App keeps the offer in step with it. */
  onChanged: (skills: UserSkill[]) => void;
  offerPending: boolean;
  onSilenceOffer: () => void;
  onReport: (msg: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [linked, setLinked] = useState<string[] | null>(null);
  // Callbacks through a ref: App passes fresh closures every render, and an
  // effect keyed on them would re-probe on every render it causes — a loop.
  const cb = useRef({ onChanged, onReport });
  cb.current = { onChanged, onReport };
  const reread = useCallback(async () => {
    try {
      cb.current.onChanged((await invoke<UserSkill[] | null>("agent_user_skills")) ?? []);
    } catch (e) { cb.current.onReport(`agent_user_skills: ${String(e)}`); }
  }, []);
  // Re-read on entry: a skill added from a terminal since launch should show.
  useEffect(() => { reread(); }, [reread]);
  const link = async () => {
    if (busy) return;
    setBusy(true);
    setLinked(null);
    try {
      setLinked((await invoke<string[] | null>("agent_link_skills")) ?? []);
    } catch (e) {
      onReport(`agent_link_skills: ${String(e)}`);
    } finally {
      await reread();
      setBusy(false);
    }
  };
  const missing = skills ? missingUser(skills) : [];
  const conflict = skills?.filter((u) => u.status === "conflict") ?? [];
  const linkedNow = skills?.filter((u) => u.status === "linked") ?? [];
  return (
    <section className="setting" data-focus={focusId}>
      <label>
        Your skills
        {missing.length > 0 && <span className="upd-tag">{missing.length} not linked</span>}
      </label>
      {!skills ? (
        <div className="hint">Checking…</div>
      ) : skills.length === 0 ? (
        <div className="hint">No skills in ~/.claude/skills.</div>
      ) : (
        <div className="hint">
          {missing.length > 0
            ? `${missing.length} of ${skills.length} skills in ~/.claude/skills ${missing.length === 1 ? "is" : "are"} not visible to Codex: ${missing.map((u) => u.name).join(", ")}.`
            : `Codex can see ${linkedNow.length} of the ${skills.length} skills in ~/.claude/skills; nothing is left to link.`}
          {" "}Linking adds symlinks in ~/.agents/skills and changes nothing else.
        </div>
      )}
      {conflict.length > 0 && (
        <div className="hint">
          Left alone — a different skill of the same name is already in ~/.agents/skills:{" "}
          {conflict.map((u) => u.name).join(", ")}
        </div>
      )}
      <div className="ver-actions">
        {missing.length > 0 && (
          <button className="ctrl sm" data-testid="agent-link-skills" disabled={busy} onClick={link}>
            {busy ? "Linking…" : `Link ${missing.length} skill${missing.length === 1 ? "" : "s"} for Codex`}
          </button>
        )}
        {/* Ends the SUGGESTION, not the feature — same contract as the Claude
            panel's: taking the offer closed the band that sent you here, so
            the section carries the same off switch where you land. */}
        {offerPending && <button className="mcp-dismiss" onClick={onSilenceOffer}>Stop suggesting this</button>}
      </div>
      {linked && <div className="hint">{linked.length > 0 ? `Linked ${linked.join(", ")}.` : "Nothing needed linking."}</div>}
    </section>
  );
}
