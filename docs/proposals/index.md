---
title: Proposals
---

# Proposals

Designs written up before they were built. They are kept after shipping because
the reasoning is worth more than the diff — but they are snapshots, not
maintained documentation. Where a proposal and the code disagree, the code wins.

| Document | Proposal |
|---|---|
| [Project settings](project-settings.html) | per-project configuration (`.worktrees.toml`) |
| [Per-place docs](place-docs.html) | a Docs tab per place — index, staleness header, and a tool-owned browser viewer for long reads and diagrams |
| [Project automations](automations.html) | **built in v0.29.0, removed in v0.41.0 (2026-10-10: not useful enough, confusing); kept for a later revisit.** A brief on a schedule, run across a project's worktrees — a fourth dock tab, a per-machine run ledger, proposals applied with one click |
| [Codex support](codex-support.html) | Codex launch, resume, MCP setup, and app behavior |
| [pi as a third harness](pi-harness.html) | pi (pi.dev) beside Claude and Codex, with harness × model as a first-class choice — launch, activity, bus, availability, node policy |
| [opencode as a harness](opencode-harness.html) | opencode beside Claude and Codex: directory-keyed resume, its in-process API for send and activity, and a gate on repo-supplied plugins (shares pi's harness × model shape) |
| [Agent guidance](agent-guidance.html) | teaching every agent — Claude, Codex, pi; orchestrator and lane — to do its own branch work in a place: which channel each harness honours, the text, per-launch delivery, guards, and an eval |
| [Cross-project reach](cross-project.html) | letting an agent see, message and (opt-in) act on places in other projects — a core-owned project registry, `<project>:<slug>` addressing, routed messages, and drag and drop of a foreign place |
| [Navigation history](nav-history.html) | back / forward (⌘[ ⌘], ⌘← ⌘→ outside text fields) through what the two panes showed — what a location is, what is noise, coalescing, gone targets |
| [Context usage](context-usage.html) | how full each agent session's context window is — per-harness numerator and window, compaction, a threshold-gated nav mark and MCP `place_status` |
| [Pull requests](pull-requests.html) | a project's open PRs and each lane's own — one `gh api graphql` call per project, a header chip on the lane, a sixth dock tab, and a cached `pr` on MCP `place_status` later |
| [Nested lanes](nested-lanes.html) | hubs with child lanes — declared parentage, scoped orchestration, target-aware close-out, tree navigation, and per-project tmux servers first |
| [Server Edition](server-edition.html) | **Phases 0 and 1 built in v0.43.0 (2026-10-21).** One machine runs the engine and any of the user's devices drives it — the `worktrees-api` extraction, `worktrees serve`, one engine process per host, the loopback and remote rule sets, enrollment, and the iOS client |
| [Server Edition, Phase 2](server-edition-phase2.html) | **Designed 2026-10-20, being built; status at 2026-10-23:** slices 0–3, 1b, 2b, 6a and slice 5's server side on main, slices 4 and 6 in open PRs. The gate before serve leaves loopback: QR enrollment over pinned mutual TLS, per-device capabilities and project scopes declared in the dispatch table, the remote listener (overlay or LAN, with mDNS rediscovery) and its identity witness, the audit log, limits, the agent-instruction surface, and the known gaps |
| [iOS MVP](ios-mvp.html) | **Plan and handoff, 2026-10-22, updated 2026-10-23** (its own repository, Phase 0 installed from Xcode). A phone that pairs by QR with the user's own host over Tailscale or the LAN and drives sessions — the dots, a place's detail, answering a prompt, sending a line, starting and closing a session — with no terminal and no files: the stack (Tauri 2 iOS), every feature mapped to its dispatch row and capability, the four server rows still missing, the hardware spike as Phase 0, and a handoff for a fresh session |
| [Chat session view](chat-session-view.html) | **Exploration, 2026-10-30; a prototype behind a Settings switch, not merged.** A Chat \| Terminal switch on each agent cell: the iOS View session on serve's four agent rows, live from `sessions:busy` plus a busy-only poll, the terminal one click away, and the server asks it would like |
| [Owned planning](owned-planning.html) | worktrees owns the planning layout, resolution and agent hooks, opt-in globally or per project — an after-update offer, a `[plan] project` key for tracked goals, no stale-directory guessing, one resolver for the Plan tab and every hook |
| [Settings reorganisation](settings-reorg.html) | **Plan, 2026-11-04; nothing built.** Every setting inventoried with its home and scope, six groups and 17 pages replacing 19 flat categories, Beginner / Advanced / Expert modes that hide but never disable, Cautious / Balanced / Autonomous agent-policy presets, new options ranked by worth, and a phased plan |
| [Ideas](idea-board.html) | **Exploration, version 2, 2026-11-06; superseded as a direction by the phase plan on 2026-11-08.** Thinking before a place exists, revised after independent reviews: a no-code rehearsal with a failure table first, then at most a bridge beside the owner's notes — a reviewed brief, a durable commission record separate from any live link, and a deliberate result review where merged is not answered. Product invariants apart from candidate commitments, a freshness contract for reviewed memory, the research runtime as an explicit gate, and Jellyfin workflows where sources disagree, direction changes and a result invalidates a decision |
| [Ideas: the phase plan](idea-board-phases.html) | **Plan, revision 2, 2026-11-08; nothing built.** The owner's decision to build ideas in phases, desktop only: a companion window, markdown documents in a CodeMirror 6 editor with Bear as the north star, named workspaces of registered projects in the main window, ideas that relate, merge and split with nothing lost, then ideas → plan → workspace → places with a reviewed brief, drag and drop throughout, and agents that propose while you act — each phase with a real-app check and a size |
