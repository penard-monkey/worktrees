// Rich fixture workspace for the browser design harness (VITE_MOCK=1).
// Mirrors the shapes list_workspace returns so the REAL App.tsx renders against
// it with no code changes. Covers every lifecycle group + pinned + main + a
// dead/broken project, so the design review sees all states at once.

export type Declared = {
  lifecycle?: string;
  pinned?: boolean;
  title?: string;
  note?: string;
  last_opened_epoch?: number;
  last_worked_epoch?: number;
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
  profile_name?: string | null;
  profile_stale?: boolean;
  declared: Declared;
  lifecycle_effective: string;
};
export type Snapshot = { repo: string; prefix: string; places: Place[]; unborn?: boolean };
export type ProjectView = { root: string; ok: boolean; error: string | null; snapshot: Snapshot | null };
export type Workspace = { projects: ProjectView[] };

const NOW = 1784332800; // ~2026-07-21
const DAY = 86400;
/** Afterglow ages must be relative to the WALL CLOCK, not the frozen `NOW`:
 *  `doneTier` measures against `Date.now()`, so a fixed epoch would age out of
 *  every tier and the harness would boot showing no embers at all. */
const REAL_NOW = Math.floor(Date.now() / 1000);
const MIN = 60;

/** The canonical tmux session name for a place — `Project::session_name`
 *  (project.rs), including the `.` → `-` replacement tmux needs. Anything that
 *  builds the name by hand drifts the moment a fixture slug has a dot in it. */
export const sessionName = (prefix: string, slug: string) => `${prefix}-${slug}`.replace(/\./g, "-");

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
    tmux_session: o.tmux_session ?? { name: sessionName(prefix, o.slug), up: false },
    claude_session_present: o.claude_session_present ?? false,
    declared: o.declared ?? null,
    lifecycle_effective: o.lifecycle_effective ?? "closed",
    // Must be listed explicitly: this builder constructs the object field by
    // field rather than spreading `o`, so anything omitted here is silently
    // dropped even though `Opt` accepts it and `Place` declares it optional —
    // type-valid, and a lie. (The profile badge was invisible in the harness
    // for exactly this reason.)
    profile_name: o.profile_name ?? null,
    profile_stale: o.profile_stale ?? false,
  };
}

function cdv(): ProjectView {
  const root = "/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo";
  const P = "cdv";
  const places: Place[] = [
    place(P, root, {
      slug: "(main)", branch: "main", is_main: true,
      tmux_session: { name: `${P}-(main)`, up: true }, ahead: 0, behind: 0,
      last_commit_subject: "chore: bump deps", lifecycle_effective: "active",
      // The topbar profile badge, in its ordinary state.
      profile_name: "Work", profile_stale: false,
    }),
    place(P, root, {
      slug: "messaging", branch: "feat/messaging-sse",
      dirty: true, dirty_files: 4, ahead: 2, behind: 0,
      tmux_session: { name: `${P}-messaging`, up: true }, claude_session_present: true,
      // …and in its stale state, so "restart to apply" is reachable by clicking
      // rather than only existing in the backend.
      profile_name: "Work", profile_stale: true,
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
      // ADOPTED session: the name is not `<prefix>-<slug>`, so this tool did not
      // write it — a session left under a previous prefix (proposal §5), or one
      // started by hand. Closing it needs the user's word; the fixture exists so
      // that two-click arm is drivable headlessly.
      tmux_session: { name: "dev-kitchen-sink", up: true }, claude_session_present: true,
      last_commit_subject: "detached experiment", declared: { last_opened_epoch: NOW - 1200 },
      lifecycle_effective: "active",
    }),
    place(P, root, {
      slug: "search-index", branch: "feat/search-opensearch",
      ahead: 0, behind: 3, last_commit_subject: "index mapping draft",
      // afterglow t1 — freshest tier, full ember + halo
      declared: { last_opened_epoch: NOW - 2 * DAY, last_worked_epoch: REAL_NOW - 4 * MIN, note: "waiting on infra ticket" },
      lifecycle_effective: "idle",
    }),
    place(P, root, {
      slug: "hotfix-login", branch: "fix/login-loop", dirty: true, dirty_files: 1,
      // afterglow t2 — worked this block, session since closed
      last_commit_subject: "guard null session",
      declared: { lifecycle: "closed", last_opened_epoch: NOW - 20 * DAY, last_worked_epoch: REAL_NOW - 45 * MIN },
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
      tmux_session: { name: `${P}-(main)`, up: false }, last_commit_subject: "docs: readme",
      // main glows too — a session run in the repo root stamps under the `(main)`
      // store key, same as any other place (lib.rs place_key_for)
      declared: { last_worked_epoch: REAL_NOW - 30 * MIN },
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
      // afterglow t3 — this morning's work, nearly out
      last_commit_subject: "retry tmux smoke",
      declared: { lifecycle: "closed", last_opened_epoch: NOW - 9 * DAY, last_worked_epoch: REAL_NOW - 5 * 3600 },
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
