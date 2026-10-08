# Orfis feedback — integration status

**The integration is live.** Settings → Data & Logs shows nothing in a build
without configuration; a configured build initializes the real SDK and the
form can be opened from the nav rail or from Settings. Every external
dependency landed first — the three SDK defects, the MIT license, and both
product keys — and the acceptance gate that held it shut has been removed.
What now holds it shut in an unconfigured build is configuration itself:
`feedbackConfig` refuses anything that is not a well-formed `pk_` key on an
https (or loopback-in-dev) origin, and the adapter is never reached without
one.

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
  "appVersion": "<version from the Rust binary>",
  "email": "<only if the user typed one>"
}
```

`email` is optional and user-entered — the field is shown, nothing is sent
unless it is filled in. Nothing else *in the body*. Not logs, terminal contents, prompts, source,
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
| Escape, focus return, background inerting | `feedback.ts`, `useEscape.ts` | `registerEscape` puts the shadow-root dialog on the app's ONE Escape stack, LIFO with Settings; `opened()` is driven over a fake body so the dialog's host stays interactive while the app goes inert, and the adapter's choice of host is asserted separately |
| Queued ≠ accepted | `feedback.ts` | the two notices are distinct strings and `feedback-check.mjs` asserts both |
| A failed start says so and leaks nothing | `feedback.ts` | the log line carries no report text, endpoint, key or native error detail |

`app/scripts/feedback-check.mjs` runs the real TypeScript, not a paraphrase.
Each invariant above was confirmed by breaking it and watching the check go red.

## How it was unblocked

`feedbackWidget.ts` used to carry a constant, `ACCEPTANCE_PENDING`, that made
`createFeedbackWidget` throw. It is gone rather than set to `false`: a dead
flag reads like a switch someone might flip back, with nothing guarding it.
`feedback-check.mjs` asserts it is absent.

The main path was exercised by hand against a live endpoint before it was
removed — the form opened from both triggers, a report composed and sent, and
the dialog's own queue/acceptance states observed. The broader list below was
NOT walked end to end; it is still the right list for release verification.

**The three SDK defects are fixed.** Orfis `3222a74` resolved all of them —
ambient metadata surviving `captureDiagnostics: false`, any `res.ok` counting
as accepted, and a shed queue reporting success. Its suite passes 80/80, and
the acceptance-critical tests were run by name here (202 handling, pasted-secret
scrubbing, metadata opt-out, quota). The revision also scrubs secrets from the
message on the device. `docs/orfis-sdk-requests.md` is the original request and
is now answered.

**The license is settled.** Orfis `3f8c14b` (PR #15) licenses
`packages/sdk-web` under MIT and the built artifact carries the terms in its
own first line, so a copy of the bundle alone still names them. The pin moved
to that revision; `packages/sdk-web/src` is byte-identical to `3222a74`, so the
80/80 suite result carries across and the artifact grew by exactly the 66 bytes
of the banner. Orfis built the same revision independently and got the same
hash, which is the first time this pin's reproducibility has been checked by
someone other than the script that makes it. `"private": true` stays, and
governs registry publishing rather than redistribution.

**Both keys are minted and wired** (see
[Where the keys live](#where-the-keys-live--and-why-they-are-committed)),
against the measured packaged Origin below — a
production key origin-locked to exactly `tauri://localhost` for
`https://orfis.otterly.digital`, and a laptop key for `http://localhost:4100`
that also accepts `http://localhost:1420` so `tauri dev` works. Orfis verified
both directions of the production origin check (a `tauri://localhost` preflight
allowed, a foreign origin `403 origin_not_allowed`) without sending a real
report.

**Still owed at release verification**, and not claimed here: a packaged
`tauri build` against production, a queued report surviving quit and relaunch,
`429` and offline retry behaviour, duplicate submission, and a stored row
inspected to confirm the payload carries nothing beyond the documented
fields.

### The packaged Origin — measured

Captured from a real `tauri build` bundle of `net.casadelvalle.worktrees` on
macOS arm64, by a throwaway boot-time `fetch` to a local listener. Not
inferred, and not from `tauri dev` — a dev build serves over
`http://localhost:1420` and would have answered the wrong question.

```
Origin:          tauri://localhost
Sec-Fetch-Site:  cross-site
User-Agent:      Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)
                 AppleWebKit/605.1.15 (KHTML, like Gecko)
```

Three things follow, beyond the key:

- `Sec-Fetch-Site: cross-site` — the webview treats this as a genuine
  cross-origin request, so `tauri://localhost` has to be in the **CORS /
  origin allow-list**, not only attached to the key.
