# Orfis feedback — integration status

**The integration is complete and still gated.** Settings → Data & Logs shows
nothing in a build without configuration, and a build *with* configuration
still refuses to initialize — not because of the SDK any more, but because the
last two items are not ours alone to settle. See
[What is still gating it](#what-is-still-gating-it).

Scope of this first cut, from the upstream handoff
(`orfis/docs/integrations/worktrees.md`, 2026-10-05): a Send-feedback action in
Settings → Data & Logs, a locally bundled pinned widget, one main-window
adapter started at boot, the real Rust app version and a `macos-native`
surface. No diagnostic capture and no attached native logs.

## What a report may contain

```json
{
  "key": "pk_worktrees_<publishable-key>",
  "surface": "macos-native",
  "type": "bug",
  "message": "<the user's text>",
  "appVersion": "<version from the Rust binary>"
}
```

Nothing else *in the body*. Not logs, terminal contents, prompts, source,
diffs, local metrics, repository or project paths, usernames, remotes, or any
generated installation identifier.

**The stored row has more than the body**, and the Settings copy says so:
Orfis derives browser and OS name and version from the webview's `User-Agent`
on every row. WKWebView typically reports a frozen `Mac OS X 10_15_7`, so the
stored OS version may not be the user's. Confirm against a real stored row at
release verification. The client IP keys the rate-limit counter and is never
written to the row. The Settings copy says so in the same words, because a
disclosure that drifts from the payload is worse than none.

## What is implemented and guarded

| Concern | Where | Guarded by |
|---|---|---|
| Routing comes from the RELEASE, never a repo or user setting | `feedbackConfig.ts` | `feedback-check.mjs` — refuses non-HTTPS off loopback, credentials, a path, a query, a fragment, and `javascript:` |
| One widget, main window only, started once outside StrictMode | `feedback.ts`, `main.tsx` | concurrent `startFeedback()` must share one promise; a non-`main` window must create nothing |
| Teardown beats a slow start | `feedback.ts` | a `stopFeedback()` during startup must leave no widget, even after the import resolves |
| The mock never reaches the SDK | `mock/feedback.ts` | the mock path must not even *import* `feedbackWidget` |
| Escape, focus return, background inerting | `feedback.ts`, `useEscape.ts` | `registerEscape` puts the shadow-root dialog on the app's ONE Escape stack, LIFO with Settings |
| Queued ≠ accepted | `feedback.ts` | the two notices are distinct strings and `feedback-check.mjs` asserts both |
| A failed start says so and leaks nothing | `feedback.ts` | the log line carries no report text, endpoint, key or native error detail |

`app/scripts/feedback-check.mjs` runs the real TypeScript, not a paraphrase.
Each invariant above was confirmed by breaking it and watching the check go red.

## What is still gating it

`feedbackWidget.ts` carries one constant, `ACCEPTANCE_PENDING`, and everything
around it is the real integration: `init` with all three privacy opt-outs
passed explicitly, `onOpen`/`onClose` on the app's Escape stack, `setTheme`
driven by a `[data-theme]` observer, and the truthful queue result wired to the
notice. `feedback-check.mjs` asserts the constant is still `true`, so opening
the gate is a deliberate act that has to update the test too.

**The three SDK defects are fixed.** Orfis `3222a74` resolved all of them —
ambient metadata surviving `captureDiagnostics: false`, any `res.ok` counting
as accepted, and a shed queue reporting success. Its suite passes 80/80, and
the acceptance-critical tests were run by name here (202 handling, pasted-secret
scrubbing, metadata opt-out, quota). The revision also scrubs secrets from the
message on the device. `docs/orfis-sdk-requests.md` is the original request and
is now answered.

Two things remain, neither of which code can settle:

1. **No product key.** Orfis mints the key against the `Origin` a packaged
   build sends, and we have not reported it. Production is live at
   `https://orfis.otterly.digital`; the dev key is minted against
   `http://localhost:1420`. **Capturing the packaged Origin is the next
   action, and it unblocks everything else.**
2. **No license at the pinned revision.** `packages/sdk-web` is `"private": true`
   with no license file and no license field, so the bundled artifact may not
   be redistributed in an enabled build until the owner confirms rights.

Once both land: run the packaged acceptance list below, then set
`ACCEPTANCE_PENDING = false`.

## Bundle provenance

See [`app/src/vendor/orfis/README.md`](../app/src/vendor/orfis/README.md): the
pinned revision, the artifact's SHA-256, and `app/scripts/vendor-orfis.mjs`,
which rebuilds it from a clean `git archive` in a temporary directory and
refuses to write a bundle whose hash differs from the reviewed one. Builds and
runtime need no Orfis checkout, registry package or remote script; the checkout
path is a regeneration input only.

⚠ **No license file exists at that private revision.** Redistribution rights
have to be confirmed by the owner before an enabled integration is distributed.

## Unresolved before this can ship

None of these are Worktrees-side code, and none should be invented locally:

1. The `Origin` a packaged build sends — **owed by us to Orfis**, and the
   gating input for the two keys.
2. Redistribution rights for the bundled artifact (no license at `3222a74`).
3. Acceptance of a first release without attached logs, and sign-off on the
   disclosure wording and Settings placement.
4. Ownership of rebuilding the pinned artifact for later releases.

## Acceptance still owed once those land

Everything below needs a real key and a packaged build, so none of it can be
claimed from this branch:

- A report arrives under Worktrees in the Orfis admin, carrying `macos-native`
  and the real binary version; the payload inspected to confirm nothing else.
- A queued report survives quit and relaunch and flushes without reopening
  Settings, with correct Written/Received timing.
- Duplicate submission succeeds; `429` waits; offline retries; full or
  unavailable storage never produces a false acceptance notice.
- A packaged macOS build passes the origin check — **capture the observed
  Origin as the evidence**.
- The dialog over Settings: keyboard navigation, Escape, focus return, light
  and dark themes, at the app's minimum window size.

This branch establishes none of that, and the code says so rather than
pretending otherwise: with no configuration the action is hidden, and with
configuration the adapter still refuses.
