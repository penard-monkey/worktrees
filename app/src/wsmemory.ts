/** A workspace's memory on screen: the RULES, pure (workspace-view-design.md
 *  §4.4, §4.5). `scripts/wsmemory-check.mjs` loads this file alone. */
import type { MemoryChange, MemoryFile, MemoryFileChange, MemoryFolder, MemoryView } from "./doc/memory";

/** One part of the file list: the memory's own files, a folder, or a month of
 *  notes. `open`: how it starts. */
export type FileGroup = { key: string; name: string; files: MemoryFile[]; open: boolean };
/** Notes are folded by month from this many. */
export const MONTHS_FROM = 13;
/** A folder with more than this starts folded. */
export const FOLD_FROM = 40;

const DATED = /^(\d{4})-(\d{2})-\d{2}/;
const monthName = (ym: string): string => {
  const [y, m] = ym.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, 15)).toLocaleString([], { month: "long", year: "numeric", timeZone: "UTC" });
};
/** The month a note is of: its name's date, else when it last changed. */
const monthOf = (f: MemoryFile): string => {
  const m = DATED.exec(f.name);
  if (m) return `${m[1]}-${m[2]}`;
  return new Date(f.mtime_ms).toISOString().slice(0, 7);
};
/** How a file is called in the list: a note by its heading, anything else by its name. */
export const fileName = (f: MemoryFile): string => (f.path.startsWith("notes/") && f.title ? f.title : f.name);

const ORDER = ["", "notes", "topics", "files"];
const rank = (n: string) => { const i = ORDER.indexOf(n); return i < 0 ? ORDER.length : i; };
const newest = (a: MemoryFile, b: MemoryFile) => (a.name < b.name ? 1 : a.name > b.name ? -1 : 0);

/** The list: the index and what is beside it, then notes (the newest first),
 *  topics, files, and any other folder by name. A filter keeps what it names,
 *  everything open. */
export function fileGroups(folders: readonly MemoryFolder[], filter = ""): FileGroup[] {
  const q = filter.trim().toLowerCase();
  const hit = (f: MemoryFile) => !q || f.path.toLowerCase().includes(q) || (f.title ?? "").toLowerCase().includes(q);
  const out: FileGroup[] = [];
  for (const d of [...folders].sort((a, b) => rank(a.name) - rank(b.name) || (a.name < b.name ? -1 : 1))) {
    const files = d.files.filter(hit);
    if (!files.length) continue;
    if (d.name === "") {
      out.push({ key: "", name: "", open: true, files: [...files].sort((a, b) => Number(b.path === "MEMORY.md") - Number(a.path === "MEMORY.md") || (a.name < b.name ? -1 : 1)) });
    } else if (d.name === "notes" && !q && files.length >= MONTHS_FROM) {
      const months = new Map<string, MemoryFile[]>();
      for (const f of [...files].sort(newest)) months.set(monthOf(f), [...(months.get(monthOf(f)) ?? []), f]);
      [...months].sort((a, b) => (a[0] < b[0] ? 1 : -1)).forEach(([ym, fs], i) => out.push({ key: `notes/${ym}`, name: `Notes · ${monthName(ym)}`, files: fs, open: i === 0 }));
    } else {
      out.push({ key: d.name, name: d.name, open: !!q || files.length < FOLD_FROM, files: d.name === "notes" ? [...files].sort(newest) : files });
    }
  }
  return out;
}

export const sizeWords = (n: number): string => (n < 1024 ? `${n} B` : n < 1024 * 1024 ? `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} KB` : `${(n / 1024 / 1024).toFixed(1)} MB`);
export const fileOf = (v: MemoryView | null | undefined, path: string): MemoryFile | null => v?.folders.flatMap((d) => d.files).find((f) => f.path === path) ?? null;
/** The other sentence over an index past its budget. */
export const longIndex = (v: MemoryView): string | null =>
  v.index?.over_budget ? `The index is long (${sizeWords(v.index.size)}). Every chat reads all of it.` : null;

/** Who made a change, and what it is called. */
export function changeWords(c: MemoryChange): { by: string; name: string } {
  switch (c.kind) {
    case "made": return { by: "made", name: "Memory made" };
    case "fold": return { by: "fold", name: c.subject.replace(/^fold: /, "") };
    case "undo": return { by: "undo", name: c.subject.replace(/^Undo: (fold: )?/, "") };
    case "redo": return { by: "redo", name: c.subject.replace(/^Redo: (fold: )?/, "") };
    default: return { by: "edit", name: c.subject };
  }
}
/** "Changed … by the fold of X". */
export function changedBy(c: MemoryChange): string {
  const { by, name } = changeWords(c);
  switch (by) {
    case "made": return "Made";
    case "fold": return `Changed by the fold of “${name}”`;
    case "undo": return `“${name}” was undone`;
    case "redo": return `“${name}” was put back`;
    default: return /^Chat closed: |^Fold stopped: |^Before /.test(name) ? `Changed: ${name}` : "Edited by hand";
  }
}
/** What a change's Undo reverts: itself, or, undone already, what undid it. */
export const revertOf = (c: MemoryChange): { commit: string; redo: boolean } | null =>
  c.kind === "made" ? null : c.reverted_by ? { commit: c.reverted_by, redo: true } : { commit: c.commit, redo: false };
