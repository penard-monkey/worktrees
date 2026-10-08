import { useSyncExternalStore } from "react";
import { feedbackSnapshot, subscribeFeedback, openFeedback, dismissFeedbackNotice } from "./feedback";
import * as Icons from "./icons";

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
      No logs, terminal contents, file paths or usage records are attached. The email field is optional
      and is sent only if you fill it in. Secrets in your message are removed before it is sent, but
      avoid including them.</div>
  </section>;
}

/** The nav rail's feedback trigger, between add-project and the gear.
 *
 *  Same `available` gate as the Settings row, and deliberately the same
 *  `openFeedback` call rather than a second path into the SDK: the widget is a
 *  singleton, so two triggers must be two callers of one opener, not two
 *  owners. `openFeedback` takes the element so focus returns HERE on close —
 *  a WKWebView click does not focus a button on its own (AGENTS.md), so
 *  without the argument Escape would drop focus to the body.
 *
 *  Module scope, with no props: a component declared inside App() gets a new
 *  identity every render and would remount — the rule that costs state and
 *  focus everywhere else in this app. */
export function FeedbackRailButton() {
  const state = useSyncExternalStore(subscribeFeedback, feedbackSnapshot);
  if (!state.available) return null;
  return <button
    className="rail-icon"
    data-track="feedback"
    title="Send feedback"
    onClick={e => openFeedback(e.currentTarget)}
  ><Icons.MessageSquare size={17} /></button>;
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
