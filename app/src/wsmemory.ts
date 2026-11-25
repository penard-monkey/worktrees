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
/** The file a chat's note is, in the memory (`chats.json` names it from the
 *  workspace's folder). */
export const notePath = (note: string): string => note.replace(/^memory\//, "");
/** The files one change shows, the index first. */
export const changeFiles = (c: MemoryChange): MemoryFileChange[] => [...c.files].sort((a, b) => Number(b.path === "MEMORY.md") - Number(a.path === "MEMORY.md"));
