// The mock's stand-in for a workspace's memory (`worktrees_api::memory`): the
// files, the commits, and the poll's event a tick after any change, our own
// saves included. The rules are core's, simplified. Every workspace the mock
// knows HAS one memory, seeded the first time it is asked for, except where
// `?memory=none` says none has (Make, then, makes it).
//
// `?memtick=<ms>` is the poll's tick (300; the real one is 3 s) and
// `?memquiet=<ms>` the quiet minute (60000).
type F = { text: string | null; b64: string | null; mtime: number };
type Files = Map<string, F>;
type C = { commit: string; at: number; subject: string; kind: string; before: Map<string, F>; after: Map<string, F>; reverts: string | null; live: string[] };
type Mem = { name: string; files: Files; commits: C[]; seen: Map<string, string>; tick: number | null; quiet: number | null };
type Emit = (event: string, payload: unknown) => void;

const Q = new URLSearchParams(location.search);
const TICK = Number(Q.get("memtick") ?? 300);
const QUIET = Number(Q.get("memquiet") ?? 60_000);
const mems = new Map<string, Mem>();
const writes: Record<string, unknown>[] = [];
let seq = 0;
let clock = 0;
const now = () => (clock = Math.max(clock + 1, Date.now()));
const sha = () => (++seq).toString(16).padStart(8, "0").repeat(5);
const name = (p: string) => p.split("/").pop()!;
const ext = (p: string) => /\.([^./]+)$/.exec(p)?.[1]?.toLowerCase() ?? "";
const IMAGES: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", bmp: "image/bmp", svg: "image/svg+xml" };
const kind = (p: string) => (IMAGES[ext(p)] ? "image" : ["md", "markdown", "txt", "csv", "json", "yaml", "yml", "toml"].includes(ext(p)) ? "text" : "other");
const title = (t: string | null) => (t === null ? null : /^ {0,3}# +(.+?)\s*$/m.exec(t)?.[1]) ?? null;
const size = (f: F) => (f.text !== null ? new TextEncoder().encode(f.text).length : Math.floor((f.b64!.length * 3) / 4));
const bad = (p: unknown) => typeof p !== "string" || !p || p.startsWith("/") || /[\\\u0000-\u001f]/.test(p) || p.split("/").some((x) => !x || x.startsWith("."));
const same = (a: F | undefined, b: F | undefined) => a?.text === b?.text && a?.b64 === b?.b64;

const TEMPLATE = (ws: string) => `# Memory: ${ws}\n\n## What this workspace is for\n\n## Projects\n\n## Decisions\n\n## How we work here\n\n## Open\n`;

/** A rack drawn, so the image is one a person can see. */
const rack = (led: string) => btoa(
  `<svg xmlns="http://www.w3.org/2000/svg" width="240" height="160" viewBox="0 0 240 160"><rect x="30" y="10" width="180" height="140" rx="6" fill="#39424e"/>` +
  [0, 1, 2, 3].map((i) => `<rect x="42" y="${22 + i * 31}" width="156" height="24" rx="3" fill="#59636f"/><circle cx="186" cy="${34 + i * 31}" r="4" fill="${i === 2 ? led : "#7ccf8a"}"/>`).join("") + `</svg>`,
);

const INDEX = (ws: string, o: { naming?: string; storage?: string; edited?: boolean }) => `# Memory: ${ws}

## What this workspace is for

The media box and the two apps that run on it.

## Projects

- **worktrees**: the app everything is built with.
- **the media box**: compose files and backups.${o.storage ? " See [NAS layout](topics/nas-layout.md)." : ""}

## Decisions

${o.naming ? `- Roles are named for what they read. → [note](${o.naming})\n` : ""}${o.storage ? `- Media lives on the NAS, never in a container. → [note](${o.storage})\n` : ""}${o.edited ? "- Deploys go out on Fridays. See [how we work](notes/2026-11-19-how-we-work.md).\n" : ""}
## How we work here

One place per branch. Work is proposed in the plan.

## Open

- Which disks go in the second shelf.
`;
const NAS = (shelf: string, note: string) => `# NAS layout

![The rack](../files/rack.svg)

| Path | What |
|---|---|
| \`/volume1/media\` | Films and series, read only to guests |
| \`/volume1/backup\` | Nightly, kept thirty days |

Second shelf: ${shelf}. Decided in [storage layout](../${note}).
`;
const NOTE = (t: string, asked: string, decided: string) => `# ${t}

## Asked

${asked}

## Decided

${decided}

## Done

Written down here and in the index.

## Open

Nothing.
`;

/** Where a chat of that title keeps its note, in the memory. */
type NoteFor = (title: string) => string | null;

/** Made, a fold, its undo, a second fold, an edit by hand. The folds are the
 *  workspace's own chats', at the notes their records name, where it has any. */
function seed(ws: string, noteFor: NoteFor): Mem {
  const m: Mem = { name: ws, files: new Map(), commits: [], seen: new Map(), tick: null, quiet: null };
  const day = 86_400_000;
  const t = Date.now();
  const naming = noteFor("Naming") ?? "notes/2026-11-19-naming.md";
  const storage = noteFor("Storage layout") ?? "notes/2026-11-22-storage-layout.md";
  const at = (days: number, subject: string, k: string, acts: (set: (p: string, text: string | null, b64?: string) => void) => void, reverts: string | null = null) => {
    acts((p, text, b64) => { if (text === null && !b64) m.files.delete(p); else m.files.set(p, { text, b64: b64 ?? null, mtime: t - days * day }); });
    return commit(m, subject, k, t - days * day, reverts)!;
  };
  at(6, "Memory made", "made", (set) => set("MEMORY.md", TEMPLATE(ws)));
  const fold = at(5, "fold: Naming", "fold", (set) => {
    set(naming, NOTE("Naming", "Names for the roles.", "Named for what they read."));
    set("MEMORY.md", INDEX(ws, { naming }));
  });
  at(4.9, "Undo: fold: Naming", "undo", (set) => { set(naming, null); set("MEMORY.md", TEMPLATE(ws)); }, fold.commit);
  at(1, "fold: Storage layout", "fold", (set) => {
    set(storage, NOTE("Storage layout", "Where the media should live.", "On the NAS, mounted read only for guests."));
    set("topics/nas-layout.md", NAS("not bought yet", storage));
    set("files/rack.svg", null, rack("#e2c16b"));
    set("MEMORY.md", INDEX(ws, { storage }));
  });
  at(0.2, "Edited: MEMORY.md, files/rack.svg, notes/2026-11-19-how-we-work.md and 1 more", "edit", (set) => {
    set("notes/2026-11-19-how-we-work.md", "# How we work\n\nOne place per branch, and nothing merges on a Friday afternoon.\n");
    set("MEMORY.md", INDEX(ws, { storage, edited: true }));
    set("topics/nas-layout.md", NAS("four 8 TB disks", storage));
    set("files/rack.svg", null, rack("#e06c75"));
  });
  m.seen = stamp(m);
  return m;
}

const stamp = (m: Mem) => new Map([...m.files].map(([p, f]) => [p, `${f.mtime}|${size(f)}`]));
const head = (m: Mem): Files => (m.commits.length ? tree(m, m.commits.length) : new Map());
function tree(m: Mem, n: number): Files {
  const out: Files = new Map();
  for (const c of m.commits.slice(0, n)) for (const p of new Set([...c.before.keys(), ...c.after.keys()])) {
    const a = c.after.get(p);
    if (a) out.set(p, a); else out.delete(p);
  }
  return out;
}
function commit(m: Mem, subject: string | null, k: string, at = Date.now(), reverts: string | null = null, live: string[] = []): C | null {
  const h = head(m);
  const before = new Map<string, F>(), after = new Map<string, F>();
  for (const p of new Set([...h.keys(), ...m.files.keys()])) {
    const a = h.get(p), b = m.files.get(p);
    if (same(a, b)) continue;
    if (a) before.set(p, a);
    if (b) after.set(p, { ...b });
  }
  const paths = [...new Set([...before.keys(), ...after.keys()])].sort();
  if (!paths.length) return null;
  const c: C = { commit: sha(), at: Math.floor(at / 1000), subject: subject ?? `Edited: ${paths.slice(0, 3).join(", ")}${paths.length > 3 ? ` and ${paths.length - 3} more` : ""}`, kind: k, before, after, reverts, live };
  m.commits.push(c);
  return c;
}

// ── a line diff, small: common prefix and suffix, the middle replaced ───────
const lines = (t: string) => (t === "" ? [] : t.replace(/\n$/, "").split("\n"));
/** The longest common subsequence's unmatched lines, as `-`/`+`/` ` rows. */
function rows(a: string[], b: string[]): string[] {
  const L = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) for (let j = b.length - 1; j >= 0; j--) L[i][j] = a[i] === b[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const out: string[] = [];
  let i = 0, j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) { out.push(` ${a[i]}`); i++; j++; }
    else if (j < b.length && (i >= a.length || L[i][j + 1] > L[i + 1][j])) out.push(`+${b[j++]}`);
    else out.push(`-${a[i++]}`);
  }
  return out;
}
function patch(p: string, a: F | undefined, b: F | undefined): string {
  const r = rows(lines(a?.text ?? ""), lines(b?.text ?? ""));
  let s = `diff --git a/${p} b/${p}\n--- ${a ? `a/${p}` : "/dev/null"}\n+++ ${b ? `b/${p}` : "/dev/null"}\n`;
  // git's hunks at three lines of context.
  const near = r.map((_, i) => r.slice(Math.max(0, i - 3), i + 4).some((x) => x[0] !== " "));
  let o = 1, n = 1, i = 0;
  while (i < r.length) {
    if (!near[i]) { o++; n++; i++; continue; }
    const from = i, o0 = o, n0 = n;
    while (i < r.length && near[i]) { if (r[i][0] !== "+") o++; if (r[i][0] !== "-") n++; i++; }
    s += `@@ -${o - o0 ? o0 : o0 - 1},${o - o0} +${n - n0 ? n0 : n0 - 1},${n - n0} @@\n${r.slice(from, i).join("\n")}\n`;
  }
  return s;
}
function fileChange(c: C, p: string) {
  const a = c.before.get(p), b = c.after.get(p);
  const binary = (a ?? b)!.text === null && ext(p) !== "svg";
  const r = binary ? [] : rows(lines(a?.text ?? (a?.b64 ? atob(a.b64) : "")), lines(b?.text ?? (b?.b64 ? atob(b.b64) : "")));
  return { path: p, status: !a ? "A" : !b ? "D" : "M", added: binary ? null : r.filter((x) => x[0] === "+").length, removed: binary ? null : r.filter((x) => x[0] === "-").length, binary };
}
function change(m: Mem, c: C) {
  const files = [...new Set([...c.before.keys(), ...c.after.keys()])].sort().map((p) => fileChange(c, p));
  // Newest first: a revert counts while nothing has reverted it.
  const by = new Map<string, string>();
  for (const x of [...m.commits].reverse()) if (x.reverts && !by.has(x.commit) && !by.has(x.reverts)) by.set(x.reverts, x.commit);
  return { commit: c.commit, at: c.at, subject: c.subject, kind: c.kind, files, added: files.reduce((n, f) => n + (f.added ?? 0), 0), removed: files.reduce((n, f) => n + (f.removed ?? 0), 0), reverts: c.reverts, reverted_by: by.get(c.commit) ?? null, live: c.live };
}

