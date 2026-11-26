// A sent line's attachments (chat attachments, `attach.rs`), for the GUEST's
// session view: ` [attached: <path>, …]`, the tail `attachLine` (chat.ts)
// writes. A copy of chat.ts's `splitAttached`, because the guest view may not
// reach chat.ts (guestview-check.mjs) and chat.ts may gain no import its
// checks do not stub; chatimage-check.mjs holds the two regexes equal. The
// host's guest row reads the same tail (`shareimage.rs`, `attached`).

const ATTACHED = /^(.*) \[attached: (\.worktrees-attachments\/\d{8}-[0-9a-f]{16}\.[a-z]{2,4}(?:, \.worktrees-attachments\/\d{8}-[0-9a-f]{16}\.[a-z]{2,4})*)\]$/su;

/** A sent line's attachments, split off its end so they can be drawn as
 *  files: what came before ` [attached: …]`, and the paths. Only the host's
 *  own path shape counts; anything else is all text. */
export function splitAttached(line: string): { text: string; paths: string[] } {
  const m = ATTACHED.exec(line);
  return m ? { text: m[1], paths: m[2].split(", ") } : { text: line, paths: [] };
}

/** The kind a stored file's extension says (the host chose it from the bytes). */
export const attachedKind = (path: string): "image" | "pdf" | "text" =>
  /\.(png|jpg|gif|webp)$/.test(path) ? "image" : path.endsWith(".pdf") ? "pdf" : "text";
