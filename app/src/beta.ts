// Beta features: code that ships in a release but stays out of sight until
// the person turns it on in Settings → General → Beta features.
//
// Each feature is ONE per-viewer boolean in `ui-state.json` (settings.ts),
// default off, and ONE helper here. Everything that draws or opens the
// feature asks its helper, never the settings key itself
// (`beta-check.mjs` holds App.tsx to that). The engine is never gated: its
// rows are invisible until something on screen calls them, and a second copy
// of the rule there could only disagree with this one.
//
// The next beta feature is one more key, one more helper below, and one more
// row in the Settings section (pages/general.tsx, `beta` in settingsMap.ts).
// This is a local setting, not a registry and not remote config.
import type { Settings } from "./settings";

/** Ideas and workspaces (idea board phases 0–2): the Ideas window (the rail's
 *  lightbulb, ⌘⇧I, its reopening at launch) and the nav's named workspaces
 *  (the All projects | Workspaces switch, every workspace menu item, and the
 *  "All projects" wording that came with them). */
export const ideasBeta = (s: Pick<Settings, "beta_ideas">): boolean => s.beta_ideas === true;
