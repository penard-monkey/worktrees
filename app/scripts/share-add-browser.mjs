// Headless drive of the mock harness as the OWNER, in WebKit AND Chromium: a
// shared lane takes another guest (found in the v0.52.0 three-VM run,
// 2026-11-14, where *Sharing…* had no way to add one).
//
//  A. The lane's *Sharing…* → *Add someone…* → Someone new "Cy" → the pair
//     window → Allow → v3's warning and *Let Cy in*: one `share_allow`, no
//     `share_create`, and Cy is on the share, allowed.
//  B. A request for a shared place, from a guest not on it, offers *Let Bret
//     in…* in the corner; it opens the add dialog with Bret ticked, lets him
//     in, and the request is gone.
//
//   app/: VITE_MOCK=1 ./node_modules/.bin/vite --port 1437 --force
//   PLAYWRIGHT_MODULE=<a local playwright> node app/scripts/share-add-browser.mjs [port]
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = Number(process.argv[2] ?? 1437);
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));

const spy = () => {
  const t = window.__TAURI_INTERNALS__;
  if (t.__spied) return;
  const orig = t.invoke.bind(t);
  window.__calls = [];
  t.invoke = (cmd, args, opts) => { window.__calls.push(cmd); return orig(cmd, args, opts); };
  t.__spied = true;
};
const livePlace = () => window.__TAURI_INTERNALS__.invoke("list_workspace", {}).then((ws) => {
  for (const pv of ws.projects) for (const p of pv.snapshot?.places ?? []) {
    const a = p.agent_sessions ?? {};
    if (a.claude?.up) return { repo: pv.root, slug: p.slug };
  }
  return null;
});
const shareOf = (page, slug) => page.evaluate(async (slug) => (await window.__TAURI_INTERNALS__.invoke("shares_all", {})).shares.find((s) => s.slug === slug), slug);

