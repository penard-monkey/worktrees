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

  // ── the wiring ───────────────────────────────────────────────────────────
  const pane = fs.readFileSync(path.join(ROOT, "src/ChatPane.tsx"), "utf8");
  const uses = pane.match(/<Markdown [^>]*renderImage=\{renderImage\}/g) ?? [];
  check(uses.length === 2, `ChatPane gives both the user's and the agent's bubble the hook (${uses.length} of 2)`);
  check(/readImagePath\(c\.name, c\.summary, c\.failed\)/.test(pane) && /ImageView[^>]*thumb/.test(pane), "a Read of an image draws a thumbnail");
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
