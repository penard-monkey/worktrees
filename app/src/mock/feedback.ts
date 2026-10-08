import type { FeedbackCallbacks, FeedbackWidget } from "../feedbackTypes";

/** No SDK import, network, timers, storage, or replay — including existing outboxes. */
export function createMockFeedback(callbacks: FeedbackCallbacks): FeedbackWidget {
  let host: HTMLDivElement | null = null;
  const close = () => {
    if (!host) return;
    host.remove();
    host = null;
    callbacks.closed();
  };
  return {
    open() {
      if (host) return;
      host = document.createElement("div");
      host.className = "modal-scrim stacked";
      host.dataset.feedbackMock = "";
      host.innerHTML = `<section class="modal feedback-mock" role="dialog" aria-modal="true" aria-label="Send feedback about Worktrees">
        <h2>Send feedback about Worktrees</h2>
        <p>Offline preview. Nothing is sent or saved.</p>
        <label>Your message<textarea aria-label="Your message"></textarea></label>
        <div class="ver-actions">
          <button class="ctrl" data-queued>Simulate queued (durable)</button>
          <button class="ctrl" data-queued-memory>Simulate queued (memory only)</button>
          <button class="ctrl" data-accepted>Simulate accepted</button>
          <button class="ctrl" data-close>Close</button>
        </div>
      </section>`;
      host.addEventListener("click", (e) => { if (e.target === host) close(); });
      host.querySelector("[data-close]")!.addEventListener("click", close);
      // Both outcomes, because they say different things to the user and only
      // one of them is safe to quit on. Not `addEventListener(…, callbacks.queued)`:
      // the listener's argument is an Event, not a QueuedResult.
      host.querySelector("[data-queued]")!.addEventListener("click", () => callbacks.queued({ durable: true }));
      host.querySelector("[data-queued-memory]")!.addEventListener("click", () => callbacks.queued({ durable: false }));
      host.querySelector("[data-accepted]")!.addEventListener("click", callbacks.accepted);
      host.addEventListener("keydown", (e) => {
        if (e.key !== "Tab" || !host) return;
        const fields = [...host.querySelectorAll<HTMLElement>("textarea, button")];
        const first = fields[0], last = fields[fields.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      });
      document.body.append(host);
      callbacks.opened(host);
      host.querySelector("textarea")!.focus();
    },
    close,
    destroy: close,
  };
}
