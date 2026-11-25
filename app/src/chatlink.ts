// A markdown link in a chat reply: what a click does. Pure, and with no
// imports, so the guest's views (which may not import chat.ts) share it.

/** What a click on a markdown link in a chat reply does. A reply is untrusted
 *  input, so it follows FilesPane's and PlanPane's rule: http(s) goes to the
 *  browser, any other scheme is refused in words (never silently), a bare
 *  `#anchor` is nothing (a chat has no headings), and the rest is a PATH,
 *  opened in the Files tab — whose own Open button is the step before anything
 *  runs. Never straight into another app: an `.html` an agent wrote would run.
 *  `root` is the place's folder, the agent's cwd, for a relative path. */
export type ChatLinkAction =
  | { kind: "url"; url: string }
  | { kind: "path"; path: string }
  | { kind: "refuse"; scheme: string }
  | { kind: "none" };

export function chatLinkAction(href: string, root: string): ChatLinkAction {
  const h = href.trim();
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(h);
  if (scheme) {
    return /^https?$/i.test(scheme[1]) ? { kind: "url", url: h } : { kind: "refuse", scheme: scheme[1].toLowerCase() };
  }
  if (h === "" || h.startsWith("#")) return { kind: "none" };
  let raw = h.split("#")[0].split("?")[0];
  try { raw = decodeURIComponent(raw); } catch { /* a stray % is part of the name */ }
  if (raw === "") return { kind: "none" };
  const parts = (raw.startsWith("/") ? raw : `${root}/${raw}`).split("/");
  const out: string[] = [];
  for (const p of parts) {
    if (p === "." || p === "") continue;
    if (p === "..") out.pop();
    else out.push(p);
  }
  return { kind: "path", path: `/${out.join("/")}` };
}
