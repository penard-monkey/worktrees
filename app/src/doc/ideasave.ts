// When an open document (an idea, a plan) is saved, and what an
// `ideas:changed` means for it — the rules, pure, the same in both windows. No import at runtime: ideas-save-check.mjs loads this file as a
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
//   the next. **Leaving drains** (`drain()`): it saves until the file has the
//   editor's text, or says it cannot.
// - **A refused save** (the file changed on disk since `mtime`) never
//   discards the person's text: the session goes to `conflict` and stops
//   saving until they choose `reload()` (take the file's) or `keepMine()`
//   (write theirs over it, against the file's current mtime — and with
//   `keepHistory`, so the text it overwrites is copied to `.history/` first:
//   no text is ever lost, including the other editor's).
// - **`ideas:changed`** (`external`): an entry for this document, changed or
//   removed, READS the file, and its text decides. The text we last saved or
//   loaded: our own save a tick late, or a touch, and nothing to do but know
//   its mtime. Another text: an edit made elsewhere, reloaded when the editor
//   is clean, a conflict when it is not. Never the mtime alone: two programs
//   can write in one millisecond, and an event can come late.
//   An event that arrives while a write is in flight waits for its answer.
// - **A save can rename the file**: the first heading names it
//   (`ideas::write_at`), and `idea_write` answers with the idea as saved —
//   its mtime and its `file`. The move keeps the mtime, so the watcher's
//   vanish-and-appear of the old and new names reads as the text we have (and
//   a `removed` for the instant two names hold the id is confirmed by id,
//   below). The window hears the new name through
//   `onSaved`, never by reading the list.
// - **A `removed` entry** is confirmed with `idea_read` before anything
//   closes, because an editor that saves by rename makes a file vanish for an
//   instant. Found: it is a change like any other. Gone: `removed`, and the
//   text stays in the editor until the person closes it or saves it as a new
//   idea (the window's act, not this module's).

export const SAVE_PAUSE_MS = 800;
/** How many saves leaving waits for, when each meets new typing. */
export const DRAIN_ROUNDS = 20;

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
export type Saved = { mtime_ms: number; file: string; title: string; rev?: string };

/** The file as read: its name and title too, when the source gives them. */
export type Read = { content: string; mtime_ms: number; file?: string; title?: string; rev?: string };

export type Rows = {
  /** `keepHistory`: copy the file's current bytes to `.history/` before the
   *  replace. Keep mine only — the text it overwrites came from elsewhere and
   *  exists nowhere else. Never an ordinary save. */
  write(id: string, content: string, expectedMtime: number, keepHistory: boolean, expectedRev?: string): Promise<Saved>;
  read(id: string): Promise<Read>;
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
  /** The source's own refusals, when they are not the ideas' two sentences. */
  refusals?: { stale(e: string): boolean; gone(e: string): boolean };
  /** The editor's text, exactly as it would be saved. */
  text(): string;
  /** Put the file's text in the editor (a reload). */
  replace(content: string): void;
  onState(s: SaveState): void;
  /** A write of ours was answered: the file's name and title as saved (the
   *  first heading renames it). */
  onSaved?(s: Saved): void;
  /** The file's text was taken, changed elsewhere: its name and title may have
   *  moved with it (another window typed in the heading). */
  onTaken?(d: Read): void;
};

/** What moved in the source since it last said: `ideas:changed`'s payload. */
export type Changed = { changed: { id: string; mtime_ms: number; rev?: string | null }[]; removed: string[] };

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));

export class Session {
  readonly id: string;
  private base: string;
  private mtime: number;
  /** The name of `base`'s bytes, from a source whose rows give one. */
  private rev: string | undefined;
  private timer: unknown = null;
  private inflight: Promise<void> | null = null;
  private again = false;
  private pending: Changed[] = [];
  private state: SaveState = { kind: "clean", savedAt: null };
  private closed = false;

  constructor(id: string, content: string, mtime: number, private deps: SessionDeps, rev?: string) {
    this.id = id;
    this.rev = rev;
    this.base = content;
    this.mtime = mtime;
  }

  get current(): SaveState { return this.state; }
  get lastMtime(): number { return this.mtime; }
  get lastRev(): string | undefined { return this.rev; }
  /** True when the editor holds text the file does not. */
  dirty(): boolean { return this.deps.text() !== this.base; }
  /** Saving is held: a conflict, a removal or an error needs the person. */
  held(): boolean { return this.state.kind === "conflict" || this.state.kind === "removed"; }

  private stale(e: string): boolean { return (this.deps.refusals?.stale ?? isStale)(e); }
  private gone(e: string): boolean { return (this.deps.refusals?.gone ?? isGone)(e); }

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

  /** Save until the file has the editor's text, which is what leaving it needs:
   *  one flush is not that, an edit made during its write is not in it. `false`:
   *  it cannot be saved (the state says why), or it is typed in without a pause.
   *  True holds for the code that runs next with no await but this one's. */
  async drain(): Promise<boolean> {
    for (let n = 0; n < DRAIN_ROUNDS; n++) {
      await this.flush();
      if (this.held() || this.state.kind === "error") return !this.dirty();
      if (!this.dirty() && !this.inflight) return true;
    }
    if (this.timer === null) this.edited();
    return false;
  }

