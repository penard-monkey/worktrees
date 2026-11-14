// Optional browser gate for group chat's beta switch (beta.ts
// `groupChatBeta`), in Chromium AND WebKit, against the mock as a machine
// that is a guest of David's Mac (`?guest`, mock/guest.ts) and owns places of
// its own. groupchat-beta-check.mjs holds the same rules on the source.
//
//   VITE_MOCK=1 vite --port 1437 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<a local playwright>/index.mjs \
//     node app/scripts/groupchat-beta-browser.mjs [port]
//
// What it holds, with no reload anywhere:
//   - OFF (a fresh tab): no guest row in the nav, no guest row is ever called
//     (no guest connection at all), no lane share bar on a place running an
//     agent.
//   - ticked in Settings → General → Beta features: the host's shares appear
//     under the nav, and the lane share bar on the place.
//   - a share opened, then the switch OFF again: the guest view closes, the
//     rows go, and no guest row is called after it; the paired host is still
//     listed in Settings → Hosts.
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = Number(process.argv[2] ?? 1437);
const BASE = `http://localhost:${port}/?guest`;

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const has = async (page, sel) => (await page.$$(sel)).length > 0;
const calls = (page) => page.evaluate(() => window.__mock.guestCalls());
const openSheet = async (page) => { await page.click("[data-track=settings]"); await page.waitForSelector(".settings-body .page-title"); };
const closeSheet = async (page) => { await page.keyboard.press("Escape"); await page.waitForTimeout(200); };
const toCat = async (page, cat) => { await page.click(`.settings-cat[data-cat=${cat}]`); await page.waitForSelector(`.settings-body[data-page=${cat}]`); await page.waitForTimeout(150); };
const flip = async (page, want) => {
  await openSheet(page);
  await toCat(page, "general");
  await page.click("[data-testid=beta-group-chat]");
  const wrote = await page.waitForFunction((w) => JSON.parse(sessionStorage.getItem("wt-mock-ui-state") ?? "null")?.beta_group_chat === w, want, { timeout: 3000 }).then(() => true, () => false);
  await closeSheet(page);
  return wrote;
};
/** Select the first place whose agent is up, so the lane (and its share bar) draws. */
const selectAgentPlace = async (page) => {
  await page.evaluate(() => {
    const row = [...document.querySelectorAll(".nav-scroll [data-slug]")].find((e) => e.querySelector(".dot.busy, .dot.idle, .dot.waiting, .dot")) ?? document.querySelector(".nav-scroll [data-slug]");
    row?.click();
  });
  await page.waitForSelector(".agent-row", { timeout: 4000 }).catch(() => {});
  await page.waitForTimeout(300);
};

for (const [name, engine] of [["chromium", pw.chromium], ["webkit", pw.webkit]]) {
  const browser = await engine.launch();
  const page = await browser.newPage({ viewport: { width: 1365, height: 1000 } });
  const tag = `[${name}]`;
  try {
    await page.addInitScript((s) => {
      if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
    }, { places_side: "left", dock_side: "left", settings_rev: 5, settings_mode: "advanced", nav_pinned: true });
    await page.goto(BASE);
    await page.waitForSelector("[data-track=settings]");
    await page.waitForTimeout(1500);

    // OFF
    ok(!(await has(page, "[data-testid=guest-share-row]")), `${tag} off: no guest row in the nav`);
    const before = await calls(page);
    ok(Array.isArray(before) && before.length === 0, `${tag} off: no guest row was called (${JSON.stringify(before)})`);
    await selectAgentPlace(page);
    const lane = await has(page, ".agent-row");
    ok(lane, `${tag} a place with its agent up is selected (the lane draws)`);
    ok(!(await has(page, "[data-testid=lane-share]")), `${tag} off: no lane share bar`);

    // ON
    ok(await flip(page, true), `${tag} the Group chat checkbox writes beta_group_chat: true`);
    await page.waitForSelector("[data-testid=guest-share-row]", { state: "attached", timeout: 8000 }).catch(() => {});
    ok(await has(page, "[data-testid=guest-share-row]"), `${tag} on: the host's shares appear in the nav, no reload`);
    ok((await calls(page)).includes("shares_list"), `${tag} on: the guest connection reads the shares`);
    await page.waitForSelector("[data-testid=lane-share]", { state: "attached", timeout: 4000 }).catch(() => {});
    ok(await has(page, "[data-testid=lane-share]"), `${tag} on: the lane share bar draws`);

    // a share open, then OFF
    await page.evaluate(() => [...document.querySelectorAll("[data-testid=guest-share-row]")].find((e) => e.dataset.share.startsWith("s_live"))?.click());
    await page.waitForSelector(".guest-overlay", { timeout: 5000 }).catch(() => {});
    ok(await has(page, ".guest-overlay"), `${tag} on: a share opens the guest view`);
    ok(await flip(page, false), `${tag} the checkbox writes beta_group_chat: false`);
    await page.waitForTimeout(400);
    ok(!(await has(page, ".guest-overlay")), `${tag} off again: the open guest view closes`);
    ok(!(await has(page, "[data-testid=guest-share-row]")), `${tag} off again: the guest rows go`);
    ok(!(await has(page, "[data-testid=lane-share]")), `${tag} off again: the lane share bar goes`);
    await calls(page);
    await page.waitForTimeout(1200);
    const after = await calls(page);
    ok(after.length === 0, `${tag} off again: nothing reaches the guest host (${JSON.stringify(after)})`);
    await openSheet(page);
    await toCat(page, "remote");
    ok(await page.evaluate(() => document.querySelector(".settings-body")?.innerText.includes("David's Mac") ?? false), `${tag} off again: Settings → Hosts still lists the guest host`);
    ok(!(await has(page, ".settings-body [data-sec=guests]")), `${tag} off again: Settings → Remote has no Guests section`);
    await closeSheet(page);

    // ON once more: it starts again, no restart.
    ok(await flip(page, true), `${tag} on again`);
    await page.waitForSelector("[data-testid=guest-share-row]", { state: "attached", timeout: 8000 }).catch(() => {});
    ok(await has(page, "[data-testid=guest-share-row]"), `${tag} on again: the guest rows come back`);
    await openSheet(page);
    await toCat(page, "remote");
    ok(await has(page, ".settings-body [data-sec=guests]"), `${tag} on again: Settings → Remote has its Guests section`);
    await closeSheet(page);
  } catch (e) {
    failed++;
    console.log(`FAIL ${tag} ${e.stack ?? e}`);
  }
  await browser.close();
}
console.log(failed ? `\ngroupchat-beta-browser: ${failed} failure(s)` : "\ngroupchat-beta-browser: all good");
process.exit(failed ? 1 : 0);
