---
title: "Session: Claude plan-usage widget → v0.7.0"
---

# Session: Claude plan-usage widget → v0.7.0

- **Date:** 2026-08-01 → 2026-08-02
- **Worktree:** ui-changes
- **Branches:** ui-next (feature), close-out-usage-widget (this archive)
- **PRs:** [#74](https://github.com/penard-monkey/worktrees/pull/74) (squash-merged → `30e5088`)
- **Release:** v0.7.0 (tag pushed, release.yml green after one rerun)
- **Planning files:** planning.tar.gz alongside this summary (task_plan / findings / progress)

## What shipped

Bottom-left nav widget mirroring Claude Code's `/usage` panel — 5h session,
7d all-models, and any model-scoped weekly bucket ("Fable"), severity-colored.

- `app/src-tauri/src/lib.rs` — async `claude_usage` command: Keychain token
  (`security find-generic-password -s "Claude Code-credentials" -w` →
  `.claudeAiOauth.accessToken`) → curl GET `https://api.anthropic.com/api/oauth/usage`
  with `anthropic-beta: oauth-2025-04-20` + `User-Agent: claude-code/<ver>`
  (UA is load-bearing — foreign UA hits an aggressive 429 bucket). Parses the
  `limits[]` array only; 120s in-process cache; one 401 retry with re-read
  token; falls back to `~/.claude/widgets/rate_limits.json` (statusline
  snapshot, mtime as freshness), then `source:"unavailable"` (widget hidden).
  Hand-rolled `parse_iso8601` (days_from_civil) to avoid a chrono dep.
- `app/src/App.tsx` — module-scope `UsageWidget`, rendered between
  `.nav-scroll` and the Add-project footer; polls 180s + window focus.
- `app/src/App.css` — `.usage` block: 3px hairline bars, severity tiers
  (accent / --warn / --danger), `.stale` dim for the statusline source.
- `app/src/mock/install.ts` — `claude_usage` mock + `?usage=stale` /
  `?usage=off` harness switches.
- `CHANGELOG.md` 0.7.0 section + workspace `Cargo.toml` bump.

## Decisions

- **Data source: the undocumented OAuth usage endpoint**, not JSONL
  accounting (ccusage-style). Only the endpoint gives official percentages,
  the model-scoped bucket, severity, and reset times; token-count estimates
  can't reproduce Anthropic's limit math. Verified live before building.
- **The Fable bar lives in `limits[]` with `scope.model.display_name`,**
  NOT in the legacy `seven_day_opus` field (null on this account). `limits[]`
  treated as the authoritative shape; everything else in the payload ignored
  (response is full of experimental null buckets — `tangelo`, `nimbus_quill`…).
- **curl + `security` shell-outs, no new crates** — house style (git/tmux are
  shelled out too), keeps the TLS stack and dep tree unchanged; app already
  shells curl for the release check.
- **Missing data is never an Err** — ambient widget; a banner for a
  background poll would be worse than a blank corner. Reason goes to app.log.
- **Nav-only, no rail affordance** — rides out ⌘B inside the hidden nav,
  keeps polling (1 call/180s, backend TTL 120s floor).
- **Opus implemented from a detailed brief; fable reviewed the diff** (per
  the standing cost split). Review checked the one real risk: `onError` in
  the effect deps — `fail` is useCallback-stable, so no re-render churn.

## Dead ends / gotchas

- **Statusline `rate_limits` has no model bucket.** The pre-existing
  "Token Usage for Claude" statusline hook (`~/.claude/widgets/
  save_rate_limits.sh`) captures 5h+7d only — good fallback, insufficient
  primary. No `claude usage` CLI command exists; /usage is TUI-only.
- **Remote `ui-next` was a stale leftover** from PR #56 (squash-merged
  28 Jul): first push rejected non-fast-forward, and `gh pr create` happily
  opened #74 against the STALE head. Verified #56 merged, force-pushed with
  lease; PR then showed the right commit. Lesson: check `gh pr list --head`
  before reusing a branch name.
- **release.yml failed once** — `actions/upload-artifact` `ETIMEDOUT` on the
  aarch64 CLI job (GitHub-side blip; build itself fine). `gh run rerun
  --failed` fixed it.
- **Endpoint risks accepted:** unversioned/undocumented, has broken before
  (429 incident when UA missing), reset timestamps reportedly drift from
  observed behavior. Mitigations: severity/labels come from the server, parser
  skips unknown entries, statusline fallback, widget hides on total failure.

## Verification

- Live curl against the real endpoint with the real Keychain token (research
  phase): session 47% / weekly 62% / Fable 80% warning — matches /usage panel.
- All gates ×2 (post-implement, post-version-bump): release CLI build,
  241 bats, shellcheck+bash-3.2, 137 core tests, tsc, cargo check -p app.
- `parse_iso8601` cross-checked against Python `datetime.fromisoformat` on
  6 edge cases (epoch 0, Z, +00:00, fraction, -05:00, leap day).
- Mock harness in Playwright: bars render bottom-left, Fable amber 80%, no
  console errors (screenshot: usage-widget.png, cache dir).
- Release assets verified: 4 CLI binaries, 2 signed app bundles + .sig,
  latest.json, checksums.txt.

## Follow-ups

- **Manual smoke test pending:** launch v0.7.0, approve the one-time
  Keychain prompt, confirm live bars. (Not doable headless.)
- Usage-credits ("extra usage") display — endpoint's `spend` object; skipped
  because disabled on this account.
- Multi-harness usage rows (codex etc.) when the app grows beyond Claude.
- Endpoint schema drift watch: if `limits[]` vanishes, widget silently falls
  back to statusline — worth an applog-based canary if it ever happens.
