import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as Icons from "./icons";

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
   *  not move until then, so `fixable` stays true — this is what retires the
   *  offer instead. */
  pending: string | null;
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

const missingUser = (s: AgentSetupStatus) => s.user_skills.filter((u) => u.status === "missing");

/** Is there anything to OFFER in the nav? Only the REPO fix. Diverged dirs
 *  alone are not an offer — only a person can merge them, and a banner whose one
 *  button cannot help is a nag. Nor are the user's own skills: they are
 *  machine-wide, so an offer about them would stand under EVERY project at
 *  once. The sheet lists both. */
export function agentSetupOffers(s: AgentSetupStatus | null | undefined): boolean {
  return !!s && s.repo.fixable && !s.repo.pending;
}

/** The dismissal key: a hash of what the offer is ABOUT — the dirs and the
 *  repo skills — so a tree that changes (a new CLAUDE.md in a subdir, a new
 *  skill) re-offers. Same rule as `init_dismissed`.
 *  32-bit FNV-1a: not a security boundary, only "is this the offer I declined?". */
export function agentSetupKey(s: AgentSetupStatus): string {
  const lines = [
    ...s.repo.dirs.map((d) => `dir\t${d.kind}\t${d.dir}`),
    ...s.repo.skills.map((k) => `skill\t${k.kind}\t${k.name}`),
  ].sort();
  let h = 0x811c9dc5;
  for (const ch of lines.join("\n")) {
    h ^= ch.codePointAt(0)!;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

/** The banner's one line: the most important thing first. */
export function agentSetupLine(s: AgentSetupStatus): string {
  const kinds = new Set(s.repo.dirs.map((d) => d.kind));
  if (kinds.has("agents-only")) return "Agent instructions are only in AGENTS.md, so Claude sees nothing.";
  if (kinds.has("claude-only")) return "Agent instructions are only in CLAUDE.md. Codex reads them, but AGENTS.md-first tools don't.";
  const repoSkills = s.repo.skills.filter((k) => k.kind === "missing").length;
  return `${repoSkills} repo skill${repoSkills === 1 ? " is" : "s are"} in .claude/skills only, where Codex doesn't look.`;
}

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

/** The ProjectSheet's "Agent setup" section. Module scope with props
 *  (CLAUDE.md): defined inside a parent it would remount every render. */
export function AgentSetupSection({ root, open, focus, onChanged, onError }: {
  root: string;
  open: boolean;
  /** Scroll this section into view once it has content (the banner's Fix…). */
  focus: boolean;
  /** The status was re-read — App re-probes so the nav banner tracks it. */
  onChanged: (root: string) => void;
  onError: (msg: string) => void;
}) {
  const [status, setStatus] = useState<AgentSetupStatus | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [running, setRunning] = useState<"" | "fix" | "link">("");
  const [armed, setArmed] = useState(false);
  const [outcome, setOutcome] = useState<AgentFixOutcome | null>(null);
  const [linked, setLinked] = useState<string[] | null>(null);
  const [log, setLog] = useState("");
  const ref = useRef<HTMLElement | null>(null);
  const scrolled = useRef(false);

  const load = useCallback(async () => {
    setArmed(false);
    try {
      const s = await invoke<AgentSetupStatus | null>("agent_setup_status", { repo: root });
      setStatus(s ?? null);
      setLoadErr(null);
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
    setLinked(null);
    setLog("");
    scrolled.current = false;
    load();
  }, [open, load]);

  // The arm expires: a Fix armed and forgotten must not fire on a stray click.
  useEffect(() => {
    if (!armed) return;
    const t = setTimeout(() => setArmed(false), ARM_MS);
    return () => clearTimeout(t);
  }, [armed]);

  useEffect(() => {
    if (!focus || scrolled.current || !status || !ref.current) return;
    scrolled.current = true;
    ref.current.scrollIntoView({ block: "start" });
  }, [focus, status]);

  const fix = async () => {
    if (running) return;
    if (!armed) { setArmed(true); return; }
    setArmed(false);
    setRunning("fix");
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
      // stale `fixable` in the gap would offer the same push twice.
      await load();
      onChanged(root);
      setRunning("");
    }
  };

  const link = async () => {
    if (running) return;
    setRunning("link");
    setLinked(null);
    try {
      const names = await invoke<string[] | null>("agent_link_skills");
      setLinked(names ?? []);
    } catch (e) {
      setLog(`✗ ${String(e)}`);
      onError(`agent_link_skills: ${String(e)}`);
    } finally {
      await load();
      onChanged(root);
      setRunning("");
    }
  };

  const busy = running !== "";
  const repo = status?.repo;
  const missingSkills = repo?.skills.filter((k) => k.kind === "missing") ?? [];
  const userMissing = status ? missingUser(status) : [];
  const userConflict = status?.user_skills.filter((u) => u.status === "conflict") ?? [];
  const userLinked = status?.user_skills.filter((u) => u.status === "linked") ?? [];
  const todo = (repo?.dirs.filter((d) => d.kind === "claude-only" || d.kind === "agents-only").length ?? 0) + missingSkills.length;

  return (
    <section className="setting" ref={ref} data-testid="agent-setup">
      <label>
        Agent setup
        {repo?.pending ? <span className="upd-tag">PR waiting</span> : repo?.fixable ? <span className="upd-tag warn">{todo} to fix</span> : null}
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
        <button
          className={"ctrl sm" + (armed ? " danger armed" : "")}
          data-testid="agent-setup-fix"
          disabled={busy || !repo?.fixable || !!repo?.pending}
          title={armed ? "click again to push the branch and open a PR" : `commit the fix on 'agent-instructions' off ${repo?.reference ?? "the default branch"}, push it and open a PR`}
          onClick={fix}
        >
          {running === "fix" ? "Pushing…" : armed ? "Push branch + open PR" : "Fix…"}
        </button>
        <button className="ctrl sm" disabled={busy} onClick={load}>Re-check</button>
      </div>
      {repo?.pending && !outcome && (
        <div className="hint">
          A fix is waiting on branch <code>{repo.pending}</code>: merge its PR to finish. If that PR was
          closed, delete the branch (locally and on origin) to fix again.
        </div>
      )}
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
              <button className="ctrl sm" onClick={() => openUrl(outcome.pr_url!).catch((e) => onError(`open ${outcome.pr_url}: ${String(e)}`))}>
                Open PR <Icons.ExternalLink size={12} />
              </button>
            </div>
          )}
        </>
      )}

      <label className="sub">Your skills</label>
      {!status ? null : status.user_skills.length === 0 ? (
        <div className="hint">No skills in ~/.claude/skills.</div>
      ) : (
        <>
          <div className="hint">
            {userMissing.length > 0
              ? `${userMissing.length} of ${status.user_skills.length} skills in ~/.claude/skills ${userMissing.length === 1 ? "is" : "are"} not visible to Codex.`
              : `Codex can see ${userLinked.length} of the ${status.user_skills.length} skills in ~/.claude/skills; nothing is left to link.`}
            {" "}Linking adds symlinks in ~/.agents/skills and changes nothing else.
          </div>
          {userConflict.length > 0 && (
            <div className="hint">
              Left alone — a different skill of the same name is already in ~/.agents/skills:{" "}
              {userConflict.map((u) => u.name).join(", ")}
            </div>
          )}
          {userMissing.length > 0 && (
            <div className="ver-actions">
              <button className="ctrl sm" data-testid="agent-link-skills" disabled={busy} onClick={link}>
                {running === "link" ? "Linking…" : `Link ${userMissing.length} skill${userMissing.length === 1 ? "" : "s"} for Codex`}
              </button>
            </div>
          )}
        </>
      )}
      {linked && <div className="hint">{linked.length > 0 ? `Linked ${linked.join(", ")}.` : "Nothing needed linking."}</div>}
      {log && <pre className="update-log">{log}</pre>}
    </section>
  );
}

/** The nav offer, under a project whose default branch could use the fix (or
 *  whose user skills Codex cannot see). Box borrowed from `InitBanner`; dismissal
 *  is keyed by `agentSetupKey`, persisted in `agent_setup_dismissed`. */
export function AgentSetupBanner({ status, onOpen, onDismiss }: {
  status: AgentSetupStatus;
  onOpen: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="init-banner" data-testid="agent-setup-banner">
      <div className="init-banner-h">
        <span className="init-banner-i">⚑</span>
        Agent setup
      </div>
      <p>{agentSetupLine(status)}</p>
      <div className="ver-actions">
        <button className="ctrl sm" onClick={onOpen}>Fix…</button>
        <button className="ctrl sm" title="hide until this project's agent files change" onClick={onDismiss}>Not now</button>
      </div>
    </div>
  );
}