  private save(keepHistory = false): Promise<void> {
    if (this.closed || this.held()) return Promise.resolve();
    if (this.inflight) { this.again = true; return this.inflight; }
    const content = this.deps.text();
    if (content === this.base) return Promise.resolve();
    this.set({ kind: "saving" });
    const run = (async () => {
      try {
        const saved = await this.deps.rows.write(this.id, content, this.mtime, keepHistory, this.rev);
        const m = saved.mtime_ms;
        this.base = content;
        this.mtime = m;
        this.rev = saved.rev;
        if (!this.closed) this.deps.onSaved?.(saved);
        this.set(this.dirty() ? { kind: "dirty" } : { kind: "clean", savedAt: this.deps.clock.now() });
      } catch (e) {
        const msg = errText(e);
        this.set(this.stale(msg) ? { kind: "conflict", why: "refused" } : { kind: "error", message: msg });
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
  async external(ev: Changed): Promise<void> {
    if (this.closed) return;
    if (this.inflight) { this.pending.push(ev); return; }
    const ch = ev.changed.find((c) => c.id === this.id);
    // The bytes are named, and they are ours.
    if (ch?.rev && ch.rev === this.rev) { this.mtime = ch.mtime_ms; return; }
    if (ch || ev.removed.includes(this.id)) return this.look();
  }

  /** Something moved: our own save, a tick late, or another's edit. Only the text
   *  says which; a millisecond does not (two writes can share one). */
  private async look(): Promise<void> {
    for (;;) {
      const [base, mtime] = [this.base, this.mtime];
      let d: Read;
      try { d = await this.deps.rows.read(this.id); } catch (e) {
        if (this.closed || !this.gone(errText(e))) return;
        if (this.timer !== null) { this.deps.clock.clearTimeout(this.timer); this.timer = null; }
        this.set({ kind: "removed" });
        return;
      }
      if (this.closed || this.state.kind === "removed") return;
      // A save is out: asked again after its answer, as any event is.
      if (this.inflight) { this.pending.push({ changed: [{ id: this.id, mtime_ms: mtime }], removed: [] }); return; }
      // One landed while this read was: the text read is from before it.
      if (this.base !== base || this.mtime !== mtime) continue;
      if (d.content !== this.base) return this.elsewhere(d);
      // Touched, or ours: the next save is against the mtime it has.
      this.mtime = d.mtime_ms;
      this.rev = d.rev;
      return;
    }
  }

  /** The file has another text, and it is not ours. */
  private elsewhere(d: Read): void {
    if (this.dirty() || this.state.kind === "conflict") {
      if (this.timer !== null) { this.deps.clock.clearTimeout(this.timer); this.timer = null; }
      this.set({ kind: "conflict", why: "elsewhere" });
      return;
    }
    this.take(d);
  }

  private take(d: Read) {
    this.base = d.content;
    this.mtime = d.mtime_ms;
    this.rev = d.rev;
    if (this.deps.text() !== d.content) this.deps.replace(d.content);
    if (!this.closed) this.deps.onTaken?.(d);
    this.set({ kind: "clean", savedAt: null });
  }

  /** Conflict: take the file's text, dropping the editor's. */
  async reload(): Promise<void> {
    try { this.take(await this.deps.rows.read(this.id)); } catch (e) {
      const msg = errText(e);
      this.set(this.gone(msg) ? { kind: "removed" } : { kind: "error", message: msg });
    }
  }

  /** Conflict: write the editor's text over the file's, knowingly. */
  async keepMine(): Promise<void> {
    let d: Read;
    try { d = await this.deps.rows.read(this.id); } catch (e) {
      const msg = errText(e);
      this.set(this.gone(msg) ? { kind: "removed" } : { kind: "error", message: msg });
      return;
    }
    this.base = d.content;
    this.mtime = d.mtime_ms;
    this.rev = d.rev;
    this.set(this.dirty() ? { kind: "dirty" } : { kind: "clean", savedAt: null });
    await this.save(true);
  }

  /** The file was changed by one of the window's own acts (a rename, a restore):
   *  its new text and mtime, which are ours. Typed into while that act was out,
   *  the editor keeps the person's text and asks. */
  adopt(content: string, mtime: number, rev?: string): void {
    if (this.held()) return;
    if (this.dirty() && this.deps.text() !== content) {
      if (this.timer !== null) { this.deps.clock.clearTimeout(this.timer); this.timer = null; }
      this.set({ kind: "conflict", why: "elsewhere" });
      return;
    }
    this.take({ content, mtime_ms: mtime, rev });
  }

  /** Stop: no timer fires and no state is reported after this. */
  close(): void {
    if (this.timer !== null) { this.deps.clock.clearTimeout(this.timer); this.timer = null; }
    this.closed = true;
  }
}
