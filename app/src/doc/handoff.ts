// A document handed from one window to the other: Float (main → the Ideas window)
// and Dock (back). No import at runtime: doc-handoff-check.mjs loads it as a `data:`
// module, on a virtual clock.
//
// The one that has it ASKS and keeps it on screen; the other opens it and ANSWERS,
// naming what is open there now; only then does the asker close its own. A refusal
// (the other window has a text of its own that is not saved), a window that did not
// open and one that did not answer are said where the document still is, and nothing
// moves. The words go through localStorage, which both windows of this machine share;
// each ask has its own id, so an answer to another is never taken for it.

export type HandDoc =
  | { kind: "idea"; id: string }
  | { kind: "memory"; ws: string; name: string; path: string };
export type Ask = { ask: string; doc: HandDoc; at: number };
/** `doc`: what is open there now. */
export type Answer = { ask: string; at: number } & ({ ok: true; doc: HandDoc } | { ok: false; why: string });

/** A direction's two keys. */
export type Way = { ask: string; answer: string };
export const FLOAT: Way = { ask: "wt.ideas.show", answer: "wt.ideas.shown" };
export const DOCK: Way = { ask: "wt.main.doc", answer: "wt.main.doc.shown" };

/** How long the asker waits: a window that was closed has to start. */
export const ANSWER_MS = 12_000;
/** An ask nobody took is not acted on later than this. */
export const ASK_TTL_MS = 60_000;
const LOOK_MS = 200;

const ULID = /^[0-9A-HJKMNP-TV-Z]{26}$/;
/** A memory path as the engine takes one: relative, no way up, no dot folder. */
const PATH = (p: string) => p.length > 0 && p.length <= 1024 && !/[\\\u0000-\u001f\u007f]/.test(p) && p.split("/").every((s) => s !== "" && !s.startsWith("."));
const handDoc = (v: unknown): HandDoc | null => {
  const d = v as Record<string, unknown> | null;
  if (!d || typeof d !== "object") return null;
  if (d.kind === "idea" && typeof d.id === "string" && ULID.test(d.id)) return { kind: "idea", id: d.id };
  if (d.kind === "memory" && typeof d.ws === "string" && /^[0-9A-Za-z-]{1,64}$/.test(d.ws) && typeof d.path === "string" && PATH(d.path))
    return { kind: "memory", ws: d.ws, path: d.path, name: typeof d.name === "string" ? d.name.slice(0, 200) : "" };
  return null;
};
const parse = (raw: string | null): Record<string, unknown> | null => {
  try {
    const v = JSON.parse(raw ?? "null") as unknown;
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch { return null; }
};
const fresh = (at: unknown, now: number): at is number => typeof at === "number" && now - at <= ASK_TTL_MS && at <= now + 5_000;

export function readAsk(raw: string | null, now: number): Ask | null {
  const v = parse(raw);
  const doc = v && handDoc(v.doc);
  return v && doc && typeof v.ask === "string" && v.ask && fresh(v.at, now) ? { ask: v.ask, doc, at: v.at } : null;
}
export function readAnswer(raw: string | null, ask: string): Answer | null {
  const v = parse(raw);
  if (!v || v.ask !== ask || typeof v.at !== "number") return null;
  if (v.ok === true) { const doc = handDoc(v.doc); return doc && { ask, at: v.at, ok: true, doc }; }
  return v.ok === false && typeof v.why === "string" ? { ask, at: v.at, ok: false, why: v.why } : null;
}
export const sameHand = (a: HandDoc, b: HandDoc): boolean =>
  a.kind === "idea" ? b.kind === "idea" && a.id === b.id : b.kind === "memory" && a.ws === b.ws && a.path === b.path;

/** What the asker needs of the window it runs in. */
export type Env = {
  get(key: string): string | null;
  set(key: string, value: string): void;
  remove(key: string): void;
  /** The other window may have written `key`: the `storage` event. Returns the off. */
  heard(key: string, fn: () => void): () => void;
  every(ms: number, fn: () => void): () => void;
  now(): number;
  id(): string;
};

/** Ask the other window to open `doc`. `null`: it has it open. Else why not, for a
 *  person, and nothing has moved. `raise` opens the window, or brings it forward. */
export async function ask(env: Env, way: Way, doc: HandDoc, raise: () => Promise<unknown>, where: string): Promise<string | null> {
  const id = env.id();
  const mine: Ask = { ask: id, doc, at: env.now() };
  try { env.set(way.ask, JSON.stringify(mine)); } catch { return `${where} could not be asked: this machine's storage is not available.`; }
  const end = mine.at + ANSWER_MS;
  const withdraw = () => { try { if (readAsk(env.get(way.ask), env.now())?.ask === id) env.remove(way.ask); } catch { /* it goes stale */ } };
  return new Promise((done) => {
    let over = false;
    const offs: (() => void)[] = [];
    const finish = (why: string | null) => {
      if (over) return;
      over = true;
      for (const off of offs) off();
      if (why !== null) withdraw();
      done(why);
    };
    const look = () => {
      if (over) return;
      let a: Answer | null = null;
      try { a = readAnswer(env.get(way.answer), id); } catch { /* not yet */ }
      if (a) {
        try { env.remove(way.answer); } catch { /* it names this ask only */ }
        finish(a.ok ? (sameHand(a.doc, doc) ? null : `${where} opened another document instead.`) : a.why);
      } else if (env.now() >= end) finish(`${where} did not open it.`);
    };
    offs.push(env.heard(way.answer, look), env.every(LOOK_MS, look));
    Promise.resolve().then(raise).then(look, (e) => finish(`${where} did not open: ${typeof e === "string" ? e : e instanceof Error ? e.message : String(e)}`));
  });
}

/** Take the ask there is, once. */
export function take(env: Pick<Env, "get" | "remove" | "now">, way: Way): Ask | null {
  try {
    const a = readAsk(env.get(way.ask), env.now());
    if (a) env.remove(way.ask);
    return a;
  } catch { return null; }
}
export function answer(env: Pick<Env, "set" | "now">, way: Way, a: Ask, out: { ok: true; doc: HandDoc } | { ok: false; why: string }): void {
  try { env.set(way.answer, JSON.stringify({ ask: a.ask, at: env.now(), ...out })); } catch { /* the asker says it did not hear */ }
}

/** A real window's. */
export const windowEnv = (): Env => ({
  get: (k) => window.localStorage.getItem(k),
  set: (k, v) => window.localStorage.setItem(k, v),
  remove: (k) => window.localStorage.removeItem(k),
  heard: (key, fn) => {
    const on = (e: StorageEvent) => { if (e.key === key) fn(); };
    window.addEventListener("storage", on);
    return () => window.removeEventListener("storage", on);
  },
  every: (ms, fn) => { const h = window.setInterval(fn, ms); return () => window.clearInterval(h); },
  now: () => Date.now(),
  id: () => crypto.randomUUID(),
});
