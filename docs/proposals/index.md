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
| [Project automations](automations.html) | a brief on a schedule, run across a project's worktrees — a fourth dock tab, a per-machine run ledger, proposals applied with one click |
| [Codex support](codex-support.html) | Codex launch, resume, MCP setup, and app behavior |
| [pi as a third harness](pi-harness.html) | pi (pi.dev) beside Claude and Codex, with harness × model as a first-class choice — launch, activity, bus, availability, node policy |
| [opencode as a harness](opencode-harness.html) | opencode beside Claude and Codex: directory-keyed resume, its in-process API for send and activity, and a gate on repo-supplied plugins (shares pi's harness × model shape) |
| [Agent guidance](agent-guidance.html) | teaching every agent — Claude, Codex, pi; orchestrator and lane — to do its own branch work in a place: which channel each harness honours, the text, per-launch delivery, guards, and an eval |
| [Cross-project reach](cross-project.html) | letting an agent see, message and (opt-in) act on places in other projects — a core-owned project registry, `<project>:<slug>` addressing, routed messages, and drag and drop of a foreign place |
| [Navigation history](nav-history.html) | back / forward (⌘[ ⌘], ⌘← ⌘→ outside text fields) through what the two panes showed — what a location is, what is noise, coalescing, gone targets |
| [Context usage](context-usage.html) | how full each agent session's context window is — per-harness numerator and window, compaction, a threshold-gated nav mark and MCP `place_status` |
| [Pull requests](pull-requests.html) | a project's open PRs and each lane's own — one `gh api graphql` call per project, a header chip on the lane, a sixth dock tab, and a cached `pr` on MCP `place_status` later |
| [Nested lanes](nested-lanes.html) | hubs with child lanes — declared parentage, scoped orchestration, target-aware close-out, tree navigation, and per-project tmux servers first |
| [Owned planning](owned-planning.html) | worktrees owns the planning layout, resolution and agent hooks, opt-in globally or per project — an after-update offer, a `[plan] project` key for tracked goals, no stale-directory guessing, one resolver for the Plan tab and every hook |
