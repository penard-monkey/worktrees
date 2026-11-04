// Restarting the agent sessions that run an older Claude or Codex than the
// one installed (docs/proposals/harness-restart.md): what Settings → Updates →
// Agent CLIs shows under a tool's row, and the words of its two confirms.
//
// The host decides which sessions are older and what each is doing
// (`harness_sessions`); this file only counts what it answered and says it.
// Two actions, the owner's decision of 2026-11-03: restart the INACTIVE
// (idle) ones, or force-restart all of them, which interrupts busy turns and
// cancels open questions and so says how many, naming any (main).
//
// IMPORT-FREE on purpose: `scripts/harnessrestart-check.mjs` evaluates this
// file as a data: URL module, which has no base to resolve `./x` against.

/** One older session in a worktrees place, as `harness_sessions` lists it. */
export type HarnessSession = {
  project: string;
  project_name: string;
  slug: string;
  session: string;
  /** The build it runs; null for a Codex that is not the installed one. */
  version: string | null;
  state: "idle" | "busy" | "waiting";
  /** Why it cannot be restarted, when it cannot. */
  why?: string;
};

/** `harness_sessions {tool}`'s answer. */
export type HarnessSessions = {
  tool: string;
  installed_version: string | null;
  sessions: HarnessSession[];
  /** Older sessions outside worktrees (VS Code, a terminal): by hand only. */
  outside: { version: string | null }[];
};

/** One session's restart, as `restart_harness_sessions` answers it. */
export type HarnessRestart = {
  project: string;
  project_name: string;
  slug: string;
  session: string;
  outcome: "restarted" | "skipped" | "failed";
  version?: string;
  why?: string;
};

export type HarnessRestarted = {
  tool: string;
  force: boolean;
  installed_version: string | null;
  results: HarnessRestart[];
  restarted: number;
  skipped: number;
  failed: number;
  mains: string[];
};

/** What the buttons act on: the restartable sessions, split by activity. */
export type RestartPlan = {
  /** Every older session in a place, restartable or not. */
  older: number;
  /** Restartable now without force. */
  idle: number;
  /** Restartable only with force: mid-turn, and waiting on a prompt. */
  busy: number;
  waiting: number;
  /** Older, but no restart can resume its conversation. */
  unresumable: number;
  /** The projects whose (main) agent a force restart would interrupt. */
  activeMains: string[];
  /** The projects whose (main) agent is among the restarts at all. */
  mains: string[];
};

export function restartPlan(s: HarnessSessions | null | undefined): RestartPlan {
  const rows = s?.sessions ?? [];
  const ok = rows.filter((r) => !r.why);
  const active = ok.filter((r) => r.state !== "idle");
  const names = (xs: HarnessSession[]) => [...new Set(xs.filter((r) => r.slug === "(main)").map((r) => r.project_name))];
  return {
    older: rows.length,
    idle: ok.length - active.length,
    busy: active.filter((r) => r.state === "busy").length,
    waiting: active.filter((r) => r.state === "waiting").length,
    unresumable: rows.length - ok.length,
    activeMains: names(active),
    mains: names(ok),
  };
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const list = (xs: string[]) =>
  xs.length <= 1 ? xs.join("") : `${xs.slice(0, -1).join(", ")} and ${xs[xs.length - 1]}`;

/** The line under the tool's row; empty when nothing in a place is older. */
export function summaryLine(label: string, s: HarnessSessions | null | undefined): string {
  const rows = s?.sessions ?? [];
  if (!rows.length) return "";
  const installed = s?.installed_version ?? "a newer version";
  const known = [...new Set(rows.map((r) => r.version).filter((v): v is string => !!v))];
  const who = plural(rows.length, `${label} session`);
  if (!known.length) return `${who} started before ${label} ${installed} was installed.`;
  const verb = rows.length === 1 ? "runs" : "run";
  const one = known.length === 1 && rows.every((r) => r.version === known[0]);
  const ran = one ? `${verb} ${known[0]}` : `${verb} older versions (${known.sort(versionOrder).join(", ")})`;
  return `${who} ${ran}; ${installed} is installed.`;
}

function versionOrder(a: string, b: string): number {
  const pa = a.split(/[.-]/).map((x) => Number(x) || 0);
  const pb = b.split(/[.-]/).map((x) => Number(x) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) - (pb[i] ?? 0);
  }
  return 0;
}

/** The inactive button, and its confirm. */
export const inactiveLabel = (p: RestartPlan) => `Restart ${p.idle} inactive`;
export function inactiveConfirm(label: string, p: RestartPlan): string {
  return `This restarts ${plural(p.idle, `open ${label} session`)}. Each resumes its conversation.`;
}

/** The force button, and its stronger confirm. */
export const forceLabel = (p: RestartPlan) => `Force restart all ${p.idle + p.busy + p.waiting}`;
export function forceConfirm(label: string, p: RestartPlan): string {
  const all = p.idle + p.busy + p.waiting;
  const cut: string[] = [];
  if (p.busy) cut.push(`interrupts ${plural(p.busy, "session")} mid-turn`);
  if (p.waiting) cut.push(`cancels ${plural(p.waiting, "open question")}`);
  const head = `This restarts all ${plural(all, `${label} session`)}, and also ${list(cut)}.`;
  const mains = p.activeMains.length
    ? ` Among them: the (main) agent of ${list(p.activeMains)}.`
    : "";
  return `${head}${mains} Each resumes its conversation.`;
}

/** Whether each button is shown: inactive when one idle session can go;
 *  force only when it would do more than inactive does. */
export const showInactive = (p: RestartPlan) => p.idle > 0;
export const showForce = (p: RestartPlan) => p.busy + p.waiting > 0;

/** "restart by hand" for the sessions outside worktrees. */
export function outsideLine(label: string, s: HarnessSessions | null | undefined): string {
  const n = s?.outside?.length ?? 0;
  if (!n) return "";
  return `${plural(n, `more ${label} session`)} outside Worktrees (an editor or a terminal) ${n === 1 ? "runs" : "run"} an older version: restart ${n === 1 ? "it" : "them"} by hand.`;
}

/** One line per session of a restart's answer. */
export function resultLine(r: HarnessRestart): string {
  const where = `${r.project_name} · ${r.slug}`;
  switch (r.outcome) {
    case "restarted":
      return r.version ? `${where}: restarted, now ${r.version}` : `${where}: restarted${r.why ? ` (${r.why})` : ""}`;
    case "skipped":
      return `${where}: skipped${r.why ? ` — ${r.why}` : ""}`;
    default:
      return `${where}: failed${r.why ? ` — ${r.why}` : ""}`;
  }
}

/** The answer's one-line tally. */
export function tallyLine(r: HarnessRestarted): string {
  const parts = [`${r.restarted} restarted`];
  if (r.skipped) parts.push(`${r.skipped} skipped`);
  if (r.failed) parts.push(`${r.failed} failed`);
  return parts.join(" · ");
}
