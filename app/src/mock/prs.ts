// `project_prs` for the browser harness (install.ts). Not a mirror of the
// core's decisions — the chip state, tone and label of every row are WRITTEN
// HERE as fixtures, one per state, so each can be looked at; only the branch →
// place join is redone, the simplest possible way, so renaming a place's branch
// in the harness moves its chip the way the real backend's re-map does.
//
//   ?prs=ok (default) | gh_missing | logged_out | no_host_token | not_found
//        | error | offline | empty | slow
//
// `offline` = a stale list ("as of …"); `empty` = a GitHub project with no
// PRs; `slow` answers after 1.5s, as the real ~1.2s query does. Projects with
// no origin (deleted-thing, local*) answer `not_github` in every mode — no tab,
// no chip, no badge.
import type { PrRow, PrsReply, PlaceBranch } from "../prs";

const H = 3600_000;
const D = 24 * H;
const iso = (agoMs: number) => new Date(Date.now() - agoMs).toISOString();

type Seed = Omit<PrRow, "place" | "attention" | "url" | "base" | "ended_at"> & { ended_ago?: number };

const row = (o: Partial<Seed> & Pick<Seed, "number" | "title" | "head" | "chip" | "tone" | "label">): Seed => ({
  author: "demo", state: "open", draft: false, ci: "SUCCESS", review: null, mergeable: "MERGEABLE",
  updated_at: iso(3 * H), ...o,
});

// One open PR per chip state, each on a place of the cdv fixture project, plus
// a "no place" tail (another author's, a fork's, a stale conflict).
const OPEN: Seed[] = [
  row({ number: 212, title: "Messaging over SSE instead of polling", head: "feat/messaging-sse", chip: "ready", tone: "ok", label: "ready", review: "APPROVED", updated_at: iso(40 * 60_000) }),
  row({ number: 219, title: "Search: move to OpenSearch", head: "feat/search-opensearch", chip: "pending", tone: "warn", label: "checking", ci: "PENDING", updated_at: iso(2 * H) }),
  row({ number: 215, title: "Catalog import from the supplier CSV", head: "feat/catalog-import", chip: "failing", tone: "danger", label: "CI failing", ci: "FAILURE", updated_at: iso(5 * H) }),
  row({ number: 208, title: "Billing v2: invoices as first-class records", head: "feat/billing-v2", chip: "conflicts", tone: "danger", label: "conflicts", mergeable: "CONFLICTING", updated_at: iso(2 * D) }),
  row({ number: 201, title: "Perf budget in CI", head: "chore/perf-budget", chip: "changes_requested", tone: "danger", label: "changes requested", review: "CHANGES_REQUESTED", updated_at: iso(4 * D) }),
  row({ number: 190, title: "WIP: knex → prisma", head: "chore/knex-to-prisma", chip: "draft", tone: "mute", label: "draft", draft: true, ci: null, updated_at: iso(9 * D) }),
  row({ number: 220, title: "Bump vite from 7.3.5 to 7.3.6", head: "dependabot/npm/vite-7.3.6", chip: "ready", tone: "ok", label: "ready", author: "dependabot", updated_at: iso(6 * H) }),
  row({ number: 177, title: "Add Portuguese translations", head: "i18n-pt", chip: "conflicts", tone: "danger", label: "conflicts", mergeable: "CONFLICTING", ci: null, author: "outside-contrib", updated_at: iso(31 * D) }),
  row({ number: 174, title: "Docs: fix the setup steps", head: "patch-1", chip: "pending", tone: "warn", label: "checking", mergeable: "UNKNOWN", ci: null, author: "outside-contrib", updated_at: iso(33 * D) }),
];

const RECENT: Seed[] = [
  row({ number: 199, title: "Fix the login redirect loop", head: "fix/login-loop", chip: "merged", tone: "accent", label: "merged", state: "merged", ci: null, mergeable: null, updated_at: iso(2 * H), ended_ago: 2 * H }),
  row({ number: 198, title: "Release 4.2.0", head: "release/4.2.0", chip: "merged", tone: "accent", label: "merged", state: "merged", ci: null, mergeable: null, updated_at: iso(20 * H), ended_ago: 20 * H }),
  row({ number: 185, title: "Spike: GraphQL gateway", head: "spike/graphql", chip: "closed", tone: "mute", label: "closed", state: "closed", ci: null, mergeable: null, updated_at: iso(3 * D), ended_ago: 3 * D }),
];

export function mockProjectPrs(repo: string, places: PlaceBranch[]): PrsReply {
  const name = repo.split("/").pop() ?? "";
  const mode = new URLSearchParams(location.search).get("prs") ?? "ok";
  const checked_at = Date.now();
  const base = { host: "github.com", web: `https://github.com/demo/${name}`, view: null, fetched_at: null, checked_at, stale: false, message: null, viewer: "demo" };
  if (name === "deleted-thing" || name.startsWith("local")) return { ...base, state: "not_github", host: null, web: null, viewer: null };
  switch (mode) {
    case "gh_missing": return { ...base, state: "gh_missing", viewer: null };
    case "logged_out": return { ...base, state: "logged_out", viewer: null };
    case "no_host_token": return { ...base, state: "no_host_token", viewer: null };
    case "not_found": return { ...base, state: "not_found", viewer: "demo-alt" };
    case "error": return { ...base, state: "error", message: "error connecting to api.github.com", viewer: null };
  }
  // Only the cdv project carries the full set; the others are quiet repos.
  const busy = name === "casa-del-valle-monorepo";
  const open = busy ? OPEN : [];
  const recent = busy ? RECENT : [];
  const bySlug: Record<string, number> = {};
  const byPr = new Map<number, string>();
  for (const p of places) {
    const hit = open.find((r) => r.head === p.branch) ?? recent.find((r) => r.head === p.branch);
    if (hit) { bySlug[p.slug] = hit.number; if (!byPr.has(hit.number)) byPr.set(hit.number, p.slug); }
  }
  const finish = (r: Seed): PrRow => {
    const place = byPr.get(r.number) ?? null;
    const { ended_ago, ...rest } = r;
    return {
      ...rest,
      url: `https://github.com/demo/${name}/pull/${r.number}`,
      base: "main",
      ended_at: ended_ago != null ? iso(ended_ago) : null,
      place,
      attention: !!place && (r.tone === "danger" || r.state === "merged"),
    };
  };
  const rows = { open: (mode === "empty" ? [] : open).map(finish), recent: (mode === "empty" ? [] : recent).map(finish) };
  const fetched = mode === "offline" ? checked_at - 47 * 60_000 : checked_at;
  return {
    ...base,
    state: "ok",
    fetched_at: fetched,
    stale: mode === "offline",
    message: mode === "offline" ? "error connecting to api.github.com" : null,
    view: {
      repo: { host: "github.com", owner: "demo", repo: name },
      web: base.web,
      viewer: "demo",
      open_count: rows.open.length,
      attention_count: [...rows.open, ...rows.recent].filter((r) => r.attention).length,
      ...rows,
      places: mode === "empty" ? {} : bySlug,
    },
  };
}