// ── the rows ────────────────────────────────────────────────────────────────
export type MemoryMock = ReturnType<typeof memoryMock>;

/** `nameOf` refuses an id `workspaces.json` does not know, as the resolver
 *  does; `noteOf` is where a workspace's chat of that title keeps its note. */
export function memoryMock(emit: Emit, nameOf: (id: unknown) => string, noteOf: (id: string, title: string) => string | null, liveOf: (id: string) => string[]) {
  const none = Q.get("memory") === "none";
  // Workspaces a fold holds: no quiet commit and no undo there.
  const held = new Set<string>();
  const get = (id: string): Mem | null => {
    const ws = nameOf(id);
    if (!mems.has(id) && !none) mems.set(id, seed(ws, (t) => noteOf(id, t)));
    return mems.get(id) ?? null;
  };
  const need = (id: string) => { const m = get(id); if (!m) throw "this workspace has no memory yet"; return m; };
  const file = (m: Mem, p: unknown) => {
    if (bad(p)) throw `not a memory path: "${String(p)}"`;
    const f = m.files.get(p as string);
    if (!f) throw `no memory file: ${String(p)}`;
    return f;
  };
  const find = (m: Mem, c: unknown) => { const x = m.commits.find((y) => y.commit === c); if (!x) throw "no such change in this memory"; return x; };
  const row = (p: string, f: F) => ({ path: p, file: name(p), title: title(f.text) ?? name(p), mtime_ms: f.mtime, size: size(f) });
  function touched(id: string, m: Mem) {
    if (m.quiet !== null) clearTimeout(m.quiet);
    m.quiet = window.setTimeout(() => { m.quiet = null; if (!held.has(id)) commit(m, null, "edit", Date.now(), null, liveOf(id)); }, QUIET);
    if (m.tick !== null) return;
    m.tick = window.setTimeout(() => {
      m.tick = null;
      const s = stamp(m);
      const changed = [...s].filter(([p, x]) => m.seen.get(p) !== x).map(([path]) => ({ path, mtime_ms: m.files.get(path)!.mtime }));
      const removed = [...m.seen.keys()].filter((p) => !s.has(p));
      m.seen = s;
      if (changed.length || removed.length) emit("workspaces:memory", { id, changed, removed });
    }, TICK);
  }
  const view = (id: string) => {
    const m = get(id);
    const dir = `/Users/mock/.local/share/worktrees/workspaces/${id}/memory`;
    if (!m) return { made: false, dir, index: null, folders: [], count: 0, bytes: 0, skipped: [], truncated: false, last: null };
    const ix = m.files.get("MEMORY.md");
    const by = new Map<string, ReturnType<typeof listed>[]>();
    const listed = (p: string, f: F) => ({ path: p, name: name(p), size: size(f), mtime_ms: f.mtime, kind: kind(p), title: ["md", "markdown"].includes(ext(p)) ? title(f.text) : null });
    for (const p of [...m.files.keys()].sort((a, b) => Number(b === "MEMORY.md") - Number(a === "MEMORY.md") || (a < b ? -1 : 1))) {
      const top = p.includes("/") ? p.split("/")[0] : "";
      by.set(top, [...(by.get(top) ?? []), listed(p, m.files.get(p)!)]);
    }
    by.get("notes")?.reverse();
    const rank = (n: string) => { const i = ["", "notes", "topics", "files"].indexOf(n); return i < 0 ? 9 : i; };
    const all = [...m.files.values()];
    return {
      made: true, dir,
      index: ix && { path: "MEMORY.md", content: ix.text ?? "", mtime_ms: ix.mtime, size: size(ix), budget: 16384, over_budget: size(ix) > 16384 },
      folders: [...by].map(([n, files]) => ({ name: n, files })).sort((a, b) => rank(a.name) - rank(b.name) || (a.name < b.name ? -1 : 1)),
      count: all.length, bytes: all.reduce((n, f) => n + size(f), 0), skipped: [], truncated: false,
      last: m.commits.length ? change(m, m.commits[m.commits.length - 1]) : null,
    };
  };
  const side = (p: string, f: F | undefined) => (f ? { mime: IMAGES[ext(p)], size: size(f), b64: f.b64 ?? btoa(f.text ?? ""), refused: null } : null);

  const call = (cmd: string, a: Record<string, unknown>): unknown => {
    const id = a.id as string;
    const exp = (a.expectedMtime ?? a.expected_mtime ?? null) as number | null;
    switch (cmd) {
      case "workspace_memory": return view(id);
      case "workspace_memory_make": {
        if (!get(id)) {
          const m: Mem = { name: nameOf(id), files: new Map(), commits: [], seen: new Map(), tick: null, quiet: null };
          m.files.set("MEMORY.md", { text: TEMPLATE(m.name), b64: null, mtime: now() });
          commit(m, "Memory made", "made");
          m.seen = stamp(m);
          mems.set(id, m);
        }
        return view(id);
      }
      case "workspace_memory_file": {
        const m = need(id);
        if (a.commit != null) {
          if (bad(a.path)) throw `not a memory path: "${String(a.path)}"`;
          const f = tree(m, m.commits.indexOf(find(m, a.commit)) + 1).get(a.path as string);
          if (!f) throw `no memory file: ${String(a.path)} in that change`;
          return { ...row(a.path as string, f), mtime_ms: 0, content: f.text ?? "" };
        }
        const f = file(m, a.path);
        if (f.text === null) throw `${String(a.path)} is not text`;
        return { ...row(a.path as string, f), content: f.text };
      }
      case "workspace_memory_image": {
        const f = file(need(id), a.path);
        const mime = IMAGES[ext(a.path as string)];
        if (!mime) throw `not an image: ${String(a.path)}`;
        const etag = `${f.mtime}-${size(f)}`;
        const unchanged = a.etag === etag;
        return { b64: unchanged ? "" : f.b64 ?? btoa(f.text ?? ""), size: size(f), truncated: false, mtime: f.mtime, mime, etag, unchanged };
      }
      case "workspace_memory_write": {
        const m = need(id);
        const p = a.path as string;
        const keep = (a.keepHistory ?? a.keep_history) === true;
        writes.push({ id, path: p, content: a.content, expected_mtime: exp, keep_history: keep });
        if (bad(p)) throw `not a memory path: "${p}"`;
        if (kind(p) !== "text") throw `${p} is not a text file`;
        if (exp === null) { if (m.files.has(p)) throw `there is already a file at ${p}`; }
        else if (file(m, p).mtime !== exp) throw `the file changed on disk since you opened it: ${p}`;
        const old = m.files.get(p);
        if (old && old.text === a.content) return row(p, old);
        if (keep) commit(m, `Before keeping your text: ${p}`, "edit");
        m.files.set(p, { text: a.content as string, b64: null, mtime: now() });
        touched(id, m);
        return row(p, m.files.get(p)!);
      }
      case "workspace_memory_put": {
        const m = need(id);
        const n = String(a.name ?? "");
        if (bad(n) || n.includes("/")) throw `not a file name: "${n}"`;
        const data = String(a.data ?? "");
        if ((data.length * 3) / 4 > 4 * 1024 * 1024 + 2) throw `${n} is over the 4 MB a memory takes`;
        const [, stem, e] = /^(.*?)(\.[^.]*)?$/.exec(n)!;
        let p = `files/${n}`;
        for (let i = 2; m.files.has(p); i++) p = `files/${stem}-${i}${e ?? ""}`;
        m.files.set(p, { text: null, b64: data, mtime: now() });
        touched(id, m);
        return { ...row(p, m.files.get(p)!), title: name(p) };
      }
      case "workspace_memory_log": {
        const m = get(id);
        if (!m) return { changes: [], more: false };
        if (a.path != null && bad(a.path)) throw `not a memory path: "${String(a.path)}"`;
        let all = [...m.commits].reverse();
        if (a.before != null) all = all.slice(all.indexOf(find(m, a.before)) + 1);
        if (a.path != null) all = all.filter((c) => c.before.has(a.path as string) || c.after.has(a.path as string));
        const n = Math.min(200, Math.max(1, Number(a.n ?? 50)));
        return { changes: all.slice(0, n).map((c) => change(m, c)), more: all.length > n };
      }
      case "workspace_memory_diff": {
        const m = need(id);
        const c = find(m, a.commit);
        const ch = change(m, c);
        if (a.path == null) return { change: ch, file: null };
        const p = a.path as string;
        const fc = ch.files.find((f) => f.path === p);
        if (!fc) throw `that change does not touch ${p}`;
        const [x, y] = [c.before.get(p), c.after.get(p)];
        const diff = IMAGES[ext(p)] ? { kind: "image", before: side(p, x), after: side(p, y) } : fc.binary ? { kind: "binary" } : { kind: "text", patch: patch(p, x, y), truncated: false };
        return { change: ch, file: { ...fc, diff } };
      }
      case "workspace_memory_revert": {
        if (held.has(id)) throw "a chat is folding into this memory: wait for it, or stop it";
        const m = need(id);
        const c = find(m, a.commit);
        if (c.kind === "made") throw "the change that made this memory cannot be undone";
        if (change(m, c).reverted_by) throw "that change is already undone";
        commit(m, null, "edit");
        const paths = [...new Set([...c.before.keys(), ...c.after.keys()])];
        // Coarser than git: a file that is not as the change left it conflicts.
        const conflicts = paths.filter((p) => !same(m.files.get(p), c.after.get(p))).sort();
        if (conflicts.length) return { change: null, conflicts };
        for (const p of paths) {
          const b = c.before.get(p);
          if (b) m.files.set(p, { ...b, mtime: now() }); else m.files.delete(p);
        }
        const what = c.subject.replace(/^(Undo|Redo): /, "");
        const done = commit(m, `${c.kind === "undo" ? "Redo" : "Undo"}: ${what}`, c.kind === "undo" ? "redo" : "undo", Date.now(), c.commit)!;
        touched(id, m);
        return { change: change(m, done), conflicts: [] };
      }
      default: throw `unknown command: ${cmd}`;
    }
  };
  return {
    call,
    /** An edit made outside the app (an agent's): `null` removes the file. */
    edit(id: string, path: string, text: string | null) {
      const m = need(id);
      if (text === null) m.files.delete(path); else m.files.set(path, { text, b64: null, mtime: now() });
      touched(id, m);
    },
    writes: () => writes.slice(),
    /** A chat's start and resume: make it, or commit what is dirty. */
    beforeLaunch(id: string) {
      const m = get(id);
      if (!m) call("workspace_memory_make", { id });
      else commit(m, null, "edit");
    },
    /** What a chat's record says of the fold of that title, as git has it. */
    fold(id: string, title: string, sha?: string): { commit: string; files: number; lines: { added: number; removed: number }; reverted: boolean } | null {
      const m = get(id);
      const c = m?.commits.find((x) => x.commit === sha) ?? m?.commits.filter((x) => x.subject === `fold: ${title}`).pop();
      if (!m || !c) return null;
      const ch = change(m, c);
      return { commit: ch.commit, files: ch.files.length, lines: { added: ch.added, removed: ch.removed }, reverted: ch.reverted_by !== null };
    },
    hold: (id: string) => { held.add(id); },
    release: (id: string) => { held.delete(id); },
    /** Commit what is dirty under `subject`: its id, or null for nothing. */
    commit(id: string, subject: string | null, kind = "edit"): string | null {
      const m = get(id);
      return (m && commit(m, subject, kind)?.commit) || null;
    },
    /** How many files are changed and not committed. */
    dirty(id: string): number {
      const m = get(id);
      if (!m) return 0;
      const h = head(m);
      return [...new Set([...h.keys(), ...m.files.keys()])].filter((p) => !same(h.get(p), m.files.get(p))).length;
    },
    /** What a fold's Claude writes: the chat's note, and a line in the index. */
    foldWrites(id: string, note: string, chat: string) {
      const m = need(id);
      const p = note.replace(/^memory\//, "");
      m.files.set(p, { text: NOTE(chat, "What the chat was asked.", "What it decided."), b64: null, mtime: now() });
      const i = m.files.get("MEMORY.md");
      const line = `- ${chat}. → [note](${p})\n`;
      if (i?.text != null && !i.text.includes(`(${p})`)) m.files.set("MEMORY.md", { ...i, text: i.text.replace("## Decisions\n\n", `## Decisions\n\n${line}`), mtime: now() });
      touched(id, m);
    },
    /** A note is on disk, by its path in the workspace's folder. */
    has: (id: string, path: string) => !!get(id)?.files.has(path.replace(/^memory\//, "")),
    /** Its workspace was removed: the folder, if it had one, is moved away. */
    remove(id: string): boolean {
      const m = mems.get(id);
      if (m?.tick != null) clearTimeout(m.tick);
      if (m?.quiet != null) clearTimeout(m.quiet);
      return mems.delete(id);
    },
  };
}
