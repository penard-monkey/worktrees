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
export const ACCEPTED_NOTICE = "Orfis received your feedback. Thank you.";
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

/** Background is inert while the SDK's shadow-root dialog owns focus. Its own
 * Tab handler wraps focus; inert also covers navigation starting outside it. */
function opened(host: HTMLElement) {
  if (releaseDialog) return;
  host.classList.add("modal-scrim"); // App's chord guard sees shadow-hosted modals too.
  const background = [...document.body.children]
    .filter((el): el is HTMLElement => el instanceof HTMLElement && el !== host && !el.inert);
  for (const el of background) el.inert = true;
  const releaseEscape = registerEscape(() => widget?.close());
  releaseDialog = () => {
    releaseEscape();
    host.classList.remove("modal-scrim");
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
  accepted: () => update({ notice: ACCEPTED_NOTICE, error: false }),
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
