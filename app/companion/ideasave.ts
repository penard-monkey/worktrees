// When an open idea is saved, and what an `ideas:changed` means for it — the
// rules, pure. No import at runtime: ideas-save-check.mjs loads this file as a
// `data:` module and drives it on a VIRTUAL clock with scripted rows, because
// a test that races two sleeps asserts the scheduler (AGENTS.md).
//
// One `Session` per open idea. It knows the text it last saved or loaded
// (`base`) and the file's mtime then; the editor's text is asked for
// (`deps.text()`), never copied in on every keystroke.
//
// - **Autosave** after `SAVE_PAUSE_MS` without an edit, and at once on
//   `flush()` (the editor's blur, switching ideas, the window hiding or
//   closing). A text equal to `base` writes NOTHING: no `idea_write` is sent.
//   One write at a time; an edit made while one is in flight is saved by
//   the next.
// - **A refused save** (the file changed on disk since `mtime`) never
//   discards the person's text: the session goes to `conflict` and stops
//   saving until they choose `reload()` (take the file's) or `keepMine()`
//   (write theirs over it, against the file's current mtime — and with
//   `keepHistory`, so the text it overwrites is copied to `.history/` first:
//   no text is ever lost, including the other editor's).
// - **`ideas:changed`** (`external`): a `changed` entry for this idea whose
//   mtime is one a write of OURS returned is our own save, a tick late.
//   ANY mtime ours ever got, not only the last: a tick that fell between
//   two of our saves reports the first. Anything else is an edit made
//   elsewhere: reloaded when the editor is clean, a conflict when it is not.
//   An event that arrives while a write is in flight waits for its answer —
//   until then the mtime it carries may be that write's.
// - **A save can rename the file**: the first heading names it
//   (`ideas::write_at`), and `idea_write` answers with the idea as saved —
//   its mtime and its `file`. The move keeps the mtime, so the watcher's
//   vanish-and-appear of the old and new names carries an mtime that is
//   already ours (and a `removed` for the instant two names hold the id is
//   confirmed by id, below). The window hears the new name through
//   `onSaved`, never by reading the list.
// - **A `removed` entry** is confirmed with `idea_read` before anything
//   closes, because an editor that saves by rename makes a file vanish for an
//   instant. Found: it is a change like any other. Gone: `removed`, and the
//   text stays in the editor until the person closes it or saves it as a new
//   idea (the window's act, not this module's).

export const SAVE_PAUSE_MS = 800;

/** The refusal `idea_write` gives when `expected_mtime` no longer matches. */
export const isStale = (e: string) => /changed on disk/.test(e);
/** `idea_read`'s answer for an id no file carries. */
export const isGone = (e: string) => /no idea has the id/.test(e);

export type Clock = {
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(h: unknown): void;
  now(): number;
};

/** `idea_write`'s answer: the idea as saved, without its text. */
export type Saved = { mtime_ms: number; file: string; title: string };

export type Rows = {
  /** `keepHistory`: copy the file's current bytes to `.history/` before the
   *  replace. Keep mine only — the text it overwrites came from elsewhere and
   *  exists nowhere else. Never an ordinary save. */
  write(id: string, content: string, expectedMtime: number, keepHistory: boolean): Promise<Saved>;
  read(id: string): Promise<{ content: string; mtime_ms: number }>;
};

export type SaveState =
  | { kind: "clean"; savedAt: number | null }
  | { kind: "dirty" }
  | { kind: "saving" }
  /** `why`: our save was refused, or an edit elsewhere met unsaved text. */
  | { kind: "conflict"; why: "refused" | "elsewhere" }
  | { kind: "removed" }
  | { kind: "error"; message: string };

export type SessionDeps = {
  rows: Rows;
  clock: Clock;
  /** The editor's text, exactly as it would be saved. */
  text(): string;
  /** Put the file's text in the editor (a reload). */
  replace(content: string): void;
  onState(s: SaveState): void;
  /** A write of ours was answered: the file's name and title as saved (the
   *  first heading renames it). */
  onSaved?(s: Saved): void;
};

export type IdeasChanged = { changed: { id: string; mtime_ms: number }[]; removed: string[] };

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

export class Session {
  readonly id: string;
  private base: string;
  private mtime: number;
  /** Every mtime a write of ours was answered with. */
  private own = new Set<number>();
  private timer: unknown = null;
  private inflight: Promise<void> | null = null;
  private again = false;
  private pending: IdeasChanged[] = [];
  private state: SaveState = { kind: "clean", savedAt: null };
  private closed = false;

  constructor(id: string, content: string, mtime: number, private deps: SessionDeps) {
    this.id = id;
    this.base = content;
    this.mtime = mtime;
  }

  get current(): SaveState { return this.state; }
  get lastMtime(): number { return this.mtime; }
  /** True when the editor holds text the file does not. */
  dirty(): boolean { return this.deps.text() !== this.base; }
  /** Saving is held: a conflict, a removal or an error needs the person. */
  held(): boolean { return this.state.kind === "conflict" || this.state.kind === "removed"; }

  private set(s: SaveState) {
    this.state = s;
    if (!this.closed) this.deps.onState(s);
  }

