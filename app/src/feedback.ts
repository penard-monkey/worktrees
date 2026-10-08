import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { feedbackConfig } from "./feedbackConfig";
import { registerEscape } from "./useEscape";
import type { FeedbackCallbacks, FeedbackWidget } from "./feedbackTypes";

export const QUEUED_NOTICE = "Feedback is waiting for retry, not yet received. Retries are limited to 7 days and 20 attempts; storage limits may remove older reports.";
/** The memory-only case, said plainly. The old copy hedged both ways in one
 *  sentence because the SDK could not tell us which had happened; it can now
 *  (`QueuedResult.durable`), so the notice stops guessing. */
export const QUEUED_MEMORY_NOTICE = "Feedback is waiting for retry, not yet received — and it is held in memory only, so quitting Worktrees loses it.";
/** There is deliberately no "we received it" notice. The SDK's own dialog
 *  already closes on "Your note is in the queue. Nothing else is needed from
 *  you." — a toast afterwards re-tells the user something they were just told
 *  and promised they could forget, and it lands over the app rather than in
 *  the surface they were using. The queued and error notices stay, because
 *  those say something the dialog did not. */
export type FeedbackState = { available: boolean; notice: string; error: boolean };
let state: FeedbackState = { available: false, notice: "", error: false };
const listeners = new Set<() => void>();
export const feedbackSnapshot = () => state;
export const subscribeFeedback = (fn: () => void) => { listeners.add(fn); return () => { listeners.delete(fn); }; };
function update(patch: Partial<FeedbackState>) {
  state = { ...state, ...patch };
  for (const fn of listeners) fn();
}
export const dismissFeedbackNotice = () => update({ notice: "", error: false });
let widget: FeedbackWidget | null = null;
let starting: Promise<void> | null = null;
let generation = 0;
let releaseDialog: (() => void) | null = null;
let returnFocus: HTMLElement | null = null;

/** Marks "a shadow-hosted dialog is open" for App.tsx's chord guard. An
 *  attribute, not a class, and deliberately not `.modal-scrim`: it must carry
 *  no styling of any kind. App.tsx's `modalOpen`/`onlySettingsOpen` query it;
 *  `feedback-check.mjs` asserts the two stay in step. */
export const DIALOG_MARKER = "data-dialog-open";

/** Background is inert while the SDK's shadow-root dialog owns focus. Its own
 * Tab handler wraps focus; inert also covers navigation starting outside it.
 *
 * The exemption is resolved from `host` rather than trusted: what must stay
 * live is the BODY-LEVEL element the dialog sits in, and a caller that hands
 * over anything else — `document.documentElement` was the first one — matches
 * no body child, so every child is inerted, the dialog with them. That failure
 * is invisible to look at: the form renders correctly and silently refuses
 * every click. If the element cannot be placed under <body>, inert NOTHING; a
 * live background is a far smaller defect than a dialog nobody can use. */
function opened(host: HTMLElement | null) {
  if (releaseDialog) return;
  const kids = [...document.body.children].filter((el): el is HTMLElement => el instanceof HTMLElement);
  const top = host ? kids.find(el => el === host || el.contains(host)) : undefined;
  const background = top ? kids.filter(el => el !== top && !el.inert) : [];
  for (const el of background) el.inert = true;
  // The app's chord guard must see shadow-hosted modals too, but it cannot see
  // one the way it sees the others: its rule is "every overlay PAINTS a
  // `.modal-scrim`", and this dialog paints its backdrop inside the shadow
  // root, where no light-DOM class exists to find. Borrowing `.modal-scrim` as
  // a marker is what it did first, and that class is not a marker — it is a
  // full-screen fixed scrim with its own background, padding and z-index, so
  // the host painted a SECOND scrim over the SDK's own and trapped the dialog
  // in a z-index 100 stacking context. This marker paints nothing; App.tsx
  // queries for it alongside the two scrim classes.
  const marker = top ?? document.documentElement;
  marker.setAttribute(DIALOG_MARKER, "");
  const releaseEscape = registerEscape(() => widget?.close());
  releaseDialog = () => {
    releaseEscape();
    marker.removeAttribute(DIALOG_MARKER);
    for (const el of background) el.inert = false;
  };
}
function closed() {
  releaseDialog?.();
  releaseDialog = null;
  if (returnFocus?.isConnected) returnFocus.focus({ preventScroll: true });
  returnFocus = null;
}
const callbacks: FeedbackCallbacks = {
  opened,
  closed,
  queued: ({ durable }) => update({ notice: durable ? QUEUED_NOTICE : QUEUED_MEMORY_NOTICE, error: false }),
  // Not a notice of its own — but it must CLEAR a queued one. Having said
  // "waiting for retry", staying silent once it lands would leave that
  // standing as a lie until something else happened to replace it.
  accepted: () => update({ notice: "", error: false }),
};

/** Called once by main.tsx, outside React/StrictMode. Duplicate calls share the
 * same promise; teardown invalidates a pending version lookup/import. */
export function startFeedback(): Promise<void> {
  if (starting) return starting;
  const epoch = generation;
  starting = (async () => {
    const mock = !!import.meta.env.VITE_MOCK;
    const config = feedbackConfig(import.meta.env.VITE_ORFIS_KEY, import.meta.env.VITE_ORFIS_API_URL, import.meta.env.DEV);
    if (!mock && !config) return;
    if (!mock && getCurrentWindow().label !== "main") return;
    if (mock) {
      const { createMockFeedback } = await import("./mock/feedback");
      if (epoch !== generation) return;
      widget = createMockFeedback(callbacks);
    } else {
      const { version } = await invoke<{ version: string }>("get_changelog");
      if (!version || version.length > 64) throw new Error("Missing app version");
      const { createFeedbackWidget } = await import("./feedbackWidget");
      if (epoch !== generation) return;
      widget = createFeedbackWidget(config!, version, callbacks);
    }
    update({ available: true });
  })().catch(() => {
    if (epoch !== generation) return;
    // Never include report text, endpoint, key or native error details in logs.
    update({ available: false, notice: "Feedback is unavailable in this build.", error: true });
    void invoke("log_event", { level: "error", msg: "Feedback initialization failed" }).catch(() => {});
  });
  return starting;
}

export function openFeedback(trigger: HTMLElement) {
  if (!widget || releaseDialog) return;
  returnFocus = trigger; // WKWebView clicks do not necessarily focus buttons.
  try {
    widget.open();
  } catch {
    closed();
    update({ notice: "The feedback form could not open. Please try again.", error: true });
  }
}
export function stopFeedback() {
  generation++;
  widget?.destroy();
  widget = null;
  closed();
  starting = null;
  update({ available: false });
}
