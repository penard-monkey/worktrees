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
  /** The repository has an `origin` remote (without one, applying merges locally). */
  has_origin?: boolean;
  /** The place holding the pending branch — its review lane. */
  pending_place?: string | null;
  /** GitHub says the pending branch's PR is merged (asked only for a pushed
   *  branch that still has a place). */
  pending_merged?: boolean;
};
export type UserSkill = { name: string; status: "linked" | "missing" | "conflict" };
/** `rules::State` — one managed piece of the worktrees rules (ADR 0002). */
export type RulesState = "absent" | "current" | "outdated" | "edited" | "newer" | "malformed" | "foreign";
export type RulesFound = { state: RulesState; version: number | null; detail?: string };
/** `rules::Status` — from the same default-branch ref as `AgentReport`. */
export type RulesStatus = {
  reference: string;
  /** The pack version this worktrees writes. */
  version: number;
  file: string | null;
  block: RulesFound;
  skill: RulesFound;
  config: boolean;
  /** Why the block cannot be written at all (a diverged or foreign-linked root). */
  blocked: string | null;
  /** Something apply would change without force. */
  applicable: boolean;
  pending: string | null;
  pending_on_origin: boolean;
};
/** `rules` is null when the backend could not read them (logged there). */
export type AgentSetupStatus = { repo: AgentReport; user_skills: UserSkill[]; rules?: RulesStatus | null };
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


// How long the Fix / Apply buttons stay armed. Short on purpose: an arm that
// outlives the reader's attention is a one-click commit (or push).
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
  /** Which of the two is running, so only that button says so. */
  busy: "fix" | "rules" | null;
  /** Which button is armed. One arm for both: they write the same branch. */
  armed: "fix" | "rules" | null;
  /** Push the branch and open a PR too — one switch PER BUTTON, each beside
   *  its own button, so ticking one never shows up ticked somewhere else.
   *  Off by default: what leaves the machine is the user's call (2026-10-14). */
  push: { fix: boolean; rules: boolean };
  setPush: (which: "fix" | "rules", v: boolean) => void;
  outcome: AgentFixOutcome | null;
  /** Which button produced `outcome`/`log`, so only its section shows them. */
  ran: "fix" | "rules" | null;
  log: string;
  load: () => Promise<void>;
  /** First call arms, second call (within ARM_MS) commits. */
  fix: () => Promise<void>;
  /** The same for the worktrees rules; `force` replaces hand-edited text. */
  applyRules: (force: boolean) => Promise<void>;
};

/** `onLoaded` gets EVERY successful read, not only the one after a Fix: the
 *  sheet's read is fresher than App's 5-minute sweep, and the header badge,
 *  the menu and the sheet's own To do list must agree the moment it lands —
 *  the doctor side already hands its report up the same way (`onReport`). */
/** `onCommitted` runs after Fix / Apply committed LOCALLY: the branch's review
 *  lane opens there (App's `reviewLane`), so it is never a branch nothing
 *  points at. A pushed commit has a PR instead and gets no lane. */
export function useAgentSetup(
  root: string,
  open: boolean,
  onLoaded: (root: string, s: AgentSetupStatus) => void,
  onError: (msg: string) => void,
  onCommitted?: (root: string) => void,
): AgentSetupCtl {
  const [status, setStatus] = useState<AgentSetupStatus | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [busy, setBusy] = useState<"fix" | "rules" | null>(null);
  const running = busy !== null;
  const [armed, setArmed] = useState<"fix" | "rules" | null>(null);
  const [push, setPushState] = useState({ fix: false, rules: false });
  const setPush = (which: "fix" | "rules", v: boolean) => setPushState((p) => ({ ...p, [which]: v }));
  const [outcome, setOutcome] = useState<AgentFixOutcome | null>(null);
  const [ran, setRan] = useState<"fix" | "rules" | null>(null);
  const [log, setLog] = useState("");
  // Through a ref: App passes a fresh closure every render, and `load` keyed on
  // it would re-run the open effect (and re-probe) on every render it causes.
  const cb = useRef(onLoaded);
  cb.current = onLoaded;
  const committed = useRef(onCommitted);
  committed.current = onCommitted;

  const load = useCallback(async () => {
    setArmed(null);
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
    const t = setTimeout(() => setArmed(null), ARM_MS);
    return () => clearTimeout(t);
  }, [armed]);

  const commit = async (which: "fix" | "rules", cmd: string, args: Record<string, unknown>) => {
    if (running) return;
    if (armed !== which) { setArmed(which); return; }
    setArmed(null);
    setBusy(which);
    setRan(which);
    setOutcome(null);
    setLog("");
    let lane = false;
    try {
      const o = await invoke<AgentFixOutcome | null>(cmd, { repo: root, push: push[which], ...args });
      setOutcome(o ?? null);
      lane = !!o?.commit && !o.pushed;
    } catch (e) {
      setLog(`✗ ${String(e)}`);
      onError(`${cmd} ${root}: ${String(e)}`);
    } finally {
      // Re-read BEFORE the button comes back (ProjectSheet's `run` hazard): a
      // stale `fixable` in the gap would offer the same commit twice. The
      // re-read hands itself up through `onLoaded`, like every other read.
      await load();
      setBusy(null);
    }
    if (lane) committed.current?.(root);
  };
  const fix = () => commit("fix", "agent_setup_fix", {});
  const applyRules = (force: boolean) => commit("rules", "rules_apply", { force });

  return { status, loadErr, running, busy, armed, push, setPush, outcome, ran, log, load, fix, applyRules };
}

