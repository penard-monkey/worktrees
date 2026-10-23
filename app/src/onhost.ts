// The path-taking plugin calls, behind the one predicate that says whether
// they mean anything here (Phase 2 §4.4).
//
// The opener and dialog plugins act on the machine the WINDOW runs on. On the
// host that is also the engine's machine, so a host path names the same file.
// On a window that drives a paired host it does not — a reveal, an open, or a
// folder picked here and handed to a host command would each act on the wrong
// machine — and a browser page (the dev stack) has no plugins at all. So every
// such call goes through here: it runs when `windowOnHost()` says the window
// is on its engine's host, and otherwise is refused WITH the reason, never
// silently; and `hostOnly(what)` greys the button that would make it, with the
// same reason as its title. `openUrl` is not here: a URL means the same on
// every machine. onhost-check.mjs fails on any other file importing these.
import { open as dialogOpen } from "@tauri-apps/plugin-dialog";
import { openPath as openerOpenPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { notOnHost, windowOnHost } from "./ipc";

export { windowOnHost };

/** Props that grey a button whose act needs the host's paths: disabled, with
 *  the reason as its title. Spread them in; nothing when the window is on its
 *  host (the button's own title, if any, then stands). */
export function hostOnly(what: string): { disabled?: boolean; title?: string } {
  return windowOnHost() ? {} : { disabled: true, title: notOnHost(what) };
}

/** Reveal a HOST path in the file manager. */
export function revealOnHost(path: string): Promise<void> {
  return windowOnHost() ? revealItemInDir(path) : Promise.reject(notOnHost("Reveal in Finder"));
}

/** Open a HOST path with its default app. */
export function openOnHost(path: string): Promise<void> {
  return windowOnHost() ? openerOpenPath(path) : Promise.reject(notOnHost("Open"));
}

/** Pick a folder for a HOST command. `null` when the person cancels. */
export async function pickDirForHost(title: string): Promise<string | null> {
  if (!windowOnHost()) throw notOnHost("Choosing a folder");
  const d = await dialogOpen({ directory: true, title });
  return typeof d === "string" ? d : null;
}
