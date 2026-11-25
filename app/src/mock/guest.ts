// The design harness as a GUEST host (group chat §7): David's Mac, which this
// "machine" (Bret's) is a guest of. `?guest` puts it in the paired hosts; the
// guest door's mock dial (guestboot.ts) sends every call here with
// `__guestHost`, and the answers mirror serve's (`serve/shares.rs`,
// `serve/sharethread.rs`, `serve/history.rs`). Every row but the guest rows is
// refused as serve's resolver refuses it — `denied` — so a guest view that
// asked for anything else would show the refusal, not an answer.
//
// `?guestimage=old` is a host from before `shared_image`; `?imgslow=<ms>` delays
// each picture.
// `?guest=offline` makes the connection fail (guestboot.ts). Helpers on
// `window.__mock`: `guestShare(slug)` shares another place (a toast and a
// badge), `guestTurn(text)` appends a turn to the live share, `guestEnd()`
// ends its binding, `guestAllow()` lets Bret in on the invited share,
// `guestPost(text)` posts as David, `guestCalls()` the commands asked,
// `guestImages()` the `shared_image` calls and the most in flight at once,
// `guestDrop()`/`guestBack()` the host quitting and returning, and
// `guestRevoke()` David removing this guest (`removed`, never offline).

export const GUEST_HOST = {
  id: "h_guestdavidmacaaaaaaaaaaaaa",
  name: "David's Mac",
  addr: "100.101.1.2:7461",
  device_id: "d_bretmac",
  expiry: { idle_days: 30 },
  paired_at: 1_799_000_000,
  principal_id: "g_bretaaaaaaaaaaaaaaaaaaaaaa",
  guest: true,
};
const BRET = GUEST_HOST.principal_id;
const OWNER = "David";

/** dispatch.rs's `GUEST_ROWS`. */
const ROWS = new Set(["shares_list", "share_join", "guest_places", "share_request", "thread_read", "thread_post", "shared_turns", "shared_tool_call", "shared_image"]);

type Binding = { harness: string; state: "active" | "ended"; bound_at: number; ended_at: number | null };
type Share = {
  id: string; project: string; project_id: string; slug: string; title: string | null;
  state: "invited" | "asked" | "allowed"; binding: Binding | null; ended: Binding[]; created_at: number;
};
type Entry = { seq: number; at: number; kind: string; id?: string; principal?: string; device?: string; text?: string; name?: string | null; device_name?: string | null };
type MTurn = Record<string, unknown> & { seq: number };

const T0 = 1_800_000_000;
const now = () => Math.floor(Date.now() / 1000);
const PID = { worktrees: "a1b2c3d4e5f60718", garden: "0f1e2d3c4b5a6978" };

const shares: Share[] = [
  { id: "s_livechataaaaaaaaaaaaaaaaaa", project: "worktrees", project_id: PID.worktrees, slug: "feat-chat-redesign", title: "Chat redesign", state: "allowed",
    binding: { harness: "claude", state: "active", bound_at: T0, ended_at: null }, ended: [], created_at: T0 },
  { id: "s_invitedloginaaaaaaaaaaaaa", project: "worktrees", project_id: PID.worktrees, slug: "fix-login", title: null, state: "invited",
    binding: { harness: "codex", state: "active", bound_at: T0 + 50, ended_at: null }, ended: [], created_at: T0 + 50 },
  { id: "s_endedspikeaaaaaaaaaaaaaaa", project: "garden", project_id: PID.garden, slug: "spike-sensors", title: null, state: "allowed",
    binding: { harness: "claude", state: "ended", bound_at: T0 - 9000, ended_at: T0 - 3000 }, ended: [], created_at: T0 - 9000 },
];
const places: Record<string, { slug: string; title: string | null }[]> = {
  worktrees: [{ slug: "(main)", title: null }, { slug: "feat-chat-redesign", title: "Chat redesign" }, { slug: "fix-login", title: null }, { slug: "refactor-store", title: "Store refactor" }],
  garden: [{ slug: "(main)", title: null }, { slug: "spike-sensors", title: null }],
};