/** The Fix button, shared by the To do row and the section so the two can
 *  never drift in what they say the second click does. */
/** What the armed second click does, in the words both buttons use. */
const commitWords = (push: boolean) => (push ? "Commit, push + open PR" : "Commit on agent-instructions");
const runningWords = (push: boolean) => (push ? "Pushing…" : "Committing…");

export function AgentFixButton({ ctl, disabled = false, onPress }: { ctl: AgentSetupCtl; disabled?: boolean; onPress?: () => void }) {
  const repo = ctl.status?.repo;
  const armed = ctl.armed === "fix";
  const push = ctl.push.fix;
  const where = `on 'agent-instructions' off ${repo?.reference ?? "the default branch"}`;
  return (
    <button
      className={"ctrl sm" + (armed ? " danger armed" : "")}
      data-testid="agent-setup-fix"
      disabled={disabled || ctl.running || !repo?.fixable || !!repo?.pending}
      title={armed
        ? (push ? "click again to commit, push the branch and open a PR" : "click again to commit the fix on a local branch")
        : push ? `commit the fix ${where}, push it and open a PR` : `commit the fix ${where} — nothing is pushed`}
      onClick={() => { onPress?.(); ctl.fix(); }}
    >
      {ctl.busy === "fix" ? runningWords(push) : armed ? commitWords(push) : "Fix…"}
    </button>
  );
}

/** The push switch for ONE button (its own state; see `AgentSetupCtl.push`). */
export function PushToggle({ ctl, which }: { ctl: AgentSetupCtl; which: "fix" | "rules" }) {
  return (
    <label className="tier-toggle setting-check" title="Without it, the branch stays on this machine for you to review and push.">
      <input type="checkbox" data-testid={`push-${which}`} checked={ctl.push[which]} disabled={ctl.running}
        onChange={(e) => ctl.setPush(which, e.currentTarget.checked)} />
      {which === "fix" ? "Fix also pushes the branch and opens a PR" : "Apply also pushes the branch and opens a PR"}
    </label>
  );
}

/** The pending branch, in words: a PR to merge, or a local branch to review
 *  in its lane. `place` is the place holding it, when there is one. */
export function PendingHint({ pending, onOrigin, place = null, hasOrigin = true }: { pending: string; onOrigin: boolean; place?: string | null; hasOrigin?: boolean }) {
  const lane = place
    ? <>Its review place is <code>{place}</code>: the agent there explains the change and applies or abandons it on your word.</>
    : <>Review opens a place for it with an agent that explains the change and applies or abandons it on your word.</>;
  return onOrigin ? (
    <div className="hint">
      Branch <code>{pending}</code> is waiting: merge its PR to finish. If that PR was closed,{" "}
      <code>worktrees rules abandon --yes</code> deletes the branch so you can commit again. {lane}
    </div>
  ) : (
    <div className="hint">
      Branch <code>{pending}</code> is committed on this machine and not pushed.{" "}
      {hasOrigin
        ? <>Applying it pushes it and opens a PR (<code>worktrees rules land</code>).</>
        : <>There is no origin remote, so applying it merges it into the default branch here (<code>worktrees rules land</code>).</>}{" "}
      {lane}
    </div>
  );
}

