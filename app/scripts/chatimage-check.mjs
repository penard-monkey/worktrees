// chatimage-check — images in the chat (feat/chat-images).
//
// Runs the REAL source (vite's SSR loader, so the TS and TSX are the files the
// app builds, not paraphrases): `imgsrc.ts`'s rule, the real `Markdown` with
// the real `chatImageNode` (the chat's `renderImage`), and `ImageView.tsx`'s
// cache. The ipc door is the one stub, a virtual module.
//
// What it holds:
//   - a relative path and an absolute one under the place SHOW; a remote URL,
//     data:, any other scheme, a path outside the place, `..` and a file that
//     is no image do NOT, and each says so in a note that names the path
//     (never an `<img>` for a remote one, never a silent drop);
//   - the cache: a fresh image costs no call, a stale one costs ONE call that
//     carries its etag, `unchanged` keeps the pixels the client holds, a
//     changed file replaces them, one call per image is in flight, and the
//     cache is bounded;
//   - a GUEST's view of a shared session: `shared_image` over the guest
//     connection, naming the share, turn and path and no repo or slug; one
//     picture in flight per connection; the agent's bubble alone; nothing
//     through the engine door;
//   - ChatPane gives BOTH bubbles the hook and a Read of an image its thumb.
//
// CHATIMAGE_ROOT points the check at another app/ (to show it red).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const here = (p) => path.join(path.dirname(fileURLToPath(import.meta.url)), p);
const ROOT = process.env.CHATIMAGE_ROOT ?? here("..");
let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const check = (cond, m) => (cond ? ok(m) : fail(m));

