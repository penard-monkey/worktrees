---
title: "Proposal — per-place docs"
---

# Proposal — per-place docs

**Status:** **phases 1, 2 and 3 BUILT** — 1 and 2 on 2026-09-16, 3 on 2026-09-19
(§15, which also records two claims in *this* section that the build found
wrong). The §13 gate is now enforced at runtime on every spawn rather than
settled once at build time, and §8's "does not ship on a stock `mo`" is an
assertion in two places rather than a decision someone has to remember.
Originally: the Docs tab, the staleness header
and the name filter, behind no new dependency, process or config. §7.3's open
questions are answered in §11, and **two claims in this document were wrong**;
both are corrected there, and one of them (§7.1's `behind … upstream`) would
have shipped a silently wrong staleness header — the exact failure §1.1 says
this feature exists to prevent. The viewer question (§5.2) **was**
spiked, on this date, and is now answered — the answer changed the shape of §5.3,
which is worth reading even if the rest is skimmed. Phase 1 never depended on it.
**Supersedes:** nothing. No design of a documentation surface exists in this repo
(§10). The nearest thing — `DESIGN.md`'s unbuilt "infra strip … open-localhost
links" (`DESIGN.md:31-32`, `467-468`) — is adjacent, not superseded; its
executable half was already reversed by ADR 0001.
**Owner:** this repo. First consumer: `valleos` — eleven live places, 424
markdown files, and the hazard in §1.1.
**Backing research:** four files in the consumer's working memory
(`valleos/.worktrees/docs/.planning/live-docs/{findings,design,host-recon,spike}.md`,
gitignored there, archived at that session's close-out): findings F1–F9, the
three-way design fork (A native tab / B embedded service / C native index +
browser viewer; C chosen), and 656 lines of `file:line` recon on this app at
`29671fc` (v0.23.1) — plus `spike.md`, an executed spike against the candidate
viewer with observed output. Every citation below was re-read against that commit.

---

## 1. The gap

**The app already reads markdown.** The Files dock has rendered it since v0.8.0
(`CHANGELOG.md:1033-1043`): `app/src/markdown.tsx`, 304 lines, `marked` as a
lexer only, React nodes built by hand, no `innerHTML`, `safeHref` on every link,
raw HTML shown as literal text (`markdown.tsx:117-119`, `217-218`). Relative
links, anchors, reading mode, per-place zoom, ⌘F, side-by-side diff. That is
most of "browse the repo's docs", and this proposal does not touch it.

What is missing is narrower than "a docs viewer", and it is three things:

1. **Diagrams do not render at all.** There is no diagram library anywhere in
   `app/src` or `package.json`. A ```` ```mermaid ```` fence goes through
   `fenceLang` (`markdown.tsx:269`) to `""` and is shown as a plain code block.
   `svg` is aliased to the `"xml"` grammar (`markdown.tsx:266`), so inline SVG
   is highlighted text; an SVG *file* is an `<img>` over a `data:` URI
   (`filekind.ts:135`, `FilesPane.tsx:641-644`), which cannot follow an `<a>`
   inside it. "Click a box to descend into that subsystem" has no path at all.
2. **There is no docs-shaped index.** The dock offers a file tree. It has no
   notion of "the architecture pages" or "the timeline" — the structure a
   documented project encodes in its `docs/` layout and, where it has them, in
   page titles.
3. **The reading experience shows no staleness signal.** `Place` already
   carries `branch`, `behind`, `upstream`, `dirty`, `dirty_files`,
   `last_commit_subject`, `last_commit_epoch` (`crates/worktrees-core/src/model.rs:26-36`).
   The reader never shows any of it. §1.1 is why that matters.

### 1.1 The live hazard: the docs are already two different worlds on disk

This is not hypothetical. `worktrees ls --json` on `valleos`, 2026-09-16,
**eleven places, every one `lifecycle_effective: active` with a live tmux
session**:

| place | behind | last commit |
|---|---|---|
| `(main)` | 0 | |
| `messaging` | 2 | joins the two halves of the spine |
| `communications` | 12 | *dirty, 1 file* |
| `docs` | 25 | |
| `vendor-api` | 41 | |
| `publish-automation` | 47 | |
| `ssdlc` | 53 | |
| `qa` | 59 | |
| `messaging-extraction` | 68 | |
| `dev-env` | 76 | |
| `regression-locks` | 85 | |

A `docs/` restructure landed on main the day before. `ls docs` per place, same
day:

```
(main), communications, docs, messaging          architecture/ archive/ browse/ channels/   ← the new tree
dev-env, messaging-extraction, publish-automation,
qa, regression-locks, ssdlc, vendor-api          00_acuerdo…pdf 01_acuerdo…md 02_cuestionario…md   ← the old flat tree
```

**Seven of eleven places carry the old tree. Both states are correct for their
branch.** An ADR written on `ssdlc` reads as current there; on main the same
document has been moved and renumbered. A viewer that shows "the docs" without
saying *which place, and how far behind what* is not neutral — it is silently
wrong, and the failure is invisible at the moment it costs the most: reading a
53-commits-old decision as if it were the decision.

Three consequences that shape everything below:

- **The place is the primary axis, not the repo.** There is no "the docs" for
  a project; there is a docs tree per place, and the app already models places.
- **Staleness must be on screen while reading, not one click away.** The data
  is free — core computes it on every `ls`.
- **Nothing may run per place.** Eleven of anything at a few hundred MB each is
  the cost the disk is already paying once for `node_modules`. And four of the
  eleven have no docs tooling on their branch at all — a viewer that depends on
  the project's own `apps/docs` works on some places and not others, which is
  exactly the inconsistency being fixed.

---

## 2. Design principles

1. **Place first.** Every screen names the place it is showing. There is no
   repo-wide docs view in this proposal.
2. **Staleness is a header, not a badge.** Branch, `N behind <base ref>` (NOT
   `<upstream>` — §11.4), dirty count, last commit subject and age — always visible in the index and in the
   viewer, never collapsible. "Behind is never sickness" (`health.rs:11-13`)
   still holds: it is context, not a warning colour.
3. **Zero config, richer with structure.** A repo containing nothing but
   markdown files gets a usable index from a fixed convention (§3.1). A repo
   with a `docs/` tree, titled pages, or an allow-listed `[docs]` section gets a
   better one. Absent config ⇒ byte-identical behaviour to today, elsewhere in
   the app.
4. **The renderer is tool-owned.** Derived from ADR 0001 in §4.1, not chosen.
5. **The app stays a control surface.** Long reads and diagrams happen in the
   user's real browser, opened through the `openUrl` path the app already uses
   (`App.tsx:4108`, `FilesPane.tsx:600`). No new UI library enters `app/`, no
   iframe, no second webview.
6. **A document can never execute.** No raw HTML, no mermaid callbacks, no
   outbound network from the viewer. §4.3.
7. **Fail per file, loudly.** A page that does not render is a card that says
   so, beside pages that did — the shape `ViewErrorBoundary` already gives the
   Files dock (`FilesPane.tsx:458`). Never a dead server for the whole place.

---

## 3. The format — what counts as docs

### 3.1 The convention (no config)

With no `.worktrees.toml`, or one without `[docs]`, the index for a place is:

```
README.md CLAUDE.md DESIGN.md ROADMAP.md CHANGELOG.md   # root, in this order, if present
docs/**/*.md                                             # the tree, recursively
*.md                                                     # any other root-level markdown
.planning/brief.md                                       # the agent brief — tool-owned already (ops::BRIEF_PATH)
```

The walk never descends `.git/`, `.worktrees/`, `node_modules/`, `target/`,
`dist/`, and never follows a symlink (§4.2). `.worktrees/` is not a
performance rule: on `(main)` it *contains every other place*, and walking it
would attribute ten places' documentation to one.

Entry title: frontmatter `title:` if the file starts with a `---` block that has
one; else the first `# ` heading; else the filename. That is the whole
"structure" the index reads — `title` is the one key Jekyll, VitePress, Docusaurus
and MkDocs all agree on, and reading anything more is per-generator work the
tool should not own. Everything is `read_file`-shaped: bytes under a registered
project root, through `guard_under_projects` (`lib.rs:3070-3082`).

Whether `.planning/brief.md` belongs in the list is an open question (§7.3): it
is gitignored working memory, but it is also the document the tool itself
treats as special.

### 3.2 The optional `[docs]` section

```toml
# .worktrees.toml — committed, allow-listed, DATA. Never a command.
[docs]
paths = ["docs", "packages/db/README.md", "apps/api/docs"]   # replaces the convention's tree, keeps the root files
index = "docs/index.md"                                       # optional landing page; default README.md
```

- `paths` entries are `RelPath`s (`projcfg.rs:98`), so they pass Layer A on
  parse: no absolute paths, no `~`, no `$`, no `..`, no `.git` component. A
  directory is walked; a file is listed. No globs in v1 — a glob is still data,
  but it adds an expansion step to a security-relevant list for no consumer
  that has asked.
- `[docs]` joins the closed known-key set that `survey_keys` checks
  (`projcfg.rs:484`). Today an unknown table is warned about and dropped
  (`projcfg.rs:494-499`); a user-only key smuggled inside `[docs]` stays a hard
  error, because the survey refuses `is_user_only` names in every known scope
  (`projcfg.rs:426-427`).
- `WORKTREES_NO_PROJECT_CONFIG=1` (`projcfg.rs:366-377`) disables `[docs]` with
  the rest of the project rung — the "auditing an untrusted clone" switch
  applies here without new code.
- **This section is optional in the strongest sense: phase 1 shipped without it
  (§8).** The convention has to be good enough that most repos never write it.

**Built, with two decisions the draft did not make — §12.** `[docs]` is read
from **the place, not the main worktree** (the only `projcfg` consumer that
diverges, and the feature defeats itself otherwise), and a `.worktrees.toml`
that does not parse falls back to the convention and says so on screen rather
than blanking the tab.

---

## 4. Security boundary

### 4.1 ADR 0001 decides the architecture

`docs/adr/0001-no-repo-supplied-argv.md:13-14`: *"Nothing a cloned repository
contains may become argv, or name a program to run."* Accepted, permanent, and
its load-bearing line is the provenance table — *the repo picks which of my
commands, never what.*

Apply that table to the obvious design and it is dead on arrival:

| Channel | Who writes the string | Verdict |
|---|---|---|
| `pnpm dev:docs`, `mkdocs serve`, `vitepress dev`, any `[docs] serve = "…"` | the cloned repo's committed file, verbatim | **never** — this is `[hooks]` wearing a docs badge |
| `[docs] paths = [...]` | the repo, as `RelPath` data the *tool* walks | allowed — same class as `[[file]] path`, `[ports] base`, `[compose] files` |
| the viewer process | the tool: its own binary, its own argv, roots it validated | allowed — the tool assembles the argv, exactly as `compose_down` does (`provision.rs`) |

So the renderer is tool-owned by derivation, not preference. A per-project
docs server is not deferred; it is the thing ADR 0001 exists to refuse, and the
ADR says there is no threshold of demand that flips it. The consequence is
accepted: a project cannot ask this tool to run its own docs toolchain. If it
wants its site, it runs its site.

### 4.2 Paths

Two layers, mirroring the project-settings boundary.

- **Layer A, parse time** — every `[docs]` path is a `RelPath`, so the
  rejections in `project-settings.md` §4 apply unchanged, including the
  case-only-duplicate rule and the `.git` name rule.
- **Layer B, walk time** — the walk canonicalises the place root once and
  checks every entry it emits with `starts_with` on the resolved path — the
  `guard_under_projects` shape (`lib.rs:3070-3082`) applied at the walk rather
  than per read. `symlink_metadata`, never `metadata`: a committed symlink
  `docs/secrets.md → ~/.ssh/id_rsa` is listed as nothing. Depth cap 12, entry
  cap 2,000 with `truncated: true` reported, per-file size left to `read_file`'s
  own 1 MiB cap (`lib.rs:3518-3530`).
- The viewer serves **only** roots the app handed it at spawn, and the app hands
  it only canonicalised place directories under registered projects.

### 4.3 The viewer

- **Loopback only, as a rule, not a default.** `127.0.0.1`, an ephemeral port,
  and the app never widens the bind. These documents carry a client's signed
  agreement and personal data; a docs viewer reachable from the LAN is a data
  leak with a nice font. A per-launch random token in the URL prefix is the
  cheapest defence against a port nobody authenticates.
  **Not a hard requirement — see §11.3**, which says what phase 3 must prove
  instead, and why the threat a token actually blocks is a web page rather than
  another process.
- **Zero outbound calls.** Renderer, highlighter and diagram engine are bundled
  in the viewer. `grip` is disqualified outright on this rule: it POSTs document
  content to GitHub's markdown API. Any candidate that phones home for anything
  is out.
- **Raw HTML in markdown is inert.** The dock shows it as text by decision
  (`markdown.tsx:1-10`, `2026-08-03-files-viewer` D2); the viewer must either do
  the same or sanitise through an allow-list. A viewer that renders raw HTML
  as-is is a viewer that renders `<script>` from a repo you just cloned.
- **Mermaid stays at `securityLevel: strict`. Never `loose`.** `loose` is the
  level at which `click NODE call fn()` — a callback *declared inside the
  document* — runs. This tool's hot path is opening a fresh clone. Drill-down
  is obtained the other way round (§5.3): mermaid renders to SVG under `strict`,
  and the viewer post-processes the SVG with a mapping *the viewer* owns. No
  callback ever originates in a document.

---

## 5. Execution — what runs, and who owns it

### 5.1 One process for the app, not one per place

The app supervises children in exactly one shape today: PTYs. `Shells` is a
`Mutex<HashMap<(repo, slug, index), Shell>>` (`lib.rs:4397-4400`), spawned on
demand, liveness by `try_wait` on every read (`lib.rs:3656-3665`), no
auto-restart, killed on `RunEvent::Exit` (`lib.rs:5050-5064`). There is no port
allocation on the app side (`provision::port_free` at `provision.rs:281-290` is
reusable; the slot allocator is not, it is per user stack), no health checks,
no service registry, and no filesystem watcher — `notify` has 0 hits in
`Cargo.lock`; the Files tab re-reads on `places:changed`, which fires on tmux
change or every 30 s (`lib.rs:4876-4882`).

The viewer is the first non-PTY child, and the proposal keeps it inside that
existing shape rather than building a supervisor:

- One managed `Viewer(Mutex<Option<ViewerProc>>)` beside `Shells`
  (`lib.rs:4731-4732`). **One process for the whole app** — F1's eleven places
  cost eleven *roots*, not eleven processes.
- Spawned lazily by the first `open_docs_viewer` command, on a free loopback
  port chosen with `port_free`; PATH is already fixed by `fixup_gui_path`
  (`lib.rs:4612-4635`).
- Liveness by `try_wait` on every `open_docs_viewer`; a dead viewer is respawned
  on the next open, never on a timer. `applog` on every failure.
- Killed in the `RunEvent::Exit` handler next to the shells. It dies with the
  app, deliberately, like they do (`lib.rs:4329-4332`).
- Roots: the app passes every registered place `(slug, canonical dir)` at spawn.
  When the place set changes (`places:changed` with a different slug set), the
  viewer is restarted on next open — not hot-reconfigured. Simple, and it makes
  the URL for `(place, path)` a pure function the frontend can build.

Freshness inside the viewer is the viewer's own live reload; freshness of the
index in the dock is the 30 s poll, tightened for free if ROADMAP's FSEvents
watcher lands (`ROADMAP.md:490-499`).

### 5.2 Which viewer — pending the spike

Two candidates, and the choice is being spiked in parallel. This section states
the acceptance criteria; it does not assert a result.

| Requirement | Why |
|---|---|
| single binary the *tool* ships and runs; no toolchain on the user's machine | ADR 0001, and `install.sh` installs a checksum-verified binary, not a runtime |
| N named roots in **one** process | §5.1; Storybook Composition's issue tracker is the cautionary tale for N processes with unreachable refs |
| bundled markdown + highlighter + mermaid; zero network | §4.3 |
| `--bind 127.0.0.1`, `--port <n>`, ideally a URL token | §4.3 |
| a rendered SVG the viewer's own code can walk after mermaid returns | §5.3 |
| raw HTML inert or allow-list sanitised | §4.3 |
| licence compatible with shipping in a release | `release.yml` builds and signs the artefacts |

Candidate A, a third-party viewer (`mo`: Go, MIT, bundled mermaid, named groups
via `--target` in one instance, `--bind localhost` default with an explicit
no-auth warning in its README, verified 2026-09-16). It gives everything on the
list except the last two rows, which are exactly what the spike is testing.
Cost: a Go binary in a Rust/Tauri release pipeline, and no say over its
markdown renderer's HTML policy. **§11.5 sizes the first of those** — it is
smaller than this paragraph implies, and for one specific reason.

Candidate B, a small tool-owned server in this workspace with a bundled JS
renderer. Full control of §4.3 and §5.3; the cost is owning a markdown renderer
and a diagram integration, which F5's landscape survey argues against
rebuilding.

If the spike shows a third-party SVG cannot be post-processed under the
security rule, the answer is B, not `loose`.

#### The spike's answer, 2026-09-16 — neither candidate as posed

**Post-processing candidate A is impossible, and drill-down works anyway.**

`mo` renders Mermaid to **inline SVG in the light DOM** — no iframe, no shadow
root, no image — and initialises it with `{startOnLoad:false, theme}` and **no
`securityLevel`**, which is strict semantics. Under strict, `click <node>
"<url>"` still renders as a real `<a href>` around the node, while `click … call
fn()` never fires. **The security posture §5.3 asks for is what this viewer
already does, by default rather than by configuration.**

But attaching anything *after* render fails: `mo` has no extension point of any
kind — no CSS, JS, template, config, plugin flag, env var or `window` hook — the
server ships raw markdown and renders browser-side, and the SPA **re-renders
every diagram from a string** on theme toggle and on live-reload. Handlers
attached from outside survived **0 of 11 nodes**. A proxy-injected script would
key on undocumented class names in a 2 MB minified bundle: a fork in all but
name.

**So the answer is A′: `mo` serving a tree we generate, not the repo.** Nodes
become clickable because the markdown `mo` reads already carries `click`
directives — and since we cannot write those into the user's own files (a viewer
that edits what it is viewing is not a viewer), they are emitted into a
**derived tree**. That tree is not a workaround. It is also where the
frontmatter noise goes away (`mo` renders frontmatter as an expanded `<details>`
"Metadata" block on every page), where the §7.1 staleness header can be injected
into the documents themselves, and where anything later becomes possible without
touching the repo. It is most of the way to owning the rendering at a fraction
of a renderer's cost — and it leaves candidate B a strict upgrade path rather
than a rewrite.

**Measured, not assumed:** 25.8 MB binary · 0.23 s to listen · **25 MB RSS for
138 files across 7 groups** · zero non-loopback sockets · state confined to
`~/.local/state/mo/` and paths only, never content. Isolation behaved: a
nonexistent path exits 1 before serving, an invalid-UTF-8 file is skipped,
broken YAML or Mermaid renders with fallbacks, and **one bad group never killed
the server** — which is §1.1's requirement, tested rather than hoped.

**Link mechanics, verified.** The working form is absolute:
`http://localhost:<port>/<group>?file=<id>`, where `id = sha256(<absolute
path>)[:8]`. A relative `./x.md` inside a `click` directive misroutes to another
group. Prose links are unaffected — `mo` rewrites `[x](./x.md)` itself and
resolves in-SPA. Generated links therefore bake in port, group and absolute
path, so **the derived tree is generated at launch, never committed.**

**LikeC4 is not an alternative viewer and is out of scope here.** It requires
its own `.c4` DSL and cannot consume Mermaid or markdown — it only *exports* to
Mermaid. Its drill-down is native and SPA-smooth, at ~45 s cold start, 336 MB
cache, 341 MB RSS and Node ≥22.22. It is worth revisiting only if the
architecture model becomes a first-class authored artefact, which is a different
decision from this one.

### 5.3 Drill-down diagrams — the requirement and the rule

The consumer's six merged mermaid diagrams carry roughly 95 nodes and **zero**
`click` directives. Drill-down is an unused capability, not a missing one — and
the naive way to use it (`click NODE "target"` written by the diagram author)
is the wrong way, because generic utility means it has to work on diagrams
nobody wrote for it.

**Requirement.** In the viewer, a node in a rendered diagram whose id or label
resolves to a known target becomes a link: to another page in the same place,
to an anchor, or to a file on disk (opened through the app, not the browser).

**Rule.** `securityLevel: strict`, always — and after the spike, the mechanism
changes while the rule does not. We do **not** walk the rendered SVG; that is
not reachable in the chosen viewer (§5.2). Instead the tool **emits `click`
directives into the derived tree** it generates, and mermaid's own strict
semantics turn them into plain anchors. The security property is stronger than
the original plan, not weaker: the directive is written by the tool, never by
the document.

**Two rules the generator must hold, both security-relevant:**

1. **Strip every author-supplied `click` directive** from a diagram before
   emitting our own. Strict mode blocks `call fn()`, but `click NODE "<href>"`
   is still an author-controlled href in a document this tool will happily open
   from a repo cloned five seconds ago. Only tool-emitted targets survive.
2. **Emit only loopback targets on the viewer's own port and group.** Anything
   that is not `http://127.0.0.1:<our port>/…` is not a drill-down target.

The mapping sources, cheapest to author first:

1. a naming convention — node id equals a page slug in the index;
2. a fenced sidecar block beside the diagram;
3. page frontmatter;
4. **derived** — a node whose label equals a package or directory name links
   there. The only source that works with zero authoring, which §2.3 says is the
   requirement. F5 tempers it honestly: derivation reads well at exactly two
   grains, package-level graphs and database ERDs; between those it is a
   hairball, so derived mapping is scoped to those two.

**Answered.** The spike ran. Post-processing is impossible in candidate A and
unnecessary: generated `click` directives in the derived tree give node→page
navigation today, under strict semantics, with no fork. What that form does
**not** give is node→*view* drill-down or SPA-smooth transitions — a click is a
full page load. If those become requirements, that is the trigger for candidate
B, and the derived tree is already the seam it would slot into.

---

## 6. Implementation seams

Honestly: **there is no plugin seam.** The right rail is a literal array and the
tab is a literal union; a third tab is an edit to this app, not an extension
point. The edits, at `29671fc`:

| Where | Change |
|---|---|
| `app/src/settings.ts:51`, `:153` | widen `dock_tab: "files" \| "terminal"` to include `"docs"` in both `PlacePanels` and `Settings`. Existing `ui-state.json` records stay valid — the union only widens. |
| `app/src/App.tsx:5512-5515` | a third `DOCK_RAIL` entry. `app/src/icons.tsx` has no book icon (20 hand-copied Lucide paths, `icons.tsx:43-210`); one new path. |
| `App.tsx:6025` | `dock-title` ternary → three-way. |
| `App.tsx:6027-6086` | the Files-only header controls are gated on `=== "files"` and stay that way; the Docs header gets its own (refresh, filter). |
| `App.tsx:6089` | `dock-body` ternary → mount `<DocsPane>` at module scope, in its own file beside `FilesPane.tsx`. Not inside `App()` — components defined there remount every render (CLAUDE.md:145-146). |
| `App.tsx:165-178` `findTarget` | ⌘F ownership. It already special-cases `"terminal"`; Docs owns ⌘F for its filter box. `keyRef.dockTab` (`App.tsx:4865`) mirrors it. |
| `App.tsx:2880`, `:4866` | `filesDockShown` / `filesTabOpen` are `=== "files"` and remain correct by accident; re-read both. |
| `App.tsx:4856-4858`, `5060-5065` | `toggleDock` and ⌘⇧T assume two tabs; confirm neither cycles. |
| `App.tsx:6187` | **`data-track={d.key === "terminal" ? "dock.terminal" : "dock.files"}`** — a third key silently reports as `dock.files`. Add `"dock.docs"` to the closed vocabulary in `usage.ts:28-29` and to `valid_token` (`lib.rs:4013`); `app/scripts/usage-check.mjs` guards it. |
| `app/src/mock/install.ts` | arms for `list_docs` and `open_docs_viewer`; unknown commands resolve `null` + `console.warn` (`install.ts:5-7`), and a typed `invoke<DocsIndex>` returning `null` is a blank pane, so the pane must tolerate it. |
| `app/src-tauri/src/lib.rs` | `list_docs(repo, slug)` — one `async fn` (sync handlers freeze the UI, CLAUDE.md:130-131) that walks per §3 through `guard_under_projects`; `open_docs_viewer(repo, slug, path)` returning a URL for the frontend to hand to `openUrl`. Registered in `generate_handler!` (`lib.rs:4963-5047`). |
| `crates/worktrees-core/src/projcfg.rs:252` | `ProjectConfig` gains `docs: Option<Docs>`; `[docs]` enters the known-key set; `RelPath` for every path. Phase 2. |
| a new managed `Viewer` state | §5.1. Phase 3. |

Two per-place-memory rules carry over. `place_panels` remembers the active
tab, so a place you were reading docs in reopens on Docs — free, via
`panelsFor` (`settings.ts`). The open file is remembered too, since #302 —
but NOT in `place_panels`: it lives in its own `files_open` record (keyed
`repo|slug`, beside `term_tab_active` and this proposal's `docs_collapsed`),
because every `place_panels` key needs a global twin that seeds unvisited
places, and one place's path means nothing in another. The objection that
kept it unstored for a year (a remembered path can be deleted, renamed or
gitignored between visits, and a failed read is an error banner) is answered
rather than dropped: the restore asks `file_readable` first and silently
opens nothing when the path is gone. A document opened from the Docs tab goes
through the same `openDockFile`, so it comes back with the place like any
file-tree row. Any new optional field in `place_panels` must stay optional or
the seed freezes (CLAUDE.md, "a `place_panels` field whose global twin is a
SEED").

One seam I could not confirm from source: whether `FilesPane` (`FilesPane.tsx:822`)
can be told from outside to open a given path. Phase 1's row action depends on
it (§7.2); if it cannot, the fallback is `open_editor`, which exists.

---

## 7. App surface

### 7.1 The Docs tab

Third icon in the right rail. Disabled with no place selected, like the other
two (`App.tsx:6181`). The pane, top to bottom:

```
┌ Docs ───────────────────────────── ↻ ┐
│ docs · 25 behind origin/main · clean │  ← the staleness header. Always. Not collapsible.
│ "docs(runbook): reconcile §4…" · 2h  │     branch, behind <upstream>, dirty/N files, last subject, age
├──────────────────────────────────────┤
│ [ filter by name…                  ] │  ← name filter. Not full-text; that is the viewer's job.
├──────────────────────────────────────┤
│ README                               │
│ CLAUDE · DESIGN · ROADMAP · CHANGELOG│
│ Brief (.planning/brief.md)           │
│ docs/                                │
│   architecture/  (6)                 │
│     Platform overview                │  ← title from frontmatter → H1 → filename
│     Message flow                     │
│   archive/       (31)                │
│   browse/        (4)                 │
├──────────────────────────────────────┤
│ [ Open this place in the browser ]   │  ← phase 3. Absent in phase 1, not greyed.
└──────────────────────────────────────┘
```

The header renders from the `Place` the app already holds — plus ONE string
from the backend: the ref `behind` is counted against. **That ref is not
`upstream`**, and this paragraph used to say it was; §11.4 has the evidence.
"25 behind" without saying behind what is the same silent error one level up,
and "25 behind `origin/live-docs`" is a worse one.

As built, the header is three lines rather than one. `branch` and the dirty
count share the first, `behind` gets the second to itself, and the last commit
subject and age take the third. The dock's floor is `DOCK_MIN = 240`
(`settings.ts:394`, §7.3's third question): one line ellipsises at any real
width, and the run it cut first was `origin/main` — turning the only fact here
that appears nowhere else in the app into `25 behind origin/m…`.

### 7.2 Row actions

- **Read** — phase 1: opens the file in the Files dock's existing renderer (if
  `FilesPane` accepts an external path, §6; else `open_editor`). Phase 3: opens
  it in the browser viewer, and the in-app read stays as the secondary action.
- **Reveal** — `revealItemInDir`, already imported and permitted.
- Nothing else. The dock viewer is read-only by decision (`2026-08-03` D1) and
  the Docs tab is not a way back in.

### 7.3 Open questions for the app surface

1. Is `.planning/brief.md` a docs entry (pinned, labelled "Brief") or not? It is
   the document the tool treats as special; it is also gitignored working memory
   and shows up in no other project's convention.
2. Should `(main)`'s index show a per-place strip — "7 places carry an older
   `docs/`" — or is that the nav's job? The proposal says the nav's: the Docs
   tab is per place, by §2.1.
3. Dock width is a tmux SIGWINCH (CLAUDE.md:229-249); a docs index wants to be
   narrow, which is the easy direction, but the header line needs a truncation
   rule at `DOCK_MIN = 240` (`settings.ts:394`).

### 7.4 The browser viewer (phase 3)

Same staleness header at the top of every page, rendered by the viewer from
data the app passed at spawn and refreshed on restart — the browser tab is
*further* from the place than the dock is, so it needs the header more, not
less. Two surfaces is the honest cost of Path C: the docs are one keystroke
from the app, not literally inside the right nav. What it buys: no mermaid
(~2–3 MB) in an app whose rule is "no UI libraries, parsers are fine"
(CLAUDE.md:488-493); no `tauri://` → `http://localhost` iframe question, which
CSP would not stop (`tauri.conf.json:22-24` is `csp: null`) but WKWebView might,
answerable only by a real-app run; and heavy rendering where a real renderer
already lives, on the second monitor, while the app stays the control surface.

---

## 8. Phasing

**Phase 1 — the Docs tab, no viewer.** The union, the rail entry, the
ternaries, `findTarget`, the usage token, the mock arm, one backend command
(`list_docs`, convention only, §3.1), the staleness header, the name filter,
Read/Reveal. **No new dependency, no new process, no capability change, no
`.worktrees.toml` change.**

This is worth shipping on its own, and it is worth arguing for on its own:
of the three gaps, staleness is the one whose failure is silent, and it is the
one that costs nothing to close — every number in the header is already in
`Place`. A user with eleven places gets, today, a per-place documentation index
that says which world they are looking at. Diagrams are a feature; the header
is a correction.

**Phase 2 — `[docs]` and titles.** The allow-listed section (§3.2) through
`RelPath` and the known-key set; frontmatter/H1 titles. Richer where structure
exists. Still no process.

**Phase 3 — the tool-owned viewer.** Gated on the spike (§5.2). The `Viewer`
state, `open_docs_viewer`, the button and the per-row browser action, the
loopback/token/no-network rules, bundling the binary in `release.yml`.
Diagrams render here for the first time.

**Phase 4 — drill-down.** SVG post-processing under `strict` (§5.3), naming
convention first, derived package-level and ERD mapping second.

**Never.** A per-project serve command or any repo-authored string that reaches
argv (ADR 0001). `securityLevel: loose`. An embedded iframe of the viewer inside
the dock — not because it is known to fail, but because it is unverified in
WKWebView and Path C makes the question unnecessary; if someone wants it later,
it is its own proposal with its own real-app evidence.

---

## 9. Test plan

Their gates, in their order: release build first, `make test`, `make lint`,
the three `cargo test` crates, `tsc --noEmit`, `cargo check -p app`. Docs-only
PRs skip CI; the moment phase 1 touches `app/src` the full suite runs.

**A new test is shown red first (CLAUDE.md:97-101).** The ones below that most
need it are marked.

`cargo test -p worktrees-core` (phase 2), pure, no filesystem, following the
`projcfg` conventions:

- a `[docs]` table no longer produces an `unknown key` finding — **red first**:
  it passes today only if the table is still being dropped, which is the bug;
- `[docs] ai_cmd = "…"` is a hard parse error naming the user config
  (`survey_keys` refuses user-only keys in every known scope);
- every Layer-A rejection on a `[docs] paths` entry: absolute, `~`, `$`, `..`,
  `.git`, `.worktrees`, case-only duplicate;
- the index walk as a pure function over a synthetic tree (`probe`/`plan`
  split, the `materialize.rs` shape): never descends `.worktrees/`, `.git/`,
  `node_modules/`; a symlink entry is omitted; the entry cap sets `truncated`;
  root-file order is fixed; title precedence frontmatter → H1 → filename.

`cargo test -p app --lib` (`lib.rs` `mod tests`, note `check`/`build` do not
compile it):

- `list_docs` on a path outside every registered project is refused — the
  `guard_under_projects` contract, asserted on the new command;
- phase 3: the viewer child is killed on exit and `try_wait` reports it —
  drain nothing, it is not a pty, but the reaped-pid trap (CLAUDE.md:396-406)
  applies to any pid sampling.

Frontend:

- `tsc` — widening the union is the compiler's job; every `=== "files"` site
  that should have become three-way shows up as a type error or a missed
  branch. Walk `App.tsx:2880`, `4866`, `6025`, `6027`, `6089` by hand anyway.
- `node app/scripts/usage-check.mjs` — the new `data-track="dock.docs"` token
  and the closed vocabulary; **red first** by adding the rail button without the
  token, which is exactly the silent misattribution §6 describes.
- **A drift check if anything is mirrored.** If the frontend ever encodes the
  docs convention (which root files, in what order, for icons or grouping), or
  the viewer URL scheme, it gets `app/scripts/docs-check.mjs` slicing the real
  source, like `dnd-check.mjs` does for `store::reconcile` (CLAUDE.md:344-350).
  The intent is that nothing is mirrored: the Rust command returns titles,
  groups and order, and the frontend renders what it is given.
- Mock harness parity for both commands; Playwright against `dev:mock` on a
  port other than 1420, one click per evaluate, `--force` after every edit
  inside `.worktrees/` (CLAUDE.md:166-186).

**Real-app pass, `app/scripts/sandbox.sh --app`** — required, not optional, for
everything the Chrome mock cannot express (CLAUDE.md:150-165):

- the third rail button in WKWebView — a `<button>` sized by flex is the
  documented WebKit trap (CLAUDE.md:295-308); measure, do not eyeball;
- `openUrl` to `http://127.0.0.1:<port>/…` from a `tauri://` page (phase 3) —
  the existing calls are `https:`; loopback `http:` is the same plugin path and
  is expected to behave identically, and "expected" is what the real-app pass
  is for;
- per-place tab memory across a place switch, on real `list_workspace` timing;
- the staleness header on a real 85-behind place, and its refresh on the 30 s
  poll after a commit from a bare terminal;
- `visibilitychange` gating still holds with a browser tab in front of the app
  (CLAUDE.md:525-529).

bats: untouched by phases 1–3; the CLI gains no verb. If a `worktrees docs`
listing is ever wanted, it follows `ls --json`'s `schema_version` template and
gets a bats file then.

---

## 10. Prior art

**In this repo: none.** A sweep of `docs/proposals`, `docs/adr`,
`docs/sessions/*/summary.md`, `ROADMAP.md`, `CHANGELOG.md`, `DESIGN.md` and
`README.md` for docs viewer / preview server / diagram / mermaid / iframe /
second window returns only vite-harness and WKWebView-sandbox hits. Adjacent,
and linked from here on purpose:

- the Files-dock markdown reader, v0.8.0 (`CHANGELOG.md:1033-1043`;
  `2026-08-03-files-viewer` D1 read-only, D2 `marked` lexer only, D3 hand-rolled
  highlighting) — the component this proposal builds beside, not over;
- per-place panel memory (`2026-08-11-space-workbench`) — the Docs tab inherits
  it;
- the permanent right rail (`2026-07-29-right-panel` D1) — the seam is
  permanent, so a third entry is in character;
- ROADMAP: the FSEvents watcher (`490-499`), the markdown viewer gaps
  (`744-750`), the global error boundary (`736-742`), and "smoke the Files tab
  in the real app" (`728-734`) — the last one is still open and the real-app
  pass above would close it as a side effect;
- `DESIGN.md`'s infra strip with "open-localhost links" (`31-32`, `467-468`):
  the only earlier design that ever put a running service with a URL in the UI,
  reversed for its command half by ADR 0001, with `[ports]` surviving as data —
  the same split this proposal makes.

**Outside**, briefly, as data from the consumer's landscape survey: Antora's
`worktrees` playbook key reads linked git worktrees from disk and names each by
its checked-out branch — direct evidence that §1.1 is a known problem with a
known shape (AsciiDoc-only, so the pattern transfers, not the tool). Storybook
Composition is the N-processes version and its tracker documents the failure to
avoid. `mo` (§5.2) is the closest zero-config single binary. `grip` is the
counter-example on §4.3. LikeC4 is what "click a box to descend" looks like
when it is done well. Everything else in the field is closed, SaaS-only,
relicensed or archived, which is an argument for owning the join and not the
parts.

---

## 11. Answers — 2026-09-16, after building phase 1

The five questions the draft left open, the three seams it could not confirm
from source, and two of its own claims that turned out to be wrong. Everything
here was read or run against `29671fc`, not reasoned about.

### 11.1 Is `.planning/brief.md` a docs entry? — **Yes, with the root files**

It is listed, ungrouped, immediately after `CHANGELOG.md`, and carries its own
`.planning/brief.md` as the row's second line so nobody mistakes it for a
committed document.

Three reasons, in the order they decided it. The tab is **per place**, and the
brief is the most place-specific document that can exist — it is what this
worktree is *for*, which is the same question the staleness header answers one
level up. It is **tool-owned**: `ops::BRIEF_PATH` is a constant, so listing it
costs no guessing and no new convention, unlike every other gitignored file.
And its cost when absent is exactly zero.

The objection — that it is gitignored working memory and therefore not
documentation — is real and is answered by the `rel` line rather than by
exclusion. A reader who sees `.planning/brief.md` knows what they are looking
at. Hiding it would be the app declining to show the one file it wrote itself.

It is deliberately grouped with the root files rather than under `.planning`:
a group of one, headed by a dotted directory name, reads as an accident.
(`the_brief_is_grouped_with_the_root_files_not_under_planning`.)

### 11.2 Does `(main)`'s index get a cross-place strip? — **No**

The draft guessed the nav's job; that is right, and for a stronger reason than
division of labour. §2.1 is *"there is no repo-wide docs view in this
proposal"*, and a strip saying "7 places carry an older `docs/`" is precisely
that view. It would also make `(main)`'s Docs tab structurally unlike every
other place's — one tab with two jobs, and the second job growing.

The hazard it was reaching for is already closed by the thing that shipped:
every place's own header names its own staleness, so the comparison happens
where the reading happens. Recorded as a **non-goal**, not a deferral.

### 11.3 Is a URL token a hard requirement? — **No, and the criterion changes**

`mo` cannot satisfy it. The spike enumerated its flags exhaustively — bind,
clear, close, dangerously-allow-remote-access, foreground, json, no-open, open,
port, recursive, restart, shutdown, status, target, unwatch, version, watch —
and there is no token, no auth, no config file and no env var. So "required if
the chosen viewer supports it" resolves to *not supported*, and the question
becomes whether that disqualifies candidate A.

It does not, because the draft named the wrong threat. "Another local process
guessing the port" is a weak threat: a process running as this user can read
`docs/` directly and does not need the viewer. The threat a token actually
blocks is a **web page** — script in any tab the user has open can `fetch`
`http://127.0.0.1:<port>/…` where it cannot touch the filesystem, and these
documents carry a client's signed agreement.

**So phase 3's acceptance criterion is not "has a token". It is: prove the
viewer refuses a cross-origin read.** Either `mo` rejects a foreign `Origin`
and a non-loopback `Host` (measurable: start it, `fetch` its API from a page on
another origin, read what comes back), or the app puts its own loopback proxy
in front that does. Both are testable; neither is phase 1's problem. Until one
is demonstrated, phase 3 does not ship — which is a firmer gate than the draft
had, arrived at by naming the attacker correctly.

### 11.4 `behind` — upstream or base ref? — **Base ref, and the draft was wrong**

`crates/worktrees-core/src/project.rs:379`:

```rust
git rev-list --left-right --count {base_ref}...HEAD
```

with the comment above it stating the reason outright: *"Divergence vs the
repo's BASE branch, not `@{u}`… Upstream told a different story — a branch that
just merged `origin/main` in showed ↑hundreds exactly when the user had brought
it in sync."* `base_ref()` (`project.rs:531-539`) is `origin/<default base>`
when a fetch has brought one, else the local base. `Place::upstream` is
carried, in the same struct, and the code calls it **informational**.

So a header reading `25 behind {upstream}` prints `origin/live-docs` on any
pushed feature branch — naming a ref the number was never measured against. It
would be wrong on most places, always plausible, and never detectably so. That
is §1.1's failure mode reproduced inside the feature built to prevent it, and
it was one paragraph away from shipping.

`base_ref` is not in `Place`, and adding it to `LsJson` would be a schema
change the CLI and bats both assert on. `list_docs` returns it instead — it
already opens the project — and an undiscoverable project yields `""`, in which
case the header says "25 behind" and names nothing, which is honest rather than
wrong.

### 11.5 A Go binary in a Rust release pipeline — **two targets, not six**

Smaller than §5.2 implies, for one reason nobody had stated: **the viewer is
used by the app, never by the CLI.** `release.yml` builds `worktrees` for four
targets (two Darwin, two Linux) and signed app bundles for two (Darwin only).
A bundled viewer belongs in the `app-bundle` job via tauri's `bundle.resources`
— so it is **2 artefacts, both macOS**, and `install.sh`'s "one
checksum-verified binary" story is untouched.

What remains is not the build; Go cross-compiles to both Darwin targets from
one runner without a linker, unlike the `aarch64-linux` case `release.yml`
already carries. What remains is **ownership**: a vendored third-party project
inside a signed bundle, whose CVEs, releases and supply chain become this
repo's, and 25.8 MB on an app binary currently under 20. That is a real cost
and it is a phase-3 decision, but it is a supply-chain question, not a pipeline
one, and it should be argued as that.

### 11.6 The three seams §6 could not confirm

| Question | Answer |
|---|---|
| Can `FilesPane` be told from outside to open a path? (`FilesPane.tsx:822`) | **Yes.** `openPath` is a prop (`FilesPane.tsx:798`) fed by App's own `dockFile` state (`App.tsx:2871`, `:6092`). The pane is fully controlled; a Docs row calls the same `setDockFile` a tree row does. No `open_editor` fallback needed. |
| Does `toggleDock` or ⌘⇧T cycle tabs? (`App.tsx:4856`, `:5060`) | **Neither cycles.** `toggleDock` flips `dock_open` alone and never reads `dock_tab`. ⌘⇧T writes `dock_tab: "terminal"` literally. Both are correct for a third tab with no edit. |
| Does `openUrl` to loopback `http:` behave like the `https:` calls? | **Unverified, and the draft's premise is off.** `FilesPane.tsx:600` already gates on `/^https?:/i`, so `http:` is on the shipped path — it is *loopback* that is untried. `opener:default` covers `allow-open-url` with no scheme scope. Still a phase-3 real-app item; nothing in phase 1 opens a URL. |

### 11.7 What phase 1 shipped, and what the harness caught

`worktrees_core::docs` (the walk, 13 tests), `list_docs` (lib.rs), `DocsPane.tsx`,
a third `DOCK_RAIL` entry, a widened `dock_tab` union, `Icons.BookText`, and
half 5 of `usage-check.mjs`.

Two things are worth recording because no gate would have found them:

- **The `data-track` ternary §6 predicted is worse than §6 said.** It was going
  to misattribute the rail *button*; `usage.ts`'s `surfaceOf` has the same
  two-way shape, so **every click inside the pane** would have recorded as
  `dock.files` too. And `valid_token` (`lib.rs:4013`) is a *shape* allowlist,
  not a vocabulary — `dock.docs` passes it today and so would anything else, so
  §6's "add it to `valid_token`" is a no-op. The only closed vocabulary is the
  TS `Surface` union. `usage-check.mjs` half 5 now reads the tab list out of
  `DOCK_RAIL` and asserts a literal key and a `surfaceOf` branch per tab; it
  fails on the pre-change source, and on each of the four ways it can regress.
- **The mock harness earned its keep.** The walk emitted the leftover root-level
  markdown *after* the `docs/` tree — matching §3.1's list order — which gave
  two separate `group: ""` runs and two React siblings keyed `(root)`. React's
  answer to a duplicate key is to duplicate and omit children, so the filter
  appeared to do nothing while reporting "Nothing matches". Fixed in the walk
  (one contiguous ungrouped run, which is also the better reading order for the
  seven flat-tree places), belted in the pane (groups keyed by path), and
  pinned by `every_group_is_one_contiguous_run`.

One §8 wording correction: phase 2 is `[docs]` **only**. Titles are part of
§3.1's zero-config convention and §7.1's own sketch shows them, so they shipped
in phase 1 — the index is otherwise a file tree with a header, which is gap 2
left open.

### 11.8 Not done

Phase 1 as scoped, and nothing beyond it. Specifically absent, by design:
`[docs]` (§3.2), the viewer and any process (§5), drill-down (§5.3), and a
`worktrees docs` CLI verb. Still owed before phase 1 is called finished: the
**real-app pass** §9 requires — the third rail button measured in WKWebView (a
`<button>` sized by flex is the documented WebKit trap), the header on a real
85-behind place, and per-place tab memory on real `list_workspace` timing.
Everything above was verified against the Chrome mock, which by construction
cannot express any of those three.

---

## 12. Phase 2 as built — 2026-09-16

`[docs]` exists, through `RelPath` and the closed known-key set, exactly as
§3.2 specified. Two decisions §3.2 did not make, both found by building it.

### 12.1 `[docs]` is read from the PLACE, not from the main worktree

This is the only consumer of `projcfg` that does not read `main_root`, and the
divergence is the point rather than an oversight.

`project_prefix`, materialize, `[ports]` and `[compose]` all read main's config
because they describe the **project**: one prefix, one port stride, one compose
file list, whatever branch you happen to be standing on. `[docs]` describes
*content that exists on a branch*, and the whole premise of this tab is that
the content differs per place.

Read main's instead and the feature defeats itself, in precisely the scenario
§1.1 is built from. A restructure lands `[docs] paths = ["handbook"]` on main.
The seven of eleven places that have not rebased do not have `handbook` — so
every one of them shows an **empty index** while its `docs/` sits there
unlisted. A place whose branch predates the key gets the convention, which is
what its tree actually looks like. Both states correct for their branch.

It grants a branch no power it did not have. `Docs` is two `RelPath` fields
with Layer A already applied, the walk re-checks containment (Layer B), and the
place directory was guarded before any of it. The worst a hostile branch can do
is point the listing at a different directory inside its own worktree — and the
app already reads a cloned repo's config for the prefix.

### 12.2 A broken config lists by convention, and says so

`load` returns `Err` on a config that does not parse, and the first shape of
this command propagated it. That is wrong for this surface: §2.7 is *"never a
dead server for the whole place"*, and a blank Docs tab because of a typo three
sections away in `.worktrees.toml` is that failure wearing a different hat.

`list_docs` now carries `config_error`, falls back to the convention, and the
pane renders an amber band above the filter saying so. Silence would be worse
than either alternative: an index quietly showing something other than what the
repo declared is the same class of wrongness as an unlabelled stale tree.

### 12.3 Three smaller things

- **`[docs]` shows in the Project sheet.** Every other section does, and this
  is the one whose *effect* is on another surface entirely — so it is the
  easiest to write wrong and never notice.
- **`paths = []` is an error, not a silent fallback.** "I declared the
  documentation tree and it is nothing" is a typo every time; a repo that wants
  the convention omits the key, which is what an `index`-only config does.
- **Declared order is listing order.** The repo chose it; sorting it would be
  the walk second-guessing the one thing it was told.

### 12.4 What building it found

`[docs] index = "docs/index.md"` named a file the tree pass then walked into,
so the row came out **twice**. The fix is one dedupe rule inside the walk's own
`push` rather than a check at each of the four call sites — a per-site check
would have closed that one route and left the next one open. Caught by a test
written before the feature, which is the only reason it was not shipped.

A styling measurement worth keeping: the config-error band first used a 12%
`--warn` tint of `--bg-panel`, and `--txt-dim` over it measured **2.92:1 in
nord and 3.1:1 in tokyo-day** — amber is a light hue, so tinting a light
surface with it moves the background *toward* the foreground. The amber is now
a border and a glyph; the text is `--txt-hi` on `--bg-elev`, 5.17:1 in the
worst theme. (Latte's `--warn` glyph is 1.93:1 there and 1.93–2.64:1 against
every surface in that theme — a property of latte's amber, shared with every
other `--warn` marker in the app, not of this pairing.)

---

## 13. The §11.3 gate, run — 2026-09-16. **`mo` fails it.**

§11.3 set phase 3's acceptance criterion: *prove the viewer refuses a
cross-origin read.* It was run against `mo` v1.6.8 (darwin_arm64 release
binary, published checksum verified, scratch dir, removed afterwards). Two
halves, and it passes one.

> **Deliberately thin, for now.** The detail behind §13.2 — the walkthrough and
> the reproduction — is held back until a fixed `mo` is released, and was
> reported to its maintainer through GitHub Private Vulnerability Reporting, as
> that project's `SECURITY.md` asks. Restore it here once upstream ships: the
> reasoning is worth keeping in the record, and this proposal is the record.

### 13.1 Cross-origin: **passes**

`mo` sends **no** `Access-Control-Allow-Origin` header, and answers a preflight
`OPTIONS` with `405`. An ordinary cross-origin `fetch` from a web page
therefore cannot read a response. Its SPA shell also ships a real CSP
(`default-src 'self'; connect-src 'self'; frame-ancestors 'none'`) — which,
incidentally, is an independent reason the embedded-iframe idea §8 lists under
**Never** would not have worked anyway.

### 13.2 Host header: **not validated, and that is the whole gate**

`mo` does not check the `Host` header, so a loopback bind does not by itself
establish that a request came from the user's own browser. `mo`'s own README
says the general form of this outright: *"mo has no authentication."*

Being local is the precondition here, not the protection. A loopback bind keeps
other **machines** out; it does not keep out the **browser on this machine**,
which runs code from strangers and can reach `127.0.0.1`. The defence is one
line of server code — the browser always sends the name it believes it is
talking to and cannot be made to lie about it — and `mo` does not have it.

**Bounded, and the bound matters.** Only documents `mo` has been *given* are
reachable. There is **no path traversal**: `/../../../etc/passwd` returns `200`
with the SPA shell at `Content-Length: 1587`, not the file — checked by reading
the body rather than the status code, because the status alone reads like a
breach and is not one. So the exposure is "the documents we registered", not
"every file the user can read". For this feature those are close to the same
sentence, and §4.3's reason for the rule is that these documents carry a
client's signed agreement.

**§5.1 makes it worse, not better.** One `mo` for the whole app with every
place as a group means one success reads *every document in every place at
once*, via `/_/api/groups`. The consolidation that makes the viewer cheap makes
the blast radius total.

### 13.3 §11.3's own remedy does not work here

§11.3 offered an alternative: *"or the app puts its own loopback proxy in front
that does."* It does not close this, and the reason is worth drawing, because a
proxy is the obvious fix and it is the wrong one:

```
   what we would build                what reaches mo anyway

   ┌──────────┐                       ┌──────────┐
   │ our proxy│ :Q  checks Host ✓     │ a page   │ tries loopback ports
   └────┬─────┘                       └────┬─────┘ 6275, 6276, 6277, …
        │ forwards                         │
        ▼                                  │  finds :P, connects directly
   ┌──────────┐ :P  checks nothing ✗  ◀────┘
   │    mo    │                            the proxy is never involved
   └──────────┘
```

`mo` has to hold a **loopback TCP port** of its own — there is no Unix-socket
bind (`--bind` takes an address; a socket path is treated as a non-loopback
address and warned about), and no auth of any kind. So the proxy adds a second
door to a house whose first door does not lock. The only versions that work are
ones where `mo` itself refuses, or where nothing is listening at all.

### 13.4 So phase 3 does not ship on a stock `mo`

Not a deferral over taste: the gate was written before the measurement, the
measurement was taken, and it says no. Everything in §5.3 and §12 survives
whichever way this goes — the derived tree, the generated `click` directives,
the staleness header injected into the documents — because none of it depends
on which process serves the bytes.

**Chosen: patch `mo` and build it from source.** It is Go and MIT; the check is
a small middleware, and it is upstreamable. This is not the "fork in all but
name" §5.2 rejected — that was a proxy injecting scripts keyed on class names
in a 2 MB minified bundle. It does change the supply chain §11.5 priced: a Go
**toolchain** in `release.yml` rather than a downloaded release binary, and we
own the build. See §14.

The two alternatives, recorded because they remain the fallbacks if upstream
declines and maintaining a fork sours: **generate static pages and serve
nothing** (no process, no port, nothing to forge; loses live-reload and
full-text search, which are `mo`'s two real gifts), or **ship it and write the
risk down** (defensible only if the served documents are not sensitive — §4.3
already says ours are).

## 14. The patch — `penard-monkey/mo`, branch `harden/loopback-host-check`

**Committed locally, deliberately not pushed.** The fork is public, so pushing
it publishes a commit describing an unfixed issue in someone else's tool; the
branch stays on disk at `~/workspace/mo` until upstream ships or declines.
Nothing needs it pushed yet — `release.yml` is not wired to it, and if the
patch lands upstream it never will be.

`WithLoopbackHostOnly`, a middleware that refuses a request whose `Host` does
not name the loopback interface, wired in `cmd/root.go` rather than inside
`NewHandler`. That placement is the whole design of the patch: `mo`'s 35
existing handler tests build requests with `httptest.NewRequest`, whose `Host`
defaults to `example.com`, so enforcing inside `NewHandler` would have broken
every one of them — and in `cmd/root.go` the policy sits next to the flag that
governs it.

It is applied **only when the bind address is itself loopback**: a deliberately
exposed server has opted into being reached by name, and
`--dangerously-allow-remote-access` turns it off, which is the escape hatch for
a reverse proxy in front of a loopback bind.

Verified, in this order:

1. the new tests fail against unpatched `mo` (undefined symbols), then pass;
2. `go test ./...` passes across all four of its packages, unchanged;
3. `go vet` and `gofmt` clean (`golangci-lint` is not installed here);
4. **end to end against a built binary** — the request that previously returned
   a document's content returns `403`, while `127.0.0.1`, `localhost` and
   `[::1]` are served exactly as before, and the escape hatch still opts out.

Reported upstream through GitHub Private Vulnerability Reporting with the patch
attached — `GHSA-6pff-wf7m-6f5h`, filed 2026-09-16, state `triage`.
`SECURITY.md` promises a response within 7 days; no disclosure deadline was
set on our side. If it lands, this fork is deleted and `release.yml` pins the fixed
release instead — which is the outcome to want, and the reason the patch was
written to be upstreamable rather than merely to work.

---

## 15. Phase 3 as built — 2026-09-19

The viewer ships. `app/src-tauri/src/viewer.rs` is the whole of it: one
supervised child, N places as named groups, a derived tree per place, and a URL
the frontend hands to `openUrl`. §5.1's shape survived contact; §5.2's "A′"
survived it exactly. What follows is what this document did not decide, what it
got wrong, and what was measured rather than assumed.

### 15.1 The gate is enforced at RUNTIME, not at build time

§13 set the criterion — *prove the viewer refuses a cross-origin read* — and
§13.4 answered it by choosing to patch and build from source. That answers it
for the artefact **we** build. It does not answer it for the binary that is
actually on the user's disk at the moment the button is pressed, and the
distance between those two is every way a file gets replaced: a release built
before the fix landed, a `WORKTREES_VIEWER_BIN` pointed at a Homebrew `mo`, a
bundle someone repaired by hand.

So the app asks. Every spawn, after the port answers and before any URL is
returned, `probe_refuses_foreign_host` sends one request carrying
`Host: worktrees-viewer-probe.invalid` and requires **403**. Anything else —
including `200` — kills the child and fails the open with the reason. It costs
one loopback round-trip on a path that is already polling the port.

This turns §13's *decision* into a *precondition*, and it is the difference
between "we believe we shipped the fixed build" and "this process, now,
refuses". Proved both ways, against real binaries: the patched build passes
end to end; an unpatched `mo` built from the commit before the patch answers
`200` and gets no URL at all.

`release.yml` runs the same gate on the binary it builds, in both directions —
a build that answered `403` to *everything* would pass a one-sided check while
being a viewer that serves nothing.

### 15.2 Where the source comes from is a repository variable

§14 keeps the patch off GitHub until upstream ships or declines, and this scope
asked `release.yml` to ship the viewer. Those pull against each other: a
`.patch` file in this repo is a published description of an unfixed issue in
someone else's tool.

`vars.VIEWER_MO_REPO` / `vars.VIEWER_MO_REF` name the source, defaulting to
upstream's own repo at a pinned tag. **With the defaults, the release fails** —
the gate refuses the binary it just built. That is the intended behaviour, not
an oversight: a viewer that cannot prove it refuses a foreign `Host` must not
reach a user's machine, and a release that quietly shipped one would look
exactly like a release that did not. Point the variables at a build that carries
the check — or, when upstream ships, at that tag, and delete nothing else.

### 15.3 Four decisions §5 left open

**The tree lives in the app's config dir and never outlives the process.**
`<config>/viewer/tree/<slug>-<hash8 of the canonical root>/`, mirroring the
place's own layout — flattening it would break every relative prose link, which
`mo` resolves against the file's own directory. It is emptied on a clean exit
*and* on the first spawn of a run, because a crash cannot honour a shutdown
hook. These are copies of documents §4.3 describes as carrying a client's signed
agreement, sitting outside the repo's gitignore in a directory Spotlight and
Time Machine both index; they do not get to persist for a viewer that is, by
design, dead.

**A removed place's derived documents go with it.** `remove_place` deletes the
tree, resolving the canonical root *before* the removal because it cannot be
resolved after. `cmd_rm` deletes the originals, so without this the derived copy
would be the last readable one — and it would be on a port. Removing the
directory is a complete deregistration: `mo` watches it and drops what leaves,
verified at two seconds to zero files and a 404 from the content endpoint.

**Generated per open, registered once per place.** `mo -wR <tree>` registers the
directory (one argument rather than up to 2,000 — the index's cap — which keeps
this off `ARG_MAX` entirely) and watches it, so regenerating the tree
live-reloads a browser tab that is already open. Re-registration is idempotent:
verified, the group's file count does not change.

**The group name is the slug, reduced to `[a-z0-9-]`, disambiguated by a hash of
the canonical root.** Two places may legitimately share a slug — the same branch
name in two projects — and `mo` *merges* same-named groups rather than refusing
them, so a collision would show one project's documents under the other's name.
That is §1.1's failure with the axes swapped.

**The staleness facts come from the frontend, the base ref does not.** Every
field but one is in the `Place` the dock is rendering from at the moment the
button is pressed; re-deriving them in the backend would be a second git fan-out
per click *for numbers that would then be allowed to disagree with the ones on
screen two inches away*. `base` stays the backend's `Project::base_ref()`,
because it is the one fact `Place` does not carry and the one §11.4 says is
wrong when guessed.

### 15.4 Two things this document said that the build found wrong

**§8's "phase 3 does not ship on a stock `mo`" is weaker than it reads.** It is
written as a shipping decision, which is a thing a future release can quietly
forget. It is now a runtime assertion and a CI assertion, and neither can be
forgotten by omission — they have to be actively removed.

**"A missing viewer must not break the build" was assumed and is false.**
Tauri's `bundle.resources` fails the whole build on a glob that matches nothing
(`glob pattern viewer/* path not found or didn't match any files`) exactly as
the map form fails on a missing file — so a local build with no viewer, which is
every local build, would have been broken in order to ship a binary local builds
do not have. A committed `viewer/README` is what makes the glob always match; it
is load-bearing and says so in its own text. Measured, not reasoned about, and
the same call found that `tauri.conf.json` is `deny_unknown_fields`, so the
explanation could not live beside the key as a `//` comment.

### 15.5 Rule 8's "doctor finding" went to `diagnostics` instead

A missing viewer is reported by `diagnostics` (Settings → Logs), by **stat**,
beside `git` and `tmux` — on demand, off every hot path, and never a banner.
Not by `doctor`: `doctor` is `ops::cmd_doctor` in core, it is per **project**,
and it is shared with the CLI, which has no viewer and never will. A repo-health
check reporting on an app bundle's helper binary would be the wrong tool
answering the wrong question on every place. The automatic half is `applog`: the
first failed open writes the reason to the app log, which is what "logging
background failures rather than bannering them" asks for.

### 15.6 What is still owed

The **real-app pass**. Everything above was verified against unit tests, the
Chrome mock and a real `mo` driven from Rust; none of that is WKWebView. Two
things need `app/scripts/sandbox.sh --app` and a human (the third was closed by
measurement and is struck through below). Stage the viewer for that run with
`cp ~/workspace/mo/mo app/src-tauri/viewer/mo && codesign --force --sign -
app/src-tauri/viewer/mo`, or point `WORKTREES_VIEWER_BIN` at it — the binary is
gitignored and `release.yml` is what normally puts it there:

- `openUrl` to `http://127.0.0.1:<port>/…` from a `tauri://` page. §11.6 left
  this open and it is still open. What has been closed since is the permission
  question: `opener:default` carries `allow-default-urls`, whose scope is a
  `glob::Pattern` of `http://*`, and `Pattern::matches` uses
  `MatchOptions::new()` — `require_literal_separator: false` — so the pattern
  matches the whole URL including path and query. No capability change is
  needed. Whether WKWebView's `open` call behaves is still unmeasured.
- ~~The footer button in WebKit.~~ **Measured, and the trap does not apply
  here.** Headless Playwright WebKit against the mock, beside Chromium: the
  button is 198px, the label 163px, the icon 13px and hit-testable, *identically
  in both engines*, at 1280px and at `DOCK_MIN`. Forcing the dock to 149px —
  narrower than the app allows — and removing the label's `flex: none` shrinks
  it to 103px and the icon to 8px, again identically. So the pin is load-bearing
  (it changes the layout) and the ENGINES AGREE, because the usage meter's bug
  needs a button that is being asked to shrink, and `.docs-foot`'s
  `align-items: flex-start` means this one never is. Worth stating as a
  measurement rather than deleting the rule: the next control put in that footer
  may well compete for width.
- The spawn latency as felt. 0.23 s to listen was measured with 7 files and the
  tree is registered afterwards, so it should not grow with the place — but
  "should not" is what a real-app pass is for.

Phase 4 (drill-down beyond node→page) is untouched. The one mapping source §5.3
lists first — a node id that names exactly one page — is what `derive::targets`
implements, and the other three arrive at that same seam.

---

## 16. Images — found by using it, 2026-09-19

**Every image in every document was broken in the browser viewer**, and the
Docs tab's own dock renders the same images fine. `write_tree` wrote rendered
markdown and nothing else; `docs::index_with` lists markdown *by design*
(`is_md`), so an image was never in the entry list and nothing copied it. A
place deriving 14 documents shipped a tree with no `assets/` directory in it at
all. For any illustrated document the viewer was therefore strictly **worse**
than the surface it exists to improve on, and better only for mermaid — which
undercuts the reason it exists.

**What was built.** The referenced images are copied into the derived tree,
**mirroring each reference's own relative path**, so the author's
`![x](images/y.png)` resolves with the document left byte-identical. No URL
rewriting: the derived text stays a copy of the file it claims to show, which
is the one thing this surface cannot trade away.

The security treatment is §4.2's, in two layers, because a reference is a string
from a repository the user may have cloned seconds ago and we are turning it
into a filesystem read:

- **Layer A** (`derive::asset_rel`, pure, no disk) refuses absolute paths, `~`,
  `$`, every URL scheme (`http:`, `data:`, `file:`, …, matched by shape rather
  than by a list that goes out of date), empty components, a `.git` component,
  and any `..` that escapes the place. Fragment, query and markdown title are
  stripped; percent escapes are deliberately **not** decoded, so `%2e%2e` can
  never become `..` after the check.
- **Layer B** (`viewer::copy_assets`) canonicalises the candidate and requires
  it to start with the canonical place root. This is the check that actually
  holds: Layer A is arithmetic on a string, and a string cannot show a symlink —
  with `docs/assets` a link to somewhere else, `../assets/x.png` is textually
  innocent and its final component really is a regular file.
  `symlink_metadata`, never `metadata`, for the same
  `docs/logo.png -> ~/.ssh/id_rsa` that `docs.rs` already refuses.
- An **allow-list** of raster extensions, per-file and per-place byte caps, and
  a cap on how many references one place may turn into a stat. A refusal is per
  file and never fatal (§2.7) and is reported through `applog`, not swallowed.

### 16.1 SVG is not copied, and adding it is not the fix

An SVG is a live document — script, `foreignObject`, external references. Inside
an `<img>` it is script-inert by spec, which is what makes "it's just an image"
sound true; but `mo` hands any asset back by direct URL
(`/_/api/groups/{g}/files/{id}/raw/{path}` → `http.ServeFile`), where it is a
top-level document served as `image/svg+xml` and script runs. We do not control
that content type and the port has no authentication. So a place's `logo.svg`
stays broken on purpose, the reasoning lives beside the allow-list, and a test
fails if someone adds three letters to it. The way to fix it is a viewer that
serves assets with a content type we chose.

### 16.2 The fingerprint's blind spot, and how it was closed

`docs::fingerprint_with` is stat-only over markdown — which is what makes it
cheap enough to run on the tick — so an **edited screenshot moved nothing** and
the tab kept serving the copy made at click time. Making the digest parse
documents is exactly the cost it exists to avoid. Instead the tick folds in a
stat of the images the *last* derive already knew about (`docs::fold_assets`),
whose paths are held on the `Group`. That catches an image edited, replaced,
deleted, or missing-and-now-arrived (the candidate list deliberately includes
what could not be copied). A brand-new reference is caught by the other half:
it means a document changed, which moves the walk's digest in the same tick.
Measured on this repo's own place, 80 documents and 6 images: `fold_assets`
costs **13–16 µs** against the walk's **2.6 ms**.

### 16.3 Two things the viewer cannot currently do, measured not guessed

- **A `../` reference does not resolve in `mo`, however correctly it is
  copied.** `resolveImageSrc` appends the author's `src` to
  `/_/api/groups/{g}/files/{id}/raw/` verbatim, so `../assets/y.png` builds a
  URL carrying a dot segment — and both a browser and Go's own mux normalise it
  away, eating the `/raw/` segment. Probed against the real binary: the literal
  form answers `307` to `…/files/{id}/assets/y.png`, which is not a route and
  serves the SPA shell. Same-directory and subdirectory references
  (`shots/a.png`, `docs/media/x.png`) return the real bytes with
  `Content-Type: image/png`, which is asserted end-to-end in
  `an_image_loads_through_a_real_viewer`. The copy is correct for the `..` case
  too — the file lands exactly where the reference points — so the remaining fix
  is upstream, in the viewer's URL construction, and it is not a one-liner: a
  path relative to the document's directory can never carry `..` through that
  route, so the raw endpoint has to be keyed on something else.
- **Raw-HTML `<img>` is a real reference here**, not an exotic one: `mo`
  renders raw HTML through `rehype-raw` + `rehype-sanitize`, whose default
  schema keeps `img`, and this repo's own README centres three screenshots that
  way. The scanner reads a quoted `src`; an unquoted one, an entity-escaped one
  and `srcset` are left alone rather than guessed at.