function turn(seq: number, at: number, user: string, reply: string, tool?: { name: string; summary: string; id: string }): MTurn {
  const steps: unknown[] = [{ kind: "text", at: at + 5, text: reply }];
  if (tool) steps.push({ kind: "tool", at: at + 6, name: tool.name, call_id: tool.id, summary: tool.summary, detail: true });
  return { seq, id: `t${seq}`, at, user, reply: "", tools: tool ? [tool.name] : [], truncated: false, steps };
}
const transcripts: Record<string, MTurn[]> = {
  s_livechataaaaaaaaaaaaaaaaaa: Array.from({ length: 45 }, (_, i) =>
    turn(i, T0 + i * 120, i === 0 ? "Let's redesign the chat view. Start by reading ChatPane.tsx." : `Step ${i}: carry on with the next part.`,
      i === 0 ? "I read `ChatPane.tsx`. It is **1475 lines**; the composer is the biggest part." : `Done with part ${i}. Next I will look at the scroll handling.`,
      i % 3 === 0 ? { name: "Bash", summary: `npm test -- part ${i}`, id: `call_${i}` } : undefined)),
  s_endedspikeaaaaaaaaaaaaaaa: [
    turn(0, T0 - 9000, "Spike the soil sensors over I2C.", "The sensor answers at 0x36. Readings look sane.", { name: "Bash", summary: "i2cdetect -y 1", id: "call_s0" }),
    turn(1, T0 - 6000, "Write it up.", "Wrote `notes/sensors.md`."),
  ],
};
// A question David answered, and one still waiting (D8: Bret sees "waiting for David").
transcripts.s_livechataaaaaaaaaaaaaaaaaa[44].steps = [
  { kind: "text", at: T0 + 44 * 120 + 5, text: "Which layout do you want?" },
  { kind: "question", at: T0 + 44 * 120 + 6, questions: [{ header: "Layout", question: "Side by side or stacked?", multi: false, options: [{ label: "Side" }, { label: "Stacked" }] }] },
];
// Pictures (`serve/shareimage.rs`): a reply that shows four and names two it
// may not, and Reads inside the place and outside it; the owner's message
// attaches one the host stored and draws one it did not.
const LANE = "/Users/david/work/worktrees/.worktrees/feat-chat-redesign";
transcripts.s_livechataaaaaaaaaaaaaaaaaa[43] = {
  ...transcripts.s_livechataaaaaaaaaaaaaaaaaa[43], user: "Show me the mockups. ![mine](shots/mine.png) [attached: .worktrees-attachments/20261124-00000000000000aa.png]", tools: ["Read", "Read"],
  steps: [
    { kind: "text", at: T0 + 43 * 120 + 5, text: "I made two mockups.\n\n**Home**: every project\n![Home view](.worktrees-attachments/agent/wide-home.svg)\n\n**Place**: the treemap\n![Place view](shots/wide%20place.svg)\n\n![Gone](shots/missing.png)\n\n![Logo](https://example.com/logo.png)" },
    { kind: "tool", at: T0 + 43 * 120 + 6, name: "Read", call_id: "call_r1", summary: `${LANE}/shots/wide-read.svg`, detail: true },
    { kind: "tool", at: T0 + 43 * 120 + 7, name: "Read", call_id: "call_r2", summary: "/private/tmp/render.png", detail: true },
  ],
};
const images = { calls: [] as string[], now: 0, most: 0 };
const threads: Record<string, Entry[]> = {
  s_livechataaaaaaaaaaaaaaaaaa: [
    { seq: 1, at: T0 + 10, kind: "joined", principal: BRET, name: "Bret" },
    { seq: 2, at: T0 + 20, kind: "post", id: "p1", principal: "owner", text: "Watch this one: the scroll bug is near the end.", name: OWNER },
    { seq: 3, at: T0 + 30, kind: "post", id: "p2", principal: BRET, device: "d_bretmac", text: "On it.", name: "Bret", device_name: "Bret's MacBook" },
  ],
  s_endedspikeaaaaaaaaaaaaaaa: [{ seq: 1, at: T0 - 3000, kind: "session_ended" }],
};

