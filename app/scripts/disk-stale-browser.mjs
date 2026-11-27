// Headless drive of the mock harness in Chromium AND WebKit: a Disk scan is
// kept until Rescan, and the view says what it knows no longer matches.
//
//   VITE_MOCK=1 vite --port 1482 --force          # in app/
//   PLAYWRIGHT_MODULE=…/playwright/index.mjs DISK_STALE_URL=http://localhost:1482 \
//     node app/scripts/disk-stale-browser.mjs      # non-zero on failure
//
// WHY THIS EXISTS. The owner, 2026-11-27: "disk usage inventory should
// probably know if the data is stale … Don't auto refresh, but if lanes are
// gone we'll know." Until then a kept scan said only "scanned in 0.7 s": a
// place removed since stayed in every total and its row still opened. The
// walk: scan a project, age the scan three days, remove one of its places and
// add another, reopen Disk → one notice naming both, the removed row marked
// and closed, the header's age, Clean up… leaving the removed place out, and
// no scan having run on its own. Rescan → the notice is gone. It fails on
// v0.54.0 at the first notice.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.DISK_STALE_URL ?? "http://localhost:1482";
let failures = 0;
const check = (ok, m) => { console.log(`${ok ? "ok" : "not ok"} — ${m}`); if (!ok) failures++; };

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    page.setDefaultTimeout(8000);
    const settle = () => page.waitForTimeout(800);
    const seen = () => page.evaluate(() => {
      const dk = document.querySelector('[data-testid="disk-view"]');
      return {
        title: dk?.querySelector(".dk-title h2")?.textContent ?? null,
        sub: dk?.querySelector(".dk-title .sub")?.textContent ?? "",
        notice: document.querySelector('[data-testid="disk-stale"] > span')?.textContent ?? null,
        rows: [...document.querySelectorAll(".dk-side .dk-r[data-slug]")].map((r) => r.dataset.slug),
        gone: [...document.querySelectorAll(".dk-side .dk-r[data-gone]")].map((r) => ({ slug: r.dataset.slug, disabled: r.disabled })),
      };
    });
    const openDisk = async () => {
      if (!(await page.locator(".project-h").first().isVisible())) { await page.locator('[data-testid="places-rail"]').click(); await settle(); }
      await page.locator(".project-h").first().click({ button: "right" });
      await page.locator('[data-testid="proj-disk"]').click();
      await page.locator(".dk-side .dk-r[data-slug]").first().waitFor();
      await settle();
    };
    // Every disk_usage the page sends, to prove no scan ran on its own.
    const scans = [];
    page.on("console", (m) => { if (m.text().startsWith("[disk-scan]")) scans.push(m.text()); });

    await page.goto(`${base}/`);
    await page.waitForFunction(() => !!window.__mock);
    await openDisk();
    const fresh = await seen();
    check(fresh.notice === null, `${name}: a fresh scan has no notice (${fresh.notice})`);
    check(/scanned just now/.test(fresh.sub), `${name}: the header says when the numbers were taken: ${fresh.sub}`);
    scans.length = 0;
    const victim = fresh.rows.find((r) => r !== "(main)");
    check(!!victim, `${name}: a place to remove: ${victim}`);

    await page.locator('[data-testid="disk-close"]').click();
    await settle();
    const repo = await page.evaluate((s) => window.__mock.diskRemovePlace(s), victim);
    await page.evaluate((r) => { window.__mock.diskAddPlace("feat-after-scan", r); window.__mock.diskAge(3 * 86400); }, repo);
    await settle();
    await openDisk();
    const stale = await seen();
    check(stale.notice?.includes(`${victim} was removed after this scan. These numbers still include it.`), `${name}: the notice names the removed place: ${stale.notice}`);
    check(stale.notice?.includes("feat-after-scan was added after this scan and is not counted."), `${name}: the notice names the added place: ${stale.notice}`);
    check(!/[—–]/.test(stale.notice ?? ""), `${name}: no dashes in the notice`);
    check(/scanned 3 days ago/.test(stale.sub), `${name}: the header says the scan is 3 days old: ${stale.sub}`);
    check(stale.gone.length === 1 && stale.gone[0].slug === victim && stale.gone[0].disabled, `${name}: only the removed row is marked, and it does not open: ${JSON.stringify(stale.gone)}`);
    check(stale.rows.includes(victim), `${name}: the removed place is still in the list it still counts`);
    check(await page.locator('[data-testid="disk-stale"]').count() === 1, `${name}: one notice on the screen`);
    const contrast = await page.evaluate(() => {
      const el = document.querySelector('[data-testid="disk-stale"]');
      return el ? getComputedStyle(el).color === getComputedStyle(document.querySelector(".dk-title h2")).color : false;
    });
    check(contrast, `${name}: the notice's words wear --txt-hi, the hue is the dot's`);
    await page.locator(".dk-side .dk-r[data-gone]").click({ force: true });
    await settle();
    check((await seen()).title === stale.title, `${name}: clicking the removed row opens nothing`);

    await page.locator('[data-testid="disk-bulk"]').click();
    const bulk = await page.evaluate(() => ({
      slugs: [...document.querySelectorAll(".dk-bulk .dk-tr b")].map((b) => b.textContent),
      left: document.querySelector('[data-testid="disk-bulk-left"]')?.textContent ?? null,
    }));
    check(!bulk.slugs.includes(victim), `${name}: Clean up… does not offer the removed place: ${bulk.slugs.join(",")}`);
    check(/1 place was removed after the scan and is left out/.test(bulk.left ?? ""), `${name}: Clean up… says it left one out: ${bulk.left}`);
    await page.keyboard.press("Escape");
    await settle();
    check(scans.length === 0, `${name}: nothing rescanned on its own: ${scans.join(" | ")}`);

    await page.locator('[data-testid="disk-stale-rescan"]').click();
    await page.waitForFunction(() => !document.querySelector('[data-testid="disk-stale"]'), null, { timeout: 8000 }).catch(() => {});
    await settle();
    const after = await seen();
    check(after.notice === null && after.gone.length === 0, `${name}: Rescan clears the notice and the marks: ${JSON.stringify(after)}`);
    check(/scanned just now/.test(after.sub) && !after.rows.includes(victim), `${name}: the new scan is just now and without ${victim}: ${after.sub}`);
  } catch (e) {
    check(false, `${name}: ${e.message.split("\n")[0]}`);
  } finally {
    await browser.close();
  }
}
console.log(failures ? `\n${failures} failure(s)` : "\nall ok");
process.exit(failures ? 1 : 0);
