// Headless drive of the isolated mock harness (port 1461 by default), in
// Chromium AND WebKit (the app's engine): images in the chat
// (feat/chat-images). Run against `VITE_MOCK=1 vite --port 1461 --force` in
// app/ (inside `.worktrees` vite's HMR is dead: restart after every edit and
// content-check what it serves), with PLAYWRIGHT_MODULE pointing at a local
// Playwright installation.
//
// `?turns=images` (mock/install.ts `mockImageTurns`) gives the place a user
// message with an image, a Read of an image in the place and one in /tmp, and
// a reply carrying: a wide image, a small one by absolute path, a remote URL,
// a path outside the place, a `..` escape, a link out and a text file.
//   1. what shows and what is a note, and that the remote one is never an <img>;
//   2. click-to-enlarge, and Esc closes it;
//   3. no refetch: leaving the chat and coming back reads each image once;
//   4. the scroll: an image that loads BELOW the fold holds a reader who has
//      scrolled up to the pixel, and a reader at the bottom stays at it.
// Screenshots land in WT_SHOTS (default ~/.cache/worktrees/worktrees/feat-chat-images).
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const { chromium, webkit } = pw.chromium ? pw : pw.default;

const base = process.env.CHAT_IMAGE_URL ?? "http://localhost:1461";
const shots = process.env.WT_SHOTS ?? path.join(os.homedir(), ".cache/worktrees/worktrees/feat-chat-images");
fs.mkdirSync(shots, { recursive: true });
const only = process.argv.find((a) => a.startsWith("--engine="))?.slice(9);
let failures = 0;
const check = (ok, message) => { if (!ok) failures++; console.log(`${ok ? "ok" : "not ok"} — ${message}`); };

async function open(page, query) {
  await page.goto(`${base}/?${query}`);
  await page.locator('[data-testid="places-rail"]').click();
  await page.locator('li.row[data-slug="messaging"]').click();
  await page.locator('[data-testid="chat-list"] > li').first().waitFor();
}
const geom = (page) => page.evaluate(() => {
  const el = document.querySelector(".chat-scroll");
  const box = el.getBoundingClientRect();
  const rows = [...document.querySelectorAll('[data-testid="chat-list"] > li')];
  const top = rows.find((li) => li.getBoundingClientRect().bottom > box.top + 1);
  return {
    st: Math.round(el.scrollTop), sh: el.scrollHeight, ch: el.clientHeight,
    fromBottom: Math.round(el.scrollHeight - el.clientHeight - el.scrollTop),
    top: top ? top.textContent.replace(/\s+/g, " ").trim().slice(0, 40) : null,
    topOff: top ? Math.round(top.getBoundingClientRect().top - box.top) : null,
  };
});
const loadedImgs = (page) => page.evaluate(() => [...document.querySelectorAll('[data-testid="chat-img"] img')].filter((i) => i.complete && i.naturalWidth > 0).length);

