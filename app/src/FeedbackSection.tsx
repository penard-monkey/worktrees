import { useSyncExternalStore } from "react";
import { feedbackSnapshot, subscribeFeedback, openFeedback, dismissFeedbackNotice } from "./feedback";

export function FeedbackSection() {
  const state = useSyncExternalStore(subscribeFeedback, feedbackSnapshot);
  if (!state.available) return null;
  return <section className="setting">
    <label>Feedback</label>
    <div className="ver-actions">
      <button className="ctrl sm" onClick={e => openFeedback(e.currentTarget)}>Send feedback</button>
    </div>
    {/* This text is a promise about the PAYLOAD, so it tracks the payload and
        not the other way round. No email field (`askForEmail: false`), and
        "nothing else" would be false: Orfis derives browser and OS from the
        webview's User-Agent and stores them on every row, so the system type
        is named here. Confirm against a real stored row at release
        verification — see docs/orfis-feedback.md. */}
    <div className="hint">Sends your message to Orfis with the app version and your Mac's system type.
      No logs, terminal contents, file paths or usage records are attached, and no email address is asked for.
      Secrets in your message are removed before it is sent, but avoid including them.</div>
  </section>;
}

/** Also surfaces startup replay acceptance when Settings has never been opened. */
export function FeedbackNotice() {
  const state = useSyncExternalStore(subscribeFeedback, feedbackSnapshot);
  if (!state.notice) return null;
  return <div className="feedback-notice" role={state.error ? "alert" : "status"}>
    <span>{state.notice}</span>
    <button className="ctrl sm" aria-label="Dismiss feedback notice" onClick={dismissFeedbackNotice}>Dismiss</button>
  </div>;
}
