---
title: "Proposal — pull requests in the app"
---

# Proposal — a project's pull requests, and each lane's own

**Status:** design, 2026-10-07; questions answered the same day (§10). Not
built. Phase 1 only — phase 2 (agents, merged → remove) is deferred.

**The ask.** When a project's remote is GitHub, see its open PRs from the app,
viewed from `(main)`; and when a lane's branch is a PR's head, see that PR on
the lane. Where it goes is the open question: a sixth rail icon with a drawer,
a list, something in the nav row?

**The recommendation in one paragraph.** Fetch with **one `gh api graphql`
call per project** (≈1.2 s, 1 rate-limit point, measured below), parsed and
mapped to places in **`worktrees-core`** so the CLI and MCP can share it, and
cached and polled by the **app** only while the window is visible. Show it in
two places that answer two different questions: a **PR chip in the lane's
header** beside the branch chip and the remote link ("is *this* branch's PR
green?" — zero clicks), and a **sixth dock tab, Pull requests**, that lists
the project's PRs with the current place's PR pinned on top ("what is open in
this project?"). The nav row gets **nothing in phase 1**, and later only a mark
for PRs that need you (failing, changes requested, conflicting, merged). For
agents, a cached `pr` field on `place_status` is phase 2 — the strongest reason
for it is that a squash-merged branch is invisible to git, and a merged PR is
the one signal that it is done.

## 1. Data source — `gh`, as a subprocess, through GraphQL

### What exists

- `gh` is already shelled out once: `agentfiles.rs:411` (`gh pr create` for
  the agent-instructions fix), with a `WORKTREES_GH_BIN` seam so the bats
  suite never reaches GitHub. It degrades to "open the PR yourself" when `gh`
  is missing (`:524`).
- The app talks to `api.github.com` with **curl and no token** for this repo's
  own releases (`lib.rs:3074`) — public data only, so it says nothing about
  auth.
- `remote_url` (`lib.rs:4793`) + `normalize_remote` (`:4807`) turn `origin`
  into an https base; `remote.ts` decides which page to open. That is the only
  remote parsing in the app.

### Recommendation: `gh api graphql`, not `gh pr list`, not our own token

`gh` owns credentials for every case we care about — keyring, `GH_TOKEN`,
several accounts, Enterprise hosts — and shelling out is the house rule
(AGENTS.md: "git/tmux are shelled out on purpose"). A token of our own would
mean storing a credential this app has never had to hold; `gh auth token`
piped into curl would put the token in our process for no gain. So `gh`.

But not `gh pr list --json …statusCheckRollup`: that field makes `gh` fetch
**every check context of every PR**. Measured on this repo (9 open PRs, gh
2.101.0, three runs each):

| Call | Wall time |
|---|---|
| `gh pr list --json` all ten wanted fields incl. `statusCheckRollup`, `mergeable` | 1.93 / 1.19 / 1.57 s |
| same without rollup and mergeable | 0.68 / 0.68 / 0.70 s |
| `gh api graphql`, ONE query: open PRs + last 20 merged/closed, rollup as a single `state` | 1.21 / 1.25 / 1.22 s — **cost 1 point** of 5000/h |
| `gh auth token -h github.com` (offline, keyring) | 0.05 s |
| `gh --version` | 0.04 s |

The single GraphQL query returns both what main's list needs and the recent
merges that close the loop on a lane (§3), for one round trip, 9.6 KB of JSON.
The rollup state (`SUCCESS`/`FAILURE`/`PENDING`/`ERROR`/null) is what a dot can
show; the individual checks are a click away on GitHub.

```graphql
query($o: String!, $r: String!) {
  repository(owner: $o, name: $r) {
    open: pullRequests(states: OPEN, first: 100, orderBy: {field: UPDATED_AT, direction: DESC}) {
      totalCount
      nodes {
        number title url isDraft updatedAt
        headRefName headRepositoryOwner { login } baseRefName
        author { login } reviewDecision mergeable mergeStateStatus
        commits(last: 1) { nodes { commit { oid statusCheckRollup { state } } } }
      }
    }
    recent: pullRequests(states: [MERGED, CLOSED], first: 20,
                         orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes { number state url headRefName headRepositoryOwner { login } mergedAt closedAt }
    }
  }
}
```

Fields: number, title, url, draft, updatedAt, head branch + head owner, base
branch, author, review decision, mergeable + merge state, CI rollup + the head
commit's oid (so "CI is for a commit you have not pushed" is detectable — the
local branch's tip ≠ `oid`).

### Auth states, as `gh` actually reports them

Captured in a scratch repo against the real binary:

| State | How it shows | What the app shows |
|---|---|---|
| `gh` not installed | spawn fails (ENOENT) | Pull requests tab: one line, "Install the GitHub CLI (`gh`) to see pull requests here", link to cli.github.com. No header chip. |
| not logged in | **exit 4**, "please run: gh auth login" (`GH_CONFIG_DIR` pointed at an empty dir) | tab: "Run `gh auth login` in a terminal"; probe first with `gh auth token -h <host>` (50 ms, offline) so we never spend a network call to learn this |
| logged in, other host (Enterprise remote, only github.com authed) | `gh auth token -h ghe.example.com` → exit 1, "no oauth token found for ghe.example.com" | "Run `gh auth login -h ghe.example.com`" |
| logged in, but this account cannot see the repo (private, other account) | GraphQL `NOT_FOUND`, `repository: null`, exit 1 — GitHub does not distinguish missing from private | "GitHub says this repository does not exist for the signed-in account (`login`)" — name the account from `gh auth status --json hosts`, because a wrong active account is the likely cause |
| offline / host unreachable | "error connecting to …", exit 1 | keep the last good result, mark it stale ("as of 14:02"); a `NOT_FOUND` is also exit 1, so classify by stderr / the `errors` array, not by the code |

`gh auth status --json hosts` (present in 2.101.0) gives login, active flag and token
source per host in one local call; the app's Settings → Logs diagnostics block
(`lib.rs`, the `PATH / git / tmux` lines) should gain a `gh` line from it.

**GUI PATH.** `gh` is Homebrew's (`/opt/homebrew/bin/gh` here). A Finder
launch gets launchd's bare PATH; `fixup_gui_path()` already fixes that before
any command runs, and this feature adds no call before it. Keyring auth works
from the GUI process (same user, same login keychain) — to be confirmed in the
sandbox in phase 1, not assumed.

## 2. Detecting GitHub — and which repo the PRs live on

`normalize_remote` already maps `git@host:o/r(.git)`, `ssh://git@host[:port]/o/r`
and `http(s)://host/o/r` to an https base, with tests (`lib.rs:10816`). Phase 1
moves that function into core (`worktrees_core::github::parse_remote` →
`{host, owner, repo}`) so the CLI and MCP get the same answer, and leaves
`remote_url` calling it.

**Is it GitHub?** The host is `github.com`, **or** `gh` has a token for that
host (`gh auth token -h <host>` exit 0). That covers Enterprise without a
host list of our own. Anything else — GitLab, Gitea, a local path, no remote —
gets **no tab, no chip, no empty state**: the rail icon is not rendered for
that project at all, the same way the remote link is not rendered without a
base (`App.tsx`, "a repo with no origin gets no link at all, not a link that
apologises").

**Forks.** When a project is a fork, its PRs live on the parent. Mirror
`gh`'s own resolution so the app never disagrees with `gh pr list` run in the
same checkout: the repo set by `gh repo set-default` (stored as
`remote.<name>.gh-resolved` in git config) wins; otherwise a remote named
`upstream`, then `github`, then `origin`. The PR's head is then the fork, so
lane mapping (§3) must match on head owner as well as branch name.

## 3. Lane ↔ PR mapping

**Rule:** a place's PR is the PR whose `headRefName` equals the place's
**current local branch** and whose `headRepositoryOwner` is the owner of the
repo the place pushes to (origin's owner — which for a fork is the fork, and
otherwise the base repo itself). Open PRs first; if none, a merged/closed PR
from `recent` that is younger than the window below.

Real cases from this repo today (`worktrees ls --json` against the query):

| Place | Branch | Upstream | PR |
|---|---|---|---|
| md-path-links | md-path-links | origin/md-path-links | #451, open, CI ✓, CLEAN |
| codex-skills-mcps | **codex-skills-mcps-close-out** | origin/… same | #350, open, CONFLICTING |
| roadmap-and-github-issues | same | same | #296, open, CONFLICTING |
| pr-panel (this lane) | pr-panel | **origin/main** | none yet |
| — no place — | fix/codex-place-session | — | #442, merged 2026-10-07 |

Three lessons in that table:

- **Match the local branch, never the upstream.** `pr-panel` tracks
  `origin/main` (it was branched from it), so "upstream's branch name" would
  map this lane onto every PR targeting main. The upstream is a fallback only
  when it names a branch that is not the base.
- **A slug is not a branch.** `codex-skills-mcps` has moved to a close-out
  branch; matching by branch makes "the place's branch changed" a non-case —
  the chip simply follows the branch, and the old PR drops off the lane and
  stays in main's list.
- **Seven of nine open PRs are CONFLICTING** and five are a month old with no
  CI. Main's list must sort by `updatedAt` and group, or the stale tail buries
  the live ones (§5).

**Several PRs for one branch.** Possible with forks (same branch name, other
owner — excluded by the owner match) or a reopened branch (one open, older
closed). Open wins; among closed, the newest. More than one OPEN for the same
head and owner cannot happen on GitHub.

**Merged and closed.** Show a merged PR on its lane for as long as the place
exists **and** the PR is in `recent` (the last 20 closed/merged, which is about
a day of this repo's traffic). This is the most useful state, not the least:
this repo squash-merges, and a squash rewrites the SHA, so git cannot see that
the branch landed — `health.rs`'s `maybe_merged` is an explicit guess for
exactly that reason, and AGENTS.md has a whole close-out rule about it ("A
matching commit SUBJECT is not a merge check"). A `merged` chip on the lane is
that check, from the source of truth, and it is the moment to offer "Remove
this place". Closed-unmerged shows as closed, dim.

**PRs with no place.** They appear in main's list (and in every place's
tab — it is the project's list) under "No place". Phase 2 adds "Open in a
place" on those rows: `worktrees new <headRefName>` already fetches
`origin/<branch>` and tracks it (`ops.rs:483`, `:796`), so same-repo PRs are
one existing call. Fork PRs need a fetch from another remote — phase 3, or
never.

## 4. Freshness, cost, and where the code lives

**Where.** Split by what can be shared:

- **`worktrees-core::github`** (new): parse the remote and resolve the PR repo
  (§2), probe auth, run the query through `gh` (with a `WORKTREES_GH_BIN` seam,
  as `agentfiles.rs` does), parse to a `PrSnapshot { fetched_at, repo, prs:
  Vec<Pr>, recent: Vec<PrRef> }`, and `fn pr_for(place, snapshot)` — the
  mapping in §3 lives once. Unit-tested against captured JSON fixtures, the
  way `tests/fixtures/pi-*` pin pi.
- **The app** owns polling and the in-memory cache, one entry per project,
  serving every place in it. A Tauri command `project_prs(repo, force)`
  returns the cached snapshot if younger than the TTL, otherwise fetches —
  `async fn`, per the house rule, and coalescing concurrent callers onto one
  in-flight fetch so a focus event and a timer tick never spend two calls.
- **Not** in `list_workspace`/`snapshot()`: that is a local git/tmux fan-out
  that runs constantly and must stay offline-fast; a 1.2 s network call in it
  would stall the whole nav. PR state is joined in the frontend by `(repo,
  branch)`.

**Cadence.**

| Trigger | Fetch? |
|---|---|
| timer while the window is visible (`pageVisible`, as `USAGE_POLL_MS` does) | every **120 s**, selected project only |
| window regains focus / becomes visible | if the snapshot is older than 30 s |
| selecting a place in another project | if that project's snapshot is older than 120 s |
| opening the Pull requests tab, or its refresh button | if older than 15 s / always |
| a push from the app (branch switcher, agent-instructions fix) | yes, then once more at +20 s — `mergeable` reads `UNKNOWN` for a few seconds after a push while GitHub recomputes it, the same lag AGENTS.md records for `mergeStateStatus` |
| hidden window | never |

Rate limit: at one point per call, a project selected all day costs ~30
points an hour against 5000. Not a concern; noted only so no one adds a
per-place call later — **one fetch per project, never per place.**

`mergeable: UNKNOWN` renders as "checking", never as a conflict.

## 5. UI

### Rejected

- **A nav-row glyph for every PR (phase 1).** The row already carries the
  activity dot, dirty count, ↑/↓, age and lifecycle; most lanes have a PR, so a
  per-row PR mark is noise that says "this lane has a PR", which is the normal
  case. Reconsidered in phase 3 as a mark for **exceptions** only (§7).
- **A sheet (the StatusSheet / ProjectSheet pattern).** Sheets are for a
  one-shot check you read and dismiss; they slide over the terminal. A PR list
  is reference you keep beside the work while an agent runs, which is what the
  dock is for.
- **A section in the nav under each project.** The nav lists PLACES; PR rows
  in it would be a second kind of row with its own selection, drag and context
  rules, and would duplicate every lane that owns one.
- **A card on Home.** `(main)` is a place, not Home; David asked for it
  "viewed from (main)", and Home has no project.
- **A detail view of checks and reviews inside the app.** GitHub's page is
  better at it and one click away; the app's job is "should I look?", not
  rendering a review thread.

### Recommended: a header chip + a sixth dock tab

**The lane's header chip** — where the eye already is, beside the branch chip
and the remote link (`App.tsx`, `.identity`). Accent colour is a **dot**, words
stay `--txt-hi`/`--txt-dim` (the status-chip contrast rule). Click opens the
PR in the browser via `openUrl` (already permitted: `opener:allow-open-url`,
the remote link uses it). Hidden in the `fit.tight` squeeze along with the
other badges.

```
┌─ md-path-links ──────────────────────────────────────────────────────────┐
│ ‹ › worktrees / md-path-links  ⎇ ↗  ● #451 ✓            ● live  📌  ⋯ │
└──────────────────────────────────────────────────────────────────────────┘
                                    │
                    dot: green = CI ✓ & mergeable, amber = pending/checking,
                         red = CI failing / changes requested / conflicting,
                         accent = merged, grey = draft or closed
                    title: "#451 · Open · CI passing · mergeable — open on GitHub"

   merged lane:      ● #449 merged   [Remove place…]
   conflicting:      ● #350 conflicts
   no PR, pushed:    (nothing)
```

**The Pull requests dock tab** — `GitPullRequest` icon, sixth on the right
rail, after Automations. Rendered only for projects that resolved to GitHub
(§2). On `(main)` the icon carries a count (`rail-count`, as the offers icon
does) of open PRs. One rendering for every place: the project's list, with the
**current place's PR pinned at the top**, so on a lane the tab answers "mine,
then everything else" and on main it is simply the list.

```
┌ Pull requests ─────────────────────── ⟳ 2 min ago ┐
│ THIS PLACE                                        │
│ ● #451  Links to .md paths open in the viewer     │
│         md-path-links · ✓ CI · mergeable · 3h     │
│                                                   │
│ IN A PLACE (2)                                    │
│ ● #350  codex skills + MCPs close-out             │
│         codex-skills-mcps · conflicts · 12d       │
│ ● #296  ROADMAP + GitHub issues                   │
│         roadmap-and-github-issues · conflicts ·31d│
│                                                   │
│ NO PLACE (6)                              ▸ show  │
│                                                   │
│ RECENTLY MERGED                           ▸ show  │
│ ○ #442  resume Codex in this place   merged 2h    │
└───────────────────────────────────────────────────┘
  row click → open on GitHub     row ⌥-click / menu → select the place
  draft rows dim; author shown only when it is not the gh login
```

"In a place" rows link to the place (menu: *Go to place*), which is what makes
the list worth keeping beside the nav. "No place" is collapsed by default —
it is the stale tail (§3's table). Empty states are one line each, from §1's
auth table; the tab never shows a spinner where it has a previous result.

Rules this has to keep: `DockTab` grows a member, which touches
`navHistory.ts`'s `DockTab`/`TAB_LABEL`, `settings.ts`'s `dock_tab` union
(twice), `DOCK_RAIL` with its own literal `track` (`usage-check.mjs` refuses
an interpolated one), and the mock harness must answer `project_prs`. The row
menu calls `useEscape` like every other menu. No new tokens: dots reuse
`--ok`/`--warn`/`--danger`, and merged takes `--accent` (there is no violet
token, and GitHub's purple is not worth one). `GitPullRequest` is a new
hand-drawn entry in `icons.tsx`, like the other 31.

## 6. Agents — MCP and CLI

Worth it, as **phase 2**, and for a reason stronger than convenience: the
orchestrator decides when a lane is finished, and today it shells `gh pr view`
or guesses from git — which cannot see a squash-merge.

- `place_status` and `list_places` gain an optional `pr: {number, state,
  ci, review, mergeable, url, as_of}` — **read from a cache, never fetched
  inline**: `place_status` is called in loops (`wait`), and a 1.2 s network
  call per call is wrong. The cache is a file in the git common dir
  (`worktrees-prs.json`), the home `messages.rs` already uses for
  cross-process state; the app writes it after each fetch, and the MCP server
  refreshes it itself (same core function) only when it is older than 120 s and
  only from an explicit `pull_requests` tool, so an agent without the app
  running still gets an answer.
- `worktrees ls --json` does **not** grow it (offline contract, and the
  `ls --json` diff rule in AGENTS.md); a separate `worktrees prs [--json]`
  verb does, for scripts.

## 7. Settings, `gh` detection, and the marks

### One switch, and automatic absence

Settings → Behavior gets **Pull requests: On / Off** (`pull_requests: boolean`
in `settings.ts`, default **on**). Off is a real off switch, the way
`usage_place: "off"` is: no `gh` call of any kind, no tab, no chip, no badge —
not a hidden component that keeps polling.

No per-project switch. A project that does not resolve to GitHub (§2) already
gets nothing — no rail icon, no chip, no empty state — so a mixed workspace
needs no configuration: the GitHub ones show PRs, the others look exactly as
they do today. And since only the selected project is polled, a GitHub
project you do not care about costs nothing while you are elsewhere. If one
turns out to be wanted later, it is a per-machine entry in `ui-state.json`
keyed by repo, never `.worktrees.toml` (a cloned repo should not decide what
this machine fetches).

### Detecting `gh`, and offering it

Yes, detect it — it decides which of four things the tab says, and it costs
40 ms (`gh --version`). The app probes **once at startup** (after
`fixup_gui_path`), and again **on window focus while it is missing**, so
installing it in a terminal is picked up without a restart. Once present, the
login probe (`gh auth token -h <host>`, 50 ms, offline) runs before each
project's first fetch.

The offer lives where the gap is felt — the Pull requests tab of a GitHub
project — and nowhere else in phase 1:

```
┌ Pull requests ────────────────────────────────────┐
│ The GitHub CLI is not installed.                  │
│   brew install gh                         [Copy]  │
│ Then sign in once:                                │
│   gh auth login                           [Copy]  │
│                                   About gh ↗      │
└───────────────────────────────────────────────────┘
```

The app shows the commands; it does not run them. `brew install` installs
software system-wide and `gh auth login` is interactive (browser + device
code), and neither is an action this app has taken on anyone's behalf before.
The rail icon still renders in that state so the tab is discoverable; it is
the one empty state allowed, because it is the reason the feature is not
working, not an absence of data. Not-logged-in and wrong-host get the same
shape with only the login line. An `offers.ts` entry (the after-update band)
is the natural place for a one-time "See your pull requests — install gh"
suggestion, gated on feature on + gh missing + at least one registered GitHub
project; listed as optional, since the tab already says it.

### Attention without nav noise

The nav row does **not** get a second dot. Its dot is agent activity, and two
dots in one row would have to be read by colour alone, which is exactly the
contrast trap AGENTS.md records for accent tokens.

Instead the attention goes on the **rail icon's badge**. The Pull requests
icon carries a count (`rail-count`, as the offers icon does):

- plain/dim: open PRs in the project, when none needs you;
- `--danger` tint: the number that **need you** — CI failing, changes
  requested, conflicting, or merged while their place still exists.

That answers "is anything on GitHub waiting for me?" from every place in the
project, with no per-row cost. The header chip (§5) then says which state the
current lane is in.

If a per-row mark is still wanted after living with that, the candidate is a
**hollow ring** (not a filled dot) at the row's trailing edge, shown only for
the same four exception states and behind its own Navigation setting, default
off — so the shape differs from the activity dot before the colour does.
Deferred; it should be looked at in the harness against a real nav before
anyone commits to it.

## 8. What this does not do

Read-only toward GitHub, in **every** phase: no merge, comment, label,
approve — and **no PR creation**. A pushed lane with no PR shows nothing; the
app does not offer "Create PR" now or later (decision 4, §10). Opening a PR
stays with `gh`, the agent, or the browser.

## 9. Phasing

**Phase 1 — useful alone.** `worktrees-core::github` (remote → repo, fork
resolution, auth probe, query, parse, mapping) with fixture tests; the app's
`project_prs` command with TTL cache and in-flight coalescing; the lane header
chip; the Pull requests dock tab with the four groups; visibility-gated
polling (selected project only); the auth-state and install empty states;
the rail badge with its attention tint; the Settings switch; the `gh` line in
diagnostics; mock
harness support. Verified in `sandbox.sh --app` against this repo, including a
Finder-style launch (bare PATH) to see keyring auth work.

**Phase 2 — deferred (David, 2026-10-07: not needed for now).** `merged` chip → "Remove place…" through the
existing RemoveDialog; `pr` on `place_status`/`list_places` from the shared
cache + `pull_requests` MCP tool; `worktrees prs`; "Open in a place" for
same-repo PRs with no place; health/StatusSheet uses a merged PR to replace
the `maybe_merged` guess.

**Phase 3 — only if wanted.** The hollow-ring nav mark (§7); ⌘K entries
("Go to PR #…"); fork PRs into a place.

## 10. Decisions (David, 2026-10-07)

1. **Tab on a lane:** the whole project list, the lane's own PR pinned on top.
2. **Merged:** stays on its lane until the place is removed.
3. **Polling:** the selected project only.
4. **No PR creation, ever.** The app stays read-only toward GitHub in every
   phase (§8). Phase 2 (agents/MCP, merged → remove, open-in-a-place) is
   not needed for now.
5. **Nav mark:** the nav is noisy enough. Attention goes on the rail icon's
   badge; a per-row mark only as a distinct shape, opt-in, later (§7).
6. **Settings:** one global on/off; non-GitHub projects show nothing on their
   own (§7).
7. **`gh` missing:** detected; the tab shows the install and login commands to
   copy. The app never runs them.