export function openPr(url: string, onError: (msg: string) => void) {
  openUrl(url).catch((e) => onError(`open ${url}: ${String(e)}`));
}

/** The ProjectSheet's "Agent setup" section. Module scope with props
 *  (CLAUDE.md): defined inside a parent it would remount every render. */
export function AgentSetupSection({ ctl, onError }: { ctl: AgentSetupCtl; onError: (msg: string) => void }) {
  const { status, loadErr } = ctl;
  // Only what the Fix produced: the Rules section shows its own.
  const outcome = ctl.ran === "rules" ? null : ctl.outcome;
  const log = ctl.ran === "rules" ? "" : ctl.log;
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
      <PushToggle ctl={ctl} which="fix" />
      {repo?.pending && !outcome && !(ctl.ran === "rules" && ctl.outcome) && <PendingHint pending={repo.pending} onOrigin={repo.pending_on_origin} place={repo.pending_place} hasOrigin={repo.has_origin} />}
      <div className="hint">
        AGENTS.md becomes the one instruction file — Codex and other agents read it directly — and
        CLAUDE.md a one-line <code>@AGENTS.md</code> import, so Claude reads the same words. Judged
        against <b>{repo?.reference ?? "the default branch"}</b>, not this checkout. Fix commits that on a
        local branch, <code>agent-instructions</code>; nothing lands until you merge it.
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

function rulesLine(what: string, f: RulesFound, version: number): { sev: "info" | "warn" | "error"; glyph: string; text: string } {
  switch (f.state) {
    case "absent": return { sev: "warn", glyph: "▸", text: `${what}: not applied yet` };
    case "current": return { sev: "info", glyph: "✓", text: `${what}: current (v${f.version ?? version})` };
    case "outdated": return { sev: "warn", glyph: "▸", text: `${what}: from an older worktrees (v${f.version ?? "?"} → v${version}) — Apply updates it in place` };
    case "edited": return { sev: "warn", glyph: "✎", text: `${what}: edited by hand — Apply leaves it alone unless you replace it` };
    case "newer": return { sev: "info", glyph: "·", text: `${what}: written by a newer worktrees (v${f.version ?? "?"}) — left alone` };
    case "malformed": return { sev: "error", glyph: "✗", text: `${what}: damaged — ${f.detail ?? "its markers do not pair up"}; fix it by hand` };
    case "foreign": return { sev: "info", glyph: "·", text: `${what}: the repository's own — left alone` };
  }
}

/** The ProjectSheet's "Worktrees rules" section: the way of working, written
 *  into the repository for agents worktrees did not start (ADR 0002). Shares
 *  the Agent setup ctl — same branch, same arm, same push switch — and, like
 *  that section, shows only what the backend judged. */
export function RulesSection({ ctl, onError }: { ctl: AgentSetupCtl; onError: (msg: string) => void }) {
  const [force, setForce] = useState(false);
  const rules = ctl.status?.rules;
  const armed = ctl.armed === "rules";
  const push = ctl.push.rules;
  const editedPieces = rules
    ? [rules.block.state === "edited" ? `the ${rules.file ?? "AGENTS.md"} block` : null, rules.skill.state === "edited" ? "the close-out skill" : null].filter((x): x is string => !!x)
    : [];
  const edited = editedPieces.length > 0;
  // A hidden checkbox must not keep sending `true`: a Re-check can clear
  // `edited` (the edit was reverted upstream) while `force` is still set.
  useEffect(() => { if (!edited) setForce(false); }, [edited]);
  const replacing = force && edited;
  // What THIS section shows as to do — not `applicable`, which also covers
  // the layout fix that Agent setup's own button makes. With every line
  // green, Apply has nothing of its own to write.
  const writable = (f: RulesFound) => f.state === "absent" || f.state === "outdated";
  const rulesTodo = !!rules && ((!rules.blocked && writable(rules.block)) || writable(rules.skill) || (!rules.config && rules.skill.state !== "foreign"));
  const can = !!rules && !rules.pending && (rulesTodo || replacing);
  // Order matters: the tag must never contradict the rows under it.
  const tag = !rules ? null
    : rules.pending ? <span className="upd-tag">{rules.pending_on_origin ? "PR waiting" : "committed locally"}</span>
    : rulesTodo ? <span className="upd-tag warn">{[rules.block, rules.skill].some((f) => f.state === "outdated") ? "outdated" : "not applied"}</span>
    : edited ? <span className="upd-tag">edited</span>
    : rules.block.state === "current" ? <span className="upd-tag">v{rules.version}</span>
    : null;
  const armedWords = replacing
    ? (push ? "Replace edited text, commit + push" : "Replace edited text + commit")
    : commitWords(push);
  const lines = rules ? [
    rules.blocked
      ? { sev: "error" as const, glyph: "✗", text: `${rules.file ?? "AGENTS.md"} block: ${rules.blocked}` }
      : rulesLine(`${rules.file ?? "AGENTS.md"} block`, rules.block, rules.version),
    rulesLine("close-out skill", rules.skill, rules.version),
    rules.config
      ? { sev: "info" as const, glyph: "✓", text: ".claude/close-out.md: present — the repository's own" }
      : { sev: "warn" as const, glyph: "▸", text: ".claude/close-out.md: Apply adds a starting point (yours to edit)" },
  ] : [];
  return (
    <section className="setting" data-testid="project-rules">
      <label>Worktrees rules{tag}</label>
      {ctl.loadErr ? (
        <div className="hint">Could not check this project — the error is under Agent setup above.</div>
      ) : !ctl.status ? (
        <div className="hint">Checking…</div>
      ) : !rules ? (
        <div className="hint">Could not read the rules on the default branch — see Settings → Logs.</div>
      ) : (
        <div className="dx-list">
          {lines.map((l) => (
            <div className="dx-row" key={l.text}>
              <span className={"dx-sev " + l.sev}>{l.glyph}</span>
              <span className="dx-msg">{l.text}</span>
            </div>
          ))}
        </div>
      )}
      <div className="ver-actions">
        <button
          className={"ctrl sm" + (armed ? " danger armed" : "")}
          data-testid="rules-apply"
          disabled={ctl.running || !can}
          title={armed
            ? `click again to ${replacing ? `REPLACE your edits to ${editedPieces.join(" and ")}, then ` : ""}commit${push ? ", push the branch and open a PR" : " on a local branch"}`
            : !rulesTodo && !replacing && rules && !rules.pending
              ? (edited ? "your edits are kept — tick Replace to overwrite them with this version" : "the rules are current — nothing to apply")
            : `commit the rules on 'agent-instructions' off ${rules?.reference ?? "the default branch"}${push ? ", push it and open a PR" : " — nothing is pushed"}`}
          onClick={() => ctl.applyRules(replacing)}
        >
          {ctl.busy === "rules" ? runningWords(push) : armed ? armedWords : "Apply rules…"}
        </button>
      </div>
      {edited && (
        <label className="tier-toggle setting-check">
          <input type="checkbox" data-testid="rules-force" checked={force} disabled={ctl.running}
            onChange={(e) => setForce(e.currentTarget.checked)} />
          Replace the hand-edited text with this version's
        </label>
      )}
      <PushToggle ctl={ctl} which="rules" />
      {rules?.pending && !(ctl.ran === "rules" && ctl.outcome) && <PendingHint pending={rules.pending} onOrigin={rules.pending_on_origin} place={ctl.status?.repo.pending_place} hasOrigin={ctl.status?.repo.has_origin} />}
      <div className="hint">
        Teaches every agent in this repository — including one worktrees did not start, and a
        teammate's — how it is worked on here: places, spinning up a lane with a brief, which worktrees
        tool to use when, planning, finishing and close-out. Apply adds a short managed section at the
        end of AGENTS.md (nothing outside its markers is touched), the <code>close-out</code> skill, and a
        <code> .claude/close-out.md</code> for this repository's own settings. It also makes the Agent
        setup fix above, in the same commit.
      </div>
      {ctl.ran === "rules" && ctl.outcome && ctl.outcome.notes.length > 0 && (
        <pre className="update-log">{ctl.outcome.notes.join("\n")}</pre>
      )}
      {ctl.ran === "rules" && ctl.outcome?.pr_url && (
        <div className="ver-actions">
          <button className="ctrl sm" onClick={() => openPr(ctl.outcome!.pr_url!, onError)}>
            Open PR <Icons.ExternalLink size={12} />
          </button>
        </div>
      )}
      {ctl.ran === "rules" && ctl.log && <pre className="update-log">{ctl.log}</pre>}
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
