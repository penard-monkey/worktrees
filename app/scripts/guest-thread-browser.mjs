// Headless drive of the mock harness as a GUEST (group chat §7), in Chromium
// AND WebKit: the thread column beside a shared session is all there — head,
// entries, composer — and each part is where a click lands.
//
//   app/: VITE_MOCK=1 ./node_modules/.bin/vite --port 1437 --force
//   PLAYWRIGHT_MODULE=<a local playwright> node app/scripts/guest-thread-browser.mjs [port]
//
// Why: on the owner's guest VM (system WKWebView, 1365×904, sidebar open) the
// column showed ONLY the composer (2026-11-14): `.thread`'s `height: 100%` did
// not resolve against `.guest-thread`, `.thread-list` collapsed and the head
// went out of view. Rects alone say nothing about reachability (AGENTS.md), so
// every part is hit-tested with `elementFromPoint` too. Measured with the
// sidebar pinned and not, at the VM's size and a short window.
// GUESTTHREAD_INJECT=<css> adds CSS to the page (a mutation, to show it red):
// `.guest-thread > .thread { height: auto !important; flex: none !important }`
// is the VM's collapse, a panel that takes its content's height.
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = Number(process.argv[2] ?? 1437);
// Group chat is a beta: the mock turns it on with ?beta=group_chat.
const URL0 = `http://localhost:${port}/?guest&beta=group_chat`;
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));

for (const engine of ["webkit", "chromium"]) {
  const browser = await pw[engine].launch();
  for (const [w, h, pin] of [[1365, 904, true], [1365, 904, false], [1100, 560, true]]) {
    const page = await browser.newPage({ viewport: { width: w, height: h } });
    const tag = `${engine} ${w}×${h}${pin ? " sidebar pinned" : ""}`;
    await page.goto(URL0);
    // A mutation hook: CSS added to the page, to show a broken column red.
    if (process.env.GUESTTHREAD_INJECT) await page.addStyleTag({ content: process.env.GUESTTHREAD_INJECT });
    await page.waitForSelector("[data-testid=guest-share-row]", { state: "attached", timeout: 15000 });
    if (pin) { await page.keyboard.press("Meta+b"); await page.waitForTimeout(400); }
    // The live share: allowed, with a thread of three entries (mock/guest.ts).
    await page.evaluate(() => [...document.querySelectorAll("[data-testid=guest-share-row]")].find((e) => e.dataset.share.startsWith("s_live")).click());
    await page.waitForSelector(".guest-thread .thread-post", { timeout: 15000 }).catch(() => {});
    const m = await page.evaluate(() => {
      const r = (s) => { const e = document.querySelector(s); if (!e) return null; const b = e.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height }; };
      const inside = (sel, b) => { if (!b) return false; const e = document.elementFromPoint(b.x + b.w / 2, b.y + Math.min(b.h / 2, 12)); return !!e?.closest(sel); };
      const col = r(".guest-thread"), head = r(".guest-thread .thread-head"), list = r(".guest-thread .thread-list"), compose = r(".guest-thread .thread-compose"), post = r(".guest-thread .thread-post");
      return {
        col, head, list, compose, post,
        hitHead: inside(".thread-head", head), hitList: inside(".thread-list", list), hitPost: inside(".thread-post", post), hitField: inside(".thread-compose", compose),
        vh: window.innerHeight,
      };
    });
    const fmt = (b) => (b ? `${Math.round(b.y)}+${Math.round(b.h)}` : "none");
    check(m.col && m.col.h > 0 && m.col.y + m.col.h <= m.vh + 1, `${tag}: the column fits the window (${fmt(m.col)} of ${m.vh})`);
    check(m.head && m.col && m.head.y >= m.col.y - 1 && m.head.h > 0 && m.hitHead, `${tag}: the thread's head is shown and on top (${fmt(m.head)})`);
    check(m.list && m.list.h >= 80 && m.hitList, `${tag}: the entries' list has room and takes a click (${fmt(m.list)})`);
    check(m.post && m.list && m.post.y >= m.list.y - 1 && m.post.y < m.list.y + m.list.h && m.hitPost, `${tag}: an entry shows inside the list (${fmt(m.post)})`);
    check(m.compose && m.col && m.compose.y + m.compose.h <= m.col.y + m.col.h + 1 && m.hitField, `${tag}: the composer sits at the column's foot (${fmt(m.compose)})`);
    await page.close();
  }
  await browser.close();
}
console.log(failed ? `\nguest-thread-browser: ${failed} failure(s)` : "\nguest-thread-browser: all good");
process.exit(failed ? 1 : 0);
