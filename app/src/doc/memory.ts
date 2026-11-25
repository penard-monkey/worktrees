// A workspace's memory as a document source (source.ts): its files are named
// by their path in the memory, read, saved and watched through the
// `workspace_memory*` rows, and its history is the folder's commits.
//
// `memoryDocs(workspaceId)` answers the SAME object for one workspace every
// time, because useDoc's effects hang on its identity. Beside what every
// source does it has what only a memory does: the folder (`view`, `make`,
// `put`, `create`), the folder's history (`log`, `change`, `diff`, `revert`),
// and how a link or an image written in one of its files resolves.
import { invoke, listen } from "../ipc";
import type { Changed, Saved } from "./ideasave";
import { isMemoryGone, isMemoryStale, memoryLink, topicPath, type MemoryLink } from "./memorypath";
import type { Doc, DocSource, Version } from "./source";

export type MemoryFile = { path: string; name: string; size: number; mtime_ms: number; kind: "text" | "image" | "other"; title: string | null };
/** The files under one top folder; `""` is the memory's own. */
export type MemoryFolder = { name: string; files: MemoryFile[] };
export type MemoryIndex = { path: string; content: string; mtime_ms: number; size: number; budget: number; over_budget: boolean };
export type MemoryFileChange = { path: string; status: string; added: number | null; removed: number | null; binary: boolean };
/** One commit. `reverted_by`: the commit that undid it, while that stands, so
 *  the row says Redo (revert THAT one) and not Undo. */
export type MemoryChange = {
  commit: string; at: number; subject: string; kind: "made" | "fold" | "edit" | "undo" | "redo";
  files: MemoryFileChange[]; added: number; removed: number; reverts: string | null; reverted_by: string | null;
};
export type MemoryView = {
  made: boolean; dir: string; index: MemoryIndex | null; folders: MemoryFolder[]; count: number; bytes: number;
  skipped: { path: string; why: string }[]; truncated: boolean; last: MemoryChange | null;
};
export type MemoryLog = { changes: MemoryChange[]; more: boolean };
/** One side of a changed image. `refused` says why there are no pixels. */
export type MemoryImageSide = { mime: string; size: number; b64: string; refused: string | null };
export type MemoryDiff =
  | { kind: "text"; patch: string; truncated: boolean }
  | { kind: "image"; before: MemoryImageSide | null; after: MemoryImageSide | null }
  | { kind: "binary" };
export type MemoryFileDiff = MemoryFileChange & { diff: MemoryDiff };
/** `conflicts`: the files that changed since; nothing was done. */
export type MemoryReverted = { change: MemoryChange | null; conflicts: string[] };
/** An image as an `<img>` takes it, or why it is not shown. */
export type MemoryImage = { src: string; size: number } | { note: string };

type File = { path: string; file: string; title: string; content: string; mtime_ms: number; size: number };
type Put = { path: string; file: string; title: string; mtime_ms: number; size: number };
type Image = { b64: string; size: number; truncated: boolean; mtime: number; mime: string; etag: string; unchanged: boolean };
type Event = { id: string; changed: { path: string; mtime_ms: number }[]; removed: string[] };

export interface MemoryDocs extends DocSource {
  readonly workspace: string;
  view(): Promise<MemoryView>;
  make(): Promise<MemoryView>;
  /** A new file; refused when there is one at `path`. */
  create(path: string, content: string): Promise<Doc>;
  /** A dropped file, put under `files/`: the path it got. */
  put(name: string, bytes: Uint8Array): Promise<Put>;
  /** 50 a page, newest first; `path` for one file's, `before` the last
   *  commit of the page before. */
  log(o?: { path?: string; before?: string; n?: number }): Promise<MemoryLog>;
  change(commit: string): Promise<MemoryChange>;
  diff(commit: string, path: string): Promise<MemoryFileDiff>;
  /** Undo a change, or redo an undo. */
  revert(commit: string): Promise<MemoryReverted>;
  /** The file as it was in `commit`. */
  at(commit: string, path: string): Promise<Doc>;
  /** What `href`, written in the file `from`, names. */
  link(from: string, href: string): MemoryLink;
  /** The image `src` names, written in the file `from`. */
  image(from: string, src: string): Promise<MemoryImage>;
}

const doc = (f: File): Doc => ({ id: f.path, title: f.title, file: f.file, mtime_ms: f.mtime_ms, content: f.content });
const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

/** Bytes as the base64 the engine reads (`btoa`'s own spelling). In chunks:
 *  one `String.fromCharCode(...bytes)` of a megabyte overflows the stack. */
function base64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** Pictures kept, by path: a re-mounted image costs one `unchanged`. */
const KEPT = 24;

