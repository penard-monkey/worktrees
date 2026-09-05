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
  /** The cached "Ask Claude" read (`ai_status_report`). Round-trips through the
   *  Rust store's `extra`, so on the wire it is a plain declared key. */
  status_report?: { text: string; epoch: number; verdict?: string } | null;
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
  upstream?: string | null;
  created_epoch?: number | null;
  last_commit_subject?: string | null;
  last_commit_epoch?: number | null;
  tmux_session: { name: string; up: boolean };
  claude_session_present: boolean;
  profile_name?: string | null;
  profile_stale?: boolean;
  declared: Declared;
  lifecycle_effective: string;
};
export type Stray = { path: string; branch: string | null; slug: string };
export type Snapshot = { repo: string; prefix: string; places: Place[]; unborn?: boolean; strays?: Stray[] };
export type ProjectView = { root: string; ok: boolean; error: string | null; snapshot: Snapshot | null };
export type Workspace = { projects: ProjectView[] };

/** The fixture workspace's "now" — the WALL CLOCK, never a frozen date. Every
 *  age the app renders is measured against `Date.now()`: the afterglow tiers
 *  (`doneTier`), the row age (`ago`), and the `.row.stale` dim. A pinned epoch
 *  drifts past all three, so the harness boots with no embers, a "40d" on rows
 *  their author wrote as "yesterday", and — since the header calm-down — an
 *  entirely dimmed workspace. It is also what makes the recorded media
 *  deterministic: offsets below are exactly what a reader sees, every run. */
