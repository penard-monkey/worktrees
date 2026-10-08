# Orfis SDK handoff for Worktrees feedback integration

Prepared 2026-10-05 from the Worktrees integration branch. The Orfis source
review was based on revision `1614c67f9780f65cd1bcaeb9ff2e2b3cdb084e61`.

Worktrees is adding a feedback form to the macOS Tauri app under Settings →
Data & Logs. The first release must send only the user's message and the
contract fields below. It must not attach logs, terminal content, prompts,
source files, diffs, local metrics, repository/project paths, usernames,
remotes, or generated installation identifiers.

```json
{
  "key": "pk_worktrees_<publishable-key>",
  "surface": "macos-native",
  "type": "bug",
  "message": "<normalized user text>",
  "appVersion": "<Rust app version>"
}
```

## Reproduced SDK defects

All three failures were reproduced offline against a byte-identical rebuilt
SDK bundle. No real reports were sent.

### 1. Diagnostics disabled still sends path metadata

With `captureDiagnostics: false`, `collectDeviceContext` still included a page
path and referrer host. The reproduced payload contained:

```text
/private-project/document.md
```

This violates the Worktrees first-release data boundary. Add a supported public
opt-out/override, for example `device: {}` or
`collectDeviceContext: false`, with documented precedence relative to
`captureDiagnostics`. The opt-out must remove ambient pathname, referrer,
locale, timezone, viewport, and browser-context fields from the serialized
payload. Do not require Worktrees to access private Shadow DOM or SDK internals.

Required regression test: initialize with diagnostics and device metadata
disabled, submit a report, and assert that the serialized request contains
only the contract fields and user message.

### 2. Any `res.ok` response is treated as accepted

`outbox.post` currently treats every successful HTTP response as submission
success. A mocked `204` response with an empty body, and a successful response
with malformed JSON, both triggered the accepted callback.

The authoritative contract is stricter: only `202` with a valid `accepted` or
`duplicate` response means success. Reject `204`, malformed JSON, wrong status,
and invalid response bodies according to the existing retry/terminal policy.

Required regression tests:

- `202 accepted` fires the accepted callback and removes the item.
- `202 duplicate` fires the accepted callback and removes the item.
- `204` does not fire accepted and follows retry/error handling.
- Malformed JSON does not fire accepted and follows retry/error handling.
- Terminal `4xx` remains terminal and preserves editable text as appropriate.

### 3. Queue success can be reported when the item was not retained

Under quota/storage pressure, `outbox.write` can shed the newly queued report,
leave storage as `[]`, return success, and trigger the queued callback. This
was reproduced with a storage implementation that accepted the write but did
not retain the item.

Enqueue must return a typed result or equivalent that proves whether the report
was retained, for example:

```ts
{
  stored: boolean,
  durable: boolean,
  reason?: "quota" | "unavailable" | "memory-fallback"
}
```

The widget must never say a report is queued when it is absent. It should say
it is saved for retry only when durable storage confirms retention. For a
memory-only fallback, use bounded wording that explains the report may be lost
if the app closes. Test quota failure, unavailable `localStorage`, memory
fallback, and persistence across quit/relaunch.

## Host lifecycle integration

Worktrees also needs a supported public lifecycle and host integration contract:

- idempotent `onOpen` and `onClose` (or equivalent instance hooks);
- cleanup/destroy without duplicate retry timers or widget instances;
- documented focus return and Escape ownership;
- supported light/dark theme configuration or styling hooks.

Worktrees must not depend on private `element`, `shadowRoot`, or other internal
widget fields to provide these behaviors.

## Acceptance evidence requested from Orfis

Please provide the SDK branch or commit, public API names, and test output for:

1. metadata opt-out payload inspection;
2. valid `202 accepted` and `202 duplicate` handling;
3. rejection of `204` and malformed responses;
4. truthful durable, memory-only, quota-failure, and unavailable-storage queue
   results;
5. idempotent lifecycle/open/close and host keyboard/theme integration.

Worktrees will then pin and vendor the SDK revision, rebuild its bundle, and
rerun mock plus packaged-webview checks. Production provisioning (publishable
key, HTTPS endpoint, and observed packaged Origin allow-list) remains a
separate release dependency.

Until this SDK revision exists, Worktrees should keep the real adapter
fail-closed and use a deterministic offline mock. This handoff does not request
production provisioning or any real report submission.