let calls: string[] = [];
let emit: (event: string, payload: unknown) => void = () => {};
const changed = (what: string, share?: string, seq?: number) => emit("shares:changed", { what, to: [BRET], ...(share ? { share } : {}), ...(seq ? { seq } : {}) });
const find = (id: unknown) => {
  const s = shares.find((x) => x.id === id);
  if (!s) throw `no share ${String(id)}`;
  return s;
};
const admitted = (id: unknown) => {
  const s = find(id);
  if (s.state !== "allowed") throw "not let in yet: the owner has not allowed you on this share";
  return s;
};
const SVG = btoa('<svg xmlns="http://www.w3.org/2000/svg" width="640" height="320" viewBox="0 0 640 320"><rect width="640" height="320" fill="#3b82f6"/><circle cx="320" cy="160" r="90" fill="#fde68a"/></svg>');
/** `shared_image`: what the turn draws, then the place's rule, as the host's. */
async function sharedImage(args: Record<string, unknown>) {
  const s = admitted(args.share);
  const path = String(args.path);
  const t = (transcripts[s.id] ?? []).find((x) => x.seq === Number(args.turn));
  // A person's message: only what the host stored (its upload ledger), which
  // the mock reads as an upload's own name in the `[attached: …]` tail.
  const stored = /^\.worktrees-attachments\/\d{8}-[0-9a-f]{16}\.png$/.test(path) && String(t?.user ?? "").includes(` [attached: ${path}`);
  const draws = stored || (t?.steps as { kind: string; text?: string; name?: string; summary?: string }[] | undefined)?.some((st) =>
    st.kind === "text" ? [...(st.text ?? "").matchAll(/!\[[^\]]*\]\(([^)\s]+)\)/g)].some((m) => decodeURIComponent(m[1]) === path) : st.kind === "tool" && st.name === "Read" && st.summary === path);
  images.calls.push(path);
  images.most = Math.max(images.most, ++images.now);
  try {
    await sleep(Number(new URLSearchParams(location.search).get("imgslow") ?? 0));
    if (!draws) throw `not shown in this shared session: ${path}`;
    if (path.startsWith("/") && !path.startsWith(`${LANE}/`)) throw `outside this place: ${path}`;
    if (path.includes("missing")) throw `not found: ${path}`;
    const etag = "1-1";
    const b = { size: 300, truncated: false, mtime: 1, mime: "image/svg+xml", etag };
    return args.etag === etag ? { ...b, b64: "", unchanged: true } : { ...b, b64: SVG, unchanged: false };
  } finally {
    images.now -= 1;
  }
}
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function guestInvoke(cmd: string, args: Record<string, unknown>, emitEvent: (event: string, payload: unknown) => void): unknown {
  emit = emitEvent;
  calls.push(cmd);
  if (cmd === "shared_image" && new URLSearchParams(location.search).get("guestimage") === "old") throw `unknown command: ${cmd}`;
  if (!ROWS.has(cmd)) throw "denied";
  switch (cmd) {
    case "shares_list":
      return { owner: OWNER, shares: structuredClone(shares) };
    case "guest_places":
      return { owner: OWNER, projects: Object.entries(places).map(([name, ps]) => ({ id: PID[name as keyof typeof PID], name, places: ps })) };
    case "share_join": {
      const s = find(args.share);
      if (s.state === "invited") {
        s.state = "asked";
        // David allows after a beat, unless the harness says not to.
        if (!new URLSearchParams(location.search).has("guestwait")) void sleep(2500).then(() => { s.state = "allowed"; changed("allowed", s.id); });
      }
      changed("asked", s.id);
      return { state: s.state };
    }
    case "share_request": {
      const note = args.note === undefined ? null : String(args.note);
      if (note !== null && new TextEncoder().encode(note).length > 280) throw "the note is over 280 bytes";
      changed("request");
      return null;
    }
    case "thread_read": {
      admitted(args.share);
      const after = Number(args.after ?? 0);
      const all = threads[String(args.share)] ?? [];
      const entries = all.filter((e) => e.seq > after);
      return { share: args.share, entries, more: false, last: all.length ? all[all.length - 1].seq : 0 };
    }
    case "thread_post": {
      const s = admitted(args.share);
      const all = (threads[s.id] ??= []);
      const dup = all.find((e) => e.id === args.id && e.principal === BRET);
      if (dup) return dup;
      const e: Entry = { seq: (all[all.length - 1]?.seq ?? 0) + 1, at: now(), kind: "post", id: String(args.id), principal: BRET, device: "d_bretmac", text: String(args.text), name: "Bret", device_name: "Bret's MacBook" };
      all.push(e);
      changed("post", s.id, e.seq);
      return e;
    }
    case "shared_turns": {
      const s = admitted(args.share);
      const b = args.binding === undefined || args.binding === null ? s.binding : s.ended[Number(args.binding)];
      if (!b) throw args.binding === undefined ? "nothing is bound on this share" : `this share has no ended binding ${String(args.binding)}`;
      const all = transcripts[s.id] ?? [];
      const total = all.length;
      const hi = args.before === undefined ? total : Math.min(Number(args.before), total);
      const limit = Math.max(1, Math.min(Number(args.limit ?? 20), 50));
      const lo = Math.max(0, hi - limit);
      const turns = all.slice(lo, hi).reverse();
      return { harness: b.harness, state: b.state, turns, total, next_before: lo, more: lo > 0, page_full: false, page_bytes: 2097152, turn_text_max: 100000, share: s.id, binding: args.binding ?? null };
    }
    case "shared_image":
      return sharedImage(args);
    case "shared_tool_call": {
      admitted(args.share);
      const id = String(args.call_id);
      const t = (transcripts[String(args.share)] ?? []).find((x) => (x.steps as { call_id?: string }[]).some((st) => st.call_id === id));
      if (!t) throw "not found: no such call in this shared session";
      const st = (t.steps as { call_id?: string; summary?: string; name: string }[]).find((x) => x.call_id === id)!;
      return { tool: st.name, input: { command: st.summary, description: "Run the tests" }, truncated: false, bytes: 60, call_id: id, harness: "claude" };
    }
  }
  throw "denied";
}

