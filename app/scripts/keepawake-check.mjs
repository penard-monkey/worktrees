// The rail's Keep awake button and its Settings twin, on the HOST's state.
//
//   node app/scripts/keepawake-check.mjs      # exits non-zero on failure
//
// The hold is the host's (worktrees-api `keepawake`), so both controls show
// what the host answered and nothing else. What this pins, each by RUNNING the
// real KeepAwake.tsx under stubs:
//
//   1. loading reads once and sets nothing; before the answer, on a host with
//      no mechanism and on a host too old to have the row, there is NO button
//      (hidden, not greyed with no reason), and only a real failure is said;
//   2. one click is one set of the opposite of what the HOST holds, a second
//      click while it is in flight is none, and a refusal is reported and
//      re-read;
//   3. an event (another window's act) lands, and beats a read it overtook;
//   4. lit only while held; on-but-not-held says why; the title names the
//      host's computer, and a paired host by name; a device without `admin`
//      gets a disabled button that says so;
//   5. a new engine generation reads again;
//   6. App renders it once, in the Places rail, at module scope; the CSS
//      differs by more than a hue.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let failed = 0;
const is = (got, want, what) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) console.log(`ok   ${what}`);
  else { failed++; console.log(`FAIL ${what}: got ${g}, want ${w}`); }
};
const yes = (cond, what) => is(!!cond, true, what);

// The real component, its imports taken from a stub env.
const src = read("../src/KeepAwake.tsx");
const body = src.replace(/^import .*$/gm, "").replace(/^export /gm, "");
const js = (await transformWithEsbuild(body, "KeepAwake.tsx", { loader: "tsx", format: "esm", jsx: "transform", jsxFactory: "h", jsxFragment: "F" })).code;
const tick = () => new Promise((r) => setTimeout(r, 0));

const OFF = { supported: true, on: false, held: false, mechanism: "iokit", os: "macos", error: null };
const ON = { ...OFF, on: true, held: true };

/** One window: the module's store, fresh, and the two components over it. */
function win({ remote = null, refused = null } = {}) {
  const calls = [], failures = [], handlers = [], cells = new Map();
  let engine = { kind: "up", gen: 1 }, engineSub = null;
  const env = {
    invoke: (cmd, args) => new Promise((resolve, reject) => calls.push({ cmd, args, resolve, reject })),
    listen: (name, f) => { handlers.push([name, f]); return Promise.resolve(() => {}); },
    notGranted: (cmd) => (cmd === "set_keep_awake" ? refused : null), remoteHost: () => remote,
    engineState: () => engine, onEngineState: (f) => { engineSub = f; return () => {}; },
    Icons: { Coffee: "Coffee" }, SECTION: { "keep-awake": { write: "set_keep_awake" } }, F: "F",
    h: (type, props, ...children) => ({ type, props: props ?? {}, children: children.flat(Infinity) }),
  };
  let cur = null, i = 0;
  const slot = (make) => { const k = i++; return (cur[k] ??= make()); };
  env.useRef = (v) => slot(() => ({ current: v }));
  env.useSyncExternalStore = (sub, snap) => { slot(() => { sub(() => {}); return {}; }); return snap(); };
  env.useEffect = (fn, deps) => {
    const s = slot(() => ({}));
    if (s.deps && s.deps.length === deps.length && s.deps.every((d, k) => Object.is(d, deps[k]))) return;
    s.deps = deps;
    s.run = fn;
    effects.push(s);
  };
  let effects = [];
  const mod = new Function(...Object.keys(env), `${js}\nreturn { KeepAwakeRailButton, KeepAwakeSection };`)(...Object.values(env));
  const render = (name) => {
    if (!cells.has(name)) cells.set(name, []);
    cur = cells.get(name); i = 0; effects = [];
    const tree = mod[name]({ onFail: (e) => failures.push(String(e)) });
    for (const s of effects) s.run();
    return tree;
  };
  return {
    calls, failures,
    rail: () => render("KeepAwakeRailButton"),
    setting: () => render("KeepAwakeSection"),
    last: (cmd) => calls.filter((c) => c.cmd === cmd).at(-1),
    count: (cmd) => calls.filter((c) => c.cmd === cmd).length,
    event: (p) => handlers.filter(([n]) => n === "keep_awake:changed").forEach(([, f]) => f({ payload: p })),
    engine: (e) => { engine = e; engineSub?.(); },
    async answer(cmd, v) { this.last(cmd).resolve(v); await tick(); },
    async refuse(cmd, e) { this.last(cmd).reject(e); await tick(); },
  };
}
const texts = (t) => (typeof t === "string" ? t : t && typeof t === "object" ? t.children.map(texts).join("") : "");
const nodes = (t, out = []) => { if (t && typeof t === "object") { out.push(t); t.children.forEach((c) => nodes(c, out)); } return out; };
const box = (t) => nodes(t).find((n) => n.type === "input");

