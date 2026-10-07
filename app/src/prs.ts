// A project's pull requests — the app side of docs/proposals/pull-requests.md.
//
// Everything that DECIDES lives in `worktrees_core::github`: which repo, whose
// branch is whose PR, what a PR's chip says, what needs you. The backend's
// `project_prs` joins its cached snapshot to the places we pass and hands back
// rows that are ready to draw, so nothing here mirrors a core rule. What lives
// here is WHEN to ask, which is the app's business alone:
//
//   selected project only · window visible · every PR_POLL_MS · on focus if the
//   answer is older than 30s (or at once while `gh` is missing, so installing
//   it in a terminal is noticed) · opening the tab, 15s · its refresh button,
//   always.
//
// `enabled: false` (Settings → Behavior → Pull requests off) makes NO call.
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

export type PrState =
  | "ok"
  | "not_github"
  | "gh_missing"
  | "logged_out"
  | "no_host_token"
  | "not_found"
  | "error";

export type PrTone = "ok" | "warn" | "danger" | "accent" | "mute";

export type PrRow = {
  number: number;
  title: string;
  url: string;
  head: string;
  base: string;
  author: string | null;
  state: "open" | "merged" | "closed";
  draft: boolean;
  chip: string;
  tone: PrTone;
  label: string;
  ci: "SUCCESS" | "FAILURE" | "ERROR" | "PENDING" | "EXPECTED" | null;
  review: string | null;
  mergeable: string | null;
  updated_at: string;
  ended_at: string | null;
  place: string | null;
  attention: boolean;
};

export type PrView = {
  repo: { host: string; owner: string; repo: string };
  web: string;
  viewer: string | null;
  open_count: number;
  attention_count: number;
  open: PrRow[];
  recent: PrRow[];
  places: Record<string, number>;
};

export type PrsReply = {
  state: PrState;
  host: string | null;
  web: string | null;
  view: PrView | null;
  fetched_at: number | null;
  checked_at: number;
  stale: boolean;
  message: string | null;
  viewer: string | null;
};

export type PlaceBranch = { slug: string; branch: string };

/** Timer cadence while the window is visible (proposal §4). One point of
 *  GitHub's 5000/h per call, one call per project — never per place. */
export const PR_POLL_MS = 120_000;
const FOCUS_MAX_AGE_S = 30;
const SELECT_MAX_AGE_S = 120;
export const TAB_MAX_AGE_S = 15;

/** States where the answer can change without GitHub changing: a missing `gh`
 *  installed in a terminal, a login done there. Re-asked on every focus. */
const LOCAL_FIX: ReadonlySet<PrState> = new Set(["gh_missing", "logged_out", "no_host_token"]);

/** Whether the project shows PR UI at all: a GitHub project (or one we cannot
 *  tell yet because `gh` is missing on a github.com remote). Before the first
 *  reply: nothing, rather than a tab that appears and vanishes. */
export const prsShown = (r: PrsReply | null | undefined): r is PrsReply => !!r && r.state !== "not_github";

/** The row for a place's PR, open or recently closed. */
export function prOfPlace(r: PrsReply | null | undefined, slug: string): PrRow | null {
  const n = r?.view?.places[slug];
  if (n == null || !r?.view) return null;
  return r.view.open.find((p) => p.number === n) ?? r.view.recent.find((p) => p.number === n) ?? null;
}

/** "#451 · Open · CI passing · mergeable" — the chip's tooltip and the row's
 *  accessible name. Words only; colour is the dot's job. */
export function prSummary(p: PrRow): string {
  const parts = [`#${p.number}`];
  if (p.state !== "open") parts.push(p.state === "merged" ? "Merged" : "Closed");
  else {
    parts.push(p.draft ? "Draft" : "Open");
    const ci = p.ci === "SUCCESS" ? "CI passing"
      : p.ci === "FAILURE" || p.ci === "ERROR" ? "CI failing"
        : p.ci === "PENDING" || p.ci === "EXPECTED" ? "CI running" : null;
    if (ci) parts.push(ci);
    if (p.review === "CHANGES_REQUESTED") parts.push("changes requested");
    else if (p.review === "APPROVED") parts.push("approved");
    parts.push(p.mergeable === "CONFLICTING" ? "conflicts" : p.mergeable === "MERGEABLE" ? "mergeable" : "checking mergeability");
  }
  return parts.join(" · ");
}

/** Compact age of an ISO time: "now", "5m", "3h", "12d". */
export function ageOf(iso: string | null | undefined, now = Date.now()): string {
  const t = iso ? Date.parse(iso) : NaN;
  if (!Number.isFinite(t)) return "";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** Poll one project's PRs. Answers are kept per project, so switching back to
 *  a project shows its last list at once while a fresher one is asked for. */
export function useProjectPrs(
  enabled: boolean,
  repo: string | null,
  places: PlaceBranch[],
  pageVisible: boolean,
  onError: (e: unknown) => void,
) {
  const [replies, setReplies] = useState<Record<string, PrsReply>>({});
  // Branches change without the project changing (a switch in the header);
  // the key re-maps against the cached list — no fetch, the cache answers.
  const placesKey = useMemo(() => JSON.stringify(places.map((p) => [p.slug, p.branch])), [places]);
  const live = useRef({ repo, places, enabled });
  live.current = { repo, places, enabled };
  const repliesRef = useRef(replies);
  repliesRef.current = replies;

  const ask = useCallback((maxAgeSecs: number) => {
    const { repo: r, places: ps, enabled: on } = live.current;
    if (!on || !r || document.visibilityState === "hidden") return;
    invoke<PrsReply>("project_prs", { repo: r, maxAgeSecs, places: ps })
      .then((reply) => {
        // a reply that lands after the switch is turned off must not repopulate
        if (!live.current.enabled) return;
        setReplies((m) => ({ ...m, [r]: reply }));
      })
      .catch(onError);
  }, [onError]);

  // selecting a project, or its places' branches moving
  useEffect(() => {
    if (enabled && repo) ask(SELECT_MAX_AGE_S);
  }, [enabled, repo, placesKey, ask]);

  // the timer and focus — visible windows only
  useEffect(() => {
    if (!enabled || !repo || !pageVisible) return;
    // becoming visible is not always a focus event (un-minimise, a Space
    // switch back): without this the list sits up to a whole period stale
    ask(FOCUS_MAX_AGE_S);
    // just under the period, so a tick always finds the last answer old enough
    const timer = setInterval(() => ask(PR_POLL_MS / 1000 - 5), PR_POLL_MS);
    const onFocus = () => {
      const cur = live.current.repo ? repliesRef.current[live.current.repo] : undefined;
      ask(cur && LOCAL_FIX.has(cur.state) ? 0 : FOCUS_MAX_AGE_S);
    };
    window.addEventListener("focus", onFocus);
    return () => { clearInterval(timer); window.removeEventListener("focus", onFocus); };
  }, [enabled, repo, pageVisible, ask]);

  // off is off: drop what was shown
  useEffect(() => { if (!enabled) setReplies({}); }, [enabled]);

  return {
    reply: enabled && repo ? replies[repo] ?? null : null,
    /** opening the tab (`TAB_MAX_AGE_S`) or its refresh button (0) */
    refresh: ask,
  };
}