/** The `window.__mock` helpers. */
export const guestHelpers = {
  guestShare(slug = "refactor-store") {
    const s: Share = { id: `s_new${slug.replace(/[^a-z]/g, "")}aaaaaaaaaaaaaaaaaaaa`.slice(0, 28), project: "worktrees", project_id: PID.worktrees, slug, title: null, state: "invited",
      binding: { harness: "claude", state: "active", bound_at: now(), ended_at: null }, ended: [], created_at: now() };
    shares.push(s);
    changed("shared", s.id);
  },
  guestTurn(text = "Another turn, live.") {
    const all = transcripts.s_livechataaaaaaaaaaaaaaaaaa;
    all.push(turn(all.length, now(), "And then?", text));
  },
  guestEnd() {
    const s = shares[0];
    if (s.binding) { s.binding = { ...s.binding, state: "ended", ended_at: now() }; changed("ended", s.id); }
  },
  guestAllow() {
    for (const s of shares) if (s.state !== "allowed") { s.state = "allowed"; changed("allowed", s.id); }
  },
  guestPost(text = "A note from David.") {
    const all = threads.s_livechataaaaaaaaaaaaaaaaaa;
    const e: Entry = { seq: all[all.length - 1].seq + 1, at: now(), kind: "post", id: `o${all.length}`, principal: "owner", text, name: OWNER };
    all.push(e);
    changed("post", "s_livechataaaaaaaaaaaaaaaaaa", e.seq);
  },
  guestDrop() {
    const w = window as unknown as { __guestLost?: (m: string) => void; __guestDown?: boolean };
    w.__guestDown = true;
    w.__guestLost?.("the host closed the connection: going away");
  },
  /** David removed this guest (Settings → Remote → Guests → Remove): the
   *  host closes with `revoked`, and every redial's key is refused at TLS. */
  guestRevoke() {
    const w = window as unknown as { __guestLost?: (m: string) => void; __guestDown?: boolean | "revoked" };
    w.__guestDown = "revoked";
    w.__guestLost?.("the host closed the connection: revoked");
  },
  guestBack() {
    (window as unknown as { __guestDown?: boolean | "revoked" }).__guestDown = false;
  },
  guestImages() {
    const r = { calls: images.calls, most: images.most };
    images.calls = [];
    images.most = 0;
    return r;
  },
  guestCalls() {
    const c = calls;
    calls = [];
    return c;
  },
};
