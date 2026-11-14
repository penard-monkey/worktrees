// Beta features: code that ships in a release but stays out of sight until
// the person turns it on in Settings → General → Beta features.
//
// **One feature, one flag, one checkbox** (the owner, 2026-11-14). Every beta
// feature is its OWN entry in `BETA_FEATURES` below: its own per-viewer
// boolean in `ui-state.json` (settings.ts, `beta_<name>`, default off), its own
// checkbox in the Settings section (pages/general.tsx draws one per entry) and
// its own helper. Never put a second feature under an existing flag: Ideas and
// workspaces shared one switch until workspaces left the beta, and nobody
// could have one without the other. beta-check.mjs holds the table, the
// Settings rows and the keys to each other.
//
// Everything that draws or opens a feature asks its helper, never the settings
// key itself (`beta-check.mjs` holds App.tsx to that). The engine is never
// gated: its rows are invisible until something on screen calls them, and a
// second copy of the rule there could only disagree with this one.
//
// A feature LEAVES the beta by deleting its gates, its entry and its helper,
// never its settings key: an older build still reads the blob (AGENTS.md,
// "Retiring a key from ui-state.json breaks a downgrade"). Workspaces left on
// 2026-11-14; `beta_ideas` was theirs too and stays Ideas' alone.
//
// This is a local setting, not a registry and not remote config. No runtime
// import: beta-check.mjs loads this file on its own.
import type { Settings } from "./settings";

/** The keys a beta feature may use: each its own `beta_<name>` boolean. */
export type BetaKey = "beta_ideas" | "beta_group_chat";

export type BetaFeature = {
  /** Its own settings key, default off (settings.ts). */
  key: BetaKey;
  /** The checkbox's words. */
  label: string;
  /** One line under it: what turning it on adds. */
  what: string;
  /** The checkbox's test id. */
  testId: string;
};

/** Every beta feature, in the order Settings lists them. One entry each. */
export const BETA_FEATURES: readonly BetaFeature[] = [
  {
    key: "beta_ideas",
    label: "Ideas",
    what: "Adds the Ideas window: the lightbulb in the rail, and ⌘⇧I. Still changing. Turning it off hides it and keeps what you wrote.",
    testId: "beta-ideas",
  },
  {
    key: "beta_group_chat",
    label: "Group chat",
    what: "Share a running session with a guest, talk beside it in a thread, and see what a host shares with you. Turning it off hides it and keeps every pairing and share.",
    testId: "beta-group-chat",
  },
];

/** Ideas (idea board phases 0, 1, 3 and S): the Ideas window, the rail's
 *  lightbulb, ⌘⇧I and its reopening at launch (the shim reads the same key,
 *  companion.rs `BETA`). Named workspaces left the beta on 2026-11-14. */
export const ideasBeta = (s: Pick<Settings, "beta_ideas">): boolean => s.beta_ideas === true;

/** Group chat (docs/proposals/group-chat.md, PRs #650–#664): the owner's
 *  Share bar, thread, join asks and Settings → Remote → Guests, and the
 *  guest's connections, David's Places, Shared with me, toasts and view.
 *  Surfaces outside App (main.tsx's guests, RemoteLayer) read it through
 *  betaLive.ts, which App feeds from this helper. */
export const groupChatBeta = (s: Pick<Settings, "beta_group_chat">): boolean => s.beta_group_chat === true;
