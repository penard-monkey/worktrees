// Where a document comes from: the rows that read it, save it and keep its
// history. The pane (DocumentPane.tsx, useDoc.ts) knows this interface and no
// row, so a third source (a workspace's memory file, by workspace and path:
// workspace-chats.md §4.5) is one more object here and nothing else.
//
// A source is a FAMILY of documents, and a document is named in it by one
// string: an idea's id, or for a memory source bound to its workspace, a path.
import { invoke, listen } from "../ipc";
import type { DocKind } from "./docmode";
import type { HandDoc } from "./handoff";
import type { Changed, Saved } from "./ideasave";
import { ideaHref, readableLinks, type Row } from "./ideasrelate";
import { bodyOf, withId } from "./ideatext";

/** A document as its source reads it. */
export type Doc = {
  id: string;
  title: string;
  /** The file's name, for a person. */
  file: string;
  mtime_ms: number;
  content: string;
  kind?: DocKind;
  /** The id asked for was merged into this one. */
  merged?: { id: string; title: string | null; date: string | null };
};
/** `size` is the copy's bytes, where the source knows it (a memory's versions
 *  are commits, and a commit's list does not say). */
export type Version = { version: string; at: string; size?: number; title: string | null };

/** One document, wherever it comes from: its source, and its name in it. */
export type DocImage = { src: string; size: number } | { note: string };
export type DocTarget = { source: DocSource; name: string };
export const sameDoc = (a: DocTarget | null, b: DocTarget | null): boolean =>
  !!a && !!b && a.source === b.source && a.name === b.name;

export interface DocSource {
  /** What one is called: "This idea changed on disk…". */
  readonly noun: string;
  read(id: string): Promise<Doc>;
  /** Save, refused when the file is no longer at `expectedMtime`.
   *  `keepHistory`: copy what is being overwritten to history first. */
  write(id: string, content: string, expectedMtime: number, keepHistory: boolean): Promise<Saved>;
  /** Hear what changed under the source's root. Returns the unlisten. */
  watch(on: (ev: Changed) => void): () => void;
  /** Where a text that is overwritten on purpose (Keep mine) goes first. */
  readonly keeps: string;
  /** One document's own history, for the pane's panel. A source whose history
   *  is its folder's (memory) leaves both out and hands the pane a panel of its
   *  own (DocumentPane's `history`). */
  versions?(id: string): Promise<Version[]>;
  restore?(id: string, version: string, expectedMtime: number | undefined): Promise<Doc>;
  /** The file's name from a title, when the source can. */
  rename?(id: string, title: string, expectedMtime: number): Promise<Doc>;
  /** A removed document's text, kept as a new one. */
  keepAsNew?(title: string, text: string): Promise<Doc>;
  /** The text as Read draws it (the frontmatter is the file's, not the prose). */
  readBody(text: string): string;
  /** The document a link written in `from` points at, in this source or
   *  another. `null`: not one, and nothing happens. */
  target(from: string, href: string): DocTarget | null;
  /** An image written in `from`, as an `<img>` takes it, or why it is not
   *  shown. Left out: its words are drawn instead. */
  image?(from: string, src: string): Promise<DocImage>;
  /** The refusals this source's rows give, when they are not the ideas'
   *  (ideasave.ts `isStale`, `isGone`). */
  readonly refusals?: { stale(e: string): boolean; gone(e: string): boolean };
}

/** Ideas and plans: one folder, one set of rows, named by id. */
export const ideaDocs: DocSource = {
  noun: "idea",
  keeps: ".history",
  // `idea:` links become `#idea:` ones, which the shared renderer links.
  readBody: (text) => readableLinks(bodyOf(text)),
  target(_from, href) {
    const id = ideaHref(href);
    return id ? { source: ideaDocs, name: id } : null;
  },
  read: (id) => invoke<Doc>("idea_read", { id }),
  write: (id, content, expectedMtime, keepHistory) =>
    invoke<Saved>("idea_write", keepHistory ? { id, content, expectedMtime, keepHistory: true } : { id, content, expectedMtime }),
  watch(on) {
    let off: (() => void) | null = null;
    let gone = false;
    listen<Changed>("ideas:changed", (ev) => {
      on({ changed: ev.payload?.changed ?? [], removed: ev.payload?.removed ?? [] });
    }).then((u) => { if (gone) u(); else off = u; }).catch(() => {});
    return () => { gone = true; off?.(); };
  },
  versions: (id) => invoke<Version[]>("idea_versions", { id }),
  restore: (id, version, expectedMtime) => invoke<Doc>("idea_restore", { id, version, expectedMtime }),
  rename: (id, title, expectedMtime) => invoke<Doc>("idea_rename", { id, title, expectedMtime }),
  // Two rows, each the ordinary one: create, then write the text with the new id.
  async keepAsNew(title, text) {
    const d = await invoke<Doc>("idea_create", { title });
    const content = withId(text, d.id) ?? d.content;
    const w = await invoke<Saved>("idea_write", { id: d.id, content, expectedMtime: d.mtime_ms });
    return { ...d, ...w, content };
  },
};

/** An idea or a plan, as the other window is asked for it. */
export const ideaHand = (id: string): HandDoc => ({ kind: "idea", id });

/** What the pane needs to know of the other documents: the `[[` picker's rows. */
export type DocRows = readonly (Row & { kind?: "idea" | "plan" })[];