  /** The editor's text changed (typing, not a reload). */
  edited(): void {
    if (this.closed || this.held()) return;
    if (this.timer !== null) this.deps.clock.clearTimeout(this.timer);
    this.timer = null;
    if (!this.dirty()) {
      if (!this.inflight) this.set({ kind: "clean", savedAt: this.state.kind === "clean" ? this.state.savedAt : null });
      return;
    }
    if (this.state.kind !== "saving") this.set({ kind: "dirty" });
    this.timer = this.deps.clock.setTimeout(() => { this.timer = null; void this.save(); }, SAVE_PAUSE_MS);
  }

  /** Save now, if there is anything to save. */
  flush(): Promise<void> {
    if (this.timer !== null) { this.deps.clock.clearTimeout(this.timer); this.timer = null; }
    return this.save();
  }

  private save(keepHistory = false): Promise<void> {
    if (this.closed || this.held()) return Promise.resolve();
    if (this.inflight) { this.again = true; return this.inflight; }
    const content = this.deps.text();
    if (content === this.base) return Promise.resolve();
    this.set({ kind: "saving" });
    const run = (async () => {
      try {
        const saved = await this.deps.rows.write(this.id, content, this.mtime, keepHistory);
        const m = saved.mtime_ms;
        this.base = content;
        this.mtime = m;
        this.own.add(m);
        if (!this.closed) this.deps.onSaved?.(saved);
        this.set(this.dirty() ? { kind: "dirty" } : { kind: "clean", savedAt: this.deps.clock.now() });
      } catch (e) {
        const msg = errText(e);
        this.set(isStale(msg) ? { kind: "conflict", why: "refused" } : { kind: "error", message: msg });
      }
    })();
    this.inflight = run.finally(() => {
      this.inflight = null;
      const queued = this.pending.splice(0);
      const next = this.again && this.state.kind !== "error";
      this.again = false;
      return (async () => {
        for (const ev of queued) await this.external(ev);
        if (next) await this.save();
        // An edit made during the write, and not saved by `again`, waits for
        // the pause like any other.
        else if (this.state.kind === "dirty" && this.timer === null) this.edited();
      })();
    });
    return this.inflight;
  }

  /** An `ideas:changed` payload. Resolves once it has been acted on. */
  async external(ev: IdeasChanged): Promise<void> {
    if (this.closed) return;
    if (this.inflight) { this.pending.push(ev); return; }
    const ch = ev.changed.find((c) => c.id === this.id);
    if (ch) {
      if (this.own.has(ch.mtime_ms) || ch.mtime_ms === this.mtime) return;
      return this.elsewhere();
    }
    if (ev.removed.includes(this.id)) {
      try {
        const d = await this.deps.rows.read(this.id);
        if (this.own.has(d.mtime_ms) || d.mtime_ms === this.mtime) return;
        return this.elsewhere(d);
      } catch (e) {
        if (isGone(errText(e))) {
          if (this.timer !== null) { this.deps.clock.clearTimeout(this.timer); this.timer = null; }
          this.set({ kind: "removed" });
        }
      }
    }
  }

  /** The file changed and the change is not ours. */
  private async elsewhere(known?: { content: string; mtime_ms: number }): Promise<void> {
    if (this.state.kind === "removed") return;
    if (this.dirty() || this.state.kind === "conflict") {
      if (this.timer !== null) { this.deps.clock.clearTimeout(this.timer); this.timer = null; }
      this.set({ kind: "conflict", why: "elsewhere" });
      return;
    }
    let d = known;
    if (!d) {
      try { d = await this.deps.rows.read(this.id); } catch (e) {
        if (isGone(errText(e))) this.set({ kind: "removed" });
        return;
      }
    }
    // Typing may have begun while the read was out.
    if (this.dirty()) { this.set({ kind: "conflict", why: "elsewhere" }); return; }
    this.take(d);
  }

  private take(d: { content: string; mtime_ms: number }) {
    this.base = d.content;
    this.mtime = d.mtime_ms;
    if (this.deps.text() !== d.content) this.deps.replace(d.content);
    this.set({ kind: "clean", savedAt: null });
  }

  /** Conflict: take the file's text, dropping the editor's. */
  async reload(): Promise<void> {
    try { this.take(await this.deps.rows.read(this.id)); } catch (e) {
      const msg = errText(e);
      this.set(isGone(msg) ? { kind: "removed" } : { kind: "error", message: msg });
    }
  }

  /** Conflict: write the editor's text over the file's, knowingly. */
  async keepMine(): Promise<void> {
    let d: { content: string; mtime_ms: number };
    try { d = await this.deps.rows.read(this.id); } catch (e) {
      const msg = errText(e);
      this.set(isGone(msg) ? { kind: "removed" } : { kind: "error", message: msg });
      return;
    }
    this.base = d.content;
    this.mtime = d.mtime_ms;
    this.set(this.dirty() ? { kind: "dirty" } : { kind: "clean", savedAt: null });
    await this.save(true);
  }

  /** The file was changed by one of the window's own acts (a rename): its
   *  new text and mtime, which are ours. */
  adopt(content: string, mtime: number): void {
    this.own.add(mtime);
    this.take({ content, mtime_ms: mtime });
  }

  /** Stop: no timer fires and no state is reported after this. */
  close(): void {
    if (this.timer !== null) { this.deps.clock.clearTimeout(this.timer); this.timer = null; }
    this.closed = true;
  }
}