async function show(browserType, name) {
  const browser = await browserType.launch();
  console.log(`# ${name}`);
  try {
    // ── 1–3: what shows ────────────────────────────────────────────────────
    {
      const page = await (await browser.newContext({ viewport: { width: 1100, height: 900 } })).newPage();
      await open(page, "turns=images&activity=3&agentprompt=off");
      await page.waitForFunction(() => document.querySelectorAll('[data-testid="chat-img"] img').length >= 3, null, { timeout: 8000 });
      await page.waitForTimeout(300);
      const view = await page.evaluate(() => {
        const q = (s) => [...document.querySelectorAll(s)];
        const agent = q('[data-testid="chat-agent"]').at(-1);
        const user = q('[data-testid="chat-user"]').find((u) => /Look at this one/.test(u.textContent));
        return {
          agentImgs: agent.querySelectorAll('.chat-bubble [data-testid="chat-img"] img').length,
          userImgs: user.querySelectorAll('[data-testid="chat-img"] img').length,
          thumbs: q('[data-testid="chat-img-thumb"] img').length,
          thumbRows: q('[data-testid="chat-call"]').filter((r) => r.querySelector('[data-testid="chat-img-thumb"]')).length,
          notes: q('[data-testid="chat-img-note"], .chat-bubble .inline-note').map((n) => n.textContent.trim()),
          srcs: q("img").map((i) => i.getAttribute("src") ?? ""),
          sizes: q('[data-testid="chat-img"] img').map((i) => [i.naturalWidth, i.naturalHeight]),
          broken: q("img").filter((i) => i.complete && i.naturalWidth === 0).length,
        };
      });
      console.log("  notes:", JSON.stringify(view.notes));
      check(view.agentImgs === 2, `the reply shows its two place images (${view.agentImgs})`);
      check(view.userImgs === 1, `the user's own message shows its image (${view.userImgs})`);
      check(view.thumbs === 1 && view.thumbRows === 1, `a Read of an image in the place has a thumbnail, and the /tmp one has none (${view.thumbs})`);
      check(view.sizes.some(([w]) => w === 640) && view.sizes.some(([w]) => w === 64), `the images decode at their own size (${JSON.stringify(view.sizes)})`);
      check(view.broken === 0, "no broken image anywhere");
      check(!view.srcs.some((s) => /example\.com/.test(s)) && view.srcs.every((s) => s.startsWith("data:") || s === "" || /^(blob:|http:\/\/localhost)/.test(s)),
        "a remote image is never an <img> src, and nothing but data: URIs is loaded");
      const has = (re) => view.notes.some((n) => re.test(n));
      check(has(/remote image not loaded: Remote logo \(https:\/\/example\.com\/logo\.png\)/), "the remote image is a note");
      check(has(/outside this place: \/etc\/secret\.png/), "an absolute path outside the place is a note naming it");
      check(has(/outside this place: \.\.\/other\/secret\.png/), "a .. escape is a note naming it");
      check(has(/image not shown — outside this place: shots\/linked\.png/), "a link out, refused by the host, shows the host's words");
      check(has(/\[Notes\]/), "a text file is a note");
      await page.screenshot({ path: path.join(shots, `chat-images-${name}.png`) });

      // 2. click to enlarge
      await page.locator('[data-testid="chat-agent"]').last().locator('[data-testid="chat-img"] button').first().click();
      await page.locator('[data-testid="img-lightbox"]').waitFor();
      const box = await page.evaluate(() => { const i = document.querySelector('[data-testid="img-lightbox"] img'); const r = i.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)]; });
      check(box[0] > 500, `click enlarges the picture (${box.join("×")})`);
      await page.screenshot({ path: path.join(shots, `chat-images-lightbox-${name}.png`) });
      await page.keyboard.press("Escape");
      await page.waitForFunction(() => !document.querySelector('[data-testid="img-lightbox"]'));
      check(true, "Esc closes it");
      await page.locator('[data-testid="chat-agent"]').last().locator('[data-testid="chat-img"] button').first().click();
      await page.locator('[data-testid="img-lightbox"]').click();
      await page.waitForFunction(() => !document.querySelector('[data-testid="img-lightbox"]'));
      check(true, "a click closes it");

      // 3. no refetch across a remount
      const reads = () => page.evaluate(() => window.__mock.imageReads.slice());
      const first = await reads();
      const dup = first.filter((p, i) => first.indexOf(p) !== i);
      check(first.length >= 3 && dup.length === 0, `each image was read once (${first.length} reads, ${dup.length} repeats)`);
      await page.waitForTimeout(2300); // past the cache's freshness: the next mount revalidates
      const terminal = page.getByRole("button", { name: /^Terminal$/ });
      if (await terminal.count()) {
        await terminal.first().click();
        await page.getByRole("button", { name: /^Chat$/ }).first().click();
        await page.waitForFunction(() => document.querySelectorAll('[data-testid="chat-img"] img').length >= 3);
      }
      const after = await reads();
      check(after.length === first.length, `leaving the chat and coming back reads no bytes again (${first.length} → ${after.length})`);
      // an image rewritten on disk IS read again
      const target = first.find((p) => p.endsWith("preview.png"));
      await page.evaluate((p) => window.__mock.imageTouched(p), target);
      await page.waitForTimeout(2300);
      if (await terminal.count()) {
        await terminal.first().click();
        await page.getByRole("button", { name: /^Chat$/ }).first().click();
        await page.waitForTimeout(600);
        const again = await reads();
        check(again.length === first.length + 1 && again.at(-1) === target, `an image that changed on disk is read again (${again.length - first.length} new read)`);
      } else check(false, "the Chat/Terminal switch was not found");
      await page.context().close();
    }
    // ── 4: the scroll ──────────────────────────────────────────────────────
    {
      // images answer after 1.5 s, so there is time to scroll first
      const page = await (await browser.newContext({ viewport: { width: 1100, height: 700 } })).newPage();
      await open(page, "turns=images&filler=14&imgslow=1500&activity=3&agentprompt=off");
      await page.waitForTimeout(300);
      const start = await geom(page);
      check(start.fromBottom <= 2, `the chat opens at the newest (fromBottom ${start.fromBottom})`);
      // a reader at the bottom stays at it while the images arrive
      await page.waitForFunction(() => document.querySelectorAll('[data-testid="chat-img"] img').length >= 3, null, { timeout: 9000 });
      await page.waitForTimeout(500);
      const pinned = await geom(page);
      check(pinned.fromBottom <= 2 && pinned.sh > start.sh, `at the bottom: still at the bottom after the images load (fromBottom ${pinned.fromBottom}, height ${start.sh} → ${pinned.sh})`);
      await page.screenshot({ path: path.join(shots, `chat-scroll-bottom-${name}.png`) });

      // a reader who scrolled up holds, to the pixel, when images load below the fold
      const p2 = await (await browser.newContext({ viewport: { width: 1100, height: 700 } })).newPage();
      await open(p2, "turns=images&filler=14&imgslow=1500&activity=3&agentprompt=off");
      await p2.waitForTimeout(200);
      await p2.evaluate(() => { document.querySelector(".chat-scroll").scrollTop = 600; });
      await p2.waitForTimeout(150);
      const before = await geom(p2);
      const imgsBefore = await loadedImgs(p2);
      await p2.waitForFunction(() => document.querySelectorAll('[data-testid="chat-img"] img').length >= 3, null, { timeout: 9000 });
      await p2.waitForTimeout(600);
      const after = await geom(p2);
      check(imgsBefore === 0 && before.fromBottom > 100, `set up: scrolled up (fromBottom ${before.fromBottom}) with no image loaded yet`);
      check(after.sh > before.sh, `the images made the column taller (${before.sh} → ${after.sh})`);
      check(after.st === before.st && after.top === before.top && after.topOff === before.topOff,
        `scrolled up: the view holds to the pixel (st ${before.st} → ${after.st}, top row @${before.topOff} → @${after.topOff})`);
      await p2.screenshot({ path: path.join(shots, `chat-scroll-held-${name}.png`) });
      await p2.context().close();
      await page.context().close();
    }
  } finally {
    await browser.close();
  }
}

if (only !== "webkit") await show(chromium, "chromium");
if (only !== "chromium") await show(webkit, "webkit");
console.log(failures ? `\n${failures} failure(s)` : "\nall ok");
process.exit(failures ? 1 : 0);