function make(id: string): MemoryDocs {
  const kept = new Map<string, { etag: string; src: string; size: number }>();
  const write = (path: string, content: string, expectedMtime: number | null, keepHistory: boolean) =>
    invoke<Put>("workspace_memory_write", { id, path, content, ...(expectedMtime === null ? {} : { expectedMtime }), ...(keepHistory ? { keepHistory: true } : {}) });
  const read = async (path: string) => doc(await invoke<File>("workspace_memory_file", { id, path }));
  const at = async (commit: string, path: string) => doc(await invoke<File>("workspace_memory_file", { id, path, commit }));
  const log = (o: { path?: string; before?: string; n?: number } = {}) => invoke<MemoryLog>("workspace_memory_log", { id, ...o });
  const show = (commit: string, path?: string) => invoke<{ change: MemoryChange; file: MemoryFileDiff | null }>("workspace_memory_diff", { id, commit, ...(path === undefined ? {} : { path }) });
  const create = async (path: string, content: string): Promise<Doc> => {
    const w = await write(path, content, null, false);
    return { id: w.path, title: w.title, file: w.file, mtime_ms: w.mtime_ms, content };
  };
  return {
    workspace: id,
    noun: "file",
    refusals: { stale: isMemoryStale, gone: isMemoryGone },
    read,
    async write(path, content, expectedMtime, keepHistory): Promise<Saved> {
      const w = await write(path, content, expectedMtime, keepHistory);
      return { mtime_ms: w.mtime_ms, file: w.file, title: w.title };
    },
    watch(on) {
      let off: (() => void) | null = null;
      let gone = false;
      listen<Event>("workspaces:memory", (ev) => {
        const p = ev.payload;
        if (!p || p.id !== id) return;
        const c: Changed = { changed: (p.changed ?? []).map((x) => ({ id: x.path, mtime_ms: x.mtime_ms })), removed: p.removed ?? [] };
        on(c);
      }).then((u) => { if (gone) u(); else off = u; }).catch(() => {});
      return () => { gone = true; off?.(); };
    },
    // The folder's history as one file's versions: each commit that left the
    // file with a text. A version reads as it was AFTER that commit.
    async versions(path): Promise<Version[]> {
      const { changes } = await log({ path });
      return changes
        .filter((c) => c.files.some((f) => f.path === path && f.status !== "D"))
        .map((c) => ({ version: c.commit, at: new Date(c.at * 1000).toISOString(), title: c.subject }));
    },
    async restore(path, version, expectedMtime): Promise<Doc> {
      const old = await at(version, path);
      // What it replaces is committed first, so it is in the history too.
      let mtime: number | null = expectedMtime ?? null;
      if (mtime === null) {
        try { mtime = (await read(path)).mtime_ms; } catch (e) { if (!isMemoryGone(errText(e))) throw e; }
      }
      const w = await write(path, old.content, mtime, mtime !== null);
      return { ...old, mtime_ms: w.mtime_ms, title: w.title, file: w.file };
    },
    async keepAsNew(title, text): Promise<Doc> {
      for (let n = 1; ; n++) {
        try { return await create(topicPath(title, n), text); } catch (e) {
          if (n >= 20 || !/already a file/.test(errText(e))) throw e;
        }
      }
    },
    view: () => invoke<MemoryView>("workspace_memory", { id }),
    make: () => invoke<MemoryView>("workspace_memory_make", { id }),
    create,
    put: (name, bytes) => invoke<Put>("workspace_memory_put", { id, name, data: base64(bytes) }),
    log,
    change: async (commit) => (await show(commit)).change,
    async diff(commit, path) {
      const f = (await show(commit, path)).file;
      if (!f) throw `that change does not touch ${path}`;
      return f;
    },
    revert: (commit) => invoke<MemoryReverted>("workspace_memory_revert", { id, commit }),
    at,
    link: memoryLink,
    async image(from, src): Promise<MemoryImage> {
      const l = memoryLink(from, src);
      if (l.kind === "url") return { note: "Remote image, not loaded." };
      if (l.kind !== "file") return { note: l.kind === "none" ? l.why : "Not an image of this memory." };
      const have = kept.get(l.path);
      let r: Image;
      try { r = await invoke<Image>("workspace_memory_image", { id, path: l.path, ...(have ? { etag: have.etag } : {}) }); } catch (e) { return { note: errText(e) }; }
      if (r.unchanged && have) return { src: have.src, size: have.size };
      if (r.truncated) return { note: `${l.path} is too large to show.` };
      const pic = { etag: r.etag, src: `data:${r.mime};base64,${r.b64}`, size: r.size };
      kept.delete(l.path);
      kept.set(l.path, pic);
      while (kept.size > KEPT) kept.delete(kept.keys().next().value!);
      return { src: pic.src, size: pic.size };
    },
  };
}

const sources = new Map<string, MemoryDocs>();

/** Workspace `id`'s memory. One object a workspace. */
export function memoryDocs(id: string): MemoryDocs {
  let s = sources.get(id);
  if (!s) sources.set(id, (s = make(id)));
  return s;
}