for (const engine of ["webkit", "chromium"]) {
  const browser = await pw[engine].launch();
  const tag = engine;

  // ── A. Sharing… → Add someone… ──
  {
    const page = await browser.newPage({ viewport: { width: 1365, height: 904 } });
    await page.goto(`http://localhost:${port}/?beta=group_chat`);
    await page.waitForSelector("[data-track=settings]");
    await page.waitForTimeout(1500);
    const up = await page.evaluate(livePlace);
    if (!up) { fail(`${tag}: the mock has a live Claude place`); await browser.close(); continue; }
    await page.evaluate(async (up) => {
      window.__mock.guestDevice();
      await window.__TAURI_INTERNALS__.invoke("share_create", { repo: up.repo, slug: up.slug, guests: ["g_bret00001"] });
    }, up);
    await page.evaluate(spy);
    await page.evaluate((slug) => document.querySelector(`li.row[data-slug="${CSS.escape(slug)}"]`)?.click(), up.slug);
    await page.waitForSelector("[data-testid=lane-share] [data-testid=shared-marker]", { timeout: 8000 }).catch(() => {});
    await page.waitForTimeout(500);
    const sharing = await page.evaluateHandle(() => [...document.querySelectorAll("[data-testid=lane-share] button")].find((b) => b.textContent.trim() === "Sharing…") ?? null);
    if (!(await sharing.evaluate((b) => !!b))) { fail(`${tag}: the shared lane's bar shows Sharing…`); await browser.close(); continue; }
    await sharing.asElement().click();
    await page.waitForSelector("[data-testid=share-add]", { timeout: 4000 }).catch(() => {});
    check(!!(await page.$("[data-testid=share-add]")), `${tag}: Sharing… offers Add someone…`);
    await page.click("[data-testid=share-add]");
    await page.waitForSelector("[data-testid=share-dialog]");
    const title = await page.$eval("[data-testid=share-dialog]", (e) => e.getAttribute("aria-label"));
    check(title === `Add someone to ${up.slug}`, `${tag}: it opens the share dialog on the share (${title})`);
    const offered = await page.$$eval("[data-testid=share-dialog] [data-guest]", (es) => es.map((e) => e.textContent));
    check(offered.length === 0 && (await page.isChecked("[data-testid=share-someone-new]")), `${tag}: Bret, on the share already, is not offered; Someone new is chosen (${JSON.stringify(offered)})`);
    await page.fill("[data-testid=share-new-name]", "Cy");
    await page.click("[data-testid=share-invite]");
    await page.waitForSelector("[data-testid=share-reach-on], [data-testid=pair-send-link]", { timeout: 8000 });
    if (await page.$("[data-testid=share-reach-on]")) await page.click("[data-testid=share-reach-on]");
    await page.waitForSelector("[data-testid=pair-send-link]", { timeout: 8000 });
    await page.evaluate(() => window.__mock.pairAttempt("Cy's MacBook", "macos", "100.101.4.8", "551203"));
    await page.waitForSelector("[data-testid=share-pair-allow]", { timeout: 5000 });
    await page.click("[data-testid=share-pair-allow]");
    await page.waitForSelector("[data-testid=share-paired]", { timeout: 5000 });
    const c1 = await page.evaluate(() => window.__calls.slice());
    check(!c1.includes("share_allow") && !c1.includes("share_create"), `${tag}: Allow (pairing) let nobody in yet`);
    const step = await page.evaluate(() => ({
      confirm: document.querySelector("[data-testid=share-confirm]")?.textContent,
      warning: document.querySelector("[data-testid=share-warning]")?.textContent ?? "",
      admission: document.querySelector("[data-testid=share-admission]")?.textContent ?? "",
    }));
    check(step.confirm === "Let Cy in" && /^Cy can read this whole session from its first turn/.test(step.warning), `${tag}: the last step keeps v3's warning and says "Let Cy in" (${JSON.stringify(step)})`);
    check(/lets them in/.test(step.admission), `${tag}: …and says an added guest is let in at once`);
    await page.click("[data-testid=share-confirm]");
    await page.waitForSelector("[data-testid=share-dialog]", { state: "detached", timeout: 5000 }).catch(() => {});
    const c2 = await page.evaluate(() => window.__calls.slice());
    check(c2.filter((x) => x === "share_allow").length === 1 && !c2.includes("share_create"), `${tag}: Let Cy in is one share_allow and no share_create`);
    const sh = await shareOf(page, up.slug);
    const names = await page.$eval("[data-testid=lane-share] [data-testid=shared-marker]", (e) => e.textContent);
    check(sh?.guests.length === 2 && sh.guests.some((g) => g.state === "allowed" && g.principal !== "g_bret00001"), `${tag}: Cy is on the share, allowed (${JSON.stringify(sh?.guests.map((g) => g.state))})`);
    check(/Bret and Cy/.test(names), `${tag}: the bar says ${names}`);
    await page.close();
  }

  // ── B. the corner's Let Bret in… ──
  {
    const page = await browser.newPage({ viewport: { width: 1365, height: 904 } });
    await page.goto(`http://localhost:${port}/?beta=group_chat`);
    await page.waitForSelector("[data-track=settings]");
    await page.waitForTimeout(1500);
    const up = await page.evaluate(livePlace);
    // A share Bret is not on (made with him, then him taken off), and his request for it.
    await page.evaluate(async (up) => {
      const t = window.__TAURI_INTERNALS__;
      window.__mock.guestDevice();
      const sh = await t.invoke("share_create", { repo: up.repo, slug: up.slug, guests: ["g_bret00001"] });
      await t.invoke("share_remove_guest", { share: sh.id, principal: "g_bret00001" });
      window.__mock.guestRequest(up.repo, up.slug, "may I watch too?");
    }, up);
    await page.evaluate(spy);
    await page.waitForSelector("[data-testid=share-request-alert]", { timeout: 15000 }).catch(() => {});
    await page.waitForTimeout(600);
    const row = await page.evaluate(() => {
      const e = document.querySelector("[data-testid=share-request-alert]");
      return e && { text: e.textContent, share: !!e.querySelector("[data-testid=share-request-share]"), letIn: e.querySelector("[data-testid=share-request-let-in]")?.textContent ?? null };
    });
    check(row && row.letIn === "Let Bret in…" && !row.share && !/adds a guest/.test(row.text), `${tag}: a request for the shared place offers Let Bret in…, and no false sentence (${JSON.stringify(row)})`);
    await page.click("[data-testid=share-request-let-in]");
    await page.waitForSelector("[data-testid=share-dialog]", { timeout: 4000 }).catch(() => {});
    const dlg = await page.evaluate(() => {
      const m = document.querySelector("[data-testid=share-dialog]");
      return m && { title: m.getAttribute("aria-label"), ticked: [...m.querySelectorAll("[data-guest] input")].filter((b) => b.checked).length, confirm: m.querySelector("[data-testid=share-confirm]")?.textContent, warn: !!m.querySelector("[data-testid=share-warning]") };
    });
    check(dlg && dlg.title === `Add someone to ${up.slug}` && dlg.ticked === 1 && dlg.confirm === "Let Bret in" && dlg.warn, `${tag}: it opens the add dialog, Bret ticked, with the warning (${JSON.stringify(dlg)})`);
    await page.click("[data-testid=share-confirm]");
    await page.waitForSelector("[data-testid=share-dialog]", { state: "detached", timeout: 5000 }).catch(() => {});
    await page.waitForTimeout(800);
    const calls = await page.evaluate(() => window.__calls.slice());
    const sh = await shareOf(page, up.slug);
    const left = await page.$$eval("[data-testid=share-request-alert]", (es) => es.length);
    check(calls.includes("share_allow") && calls.includes("share_request_dismiss") && !calls.includes("share_create"), `${tag}: Let Bret in is share_allow, then the request is closed`);
    check(sh?.guests.some((g) => g.principal === "g_bret00001" && g.state === "allowed"), `${tag}: Bret is on the share, allowed`);
    check(left === 0, `${tag}: the request is gone from the corner (${left} left)`);
    await page.close();
  }
  await browser.close();
}
console.log(failed ? `\nshare-add-browser: ${failed} failure(s)` : "\nshare-add-browser: all good");
process.exit(failed ? 1 : 0);