const stubIpc = {
  name: "stub-ipc",
  enforce: "pre",
  resolveId(id) { return /(^|\/)ipc$/.test(id) ? "\0ipc-stub" : null; },
  load(id) {
    return id === "\0ipc-stub"
      ? "export const invoke = (...a) => globalThis.__invoke(...a); export const windowOnHost = () => true;"
      : null;
  },
};
const server = await createServer({
  root: ROOT, configFile: false, logLevel: "silent", appType: "custom", plugins: [stubIpc],
  server: { middlewareMode: true, hmr: false, watch: null },
  optimizeDeps: { noDiscovery: true, include: [] },
});
const load = (p) => server.ssrLoadModule(p);
try {
  const { chatImageSrc, readImagePath } = await load("/src/imgsrc.ts");
  const { chatImageNode } = await load("/src/chatImageNode.tsx");
  const { Markdown } = await load("/src/markdown.tsx");
  const IV = await load("/src/ImageView.tsx");
  const { createElement } = await import(path.join(ROOT, "node_modules/react/index.js"));
  const { renderToStaticMarkup } = await import(path.join(ROOT, "node_modules/react-dom/server.node.js"));

  const place = "/work/proj/.worktrees/feat-x";
  const placeRef = { repo: "/work/proj", slug: "feat-x" };
  const html = (src) =>
    renderToStaticMarkup(createElement(Markdown, { src, renderImage: (s, a) => chatImageNode(s, a, place, placeRef) }));

  // ── the rule, through the real Markdown ──────────────────────────────────
  const shows = (md) => /loading image…/.test(html(md)) && !/<img/.test(html(md));
  check(shows("![Preview](.worktrees-attachments/p.png)"), "a relative path shows (the image is asked for)");
  check(shows(`![Abs](${place}/shots/a.jpg)`), "an absolute path under the place shows");
  check(shows("![Enc](shots/my%20pic.png)"), "a percent-encoded path shows");
  for (const [md, needle, why] of [
    ["![Logo](https://example.com/l.png)", "remote image not loaded: Logo (https://example.com/l.png)", "a remote URL"],
    ["![Logo](//cdn.example.com/l.png)", "remote image not loaded", "a protocol-relative URL"],
    ["![x](data:image/png;base64,AAAA)", "data: sources are never loaded", "a data: URI"],
    ["![x](file:///etc/passwd.png)", "file: sources are never loaded", "a file: URL"],
    ["![Secret](/etc/secret.png)", "outside this place: /etc/secret.png", "an absolute path elsewhere"],
    [`![Sib](/work/proj/.worktrees/other/a.png)`, "outside this place: /work/proj/.worktrees/other/a.png", "another place's tree"],
    ["![Up](../other/s.png)", "outside this place: ../other/s.png", "a .. escape"],
    ["![Up](shots/../../s.png)", "outside this place: shots/../../s.png", "a .. in the middle"],
    [`![Dots](${place}/../x.png)`, "outside this place", "an absolute path with .."],
    ["![Prefix](/work/proj/.worktrees/feat-xyz/a.png)", "outside this place", "a sibling whose name starts like the place"],
  ]) {
    const h = html(md);
    check(h.includes(needle) && !/<img|loading image/.test(h), `${why} is a note naming it and no image`);
  }
  check(/\[Notes\]/.test(html("![Notes](notes.txt)")) && !/<img/.test(html("![Notes](notes.txt)")), "a file that is no image is a note");
  check(!/<img/.test(html("![a](https://x/y.png) ![b](/etc/p.png) ![c](../d.png)")), "no <img> is ever built for a refused source");

  // ── (main): its root holds every other place's tree ─────────────────────
  const mroot = "/work/proj";
  const kind = (src, isMain, r = mroot) => chatImageSrc(src, "", r, isMain).kind;
  check(kind(".worktrees/x/a.png", true) === "note", "(main): a relative path under .worktrees/ is a note");
  check(kind("./.worktrees/x/a.png", true) === "note", "(main): ./.worktrees/ is a note too");
  check(kind(`${mroot}/.worktrees/x/a.png`, true) === "note", "(main): an absolute path under <root>/.worktrees/ is a note");
  check(kind(".Worktrees/x/a.png", true) === "note" && kind(`${mroot}/.WORKTREES/x/a.png`, true) === "note", "(main): .Worktrees is a note (APFS is case-insensitive)");
  check(/outside this place/.test(chatImageSrc(`${mroot}/.worktrees/x/a.png`, "", mroot, true).text ?? ""), "(main): the note says `outside this place`");
  check(kind("docs/a.png", true) === "show" && kind(`${mroot}/docs/a.png`, true) === "show" && kind(".worktrees-attachments/a.png", true) === "show", "(main): docs/a.png and .worktrees-attachments/ show");
  check(kind(".worktrees/x/a.png", false) === "show" && kind(`${mroot}/.worktrees/x/a.png`, false) === "show" && kind(".Worktrees/x/a.png", false) === "show", "a non-main place: the same paths show, unchanged");
  check(kind(".worktrees/x/a.png", undefined) === "show", "the flag defaults to a non-main place");

  // ── a Read's thumbnail ───────────────────────────────────────────────────
  check(readImagePath("Read", "/p/a.PNG", false) === "/p/a.PNG", "a Read of a png has a thumbnail path");
  check(readImagePath("Read", "/p/a.ts", false) === null && readImagePath("Edit", "/p/a.png", false) === null && readImagePath("Read", "/p/a.png", true) === null,
    "no thumbnail for a text file, another tool, or a failed read");

  // ── the cache ────────────────────────────────────────────────────────────
  let calls = [];
  let disk = { etag: "1-100", b64: "AAAA", mime: "image/png" };
  globalThis.__invoke = async (cmd, args) => {
    calls.push({ cmd, ...args });
    if (args.etag === disk.etag) return { b64: "", size: 3, truncated: false, mtime: 1, mime: disk.mime, etag: disk.etag, unchanged: true };
    return { b64: disk.b64, size: 3, truncated: false, mtime: 1, mime: disk.mime, etag: disk.etag, unchanged: false };
  };
  IV.clearPlaceImages();
  const t0 = 1_000_000;
  const a = await IV.loadPlaceImage(placeRef, "a.png", t0);
  check(calls.length === 1 && calls[0].cmd === "read_place_image" && calls[0].etag === undefined && calls[0].slug === "feat-x" && a.b64 === "AAAA", "first load: one read_place_image, no etag");
  await IV.loadPlaceImage(placeRef, "a.png", t0 + 500);
  check(calls.length === 1, "a fresh image is not asked about again");
  const b = await IV.loadPlaceImage(placeRef, "a.png", t0 + 10_000);
  check(calls.length === 2 && calls[1].etag === "1-100" && b.b64 === "AAAA", "a stale one is revalidated with its etag, and the pixels the client holds stand (`unchanged`)");
  check(IV.peekPlaceImage(placeRef, "a.png")?.b64 === "AAAA", "peek gives a re-mounted bubble its picture in the first frame");
  disk = { etag: "2-120", b64: "BBBB", mime: "image/png" };
  const c = await IV.loadPlaceImage(placeRef, "a.png", t0 + 20_000);
  check(c.b64 === "BBBB" && IV.peekPlaceImage(placeRef, "a.png")?.etag === "2-120", "a file that changed on disk replaces the cached one");
  calls = [];
  const [p1, p2, p3] = [IV.loadPlaceImage(placeRef, "n.png", t0), IV.loadPlaceImage(placeRef, "n.png", t0), IV.loadPlaceImage(placeRef, "n.png", t0)];
  await Promise.all([p1, p2, p3]);
  check(calls.length === 1, "three bubbles naming one image share ONE call");
  check(IV.peekPlaceImage({ repo: "/work/proj", slug: "other" }, "a.png") === null, "the cache is per place");
  globalThis.__invoke = async () => { throw "outside this place: /etc/x.png"; };
  let refused = null;
  await IV.loadPlaceImage(placeRef, "z.png", t0).catch((e) => { refused = String(e); });
  check(refused === "outside this place: /etc/x.png" && IV.peekPlaceImage(placeRef, "z.png") === null, "a refusal reaches the viewer as the host's words and is not cached");
  IV.clearPlaceImages();
  globalThis.__invoke = async (_c, args) => ({ b64: "x".repeat(10 * 1024 * 1024), size: 1, truncated: false, mtime: 1, mime: "image/png", etag: `e-${args.path}`, unchanged: false });
  for (const n of ["1", "2", "3", "4"]) await IV.loadPlaceImage(placeRef, `${n}.png`, t0);
  check(IV.peekPlaceImage(placeRef, "1.png") === null && IV.peekPlaceImage(placeRef, "4.png") !== null, `the cache is bounded (${IV.CACHE_MAX_BYTES >> 20} MiB): the oldest goes first`);

  // ── the picture a Read was shown (outside the place, the console's) ──────
  IV.clearPlaceImages();
  calls = [];
  globalThis.__invoke = async (cmd, args) => { calls.push({ cmd, ...args }); return { call_id: args.call_id, b64: "PNGDATA", size: 7, truncated: false, mime: "image/png" }; };
  const cref = { ...placeRef, callId: "toolu_x" };
  const c1 = await IV.loadCallImage(cref, t0);
  await IV.loadCallImage(cref, t0 + 999_999);
  check(calls.length === 1 && calls[0].cmd === "agent_call_image" && calls[0].call_id === "toolu_x" && c1.b64 === "PNGDATA", "a Read's picture is asked for once by its call id, and kept (its pixels never change)");
  check(IV.peekCallImage(cref)?.mime === "image/png" && IV.peekPlaceImage(placeRef, "toolu_x") === null, "a call's picture and a place image never share a key");
  IV.clearPlaceImages();
  globalThis.__invoke = async () => { throw "not shown on this device: this picture is a file Claude read, which only the host's own console may see"; };
  let said = null;
  await IV.loadCallImage(cref, t0).catch((e) => { said = String(e); });
  check(/not shown on this device/.test(said ?? "") && IV.peekCallImage(cref) === null, "a device's refusal reaches the viewer in the host's words and is not cached");
  html("![x](a.png)"); // (the rule's html helper is unaffected)

  // ── the host disagrees: a Read the client called `show` ─────────────────
  check(/fallbackCall=\{c\.call_id \?/.test(fs.readFileSync(path.join(ROOT, "src/ChatPane.tsx"), "utf8")), "a Read with a call id hands ImageView the call to fall back to");
  const ivsrc = fs.readFileSync(path.join(ROOT, "src/ImageView.tsx"), "utf8");
  check(/fallbackCall/.test(ivsrc) && /\^outside this place/.test(ivsrc), "ImageView falls back only on the host's `outside this place`");

  // ── a guest's view of a shared session ──────────────────────────────────
  {
    const G = await load("/src/GuestImage.tsx");
    let door = 0;
    globalThis.__invoke = async () => { door += 1; throw "the engine door"; };
    const fake = (answer) => {
      const l = { sent: [], most: 0, now: 0, kind: "up", state: () => ({ kind: l.kind }) };
      l.call = async (cmd, args) => {
        l.sent.push({ cmd, ...args });
        l.most = Math.max(l.most, ++l.now);
        await new Promise((r) => setTimeout(r, 1));
        l.now -= 1;
        return answer(args);
      };
      return l;
    };
    const png = (args) => (args.etag === "1-9" ? { b64: "", size: 9, truncated: false, mtime: 1, mime: "image/png", etag: "1-9", unchanged: true } : { b64: "AAAA", size: 9, truncated: false, mtime: 1, mime: "image/png", etag: "1-9", unchanged: false });
    const link = fake(png);
    const at = { link, host: "h1", share: "s_1", binding: undefined, isMain: false, up: true };
    const ghtml = (src, a = at) => renderToStaticMarkup(createElement(Markdown, { src, renderImage: (s, alt) => G.sharedImageNode(s, alt, a, 7) }));
    const asks = (md, a) => /loading image…/.test(ghtml(md, a));
    check(asks("![Home](.worktrees-attachments/agent/disk-home.png)") && asks("![Abs](/Users/david/x/.worktrees/lane/a.png)"), "guest: a relative path and an absolute one are asked for (the host alone knows the place's root)");
    for (const [md, needle] of [["![L](https://e.com/l.png)", "remote image not loaded"], ["![x](data:image/png;base64,AA)", "data: sources are never loaded"], ["![u](../o/s.png)", "outside this place"], ["![u](/a/../s.png)", "outside this place"], ["![n](n.txt)", "[n]"]])
      check(ghtml(md).includes(needle) && !/<img|loading image/.test(ghtml(md)), `guest: ${md} is a note, never asked for`);
    check(!asks("![w](.Worktrees/x/a.png)", { ...at, isMain: true }) && asks("![w](.worktrees/x/a.png)"), "guest: (main)'s .worktrees/ is a note there too");

    IV.clearPlaceImages();
    const g1 = await G.loadSharedImage(at, 7, "a.png", t0);
    check(JSON.stringify(link.sent) === JSON.stringify([{ cmd: "shared_image", share: "s_1", turn: 7, path: "a.png" }]) && g1.b64 === "AAAA", "guest: one shared_image names the share, the turn and the path, and no repo, slug or etag");
    await G.loadSharedImage(at, 7, "a.png", t0 + 500);
    const g2 = await G.loadSharedImage(at, 7, "a.png", t0 + 10_000);
    check(link.sent.length === 2 && link.sent[1].etag === "1-9" && g2.b64 === "AAAA", "guest: fresh costs no call, stale sends its etag and `unchanged` keeps the pixels");
    check(IV.peekPlaceImage(placeRef, "a.png") === null && G.peekSharedImage({ ...at, share: "s_2" }, "a.png") === null && G.peekSharedImage({ ...at, host: "h2" }, "a.png") === null && G.peekSharedImage({ ...at, binding: 0 }, "a.png") === null, "guest: the cache is per host, share and binding, and never the owner's chat's");
    await G.loadSharedImage({ ...at, binding: 0 }, 7, "a.png", t0);
    check(link.sent[2].binding === 0, "guest: an earlier session is named by its binding");
    link.sent = [];
    await Promise.all(["1", "2", "3", "3"].map((n) => G.loadSharedImage(at, 7, `${n}.png`, t0)));
    check(link.sent.length === 3 && link.most === 1, `guest: one picture in flight per connection (${link.most}), and one call per picture (${link.sent.length})`);
    const bad = fake((a) => { if (a.path === "x.png") throw "not found: x.png"; return png(a); });
    const badAt = { ...at, host: "h3", link: bad };
    const [r1, r2] = await Promise.allSettled([G.loadSharedImage(badAt, 1, "x.png", t0), G.loadSharedImage(badAt, 1, "y.png", t0)]);
    check(r1.reason === "not found: x.png" && r2.value?.b64 === "AAAA" && G.peekSharedImage(badAt, "x.png") === null, "guest: a refusal is the host's words, not cached, and does not hold up the next picture");
    check(G.refusalShows("unknown command: shared_image", false) === "plain" && G.refusalShows("unknown command: shared_image", true) === "plain", "guest: a host from before the row looks as it did: alt text, no thumbnail");
    check(G.refusalShows("outside this place: /tmp/a.png", true) === "plain" && G.refusalShows("outside this place: a.png", false) === "words" && G.refusalShows("not found: a.png", true) === "words", "guest: a Read outside the place has no thumbnail; every other refusal is said");
    check(door === 0, "guest: nothing went through the engine door");
    const gs = fs.readFileSync(path.join(ROOT, "src/GuestSession.tsx"), "utf8");
    check((gs.match(/<Markdown [^>]*renderImage=/g) ?? []).length === 1 && /chat-bubble-agent"><Markdown [^>]*renderImage=/.test(gs), "guest: the agent's bubble draws pictures, a person's does not (a guest's own words reach it)");
    check(/<SharedThumb [^>]*readImagePath\(props\.name, props\.summary, props\.failed\)/.test(gs), "guest: a Read of an image draws its thumbnail");
    // `marked` hands the path as the host's `image_paths` reads it (the Rust
    // test's own table): the two must agree or a picture is refused.
    const rs = fs.readFileSync(path.join(ROOT, "../crates/worktrees-cli/src/serve/shareimage.rs"), "utf8");
    const body = /for \(md, want\) in \[([\s\S]*?)\n\s*\] \{/.exec(rs)?.[1] ?? "";
    const pairs = [...body.matchAll(/\("((?:[^"\\]|\\.)*)", "((?:[^"\\]|\\.)*)"\)/g)].map((m) => [JSON.parse(`"${m[1]}"`), JSON.parse(`"${m[2]}"`)]);
    const wrong = [];
    for (const [md, want] of pairs) {
      let got = null;
      renderToStaticMarkup(createElement(Markdown, { src: md, renderImage: (s) => { const r = chatImageSrc(s, "", null); got = r.kind === "show" ? r.path : r.text; return null; } }));
      if (got !== want) wrong.push(`${JSON.stringify(md)}: marked ${JSON.stringify(got)}, host ${JSON.stringify(want)}`);
    }
    check(pairs.length >= 12 && wrong.length === 0, `guest: marked draws each of the host's ${pairs.length} image forms at the path the host reads${wrong.length ? ` — ${wrong.join("; ")}` : ""}`);
    IV.clearPlaceImages();
  }

  // ── the wiring ───────────────────────────────────────────────────────────
  const pane = fs.readFileSync(path.join(ROOT, "src/ChatPane.tsx"), "utf8");
  const uses = pane.match(/<Markdown [^>]*renderImage=\{renderImage\}/g) ?? [];
  check(uses.length === 2, `ChatPane gives both the user's and the agent's bubble the hook (${uses.length} of 2)`);
  check(/readImagePath\(c\.name, c\.summary, c\.failed\)/.test(pane) && /ImageView[^>]*thumb/.test(pane), "a Read of an image draws a thumbnail");
  check(/call=\{byCall \?/.test(pane) && /thumb\?\.kind === "note" && !!c\.call_id/.test(pane), "a Read outside the place falls back to the picture the call was shown, only with a call id");
  check(!/read_file_base64/.test(pane + fs.readFileSync(path.join(ROOT, "src/chatImageNode.tsx"), "utf8")), "the chat never asks for an image through the project-wide read");
  const fp = fs.readFileSync(path.join(ROOT, "src/FilesPane.tsx"), "utf8");
  check(!/function ImageView/.test(fp) && /from "\.\/ImageView"/.test(fp), "FilesPane uses the shared ImageView, not a copy");
} finally {
  await server.close();
}
if (failed) {
  console.log(`\nchatimage-check: ${failed} failure(s)`);
  process.exit(1);
}
console.log("\nchatimage-check: all ok");