- The User-Agent confirms the frozen `Mac OS X 10_15_7` the handoff warned
  about, so the stored OS version is wrong for every user. That is why the
  Settings copy says "your Mac's system type" and names no version.
- It carries **no Safari token, no version, and no app name**, so whatever
  Orfis's `deviceContext.ts` derives for *browser* will be generic or empty.
  Worth knowing before a stored row is read as evidence of a bug.

The app sets `"csp": null`, so nothing in the webview blocks the request.

### Where the keys live — and why they are committed

Routing is two build-time vars, `VITE_ORFIS_KEY` and `VITE_ORFIS_API_URL`,
read once in `feedback.ts` and validated by `feedbackConfig`. They live in
vite's two MODE files, both committed:

| File | Loaded by | Routes to |
|---|---|---|
| `app/.env.production` | every `tauri build`, local or CI | the production endpoint |
| `app/.env.development` | `tauri dev`, `pnpm dev`, `pnpm dev:mock` | the laptop API |

**Committing a `pk_` key is the considered choice, not an oversight.** It is a
publishable key — Orfis's own description is that it routes, it does not
authorize — and it is origin-locked. It also ships inside the bundle wherever
it is stored, so anyone holding `worktrees.app` can read it with `strings`. A
repo secret would therefore protect it from nobody, while costing something
real: a local `tauri build` would silently lack a feature that CI builds have,
which is a divergence this repo has been bitten by before. Be honest about the
limit, though — an origin allow-list is browser-enforced, so it stops casual
misuse and not a forged `Origin` header; rate limiting is Orfis's side of that.
Nothing here is load-bearing for secrecy, and no release.yml change or repo
secret is involved.

Splitting dev from production by MODE rather than by one shared `.env` is what
stops a `tauri dev` session posting into production. The split is also
fail-safe rather than merely tidy, because `feedbackConfig` permits an `http://`
endpoint only when the host is loopback **and** `import.meta.env.DEV`.
Measured against the real validator, all four combinations:

```
dev key,  dev build      -> accepted
dev key,  RELEASE build  -> REFUSED   <- the laptop key cannot ship
prod key, release build  -> accepted
prod key, dev build      -> accepted  (so a dev build CAN reach production;
                                       the mode split is what prevents it)
```

That last row is the reason there are two files rather than one.
`feedback-check.mjs` guards the invariant that matters — not which key is in
which file, but that the laptop ENDPOINT can never ship — and asserts it with a
dummy key, so the URL rule is what is under test rather than the credential.

Two things were measured rather than assumed, because both would have failed
silently. A production frontend build inlines the production routing and
carries nothing from the development file (checked in `app/dist`, not inferred
from the config). And a preflight to `/v1/feedback` echoes whichever `Origin`
it is sent, with the per-key origin enforced on the POST — so the packaged
`tauri://localhost` and `tauri dev`'s `:1420` both work. Note that a preflight
to a path that does NOT exist returns a global admin-console `ACAO` and a 204,
which looks like a working allow-list and is not one; check the header, and
check the path.

## Bundle provenance

See [`app/src/vendor/orfis/README.md`](../app/src/vendor/orfis/README.md): the
pinned revision, the artifact's SHA-256, and `app/scripts/vendor-orfis.mjs`,
which rebuilds it from a clean `git archive` in a temporary directory and
refuses to write a bundle whose hash differs from the reviewed one. Builds and
runtime need no Orfis checkout, registry package or remote script; the checkout
path is a regeneration input only.

The pinned revision is `3f8c14b`, MIT, and `app/src/vendor/orfis/LICENSE` is a
verbatim copy of the upstream license file at that commit.

## Unresolved before this can ship

None of these are Worktrees-side code, and none should be invented locally:

1. ~~The `Origin` a packaged build sends~~ — **measured and reported**:
   `tauri://localhost` (see above), and both keys are minted from it.
2. ~~Redistribution rights for the bundled artifact~~ — **MIT at `3f8c14b`**.
3. Acceptance of a first release without attached logs, and sign-off on the
   disclosure wording and Settings placement.
4. Ownership of rebuilding the pinned artifact for later releases.

## Acceptance still owed at release verification

Everything below needs a packaged build against production, so none of it is
claimed by this branch — the hand check above covered the main path only:

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

With no configuration the action stays hidden, which is what keeps an
unconfigured build honest; with configuration it is live, and the list above
is what release verification owes.