export const STATUS_WORDS: Record<string, string> = { A: "new", D: "removed", M: "", R: "renamed" };
export const filesWords = (n: number): string => `${n} file${n === 1 ? "" : "s"}`;
/** A revert that was refused. */
export const conflictWords = (files: readonly string[]): string =>
  `It was not undone: ${files.length === 1 ? `${files[0]} has` : `${files.slice(0, -1).join(", ")} and ${files[files.length - 1]} have`} changed since. Nothing was touched.`;

/** A memory's dot: something changed since it was looked at here. A memory that
 *  was only made is nothing to look at. */
export const unseen = (last: MemoryChange | null, seen: string | null): boolean => !!last && last.kind !== "made" && last.commit !== seen;

/** The trail of files read in one memory, for ‹ ›. */
export type Trail = { at: number; paths: string[] };
export const trailStart = (): Trail => ({ at: -1, paths: [] });
export function trailTo(t: Trail, path: string): Trail {
  if (t.paths[t.at] === path) return t;
  const paths = [...t.paths.slice(0, t.at + 1), path].slice(-50);
  return { at: paths.length - 1, paths };
}
export const trailStep = (t: Trail, by: -1 | 1): Trail | null => (t.paths[t.at + by] === undefined ? null : { at: t.at + by, paths: t.paths });
// ── editing (slice 5) ───────────────────────────────────────────────────────
const label = (t: string): string => (t.trim() || "Untitled").replace(/\\/g, "\\\\").replace(/\[/g, "\\[").replace(/\]/g, "\\]");
const isImage = (f: MemoryFile) => f.kind === "image";
const hintOf = (f: MemoryFile): string => (isImage(f) ? "image" : f.path.startsWith("notes/") ? "note" : f.path.startsWith("topics/") ? "topic" : "file");
/** The markdown a link to a memory file is: plain, relative to the file it is
 *  written in; an image is drawn. `link` is memorypath's `linkTo`. */
export const linkText = (from: string, f: MemoryFile, link: (from: string, to: string) => string): string =>
  `${isImage(f) ? "!" : ""}[${label(isImage(f) ? f.name.replace(/\.[^.]+$/, "") : f.title ?? f.name)}](${link(from, f.path)})`;
export type MemoryPick = { id: string; title: string; hint: string; text: string };
/** What `[[` offers in a memory file: its files first (the best match first),
 *  then the workspace's linked ideas. At most ten. */
export function memoryPicks(query: string, from: string, v: MemoryView | null | undefined, ideas: readonly { id: string; title: string }[], link: (from: string, to: string) => string): MemoryPick[] {
  const q = query.trim().toLowerCase();
  const score = (t: string) => (t.toLowerCase().startsWith(q) ? 0 : 1);
  const files = (v?.folders ?? []).flatMap((d) => d.files)
    .filter((f) => f.path !== from && (!q || f.path.toLowerCase().includes(q) || (f.title ?? "").toLowerCase().includes(q)))
    .sort((a, b) => score(fileName(a)) - score(fileName(b)) || newest(a, b))
    .map((f) => ({ id: f.path, title: fileName(f), hint: hintOf(f), text: linkText(from, f, link) }));
  const linked = ideas.filter((i) => !q || i.title.toLowerCase().includes(q)).map((i) => ({ id: i.id, title: i.title, hint: "idea", text: `[${label(i.title)}](idea:${i.id})` }));
  return [...files, ...linked].slice(0, 10);
}
/** The path a new file's name asks for: a folder of its own when it has one, else
 *  under topics; a text. `null`: not a name. */
