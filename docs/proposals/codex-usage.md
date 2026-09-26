---
title: "Proposal — Codex plan usage"
---

# Codex plan usage

**Status:** approved and implemented on `codex-usage-meter`. 2026-09-26.
The design below records the approved contract; validation results are in the PR.

## Decision

Read Codex account limits through a short-lived `codex app-server` stdio
connection, using the CLI's existing ChatGPT login. Add `codex_usage` beside
`claude_usage`. Show **both providers always**, subject to independent user
toggles, in one compact meter and one popover. A provider with no reading gets
a named placeholder, never a fabricated zero. Account usage is global: changing
the selected place does not change which account limits are shown or fetched.

This extends the [Codex support proposal](codex-support.md) and its
[implementation record](../sessions/2026-09-23-codex-support/summary.md).
It does not alter the one-live-provider-per-place rule. Codex presence still
means its tmux session exists; a usage fetch says nothing about busy, idle,
completion, or work done in a particular place.

The brief calls the compact surface a header. On this branch, `useUsage` and
`UsageMeter` in `app/src/App.tsx` actually serve the bottom strip, terminal
footer and rail. Keep those placements and the existing `usage_place` setting;
the mockups below specify the compact line wherever hosted, without moving it
into the place header.

## Data sources and their cost

| Candidate | Freshness | Cost and limitations | Decision |
| --- | --- | --- | --- |
| `account/rateLimits/read` over app-server | Requests an account snapshot without waiting for a model turn. Worktrees records response-receipt time; upstream consistency is not measured. | CLI startup, handshake, authenticated network read, and Codex's own local runtime initialization. The complete 0.156.1 probe took 1.2s on this Mac, once; not a latency guarantee. No turn/model request is sent. Do not claim the endpoint has no rate limit or a guaranteed billing policy. | Primary source. |
| `event_msg` / `token_count` / `payload.rate_limits` in `$CODEX_HOME/sessions/**/rollout-*.jsonl` | A historical observation, only as current as the event that recorded it. Idle files do not refresh account usage from another device. A recently written token-count event is not proof of a fresh network read. | Local bounded disk I/O; no process or network needed. Requires touching transcript-containing files and cannot reliably bind old snapshots to the currently signed-in account. | No automatic fallback. |
| Last successful Worktrees app-server response | Original fetch time, never the last cache-access time. | Small in-memory cache, no I/O. Subject to age, reset and account rules below. | Only fallback. |
| `codex login status`, CLI `/status`, or `account/read` | Authentication/status information, depending on the command. | `account/read` is verified below; CLI help offers login/status but no standalone plan-usage command. Scraping an interactive terminal would couple the app to presentation and the user's session. | `account/read` for auth classification only; no TUI scraping. |
| Direct HTTP with tokens from Codex auth storage | Potentially a current server read. | Worktrees would own credential access, refresh, account routing and an undocumented endpoint contract. | Reject: let Codex own authentication. |
| `account/usage/read`, local token totals, or a statusline extension | Token activity is not a plan-limit percentage. No installed Codex statusline snapshot contract was established here. | Cannot derive shared quotas from token totals; a statusline installation would add configuration/setup. | Reject for this meter. |

