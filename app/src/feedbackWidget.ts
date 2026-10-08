import { init, type OrfisTheme } from "./vendor/orfis/orfis.es.js";
import type { FeedbackConfig } from "./feedbackConfig";
import type { FeedbackCallbacks, FeedbackWidget } from "./feedbackTypes";
import { THEMES, type ThemeId } from "./settings";
import { applyFeedbackSpacing } from "./feedbackStyle";

/** Still gated, and the gate is now the only thing standing between this and
 *  a real send — so it is one constant with one reason, not a buried throw.
 *
 *  Three of the four blockers are now gone. Orfis `3222a74` fixed all three SDK
 *  defects (ambient metadata surviving the opt-out, any `res.ok` counting as
 *  accepted, a shed queue reporting success), `3f8c14b` licensed the bundle
 *  under MIT, and both product keys are minted against the measured packaged
 *  Origin. What is left is ONE thing, and it is ours:
 *
 *    Packaged acceptance has not been run. It needs a real `tauri build`
 *    talking to a live endpoint and the checks in `docs/orfis-feedback.md`
 *    walked — above all that a report ARRIVES and carries nothing but the
 *    five documented fields.
 *
 *  So this constant is no longer "waiting on someone else"; it is the one
 *  claim this branch is not entitled to make yet. Flip it to `false` only
 *  after that run, and update `feedback-check.mjs` in the same commit — it
 *  asserts the constant, which is what makes opening the gate deliberate.
 *  Nothing else needs changing; everything below is the real integration. */
const ACCEPTANCE_PENDING = true;

/** The SDK's light/dark choice, from whichever `[data-theme]` is live.
 *
 *  The COLOURS do not come through here — `[data-orfis]` in tokens.css maps
 *  this app's tokens onto `--orfis-*`, so the widget re-themes with everything
 *  else and a new theme needs no code. This only tells the SDK which of its
 *  two structural variants to lay out. */
export function orfisTheme(root = document.documentElement): OrfisTheme {
  const id = root.dataset.theme as ThemeId | undefined;
  return THEMES.find((t) => t.id === id)?.appearance === "light" ? "light" : "dark";
}

/** The SDK's shadow host. `init` does `createElement("div")`,
 *  `setAttribute("data-orfis", "")`, `attachShadow(...)`, then
 *  `document.body.append(host)` — so the dialog lives in a shadow root on a
 *  BODY-LEVEL div, and that div is what the app must exempt when it inerts the
 *  background.
 *
 *  This used to pass `document.documentElement`, which is not a child of
 *  `<body>` and therefore matched nothing: every body child was inerted,
 *  the dialog included, and the form rendered perfectly while refusing every
 *  click. Queried lazily rather than captured after `init`, so a host the SDK
 *  ever re-creates is still found. */
const orfisHost = () => document.querySelector<HTMLElement>("[data-orfis]");

export function createFeedbackWidget(
  config: FeedbackConfig,
  version: string,
  callbacks: FeedbackCallbacks,
): FeedbackWidget {
  if (ACCEPTANCE_PENDING) throw new Error("Orfis packaged acceptance pending");

  // The two capture opt-outs are passed explicitly rather than left to a
  // default, and typed as literal `false` in `orfis.es.d.ts` so dropping one
  // does not typecheck. `askForEmail` is deliberately ON: the field is
  // optional and the user types it or does not, which is a different thing
  // from ambient collection. The body is then `key`, `type`, `message`,
  // `surface`, `appVersion` and — only if filled in — `email`. Settings says
  // so; that copy tracks this call, not the other way round.
  const widget = init({
    key: config.key,
    apiUrl: config.apiUrl,
    appVersion: version,
    surface: "macos-native",
    launcher: false, // Settings drives it; the SDK's own launcher button is off.
    title: "Send feedback about Worktrees",
    captureDiagnostics: false,
    collectDeviceContext: false,
    askForEmail: true,
    theme: orfisTheme(),
    onOpen: () => callbacks.opened(orfisHost()),
    onClose: callbacks.closed,
    // `durable: false` is a memory-only queue — lost on quit. Passed straight
    // through so the notice can say which it was; saying "saved for retry"
    // about a report that is not saved is the defect this result exists for.
    onQueued: (result) => callbacks.queued({ durable: result.durable !== false }),
    onSubmitted: () => callbacks.accepted(),
  });

  // The SDK's `rem` sizing resolves against this app's 15px root, not the 16px
  // it was drawn for, so the dialog rendered ~6% tight. applyFeedbackSpacing restates
  // the SDK's own sheet against a 16px base — upstream's numbers exactly.
  const host = orfisHost();
  if (host) applyFeedbackSpacing(host);

  // Re-theme with the app. `[data-theme]` is written by `applySettings`, so one
  // observer on the root catches every path that changes appearance —
  // Settings, a system light/dark flip, a profile switch — without each of
  // them having to know the widget exists.
  const watch = new MutationObserver(() => widget.setTheme(orfisTheme()));
  watch.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

  return {
    open: () => widget.open(),
    close: () => widget.close(),
    destroy: () => {
      watch.disconnect();
      widget.destroy();
    },
  };
}
