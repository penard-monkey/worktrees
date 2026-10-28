// macOS refused this folder, and the user can give it back.
//
// Shown INSTEAD of the raw `Operation not permitted (os error 1)` that a TCC
// denial produces. The decision behind it is the backend's alone (serve's
// `privacy_denial` row → `worktrees_api::macprivacy`); nothing here inspects
// an errno or an English message, because `os error 1` is localised and
// matching it in the frontend is the mirror that drifts.
//
// Three things it has to say, in this order, because that is the order the
// user needs them in:
//
//  1. WHO refused — macOS, not us. Without that the pane reads as a bug.
//  2. What to press. The Settings deep link is the only one-click path.
//  3. That the app must be REOPENED. This is the step people miss, and
//     skipping it makes a correct fix look like it failed: the decision is
//     consulted per process — serve's, which the app stops on quit and starts
//     on launch — and the tmux server this app started inherits its
//     responsible-process attribution at spawn and OUTLIVES the app
//     (AGENTS.md, "macOS re-asks its privacy prompts after every build").
//
// On a window that drives a paired host, all three are the HOST's: its
// Settings, its app. The Settings button is greyed with the reason (onhost.ts:
// it would open THIS Mac's Settings), the copy still works — the line is the
// host's, to paste in a terminal there — and the words say where.
//
// It deliberately does NOT offer a "try again" button. Nothing the app can
// call re-raises the prompt — macOS records the denial and never asks again —
// and a button that silently does nothing is worse than no button.
import { useState } from "react";
import { copyToClipboard } from "./clipboard";
import { remoteHost } from "./ipc";
import { hostOnly, openSettingsOnHost } from "./onhost";

/** `macprivacy::Denial`. */
export type Denial = {
  area: string;
  label: string;
  path: string;
  pane: string;
  reset_command: string;
};

export function PrivacyCard({ denial, onError }: { denial: Denial; onError: (e: unknown) => void }) {
  // Copy feedback is local and ephemeral: a second copy should re-confirm, so
  // it resets on a timer rather than latching.
  const [copied, setCopied] = useState(false);
  // `copyToClipboard`, never `navigator.clipboard`: the web API needs the
  // click's transient activation, and macOS 27's WebKit spends it on every
  // `evaluateJavaScript:` — which is how terminal output arrives. A card shown
  // beside a streaming session would reject with `NotAllowedError` exactly
  // when it is needed (see clipboard.ts; clipboard-check.mjs enforces it).
  const copy = () => {
    copyToClipboard(denial.reset_command)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1600);
      })
      .catch(onError);
  };
  const host = remoteHost();
  return (
    <div className="init-banner privacy-card">
      <div className="init-banner-h">
        <span className="init-banner-i">⊘</span>
        macOS is blocking {denial.label}{host && ` on ${host}`}
      </div>
      <p>
        This app asked for access and the answer was no, so macOS refuses every
        read here. It will not ask again on its own.
      </p>
      <code className="privacy-path">{denial.path}</code>
      <div className="ver-actions">
        <button
          className="ctrl sm"
          data-track="privacy.settings"
          {...hostOnly("Opening System Settings")}
          onClick={() => openSettingsOnHost(denial.pane).catch(onError)}
        >
          Open Privacy settings…
        </button>
        <button className="ctrl sm" data-track="privacy.copy" onClick={copy}>
          {copied ? "Copied" : "Copy reset command"}
        </button>
      </div>
      {host ? (
        <p className="privacy-why">
          The fix is made on {host}: in its System Settings, switch worktrees on
          in the list, then <strong>quit and reopen</strong> the app there — the
          decision is read once per process, so a running app keeps the old
          answer. If it is not listed, paste the copied command in a terminal
          on {host} to clear the refusal, then reopen; the next read asks again.
        </p>
      ) : (
        <p className="privacy-why">
          Switch worktrees on in the list, then <strong>quit and reopen</strong> the
          app — the decision is read once per process, so a running app keeps the
          old answer. If it is not listed, paste the copied command in a terminal
          to clear the refusal, then reopen; the next read asks again.
        </p>
      )}
    </div>
  );
}
