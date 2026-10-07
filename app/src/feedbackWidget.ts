import { init, type OrfisTheme } from "./vendor/orfis/orfis.es.js";
import type { FeedbackConfig } from "./feedbackConfig";
import type { FeedbackCallbacks, FeedbackWidget } from "./feedbackTypes";
import { THEMES, type ThemeId } from "./settings";

/** Still gated, and the gate is now the only thing standing between this and
 *  a real send — so it is one constant with one reason, not a buried throw.
 *
 *  What was blocking before is FIXED: Orfis `3222a74` resolved all three
 *  defects (ambient metadata surviving the opt-out, any `res.ok` counting as
 *  accepted, a shed queue reporting success) and its suite passes 80/80. What
 *  is left is ours and the owner's:
 *
 *  1. Packaged acceptance has not been run — it cannot be, because Orfis mints
 *     the key against the Origin a packaged build sends and we have not
 *     reported it yet (`docs/orfis-feedback.md`).
 *  2. The pinned artifact has no license at that private revision, so it may
 *     not be redistributed in an enabled build until the owner confirms.
 *
 *  Flip this to `false` only with both settled. Nothing else needs changing —
 *  everything below is the real integration. */
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

export function createFeedbackWidget(
  config: FeedbackConfig,
  version: string,
  callbacks: FeedbackCallbacks,
): FeedbackWidget {
  if (ACCEPTANCE_PENDING) throw new Error("Orfis packaged acceptance pending");

  // Every privacy opt-out is passed explicitly rather than left to a default.
  // With these three off, a report body is exactly `key`, `type`, `message`,
  // `surface`, `appVersion` — and the form shows no diagnostics disclosure,
  // because there is nothing to disclose. `orfis.es.d.ts` types them as the
  // literal `false`, so dropping one does not typecheck.
  const widget = init({
    key: config.key,
    apiUrl: config.apiUrl,
    appVersion: version,
    surface: "macos-native",
    launcher: false, // Settings drives it; the SDK's own launcher button is off.
    title: "Send feedback about Worktrees",
    captureDiagnostics: false,
    collectDeviceContext: false,
    askForEmail: false,
    theme: orfisTheme(),
    onOpen: () => callbacks.opened(document.documentElement),
    onClose: callbacks.closed,
    // `durable: false` is a memory-only queue — lost on quit. Passed straight
    // through so the notice can say which it was; saying "saved for retry"
    // about a report that is not saved is the defect this result exists for.
    onQueued: (result) => callbacks.queued({ durable: result.durable !== false }),
    onSubmitted: () => callbacks.accepted(),
  });

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