export function newFilePath(name: string, topic: (title: string) => string): string | null {
  const n = name.trim().replace(/^\/+|\/+$/g, "");
  if (!n || n.split("/").some((p) => !p || p.startsWith(".")) || /[\\\u0000-\u001f]/.test(n)) return null;
  if (!n.includes("/")) return topic(n);
  return /\.(md|markdown|txt|csv|json|ya?ml|toml)$/i.test(n) ? n : `${n}.md`;
}
export const titleOf = (path: string): string => path.slice(path.lastIndexOf("/") + 1).replace(/\.[^.]+$/, "").replace(/[-_]+/g, " ").replace(/^./, (c) => c.toUpperCase());
/** A file dropped in: 4 MB at most, as the engine has it. */
export const PUT_MAX = 4 * 1024 * 1024;
export const tooBig = (name: string, size: number): string | null => (size > PUT_MAX ? `${name} is ${sizeWords(size)}: a memory takes files up to 4 MB.` : null);
/** A pasted image has no name of its own worth keeping. */
export const putName = (name: string, type: string, now: Date): string =>
  /^image\.\w+$/.test(name) || !name ? `pasted-${now.toISOString().slice(0, 19).replace(/[T:]/g, "-")}.${(type.split("/")[1] ?? "png").replace("svg+xml", "svg").replace("jpeg", "jpg")}` : name.replace(/[\\/\u0000-\u001f]/g, "-").replace(/^\.+/, "") || "file";

/** Who else may write the open file. */
export function mayEdit(live: readonly string[]): string | null {
  if (!live.length) return null;
  return live.length === 1 ? `“${live[0]}” is live and may edit this file too.` : `${live.length} chats are live and may edit this file too.`;
}
/** It changed on disk with nothing unsaved here. No event says by whom: a chat is
 *  named as live, never as the one that did it. */
export const changedUnderWords = (live: readonly string[]): string =>
  `Changed on disk.${live.length === 1 ? ` “${live[0]}” is live.` : live.length ? ` ${live.length} chats are live.` : ""}`;

export type DiffLine = { kind: "ctx" | "del" | "add" | "gap"; no: number | null; text: string };
/** Two texts as lines, what is between their common head and tail by longest
 *  common subsequence (a block too large for that is all removed, all added), with
 *  three lines of context. */
export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before.split(/\r?\n/), b = after.split(/\r?\n/);
  let s = 0;
  while (s < a.length && s < b.length && a[s] === b[s]) s++;
  let ea = a.length, eb = b.length;
  while (ea > s && eb > s && a[ea - 1] === b[eb - 1]) { ea--; eb--; }
  const A = a.slice(s, ea), B = b.slice(s, eb);
  const mid: DiffLine[] = [];
  if (A.length * B.length > 1_000_000) {
    A.forEach((t, i) => mid.push({ kind: "del", no: s + i + 1, text: t }));
    B.forEach((t, i) => mid.push({ kind: "add", no: s + i + 1, text: t }));
  } else {
    const w = B.length + 1, L = new Uint32Array((A.length + 1) * w);
    for (let i = A.length - 1; i >= 0; i--) for (let j = B.length - 1; j >= 0; j--) L[i * w + j] = A[i] === B[j] ? L[(i + 1) * w + j + 1] + 1 : Math.max(L[(i + 1) * w + j], L[i * w + j + 1]);
    for (let i = 0, j = 0; i < A.length || j < B.length;) {
      if (i < A.length && j < B.length && A[i] === B[j]) { mid.push({ kind: "ctx", no: s + j + 1, text: B[j] }); i++; j++; }
      else if (j >= B.length || (i < A.length && L[(i + 1) * w + j] >= L[i * w + j + 1])) { mid.push({ kind: "del", no: s + i + 1, text: A[i] }); i++; }
      else { mid.push({ kind: "add", no: s + j + 1, text: B[j] }); j++; }
    }
  }
  if (!mid.length) return [];
  const head: DiffLine[] = b.slice(Math.max(0, s - 3), s).map((t, i) => ({ kind: "ctx", no: Math.max(0, s - 3) + i + 1, text: t }));
  const tail: DiffLine[] = b.slice(eb, eb + 3).map((t, i) => ({ kind: "ctx", no: eb + i + 1, text: t }));
  // A long unchanged run between two changes is one gap.
  const out: DiffLine[] = [...head];
  for (let i = 0; i < mid.length;) {
    let j = i;
    while (j < mid.length && mid[j].kind === "ctx") j++;
    if (j - i > 8) { out.push(...mid.slice(i, i + 3), { kind: "gap", no: null, text: "" }, ...mid.slice(j - 3, j)); i = j; }
    else if (j > i) { out.push(...mid.slice(i, j)); i = j; }
    else { out.push(mid[i]); i++; }
  }
  return [...out, ...tail];
}

/** The file a chat's note is, in the memory (`chats.json` names it from the
 *  workspace's folder). */
export const notePath = (note: string): string => note.replace(/^memory\//, "");
/** The files one change shows, the index first. */
export const changeFiles = (c: MemoryChange): MemoryFileChange[] => [...c.files].sort((a, b) => Number(b.path === "MEMORY.md") - Number(a.path === "MEMORY.md"));