// 1. loading
{
  const w = win();
  is(w.rail(), null, "no button before the host has answered");
  is(w.calls.map((c) => c.cmd), ["get_keep_awake"], "loading reads the host once and sets nothing");
  w.rail();
  is(w.calls.length, 1, "a re-render reads nothing");
  await w.answer("get_keep_awake", OFF);
  const b = w.rail();
  is([b?.type, b?.props["aria-pressed"], /\blit\b/.test(b?.props.className ?? "")], ["button", false, false], "off: a button, not pressed, not lit");
  is(b.props.title, "Keep this Mac awake, so agents can keep working", "off: the title offers it");
  yes(b.children.some((c) => c.type === "Coffee"), "the glyph is the cup");
}
{
  const w = win();
  w.rail();
  await w.answer("get_keep_awake", { ...OFF, supported: false, os: "linux", mechanism: null });
  is([w.rail(), w.failures], [null, []], "a host with no mechanism: no button, no error");
  const s = w.setting();
  is([box(s).props.disabled, /systemd-inhibit/.test(texts(s))], [true, true], "…and Settings says what it would need");
}
{
  const w = win();
  w.rail();
  await w.refuse("get_keep_awake", "unknown command: get_keep_awake");
  is([w.rail(), w.failures], [null, []], "a host from before the feature: no button, no error");
  const v = win();
  v.rail();
  await v.refuse("get_keep_awake", "boom");
  is([v.rail(), v.failures], [null, ["Keep awake: boom"]], "any other failed read is said");
}

// 2. the act
{
  const w = win();
  w.rail();
  await w.answer("get_keep_awake", OFF);
  w.rail().props.onClick();
  is([w.count("set_keep_awake"), w.last("set_keep_awake").args], [1, { on: true }], "a click sends one set, of the opposite of what the host holds");
  is(w.rail().props.disabled, true, "the button waits for the answer");
  w.rail().props.onClick();
  is(w.count("set_keep_awake"), 1, "a click while one is in flight sends nothing");
  await w.answer("set_keep_awake", ON);
  let b = w.rail();
  is([b.props["aria-pressed"], /\blit\b/.test(b.props.className), b.props.disabled], [true, true, false], "held: pressed and lit");
  is(b.props.title, "Keeping this Mac awake, so agents can keep working — click to let it sleep", "held: the title says what it holds");
  const s = w.setting();
  is([box(s).props.checked, /this Mac/.test(texts(s))], [true, true], "Settings shows the same state");
  box(s).props.onChange({ currentTarget: { checked: false } });
  is(w.last("set_keep_awake").args, { on: false }, "Settings' switch sends the same act");
  await w.refuse("set_keep_awake", "denied");
  is(w.failures, ["Keep awake: denied"], "a refusal is reported");
  is(w.count("get_keep_awake"), 2, "…and the host is read again");
  await w.answer("get_keep_awake", ON);
  b = w.rail();
  is([b.props["aria-pressed"], b.props.disabled], [true, false], "…and the button still shows what the host holds");
}

// 3. events
{
  const w = win();
  w.rail();
  w.event(ON);
  await w.answer("get_keep_awake", OFF);
  yes(/\blit\b/.test(w.rail().props.className), "an event beats the read it overtook");
  w.event(OFF);
  is(w.rail().props["aria-pressed"], false, "another window's act lands");
  is(w.count("set_keep_awake"), 0, "an event sets nothing");
}

// 4. what it says
{
  const w = win();
  w.rail();
  await w.answer("get_keep_awake", { ...ON, held: false, error: "Access denied" });
  const b = w.rail();
  is([/\blit\b/.test(b.props.className), /rail-waiting/.test(b.props.className), /Access denied/.test(b.props.title)], [false, true, true], "on but not held: not lit, marked, and the title says why");
  yes(/Access denied/.test(texts(w.setting())), "…and so does Settings");
  const l = win();
  l.rail();
  await l.answer("get_keep_awake", { ...ON, os: "linux", mechanism: "systemd-inhibit" });
  yes(/Keeping this computer awake/.test(l.rail().props.title) && /systemd-inhibit/.test(texts(l.setting())), "on Linux: 'this computer', and Settings names systemd-inhibit");
  const r = win({ remote: "studio", refused: "set_keep_awake needs admin on studio" });
  r.rail();
  await r.answer("get_keep_awake", ON);
  const rb = r.rail();
  is([rb.props.disabled, /Keeping studio awake/.test(rb.props.title), /needs admin/.test(rb.props.title)], [true, true, true], "a paired host is named, and a device without admin is told why it cannot");
}

// 5. the engine
{
  const w = win();
  w.engine({ kind: "connecting" });
  w.rail();
  is(w.calls.length, 0, "no read while there is no engine");
  w.engine({ kind: "up", gen: 7 });
  w.rail();
  is(w.count("get_keep_awake"), 1, "a read once it is up");
  await w.answer("get_keep_awake", ON);
  w.engine({ kind: "up", gen: 8 });
  w.rail();
  is(w.count("get_keep_awake"), 2, "a new engine is read again");
}

// 6. the wiring
{
  const app = read("../src/App.tsx");
  const uses = [...app.matchAll(/<KeepAwakeRailButton\b/g)].map((m) => m.index);
  const at = app.indexOf('<nav\n        className="rail"\n');
  const end = app.indexOf("</nav>", at);
  is([uses.length, at > 0 && uses[0] > at && uses[0] < end], [1, true], "App renders the button once, in the Places rail");
  yes(uses[0] > app.indexOf('<div className="rail-spacer" />', at) && uses[0] < app.indexOf("<FeedbackRailButton", at), "…in its footer, before Send feedback");
  yes(!/function KeepAwakeRailButton|const KeepAwakeRailButton/.test(app), "…a component of its own module, not App's");
  const css = read("../src/App.css").replace(/\/\*[\s\S]*?\*\//g, "");
  yes(/\.rail-icon\.keep-awake\.lit \.cup \{[^}]*fill: currentColor/.test(css), "held fills the cup: more than a hue tells on from off");
  yes(/className="cup"/.test(read("../src/icons.tsx")), "icons.tsx's cup carries the class that rule fills");
}

console.log(failed ? `\nkeepawake-check: ${failed} failure(s)` : "\nkeepawake-check: all good");
process.exit(failed ? 1 : 0);