const NOW = Math.floor(Date.now() / 1000);
const DAY = 86400;
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
    upstream: o.upstream ?? null,
    last_commit_subject: o.last_commit_subject ?? "wip",
    last_commit_epoch: o.last_commit_epoch ?? NOW - DAY,
    // Defaults to the commit date rather than to `NOW`: `created_epoch` is a rung
    // of the status check's activity max (health.rs), so a fixture that was born
    // "now" would read `active` no matter how old everything else about it is —
    // and the stale verdicts would be unreachable in the harness.
    created_epoch: o.created_epoch ?? o.last_commit_epoch ?? NOW - DAY,
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
      // `behind` on MAIN is the one ↓ the app still draws — main's base ref is
      // origin/main, so this is the classic "you have commits to pull". It is
      // also what puts main in the Attention lens.
      // The old commit date is deliberate too: main sits untouched while the
      // work happens in worktrees, so this is the row that proves `(main)` is
      // exempt from the stale dim rather than merely young enough to escape it.
      slug: "(main)", branch: "main", is_main: true,
      tmux_session: { name: `${P}-(main)`, up: true }, ahead: 0, behind: 2,
      last_commit_epoch: NOW - 45 * DAY,
      last_commit_subject: "chore: bump deps", lifecycle_effective: "active",
      // The topbar profile badge, in its ordinary state.
      profile_name: "Work", profile_stale: false,
    }),
    place(P, root, {
      slug: "messaging", branch: "feat/messaging-sse",
      dirty: true, dirty_files: 4, ahead: 2, behind: 0,
      // An upstream that has SOME of the commits: the status check's third
      // at-risk reason has three faces (no upstream / K unpushed / all pushed
      // but unmerged), and without a tracked fixture the middle one is
      // unreachable from the harness.
      upstream: "origin/feat/messaging-sse",
      tmux_session: { name: `${P}-messaging`, up: true }, claude_session_present: true,
      // …and in its stale state, so "restart to apply" is reachable by clicking
      // rather than only existing in the backend.
      profile_name: "Work", profile_stale: true,
      // The aging pin: nothing has happened here in a month, so the row dims —
      // but it keeps its place at the top of Pinned, because that is what the
      // user asked for. Also the sticky-lifecycle case: "saved" is a declared
      // state, so the header chip still says it even though the session is up.
      last_commit_subject: "wire up SSE reconnect", last_commit_epoch: NOW - 31 * DAY,
      declared: { lifecycle: "saved", pinned: true, note: "auth refactor place", last_opened_epoch: NOW - 31 * DAY },
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
      // Behind-only and clean: the base moved, this worktree did not. Nothing
      // to see — no ↓ glyph, no ↓ in the header, and it stays out of Attention.
      slug: "search-index", branch: "feat/search-opensearch",
      ahead: 0, behind: 3, last_commit_subject: "index mapping draft",
      // afterglow t1 — freshest tier, full ember + halo
      declared: { last_opened_epoch: NOW - 2 * DAY, last_worked_epoch: NOW - 4 * MIN, note: "waiting on infra ticket" },
      lifecycle_effective: "idle",
    }),
    place(P, root, {
      slug: "hotfix-login", branch: "fix/login-loop", dirty: true, dirty_files: 1,
      // afterglow t2 — worked this block, session since closed
      last_commit_subject: "guard null session",
      declared: { lifecycle: "closed", last_opened_epoch: NOW - 20 * DAY, last_worked_epoch: NOW - 45 * MIN },
      lifecycle_effective: "closed",
    }),
    place(P, root, {
      slug: "legacy-migration", branch: "chore/knex-to-prisma",
      last_commit_epoch: NOW - 40 * DAY,
      last_commit_subject: "migrate users table",
      declared: {
        lifecycle: "archived", note: "resume Q3", last_opened_epoch: NOW - 40 * DAY,
        // The ONE fixture that already has a cached "Ask Claude" read, so the
        // remembered state of that section is reachable without waiting out the
        // deliberately-slow mock spawn — and so a design pass sees the shape a
        // 250-word answer actually makes in the sheet.
        status_report: {
          text:
            "This worktree was cut to move the data layer off Knex and onto Prisma. " +
            "The plan in task_plan.md lists six tables; progress.md has three of them " +
            "checked off, and the last commit (\"migrate users table\") is the third.\n\n" +
            "It ended mid-migration. The schema and the users/sessions/accounts migrations " +
            "are committed and pushed, but the query call sites for the remaining three " +
            "tables were never touched, so the branch does not build against the new client. " +
            "Nothing is uncommitted — the work stopped at a clean point rather than being " +
            "abandoned mid-edit.\n\n" +
            "Recommend: resume. The unfinished half is mechanical and the plan that describes " +
            "it is still in the tree, which is a much cheaper restart than rediscovering the " +
            "schema decisions from the diff.",
          epoch: NOW - 6 * 3600,
          verdict: "cold",
        },
      },
      lifecycle_effective: "archived",
    }),
    place(P, root, {
      slug: "spike-graphql", branch: "spike/graphql",
      last_commit_epoch: NOW - 60 * DAY,
      last_commit_subject: "throwaway resolver", declared: { lifecycle: "abandoned", last_opened_epoch: NOW - 60 * DAY },
      lifecycle_effective: "abandoned",
    }),
  ];
  // The shape found in the real repo on 2026-09-02: a worktree a previous tool
  // (dmux) registered under its own dir. Drives the nav's ⊟ flag + the sheet.
  const strays: Stray[] = [{
    path: `${root}/.dmux/worktrees/dmux-1781998195357`,
    branch: "feature/api-default-deny-auth",
    slug: "feature-api-default-deny-auth",
  }];
  return { root, ok: true, error: null, snapshot: { repo: root, prefix: P, places, strays } };
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
      declared: { last_worked_epoch: NOW - 30 * MIN },
      lifecycle_effective: "closed",
    }),
    place(P, root, {
      slug: "feat-redesign", branch: "feat/ui-redesign", dirty: true, dirty_files: 9, ahead: 7,
      tmux_session: { name: `${P}-feat-redesign`, up: true }, claude_session_present: true,
      last_commit_subject: "design tokens + nav", declared: { pinned: true, last_opened_epoch: NOW - 600 },
      lifecycle_effective: "active",
    }),
    place(P, root, {
      // The RENAMED place, and the only fixture whose branch equals its slug —
      // the shape `worktrees new <branch>` produces. It is what proves the
      // topbar prints the directory once: the title names it, the alias names
      // the directory/session/branch, and there is no branch chip to repeat it.
      slug: "random-work", branch: "random-work",
      tmux_session: { name: `${P}-random-work`, up: true },
      last_commit_subject: "note from standup",
      declared: { pinned: true, title: "standup-and-daily-work", last_opened_epoch: NOW - 2 * 3600 },
      lifecycle_effective: "active",
    }),
    place(P, root, {
      slug: "fix-flaky-ci", branch: "fix/flaky-ci",
      // afterglow t3 — this morning's work, nearly out
      last_commit_subject: "retry tmux smoke",
      declared: { lifecycle: "closed", last_opened_epoch: NOW - 9 * DAY, last_worked_epoch: NOW - 5 * 3600 },
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
