// Headless drive of the mock harness as the OWNER (group chat §2.2), in
// WebKit AND Chromium: a guest's share request reaches the corner whatever
// lane is on screen, with *Share…* (the share dialog for that place, the asker
// ticked) and *Dismiss*; a request for a place with no live session says so.
//
//   app/: VITE_MOCK=1 ./node_modules/.bin/vite --port 1437 --force
//   PLAYWRIGHT_MODULE=<a local playwright> node app/scripts/share-requests-browser.mjs [port]
//
// Why: in the 2026-11-14 rehearsal the request showed only in its lane's
// share bar (`RequestNudge`), so the owner never saw it: the corner
// (`ShareAlerts`) showed join asks alone.
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = Number(process.argv[2] ?? 1437);
const URL0 = `http://localhost:${port}/?beta=group_chat`;
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));

for (const engine of ["webkit", "chromium"]) {
  const browser = await pw[engine].launch();
  const page = await browser.newPage({ viewport: { width: 1365, height: 904 } });
  const tag = engine;
  await page.goto(URL0);
  await page.waitForSelector("[data-track=settings]");
  await page.waitForTimeout(1500);
  // A place with a live Claude or Codex, and one with none, from the mock's own workspace.
  const pick = await page.evaluate(async () => {
    const ws = await window.__TAURI_INTERNALS__.invoke("list_workspace", {});
    let up = null, down = null;
    for (const pv of ws.projects) for (const p of pv.snapshot?.places ?? []) {
      const a = p.agent_sessions ?? {};
      const live = a.claude?.up || a.codex?.up;
      if (live && !up) up = { repo: pv.root, slug: p.slug };
      if (!live && !Object.values(a).some((x) => x?.up) && !down) down = { repo: pv.root, slug: p.slug };
    }
    return { up, down };
  });
  if (!pick.up || !pick.down) { fail(`${tag}: the mock has a live place and an idle one (${JSON.stringify(pick)})`); await browser.close(); continue; }
  await page.evaluate(({ up, down }) => {
    window.__mock.guestDevice(); // a request comes from Bret's paired device
    window.__mock.guestRequest(up.repo, up.slug, "could I watch?");
    window.__mock.guestRequest(down.repo, down.slug, null);
  }, pick);
  await page.waitForSelector("[data-testid=share-request-alert]", { timeout: 15000 }).catch(() => {});
  await page.waitForTimeout(800);
  const rows = await page.evaluate(() => [...document.querySelectorAll("[data-testid=share-request-alert]")].map((e) => {
    const b = e.getBoundingClientRect();
    const at = document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2);
    return { text: e.textContent, share: !!e.querySelector("[data-testid=share-request-share]"), why: e.querySelector("[data-testid=share-request-why]")?.textContent ?? null, hit: !!at && e.contains(at) };
  }));
  check(rows.length === 2, `${tag}: both requests are in the corner with no lane open (${rows.length})`);
  const live = rows.find((r) => r.text.includes(pick.up.slug));
  const idle = rows.find((r) => r.text.includes(pick.down.slug) && !r.text.includes("could I watch"));
  check(live && live.share && live.hit && live.text.includes("could I watch?"), `${tag}: the live place's request shows its note, offers Share… and takes a click`);
  check(idle && !idle.share && /no live session/.test(idle.why ?? ""), `${tag}: the idle place's request says it has no live session (${idle?.why})`);
  // Share… opens the share dialog on that place, the asker ticked.
  await page.evaluate((slug) => [...document.querySelectorAll("[data-testid=share-request-alert]")].find((e) => e.textContent.includes(slug)).querySelector("[data-testid=share-request-share]").click(), pick.up.slug);
  await page.waitForSelector(".ls-modal", { timeout: 4000 }).catch(() => {});
  const dlg = await page.evaluate(() => {
    const m = document.querySelector(".ls-modal");
    if (!m) return null;
    const boxes = [...m.querySelectorAll("input[type=checkbox]")];
    return { title: m.getAttribute("aria-label"), ticked: boxes.filter((b) => b.checked).length, confirm: !m.querySelector("[data-testid=share-confirm]")?.disabled };
  });
  check(dlg && dlg.title === `Share ${pick.up.slug}` && dlg.ticked === 1 && dlg.confirm, `${tag}: Share… opens the share dialog for ${pick.up.slug}, the asker ticked (${JSON.stringify(dlg)})`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(300);
  // Dismiss takes the request away.
  await page.evaluate((slug) => [...document.querySelectorAll("[data-testid=share-request-alert]")].find((e) => e.textContent.includes(slug) && !e.textContent.includes("could I watch")).querySelector("[data-testid=share-request-dismiss]").click(), pick.down.slug);
  await page.waitForTimeout(800);
  const left = await page.$$eval("[data-testid=share-request-alert]", (es) => es.length);
  check(left === 1, `${tag}: Dismiss takes the request away (${left} left)`);
  await browser.close();
}
console.log(failed ? `\nshare-requests-browser: ${failed} failure(s)` : "\nshare-requests-browser: all good");
process.exit(failed ? 1 : 0);