The official [app-server documentation](https://learn.chatgpt.com/docs/app-server)
describes account limit reads and update notifications, including window duration
and epoch reset times. Local evidence for the exact CLI version is below.

### App-server lifecycle

Use the configured environment's `CODEX_HOME`, defaulting to `~/.codex`, and
the CLI resolved through the app's repaired login-shell PATH. Resolve the
executable independently of the selected repository; never take an executable
or arguments from a cloned repo (ADR 0001). Run in an app-owned neutral working
directory so selecting a place does not select that repo's Codex configuration.
This shows the default local Codex account, not every profile, remote server,
or account that might be used by a terminal.

For each uncached attempt:

1. Spawn `codex app-server --listen stdio://` with piped stdin/stdout. Do not
   attach to the user's shared daemon or open a TCP listener.
2. Send `initialize`, wait for its response, then send `initialized`.
3. Send `account/read` with `refreshToken: false`. An explicit null account
   means not signed in; an `apiKey`/other non-ChatGPT account means this plan
   meter is unsupported. An RPC error is unavailable, not proof of logout.
4. For a ChatGPT account, send `account/rateLimits/read` with
   `excludeResetCreditDetails: true` (present in both inspected schemas).
   Omit `supportsLunaReserve`; this meter does not implement reserve routing.
5. Keep only the normalized usage fields. Close stdin, wait for exit, then
   terminate and reap the child if needed. A proposed 15s total attempt budget
   includes shutdown; reserve the last 2s for cleanup. Bound response buffering
   to 1MiB and handle EOF, malformed lines and unrelated notifications.

The requests are reads, but **app-server is not a filesystem-read-only
process**: the sandboxed probe failed because Codex needed to initialize its
SQLite runtime. Codex may also maintain its own logs and refresh credentials.
Worktrees must not write `config.toml`, auth files, or another tool's runtime
files itself. Do not send login/logout, config-write, thread, turn, command,
credit-consumption or email-nudge requests. Never log raw stdout/stderr,
credentials, account identity, or banners; log a sanitized failure category.
Unexpected server requests receive a protocol error, never automatic approval.

A short-lived process avoids a second always-on Codex service. The tradeoff is
one startup per actual refresh. A persistent connection and
`account/rateLimits/updated` can be reconsidered if measured startup cost warrants
it; notifications alone are not assumed to track other sessions or devices.

### Why no rollout fallback

`crates/worktrees-core/src/codex.rs::session_present` explicitly reads only
the first `session_meta` record and promises never to read transcript content.
The current file also contains `latest_rollout` and `rollout_model` for model
labels, so the brief's description no longer covers the entire module. Neither
is a reason to widen the resume check or make a usage poll scan conversations.
The chosen design leaves that boundary intact.

If a later proposal adds an explicitly local snapshot mode, its minimum bounds
should be: newest candidate file first, reverse-read only complete JSONL records
within a 256KiB tail, extract only the newest matching event's `rate_limits`,
and stop at the first valid hit. Cap at 32 files and 14 date directories, with
an enumeration budget as well; exhausting a cap is unavailable, not permission
to scan further. Discard partial boundary records, malformed records and
nonmatching payloads. No transcript text may be retained, returned or logged.
Reading bytes that contain conversations is still a privacy tradeoff even when
the parser immediately skips those fields; do not describe it as reading only
quota bytes.

Use file mtime only to prioritize candidates or invalidate a disk cache,
**never** as observation time or last-turn time. A new filename dates session
creation; a long-running older session can contain newer events. First-hit
selection is therefore a bounded heuristic, not a claim to the globally newest
account snapshot. Use the selected event's timestamp to label a local
observation, and still describe its network freshness as unknown. Old account
snapshots, subagent rollouts and missing account identity make a silent fallback
especially misleading. These costs outweigh its offline convenience here.

## Normalized backend contract

Keep `claude_usage` and its OAuth/statusline acquisition logic. Add an async
`codex_usage` command in the app backend, using a dedicated module for protocol,
normalization and cache state. Blocking process work belongs on a worker, never
the UI thread. A frontend adapter gives both commands a shared view model;
do not rename Claude's sources or reuse `app/src/usage.ts` (UI click telemetry).

Separate commands isolate failure/backoff and let an off toggle truly suppress
one provider. One combined `usage` command would need provider selection and
independent internal caches anyway, while changing the existing Claude command
and all its harness callers. There is no need for that migration in this work.

Proposed Codex response (conceptual shape, not committed implementation):

```text
provider: codex
state: ready | stale | unavailable | signed_out | unsupported_auth | missing_cli
source: app_server | cached | unavailable
fetched_at: unix_seconds | null       # successful observation, not attempt time
retry_at: unix_seconds | null
reason: closed sanitized code | null
limits[]:
  id: bucket_id + window_role         # stable identity, not translated label
  bucket_id, bucket_label
  window_role: primary | secondary
  window_minutes: positive_integer | null
  percent: finite_nonnegative_number
  severity: normal | warning | over
  resets_at: positive_unix_seconds | null
```

Keep an account identity/generation only inside the backend cache, never in the
UI payload or logs. Use `accountId` from the limit response when supplied. An
observed logout, auth-mode change, account change or `CODEX_HOME` change clears
old limits immediately. If identity is absent, a successful reading can render,
but do not carry its values across a failed attempt as an account-bound fallback.
An unobserved external account change can remain visible during the 120s cache
TTL; the app cannot promise instant detection. Never use rollout history to
recover a previous account's limits.

Normalization rules:

- Prefer a nonempty `rateLimitsByLimitId` map. Use `rateLimits` as compatibility
  fallback when the map is absent/null/empty, not as a duplicate row. Retain
  separate buckets; do not sum percentages or blend primary and secondary.
- Order the `codex` bucket first, then additional bucket IDs deterministically.
  Use `limitName` when present, otherwise the bucket ID (friendly `Codex` for
  `codex`). Treat labels as text and allow truncation with a full accessible label.
- Derive labels from duration: 300 minutes → `5h`; 10080 → `7d`; other positive
  values → a truthful duration. With no duration use `Primary`/`Secondary`.
  **Primary is not necessarily five hours** (see the observed weekly-only
  account below). A null window means no reported window, not 0% or unlimited.
- Preserve valid percentages, including decimals and values over 100. Round
  only the displayed number; clamp bar fill to 0–100. Reject negative/nonfinite
  values, tolerate unknown fields, and keep a valid sibling window if one fails.
  Empty valid windows mean `unavailable` / `no_limits`, not unlimited.
- Codex supplies no severity in the inspected window schema. Proposed visual
  thresholds are `<80` normal, `80..<100` warning, `>=100` over. They are local
  display rules, not a server verdict that a turn is allowed. Keep Claude's
  server-supplied severity. Do not infer access from reset time or remaining %;
  the generated `ordinaryUsageAllowed` field explicitly warns against that.
- Credits, spend controls and reserve-routing decisions are outside this
  percentage meter. Additional quota windows such as `gpt-reserve` can appear
  under their own name in detail; their presence does not promise eligibility.

### Cache, backoff and clocks

Use independent Codex state: one in-flight attempt shared by concurrent callers,
a 120s success TTL, and a minimum 60s failure holdoff. Increase consecutive
failure delays to 120s, 240s, then at most 15min; honor a longer retry delay if
the protocol supplies one. Success clears failure count. Unsupported protocol
or missing CLI rechecks at the 15min cap; a changed executable may clear that
negative cache. Signed-out checks run no more often than the normal 180s poll.
Focus and popover opening never bypass backoff.

When a transient failure follows an account-bound success, preserve that reading
for at most 30min from the original fetch, explicitly stale. Once a row's reset
has passed, suppress its old percentage/bar and show `Awaiting update`; never
animate it to 0%. A still-valid sibling row may remain. The UI's clock applies
this rule between polls too. Beyond 30min suppress all old numeric values and
show unavailable with the last-success time. Null reset times remain eligible
only under the same age ceiling. Treat future observation timestamps/clock
anomalies conservatively; TTL/backoff should use monotonic time internally.

In the app-owned `useUsage`, invoke enabled providers independently so a slow
Codex process does not delay Claude. Keep the existing 180s poll and 15s display
clock while visible; no immediate or interval poll on a hidden transition.
Resume/focus catch-up goes through backend TTL and single-flight. The current
hook has an unconditional focus listener; preserve its catch-up purpose but
check actual document visibility before future invokes. No background timers or
processes for a disabled provider; `usage_place: off` suppresses both.

Ignore late replies after disable/unmount and discard older request generations
so an error cannot overwrite a newer success. Returning cached data must not
advance `fetched_at`. Polling errors become provider state, with one sanitized
log per real attempt; unexpected invoke/programming errors still use `fail()`.
Do not introduce a global toast for expected offline/signed-out states.

## Compact meter and popover (ASCII mockups)

Both enabled providers occupy stable slots, whether or not their CLI is signed
in. Choosing only the active place's provider would hide the other account
exactly when deciding whether to switch. Choosing only signed-in providers
would make logout and transient auth failures look like a disappearing widget.
The meter never aggregates Claude and Codex into one percentage.

Each compact slot summarizes its most-used valid window in the main bucket,
with the provider name, window label, short bar and percent. Claude's compact
selection includes its model-scoped windows. Codex uses bucket `codex`, or the
compatibility single bucket when identifiable. Additional Codex buckets live in
detail; if only additional buckets exist, show `Codex details` instead of
silently treating reserve usage as the main allowance. Ties have stable ordering.
Accessible text names the exact bucket/window and that the number is used %.

Illustrative numbers below are invented, not the private probe's values.
One button opens both sections; `v` denotes its affordance, not a third control.

```text
Compact line, two providers with valid readings:
[ Claude 7d [######--] 72% | Codex 5h [####----] 48%  v ]

Popover:
+----------------------------------------------------+
| Plan usage                         Account limits  |
| Claude                         Updated 1 min ago   |
| 5h             [###-------] 35%   resets in 3h      |
| 7d             [#######---] 72%   resets in 2d      |
| Fable 7d       [######----] 61%   resets in 2d      |
|                                                    |
| Codex                          Updated just now    |
| 5h             [#####-----] 48%   resets in 2h      |
| 7d             [##--------] 21%   resets in 4d      |
| gpt-reserve                                        |
| 7d             [#---------]  8%   resets in 4d      |
+----------------------------------------------------+

Actual observed window SHAPE, illustrated percentage:
[ Claude 7d [######--] 72% | Codex 7d [##------] 24%  v ]
  Codex detail has one 7d row; no empty/fake 5h row.
```

Statuses belong to their own provider. A failure must not dim both sections.
Use explicit words alongside subdued bar styling, not opacity alone.

```text
Transient failure with usable cache:
[ Claude 7d [######--] 72% | Codex 5h 48% stale  v ]
+----------------------------------------------------+
| Claude                         Updated 1 min ago   |
| ... current Claude rows ...                        |
| Codex                          Stale - 8 min ago   |
| Could not refresh. Showing the last reading.       |
| 5h             [#####-----] 48%   resets in 2h      |
| 7d             [##--------] 21%   resets in 4d      |
+----------------------------------------------------+

Reset passes without a successful refresh:
[ Claude 7d [######--] 72% | Codex 7d 21% stale  v ]
| Codex                          Stale - 9 min ago   |
| 5h             Awaiting update                    |
| 7d             [##--------] 21%   resets in 4d      |
  If all rows expired: compact slot is "Codex stale".

No successful data / cache older than 30 minutes:
[ Claude 7d [######--] 72% | Codex unavailable  v ]
| Codex                          Unavailable         |
| Could not read usage. Last updated 42 min ago.     |
  Omit last-updated text when there has been no success.

Explicitly no account:
[ Claude 7d [######--] 72% | Codex sign in  v ]
| Codex                          Not signed in       |
| Sign in through the Codex CLI, then return here.  |
  Informational text; do not launch login from a background read.

Other explicit states:
[ Claude unavailable | Codex CLI missing  v ]
| Codex CLI was not found.                           |
[ Claude unavailable | Codex plan unavailable  v ]
| This Codex login does not provide ChatGPT plan     |
| usage to this meter.                              |

Initial load:
[ Claude checking... | Codex checking...  v ]
| Codex                          Checking usage...   |

Both unavailable: keep one reachable diagnostic popover.
[ Claude unavailable | Codex unavailable  v ]
```

Missing-cli, unsupported-auth, no-limits and protocol errors get different detail
copy even when they share neutral unavailable styling. Claude's current command
does not reliably distinguish logout from missing/locked credentials; do not
label Claude signed out merely because `source` is unavailable. Its adapter
retains `statusline` and `cached` source labels and original timestamp semantics.
The stricter Codex age/reset contract is not a claim about existing Claude logic.

For narrow line hosts, remove bars first, then window labels; preserve provider
names and numbers/state. Below the width for both compact slots, use a `Usage`
trigger with both provider names in its accessible label and both sections in
the popover. With only one provider enabled, omit the other section entirely.

```text
Narrow:       [ Claude 72% | Codex 48% v ]
Very narrow:  [ Usage v ]
Claude off:   [ Codex 5h [####----] 48% v ]
Both off:     (no plan-usage trigger, no plan-usage polling)

Rail:         [ usage glyph ]  one existing 32px tile
              same two-section popover; no unlabeled provider bars
```

Claude service-status polling stays separate. Label an incident explicitly
`Claude service status`, including inside the shared rail popover, and keep
its existing standalone status affordance when no usage provider is enabled.
Never present that incident as a Codex outage or quota exhaustion.

### Layout and accessibility constraints

- Preserve the plain block `<button>` with an inner flex span. Provider/window
  labels use `flex: none`; each bar gets a real `width` as well as `flex-basis`.
  Do not reuse `.seg`, which belongs to Settings. Measure in Playwright WebKit
  as well as Chromium; Chrome alone previously hid zero-width labels.
- Use `--accent`, `--warn` and `--danger` for fills/dots, with words and
  percentages in `--txt-hi`/`--txt-dim`. Existing `.usage-pct` severity colors
  need to be neutralized when this shared component changes. Do not copy their
  old accent-as-text behavior. Keep text readable in stale sections instead of
  applying `.usage-pop.stale { opacity: .55 }` to the whole panel.
- Keep hover delay, keyboard focus preview and click-to-pin. Use `useEscape`
  for the pinned surface per current house rules (the existing meter predates
  that rule); exclude trigger and panel from outside-pointer dismissal.
  Include `aria-expanded`, provider/window labels and text status, not color
  alone. Do not announce every countdown tick through a live region.
- Fixed positioning from the trigger rect, viewport clamping and a bounded
  scrolling popover keep extra buckets reachable. Proposed preferred width is
  360 CSS px, capped to viewport minus margins, with token-scaled text/spacing.
  Reposition on content resize. Use `focus({preventScroll:true})` if moving
  focus into a pinned panel; never autofocus a clipped header child.
- Verify all six themes, especially tokyo-day and catppuccin-latte, composing
  translucent backgrounds over their opaque ancestors before contrast checks.
  Test zoom, long labels and hit-testing, not just plausible bounding boxes.

## Settings

Under **Appearance → Usage meter**, retain Strip / Footer / Rail / Off and add
two checkboxes: `Claude plan usage` and `Codex plan usage`, both default on.
Persist booleans in frontend-owned settings (e.g. `usage_claude`, `usage_codex`);
missing keys migrate to true, explicit false survives load/save. Existing users
with placement Off stay off. All toggles off is valid and hides the plan meter.

Disabling a provider stops its polling and clears its frontend state; a pending
attempt is bounded and cannot repopulate the hidden slot. Re-enabling may use a
still-valid backend cache, without laundering its timestamp. Explain that these
are account limits read through existing CLI sign-ins. Keep the unrelated
Settings → Usage click-telemetry panel and `app/src/usage.ts` untouched.

## Evidence and reproducibility

Inspected on macOS on 2026-09-26; no conversation content was printed or copied.
The installed symlink had advanced since the brief:

```text
$ ~/.local/bin/codex --version
codex-cli 0.157.1
$ ~/.codex/packages/standalone/releases/0.156.1-aarch64-apple-darwin/bin/codex --version
codex-cli 0.156.1
```

Both versions successfully ran `app-server generate-ts --out <scratch-dir>`.
For 0.156.1 the scratch directory was `/tmp/codex-usage-protocol-01561-20260926`;
for 0.157.1, `/tmp/codex-usage-protocol-20260926`. These are reproducible local
artifacts, not repository dependencies. Relevant generated files:

- `ClientRequest.ts`: `account/read` and `account/rateLimits/read` methods.
- `v2/GetAccountParams.ts`: optional `refreshToken`.
- `v2/GetAccountRateLimitsParams.ts`: optional `excludeResetCreditDetails`.
- `v2/GetAccountRateLimitsResponse.ts`: `rateLimits`, nullable
  `rateLimitsByLimitId`, nullable `accountId`, and `ordinaryUsageAllowed`.
- `v2/RateLimitSnapshot.ts`: nullable `primary`/`secondary`, bucket ID/name.
- `v2/RateLimitWindow.ts`, in both versions:

```ts
export type RateLimitWindow = {
  usedPercent: number,
  windowDurationMins: number | null,
  resetsAt: number | null,
};
```

The exact 0.156.1 binary served these newline-delimited requests, waiting for
each response before sending the next request:

```json
{"id":1,"method":"initialize","params":{"clientInfo":{"name":"worktrees_usage_probe","title":"Worktrees usage research","version":"0.1"},"capabilities":null}}
{"method":"initialized"}
{"id":2,"method":"account/read","params":{"refreshToken":false}}
{"id":3,"method":"account/rateLimits/read"}
```

Observed: initialization succeeded; the account type was `chatgpt`; the
request sequence took 1.2s. The rate-limit response projection was:

```json
{
  "rateLimits": {
    "limitId": "codex", "limitName": null, "planType": "prolite",
    "primary": {"usedPercent": "<redacted>", "windowDurationMins": 10080, "resetsAt": "<redacted>"},
    "secondary": null
  },
  "rateLimitsByLimitId": {
    "base_model_inference": {
      "limitId": "base_model_inference", "limitName": "gpt-reserve", "planType": "prolite",
      "primary": {"usedPercent": "<redacted>", "windowDurationMins": 10080, "resetsAt": "<redacted>"},
      "secondary": null
    },
    "codex": {
      "limitId": "codex", "limitName": null, "planType": "prolite",
      "primary": {"usedPercent": "<redacted>", "windowDurationMins": 10080, "resetsAt": "<redacted>"},
      "secondary": null
    }
  }
}
```

The probe closed stdin and reaped its child. It sent no thread/turn requests
and did not edit Codex configuration. The initial sandboxed attempt exited
with `failed to initialize sqlite state runtime`; the successful read used
explicitly approved execution outside the sandbox. This measures one account
and environment, not all plans or supported operating systems.

A separate local rollout probe considered at most eight newest-mtime candidate
files and read at most 256KiB from each tail, stopping on its first valid
`event_msg` / `token_count` hit. It printed **only this rate-limit projection**
from `$CODEX_HOME/sessions/2026/09/26/rollout-<redacted>.jsonl`:

```json
{
  "rate_limits": {
    "limit_id": "codex", "limit_name": null, "plan_type": "prolite",
    "primary": {"used_percent": "<redacted>", "window_minutes": 10080, "resets_at": "<redacted>"},
    "secondary": null
  }
}
```

This demonstrates snake_case on disk versus camelCase on the wire, not a
freshness guarantee or an account match. No token totals or messages are part
of this evidence. CLI help also exposes daemon/proxy tooling, but attaching to
the user's running service was not needed or tested.

## Tests and checks for a later implementation

No implementation tests are claimed by this proposal. The later change needs:

1. **Parser unit tests:** both windows; the observed weekly-only primary;
   unknown/missing duration; null reset; partial malformed windows; decimals,
   negative/nonfinite and over-100 percentages; threshold boundaries; empty
   buckets; multi-bucket deduplication, ordering and long names; unknown fields.
   Keep fixture shapes from the exact-version evidence, with invented values.
2. **Transport tests with a fake CLI:** correct handshake/request ordering;
   ChatGPT/null/other account types; malformed RPC, unknown method, error,
   interleaved notification, buffer cap, EOF, timeout and child cleanup.
   Assert no thread/turn/config-write requests and no raw account/error output
   in logs. Test neutral cwd and `CODEX_HOME` resolution.
3. **Cache tests with injected clock and account state:** 120s success boundary;
   60s failure floor and increasing backoff; concurrent callers create one
   child; late error cannot replace success; original timestamp retained;
   30min inclusive boundary; reset invalidation even during success TTL;
   partial expiry; clock changes; logout/account/home change; missing identity
   prevents cross-failure fallback. Each test owns its state, not a global static.
4. **Hook/settings tests:** provider toggles and global Off suppress invokes;
   hidden startup and hidden transition do not fetch; visible return catches
   up; focus coalesces; countdown stops hidden; independent slow providers;
   stale replies after disable; no-provider state; legacy settings migration.
5. **Mock harness:** add `codex_usage` alongside `claude_usage`, with independent
   `codexUsage=ready|weekly|stale|expired|unavailable|signedout|missing|unsupported|multi|edge`
   fixtures and a controllable delay/failure sequence. Preserve existing
   `usage=...` Claude fixtures and `usage=demo` click telemetry. Add invocation
   counters to prove Off/visibility behavior and use a virtual clock for races.
6. **Browser checks:** both providers, each alone, both off, loading and every
   failure state; normal/minimum window sizes; rail/mirrored rail; Home, footer
   and strip; long buckets, zoom and six themes. Measure nonzero labels and
   explicit bar widths in Chromium **and WebKit**, then hit-test the panel's
   last row. Exercise real pointerdown/click, keyboard focus, pin, outside-click
   and Escape through `useEscape`. Confirm a Claude incident cannot label Codex.

Show meaningful regression tests fail with their guarded behavior removed, per
CLAUDE.md. For implementation, run its required gates: release CLI build first,
`make test`, `make lint`, core/CLI/app-lib cargo tests, frontend `tsc --noEmit`
and `cargo check -p app`. Run `app/scripts/usage-check.mjs` to ensure dynamic
meter labels do not leak into click telemetry; it does not test plan limits.
Add the WebKit/layout and contrast checks described above. This docs-only proposal needs a diff,
link and evidence review, not a release build or app launch.

### Real-login manual check before shipping

Use a real Codex ChatGPT login already established by the human. Do not log
the user out, replace credentials, create a test turn, or script input into the
installed GUI to make fixtures. Repeat the recorded read sequence, retain only
redacted limits, compare window durations/used percentages/reset times with the
CLI's own status view at approximately the same time, and record CLI version,
OS and observed timing. If the CLI uses remaining %, compare against `100-used`.
The 1.2s protocol probe above passes only the read portion of this check.

In the sandbox app used by hand, confirm both providers stay visible while
switching selected places; wait one 180s interval and inspect sanitized logs
for a single Codex attempt. Minimize/occlude for longer than a poll period,
then restore: no hidden periodic requests and one coalesced catch-up. Toggle
Codex off, then placement Off, and confirm network/process activity stops.
Briefly disconnect network to verify cached → stale → unavailable/expired
behavior, then restore it and verify recovery without a polling storm.

Use an isolated Codex home or mocks for signed-out/API-key states; never modify
the human's login for a test. Check actual WKWebView labels, neutral text and
popover reachability. Confirm this feature itself never writes Codex config or
starts/resumes a conversation. Record any credential refresh/runtime writes as
Codex-owned side effects. App-level manual checks, multi-account transitions,
other OS behavior and repeated performance measurements remain to be done when
the proposal is implemented.
