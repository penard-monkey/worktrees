// Headless drive of the mock harness in Chromium AND WebKit: every step inside
// the Disk view is a step in the app's Back / Forward history, and Disk never
// moves the selection behind it.
//
//   VITE_MOCK=1 vite --port 1481 --force          # in app/
//   PLAYWRIGHT_MODULE=…/playwright/index.mjs DISK_BACK_URL=http://localhost:1481 \
//     node app/scripts/disk-back-browser.mjs       # non-zero on failure
//
// WHY THIS EXISTS. Disk shipped (#707) outside the history location: opening it
// over Home recorded nothing, and drilling to a place SELECTED that place, so
// Back from a place's treemap jumped to Home or to whatever place came before,
// never to the project list it was opened from. Opening Disk from a place's
// menu also moved the selection to that place. The walk below is the owner's:
// Home → a project's Disk → one of its places → a folder → Back ×3 → Forward.
// It fails on v0.54.0 at the first Back.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.DISK_BACK_URL ?? "http://localhost:1481";
let failures = 0;
const check = (ok, m) => { console.log(`${ok ? "ok" : "not ok"} — ${m}`); if (!ok) failures++; };

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    page.setDefaultTimeout(8000);
    // What is on screen: the Disk view's title line and pick, the nav row lit.
    const seen = () => page.evaluate(() => {
      const dk = document.querySelector('[data-testid="disk-view"]');
      return {
        disk: dk ? `${dk.querySelector(".dk-title h2")?.textContent} | ${dk.querySelector(".dk-title .sub")?.textContent ?? ""}` : null,
        picked: document.querySelector('[data-testid="disk-selected"] .dk-selp')?.textContent ?? null,
        sel: document.querySelector("li.row.sel")?.getAttribute("data-slug") ?? null,
        home: !!document.querySelector(".briefing"),
      };
    });
    // Past navHistory's COALESCE_MS (600): two pushes closer than that are one
    // visit by design, so a quicker walk folds its steps into one entry.
    const settle = () => page.waitForTimeout(800);
    const nav = async () => {
      if (!(await page.locator("li.row[data-slug]").first().isVisible())) { await page.locator('[data-testid="places-rail"]').click(); await settle(); }
    };
    // One action, then the screen; history steps by the button where one is
    // drawn, else by its chord (no button at all is a failure of its own).
    const step = async (dir) => {
      const b = page.locator(`[data-testid="${dir < 0 ? "nav-back" : "nav-forward"}"]:visible`);
      check(await b.count() > 0, `${name}: ${dir < 0 ? "Back" : "Forward"} is on screen`);
      if (await b.count()) await b.first().click();
      else await page.keyboard.press(dir < 0 ? "Meta+BracketLeft" : "Meta+BracketRight");
      await settle();
      return seen();
    };

    await page.goto(`${base}/`);
    await page.waitForFunction(() => !!window.__mock);
    await nav();
    await page.locator(".home-item").click();
    await settle();
    const home = await seen();
    check(home.home && !home.disk, `${name}: starts on Home`);

    // Home → a project's Disk (its menu) → a place → a folder.
    await nav();
    await page.locator(".project-h").first().click({ button: "right" });
    await page.locator('[data-testid="proj-disk"]').click();
    await settle();
    const project = await seen();
    check(/^.+ \| Project · /.test(project.disk ?? ""), `${name}: the project's Disk: ${project.disk}`);
    await page.locator(".dk-side .dk-r").first().click();
    await settle();
    const place = await seen();
    check(/^Disk \| /.test(place.disk ?? ""), `${name}: one of its places: ${place.disk}`);
    check(place.sel === null, `${name}: drilling to a place does not select it behind Disk (lit: ${place.sel})`);
    const map = await page.locator('[data-testid="disk-map"]').boundingBox();
    await page.mouse.click(map.x + 30, map.y + 60);
    await settle();
    const folder = await seen();
    check(!!folder.picked, `${name}: a folder in its treemap: ${folder.picked}`);

    // Back ×3, Forward.
    const b1 = await step(-1);
    check(b1.disk === place.disk && !b1.picked, `${name}: Back → the place, nothing picked: ${JSON.stringify(b1)}`);
    const b2 = await step(-1);
    check(b2.disk === project.disk, `${name}: Back → the project's Disk it was clicked from: ${JSON.stringify(b2)}`);
    const b3 = await step(-1);
    check(b3.home && !b3.disk, `${name}: Back → Home, where Disk was opened: ${JSON.stringify(b3)}`);
    const f1 = await step(1);
    check(f1.disk === project.disk, `${name}: Forward → the project's Disk again: ${JSON.stringify(f1)}`);
    const f2 = await step(1);
    check(f2.disk === place.disk && !f2.picked, `${name}: Forward → the place: ${JSON.stringify(f2)}`);
    const f3 = await step(1);
    check(f3.picked === folder.picked, `${name}: Forward → the folder picked again: ${JSON.stringify(f3)}`);

    // From a place: its menu opens Disk for ANOTHER place, the selection stays,
    // and Escape (then ✕ on a second open) is back where you were.
    await nav();
    const rows = page.locator('li.row[data-slug]:not([data-slug="(main)"])');
    await rows.nth(0).click();
    await settle();
    const before = await seen();
    await nav();
    await rows.nth(2).click({ button: "right" });
    await page.locator('[data-testid="ctx-disk"]').click();
    await settle();
    const over = await seen();
    check(!!over.disk && over.sel === before.sel && !over.disk.includes(`› ${before.sel} `), `${name}: Disk for another place over ${before.sel}, which stays selected: ${JSON.stringify(over)}`);
    await page.locator('[data-testid="disk-close"]').first().focus().catch(() => {});
    await page.keyboard.press("Escape");
    await settle();
    const esc = await seen();
    check(!esc.disk && esc.sel === before.sel, `${name}: Escape closes it, back on ${before.sel}: ${JSON.stringify(esc)}`);
    const back = await step(-1);
    check(back.disk === over.disk && back.sel === before.sel, `${name}: Back reopens that Disk over ${before.sel}: ${JSON.stringify(back)}`);
    await page.locator('[data-testid="disk-close"]').click();
    await settle();
    const x = await seen();
    check(!x.disk && x.sel === before.sel, `${name}: ✕ closes it, back on ${before.sel}: ${JSON.stringify(x)}`);
  } catch (e) {
    check(false, `${name}: ${e.message.split("\n")[0]}`);
  } finally {
    await browser.close();
  }
}
console.log(failures ? `\n${failures} failure(s)` : "\nall ok");
process.exit(failures ? 1 : 0);
