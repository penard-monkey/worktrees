// Headless drive of the mock harness as a GUEST (group chat §7), in Chromium
// AND WebKit: the pictures in a shared session (`shared_image`). Not in CI, as
// the other *-browser.mjs are not.
//
//   app/: VITE_MOCK=1 ./node_modules/.bin/vite --port 1438 --force
//   PLAYWRIGHT_MODULE=<a local playwright> node app/scripts/guest-image-browser.mjs [port]
//
// Why: the owner's guest saw the alt text in italics where each picture should
// be (2026-11-24). It holds that the pictures SHOW (decoded, hit by a click,
// enlarged and closed), that a person's message draws only what the host
// stored (the attachment) and is quiet about the rest, that a Read outside
// the place has no thumbnail, that a host from before the row looks as it did,
// that pictures are asked for one at a time, and that one arriving late neither
// leaves a reader at the bottom short of it nor moves a reader above.
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = Number(process.argv[2] ?? 1438);
const SHOT = process.env.GUESTIMAGE_SHOT;
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));

for (const engine of ["webkit", "chromium"]) {
  const browser = await pw[engine].launch();
  const open = async (q) => {
    const page = await browser.newPage({ viewport: { width: 1365, height: 904 } });
    await page.goto(`http://localhost:${port}/?guest&beta=group_chat${q}`);
    await page.waitForSelector("[data-testid=guest-share-row]", { state: "attached", timeout: 20000 });
    await page.evaluate(() => [...document.querySelectorAll("[data-testid=guest-share-row]")].find((e) => e.dataset.share.startsWith("s_live")).click());
    await page.waitForSelector("[data-testid=guest-agent]", { timeout: 20000 });
    return page;
  };
  const fromBottom = (page) => page.evaluate(() => { const e = document.querySelector(".guest-scroll"); return e.scrollHeight - e.clientHeight - e.scrollTop; });

  {
    const page = await open("");
    await page.waitForSelector(".guest-session .chat-img-thumb img", { timeout: 20000 }).catch(() => {});
    const m = await page.evaluate(() => {
      const g = document.querySelector(".guest-session");
      const pics = [...g.querySelectorAll("[data-testid=chat-img] img")];
      return {
        alts: pics.map((i) => i.alt), decoded: pics.every((i) => i.naturalWidth === 640 && i.getBoundingClientRect().width > 100),
        thumbs: g.querySelectorAll("[data-testid=guest-agent] [data-testid=chat-img-thumb] img").length,
        reads: [...g.querySelectorAll("[data-testid=guest-call]")].filter((r) => r.textContent.startsWith("Read")).length,
        notes: [...g.querySelectorAll(".err-note, .md-img-note")].map((n) => n.textContent),
        userImg: [...g.querySelectorAll("[data-testid=guest-user] img")].map((i) => i.alt),
        userErr: [...g.querySelectorAll("[data-testid=guest-user] .err-note")].length,
        remote: [...document.querySelectorAll("img")].some((i) => /^https?:/.test(i.getAttribute("src") ?? "")),
        asked: window.__mock.guestImages(),
      };
    });
    check(JSON.stringify(m.alts) === JSON.stringify(["Home view", "Place view"]) && m.decoded, `${engine}: the reply's pictures show (${m.alts.join(", ")})`);
    check(m.thumbs === 1 && m.reads === 2, `${engine}: the Read inside the place has a thumbnail, the one outside none (${m.thumbs} of ${m.reads})`);
    check(m.notes.some((n) => n === "image not shown — not found: shots/missing.png") && m.notes.some((n) => /remote image not loaded: Logo/.test(n)) && m.notes.length === 2, `${engine}: a refusal and a remote URL are said (${m.notes.join(" | ")})`);
    check(JSON.stringify(m.userImg) === JSON.stringify(["Attached: 20261124-00000000000000aa.png"]) && m.userErr === 0 && !m.remote, `${engine}: a person's message draws the picture the host stored, says nothing of the one it refused, and nothing remote is loaded (${m.userImg.join(", ")})`);
    check(m.asked.most === 1, `${engine}: one picture at a time (${m.asked.most})`);
    if (SHOT) { await page.evaluate(() => document.querySelector(".guest-session [data-testid=chat-img]").scrollIntoView({ block: "center" })); await page.waitForTimeout(300); await page.screenshot({ path: `${SHOT}-${engine}.png` }); }
    const btn = page.locator(".guest-session .chat-img-btn").first();
    await btn.scrollIntoViewIfNeeded();
    await btn.click();
    const big = await page.locator("[data-testid=img-lightbox]").count();
    await page.keyboard.press("Escape");
    check(big === 1 && (await page.locator("[data-testid=img-lightbox]").count()) === 0, `${engine}: a click enlarges it and Esc closes it`);
    // A drop and a return keep the pictures on screen (§3.4).
    await page.evaluate(() => window.__mock.guestDrop());
    await page.waitForTimeout(400);
    const off = await page.locator(".guest-session [data-testid=chat-img] img").count();
    check(off === 2, `${engine}: offline, the pictures of the last page stay (${off})`);
    await page.close();
  }
  {
    const page = await open("&guestimage=old");
    await page.waitForTimeout(1200);
    const m = await page.evaluate(() => { const g = document.querySelector(".guest-session"); return { em: [...g.querySelectorAll(".chat-bubble-agent em:not(.md-img-note)")].map((e) => e.textContent), img: g.querySelectorAll("img").length, err: g.querySelectorAll(".err-note").length }; });
    check(JSON.stringify(m.em) === JSON.stringify(["Home view", "Place view", "Gone"]) && m.img === 0 && m.err === 0, `${engine}: a host from before the row: the alt text, no picture and no error (${JSON.stringify(m)})`);
    await page.close();
  }
  {
    const page = await open("&imgslow=700");
    await page.waitForTimeout(250);
    const before = await fromBottom(page);
    await page.waitForFunction(() => document.querySelectorAll(".guest-session [data-testid=chat-img] img").length === 2 && document.querySelector(".guest-session [data-testid=chat-img-thumb] img"), null, { timeout: 20000 });
    await page.waitForTimeout(400);
    const after = await fromBottom(page);
    check(before <= 48 && after <= 48, `${engine}: at the bottom, a picture that arrives late is followed (${Math.round(before)} then ${Math.round(after)} from the bottom)`);
    await page.close();
  }
  {
    // Reading BELOW the pictures, scrolled off the bottom. The filler turns
    // come on the view's own poll (10 s), so the pictures are slower than it.
    const page = await open("&imgslow=6000");
    await page.evaluate(() => { for (let i = 0; i < 12; i++) window.__mock.guestTurn(`Filler ${i}. ` + "More words to fill the line. ".repeat(30)); });
    await page.waitForFunction(() => document.querySelector("[data-testid=guest-list]").textContent.includes("Filler 11"), null, { timeout: 20000 });
    await page.mouse.move(500, 450);
    await page.mouse.wheel(0, -500);
    await page.waitForTimeout(200);
    // A row the reader sees, by its words.
    const row = await page.evaluate(() => { const box = document.querySelector(".guest-scroll").getBoundingClientRect(); const r = [...document.querySelectorAll(".guest-session [data-testid=guest-agent]")].find((e) => e.getBoundingClientRect().top >= box.top); return /Filler \d+\./.exec(r?.textContent ?? "")?.[0] ?? null; });
    const at = () => page.evaluate((w) => { const r = [...document.querySelectorAll(".guest-session [data-testid=guest-agent]")].find((e) => e.textContent.includes(w)); const s = document.querySelector(".guest-scroll"); return { y: r?.getBoundingClientRect().top ?? NaN, h: s.scrollHeight, fb: s.scrollHeight - s.clientHeight - s.scrollTop, n: document.querySelectorAll(".guest-session [data-testid=chat-img] img").length }; }, row);
    const a = await at();
    await page.waitForFunction(() => document.querySelectorAll(".guest-session [data-testid=chat-img] img").length === 2 && document.querySelector(".guest-session [data-testid=chat-img-thumb] img"), null, { timeout: 45000 });
    await page.waitForTimeout(400);
    const b = await at();
    check(row !== null && a.n < 2 && a.fb > 100 && b.n === 2 && b.h > a.h + 100 && Math.abs(a.y - b.y) < 2, `${engine}: a reader below them (${row}) is not moved by ${b.n - a.n} pictures arriving above (${Math.round(a.y)} → ${Math.round(b.y)}, the column ${a.h} → ${b.h})`);
    await page.close();
  }
  await browser.close();
}
console.log(failed ? `\nguest-image-browser: ${failed} failure(s)` : "\nguest-image-browser: all good");
process.exit(failed ? 1 : 0);
