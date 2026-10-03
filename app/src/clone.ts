// "Clone from URL…" — the frontend half.
//
// `cloneSource` MIRRORS `worktrees_core::clone::parse_source` (clone.rs), so the
// dialog can preview `<parent>/<name>` as you type. Core is the authority — the
// backend re-parses everything — and `app/scripts/clone-check.mjs` runs this
// function over core's own `NAME_CASES` table, so a rule changed on one side
// and not the other fails a check instead of previewing a folder the clone will
// not make.

export type CloneSource = { url: string; name: string };

/** core's `CloneProgress`: git's phase, its percentage, the rest of its line. */
export type CloneProgress = { phase: string | null; percent: number | null; detail: string | null };

/** core's `CloneErrorKind`, serialized snake_case. `clone-check.mjs` asserts
 *  this list equals the Rust enum, both ways. */
export const CLONE_ERROR_KINDS = [
  "invalid", "exists", "auth", "not_found", "host_key", "network", "cancelled", "other",
] as const;
export type CloneErrorKind = (typeof CLONE_ERROR_KINDS)[number];
export type CloneError = { kind: CloneErrorKind; message: string };

export const isCloneError = (e: unknown): e is CloneError =>
  !!e && typeof e === "object" && typeof (e as CloneError).message === "string"
  && (CLONE_ERROR_KINDS as readonly string[]).includes((e as CloneError).kind);

const SCHEMES = ["https", "http", "ssh", "git", "file", "git+ssh", "ssh+git"];

function isScpLike(s: string): boolean {
  const c = s.indexOf(":");
  return c > 0 && !s.slice(0, c).includes("/") && c + 1 < s.length;
}

function isShorthand(s: string): boolean {
  const parts = s.split("/");
  if (parts.length !== 2) return false;
  return parts.every((p) => p.length > 0 && !p.startsWith(".") && /^[A-Za-z0-9._-]+$/.test(p));
}

function deriveName(url: string): string | null {
  let path = url;
  const i = path.indexOf("://");
  if (i >= 0) {
    path = path.slice(i + 3);
    const j = path.indexOf("/");
    path = j >= 0 ? path.slice(j) : "";
  } else {
    const c = path.indexOf(":");
    if (c >= 0) path = path.slice(c + 1);
  }
  path = path.split(/[?#]/)[0] ?? "";
  const segs = path.split("/").filter((p) => p.length > 0);
  if (segs[segs.length - 1] === ".git") segs.pop();
  const last = segs[segs.length - 1];
  if (last === undefined) return null;
  const name = last.endsWith(".git") ? last.slice(0, -4) : last;
  return name ? name : null;
}

/** core's `valid_dir_name` — "" when the name is fine. */
export function dirNameProblem(n: string): string {
  if (!n) return "the folder needs a name";
  if (n === "." || n === "..") return "'.' and '..' are not names";
  if (n.startsWith(".")) return "a name starting with '.' would make a hidden folder";
  if (n.includes("/")) return "a folder name cannot contain '/'";
  // eslint-disable-next-line no-control-regex -- control chars are exactly what this rejects
  if (/[\s\u0000-\u001f\u007f]/.test(n)) return "a folder name cannot contain spaces or control characters";
  return "";
}

/** What was pasted → the source, or the sentence the dialog shows. */
export function cloneSource(input: string): CloneSource | { error: string } {
  const s = input.trim();
  if (!s) return { error: "paste a repository URL" };
  // eslint-disable-next-line no-control-regex -- control chars are exactly what this rejects
  if (/[\s\u0000-\u001f\u007f]/.test(s)) return { error: "a URL cannot contain spaces" };
  if (s.startsWith("-")) return { error: "a URL cannot start with '-'" };
  let url: string;
  const i = s.indexOf("://");
  if (i >= 0) {
    const scheme = s.slice(0, i);
    if (!SCHEMES.includes(scheme.toLowerCase()))
      return { error: `'${scheme}://' URLs are not supported — use https://, ssh:// or git@host:path` };
    url = s;
  } else if (s.includes("::")) {
    return { error: "remote-helper URLs (`helper::address`) are not supported" };
  } else if (isScpLike(s)) {
    url = s;
  } else if (isShorthand(s)) {
    url = `https://github.com/${s.endsWith(".git") ? s.slice(0, -4) : s}.git`;
  } else if (s.startsWith("/") || s.startsWith(".") || s.startsWith("~")) {
    return { error: "that is a folder on this Mac — use “Add existing…” for it" };
  } else {
    return { error: "not a git URL — paste https://…, git@host:owner/repo.git, or owner/repo" };
  }
  const name = deriveName(url);
  if (!name) return { error: "cannot tell the repository's name from that URL" };
  const bad = dirNameProblem(name);
  return bad ? { error: bad } : { url, name };
}
