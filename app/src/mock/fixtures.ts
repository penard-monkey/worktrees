// Rich fixture workspace for the browser design harness (VITE_MOCK=1).
// Mirrors the shapes list_workspace returns so the REAL App.tsx renders against
// it with no code changes. Covers every lifecycle group + pinned + main + a
// dead/broken project, so the design review sees all states at once.

export type Declared = {
  lifecycle?: string;
  pinned?: boolean;
  note?: string;
  last_opened_epoch?: number;
  up_cmd?: string | null;
} | null;

export type Place = {
  slug: string;
  path: string;
  is_main: boolean;
  registered: boolean;
  branch: string | null;
  detached: boolean | null;
  dirty: boolean | null;
  dirty_files?: number | null;
  ahead: number | null;
  behind: number | null;
  last_commit_subject?: string | null;
  last_commit_epoch?: number | null;
  tmux_session: { name: string; up: boolean };
  claude_session_present: boolean;
  declared: Declared;
  lifecycle_effective: string;
};
export type Snapshot = { repo: string; prefix: string; places: Place[] };
export type ProjectView = { root: string; ok: boolean; error: string | null; snapshot: Snapshot | null };
export type Workspace = { projects: ProjectView[] };

const NOW = 1784332800; // ~2026-07-21
const DAY = 86400;

type Opt = Partial<Place> & { slug: string; branch: string | null };
function place(prefix: string, root: string, o: Opt): Place {
  const isMain = o.is_main ?? false;
  const dir = isMain ? root : `${root}/.worktrees/${o.slug}`;
  return {
    slug: o.slug,
    path: o.path ?? dir,
    is_main: isMain,
    registered: o.registered ?? true,
    branch: o.branch,
    detached: o.detached ?? false,
    dirty: o.dirty ?? false,
    dirty_files: o.dirty_files ?? (o.dirty ? 3 : 0),
    ahead: o.ahead ?? 0,
    behind: o.behind ?? 0,
    last_commit_subject: o.last_commit_subject ?? "wip",
    last_commit_epoch: o.last_commit_epoch ?? NOW - DAY,
    tmux_session: o.tmux_session ?? { name: `${prefix}-${o.slug}`, up: false },
    claude_session_present: o.claude_session_present ?? false,
    declared: o.declared ?? null,
    lifecycle_effective: o.lifecycle_effective ?? "closed",
  };
}

function cdv(): ProjectView {
  const root = "/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo";
  const P = "cdv";
  const places: Place[] = [
    place(P, root, {
      slug: "(main)", branch: "main", is_main: true,
      tmux_session: { name: `${P}-main`, up: true }, ahead: 0, behind: 0,
      last_commit_subject: "chore: bump deps", lifecycle_effective: "active",
    }),
    place(P, root, {
      slug: "messaging", branch: "feat/messaging-sse",
      dirty: true, dirty_files: 4, ahead: 2, behind: 0,
      tmux_session: { name: `${P}-messaging`, up: true }, claude_session_present: true,
      last_commit_subject: "wire up SSE reconnect", last_commit_epoch: NOW - DAY,
      declared: { lifecycle: "saved", pinned: true, note: "auth refactor place", last_opened_epoch: NOW - DAY },
      lifecycle_effective: "saved",
    }),
    place(P, root, {
      slug: "billing-refactor", branch: "feat/billing-v2",
      ahead: 5, behind: 1, tmux_session: { name: `${P}-billing-refactor`, up: true },
      claude_session_present: true, last_commit_subject: "extract invoice service",
      declared: { last_opened_epoch: NOW - 3600 }, lifecycle_effective: "active",
    }),
    place(P, root, {
      slug: "kitchen-sink", branch: null, detached: true,
      dirty: true, dirty_files: 12, ahead: 3, behind: 4,
      tmux_session: { name: `${P}-kitchen-sink`, up: true }, claude_session_present: true,
      last_commit_subject: "detached experiment", declared: { last_opened_epoch: NOW - 1200 },
      lifecycle_effective: "active",
    }),
    place(P, root, {
      slug: "search-index", branch: "feat/search-opensearch",
      ahead: 0, behind: 3, last_commit_subject: "index mapping draft",
      declared: { last_opened_epoch: NOW - 2 * DAY, note: "waiting on infra ticket" },
      lifecycle_effective: "idle",
    }),
    place(P, root, {
      slug: "hotfix-login", branch: "fix/login-loop", dirty: true, dirty_files: 1,
      last_commit_subject: "guard null session", declared: { lifecycle: "closed", last_opened_epoch: NOW - 20 * DAY },
      lifecycle_effective: "closed",
    }),
    place(P, root, {
      slug: "legacy-migration", branch: "chore/knex-to-prisma",
      last_commit_subject: "migrate users table", declared: { lifecycle: "archived", note: "resume Q3", last_opened_epoch: NOW - 40 * DAY },
      lifecycle_effective: "archived",
    }),
    place(P, root, {
      slug: "spike-graphql", branch: "spike/graphql",
      last_commit_subject: "throwaway resolver", declared: { lifecycle: "abandoned", last_opened_epoch: NOW - 60 * DAY },
      lifecycle_effective: "abandoned",
    }),
  ];
  return { root, ok: true, error: null, snapshot: { repo: root, prefix: P, places } };
}

function worktreesRepo(): ProjectView {
  const root = "/Users/demo/workspace/worktrees";
  const P = "worktrees";
  const places: Place[] = [
    place(P, root, {
      slug: "(main)", branch: "main", is_main: true, ahead: 0, behind: 0,
      tmux_session: { name: `${P}-main`, up: false }, last_commit_subject: "docs: readme",
      lifecycle_effective: "closed",
    }),
    place(P, root, {
      slug: "feat-redesign", branch: "feat/ui-redesign", dirty: true, dirty_files: 9, ahead: 7,
      tmux_session: { name: `${P}-feat-redesign`, up: true }, claude_session_present: true,
      last_commit_subject: "design tokens + nav", declared: { pinned: true, last_opened_epoch: NOW - 600 },
      lifecycle_effective: "active",
    }),
    place(P, root, {
      slug: "fix-flaky-ci", branch: "fix/flaky-ci",
      last_commit_subject: "retry tmux smoke", declared: { lifecycle: "closed", last_opened_epoch: NOW - 9 * DAY },
      lifecycle_effective: "closed",
    }),
  ];
  return { root, ok: true, error: null, snapshot: { repo: root, prefix: P, places } };
}

// A dead/broken project node — one bad repo should grey out, not blank the app.
function brokenRepo(): ProjectView {
  return {
    root: "/Users/demo/workspace/deleted-thing",
    ok: false,
    error: "not a git repository (or any parent up to mount point)",
    snapshot: null,
  };
}

export function initialWorkspace(): Workspace {
  return { projects: [cdv(), worktreesRepo(), brokenRepo()] };
}
