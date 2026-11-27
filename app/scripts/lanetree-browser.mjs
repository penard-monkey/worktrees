// Browser gate for the lane tree in the nav (docs/proposals/lane-children.md
// §5), in Chromium AND WebKit against the mock's `?lanes` project.
//
//   VITE_MOCK=1 vite --port 1447 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     LANES_MOCK_URL=http://localhost:1447 node app/scripts/lanetree-browser.mjs
//
// What it holds, at 300, 240 and 220 px:
//   - indent by depth, and the depth stop: 14 px steps to level 4, 8 px to
//     level 3 under 280 px, the level as a number past it (§5.2);
//   - every caret is hit by `elementFromPoint` at its centre (an unread dot's
//     ring overhangs that column) and is the row's full height;
//   - the deepest name keeps at least 40 px at 220 and 60 px at 240;
//   - no lane row runs past the nav's right edge;
//   - a ghost is drawn for a closed starter, with its tier, and is no drop slot;
//   - a fold shows the pill with how many it hides; the filter shows a match's
//     starters as context;
//   - manual drag: among siblings, to the end of the family, a subtree that
//     travels, and a drop on another family refused with words.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.LANES_MOCK_URL ?? "http://localhost:1447";

let bad = 0;
const ok = (c, m) => { if (!c) { console.error(`FAIL ${m}`); bad++; } };

async function open(engine, width, sort = "recent") {
  const browser = await engine.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 1100 } });
  const errs = [];
  page.on("pageerror", (e) => errs.push(String(e)));
  await page.addInitScript(([nw, sort]) => {
    sessionStorage.setItem("wt-mock-ui-state", JSON.stringify({ nav_width: nw, nav_pinned: true, settings_rev: 5, sort_mode: sort }));
  }, [width, sort]);
  await page.goto(`${BASE}/?lanes`);
  await page.waitForSelector('li.row[data-slug="orchestrator"]');
  return { browser, page, errs };
}

/** Every row of the lanes project: slug, ghost, level, name box, caret hit. */
const rows = (page) => page.evaluate(() => {
  const proj = document.querySelector('[data-project-root]:has(li.row[data-slug="orchestrator"], li.row[data-ghost="orchestrator"])');
  const nav = document.querySelector(".nav-scroll").getBoundingClientRect();
  return [...proj.querySelectorAll("li.row")].map((r) => {
    const name = r.querySelector(".row-name").getBoundingClientRect();
    const c = r.querySelector(".lane-caret");
    let hit = null, ch = 0;
    if (c) {
      c.scrollIntoView({ block: "center" });
      const b = c.getBoundingClientRect();
      ch = Math.round(b.height - r.getBoundingClientRect().height);
      hit = c.contains(document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2));
    }
    return {
      slug: r.dataset.slug ?? r.dataset.ghost, ghost: r.classList.contains("ghost"), slot: "slug" in r.dataset,
      lvl: r.style.getPropertyValue("--lvl"), x: Math.round(name.left - nav.left), w: Math.round(name.width),
      right: Math.round(r.getBoundingClientRect().right - nav.right), text: r.querySelector(".row-name").textContent,
      tier: r.querySelector(".lane-tier")?.textContent ?? null, pill: r.querySelector(".lane-pill")?.textContent ?? null, hit, ch,
    };
  });
});

async function drag(page, from, to, frac) {
  const sel = (x) => `li.row[data-slug="${x}"], li.row[data-ghost="${x}"]`;
  await page.locator(sel(from)).scrollIntoViewIfNeeded();
  const a = await page.locator(sel(from)).boundingBox();
  await page.mouse.move(a.x + 60, a.y + a.height / 2);
  await page.mouse.down();
  for (let i = 1; i <= 6; i++) { await page.mouse.move(a.x + 60, a.y + a.height / 2 + i * 4); await page.waitForTimeout(30); }
  const b = await page.locator(sel(to)).boundingBox();
  for (let i = 1; i <= 8; i++) { await page.mouse.move(a.x + 60, a.y + (b.y + b.height * frac - a.y) * i / 8); await page.waitForTimeout(40); }
  await page.waitForTimeout(200);
  const why = await page.evaluate(() => document.querySelector(".drag-why")?.textContent ?? null);
  await page.mouse.up();
  await page.waitForTimeout(250);
  return why;
}
const order = async (page) => (await rows(page)).map((r) => r.slug);

for (const [label, engine] of [["chromium", chromium], ["webkit", webkit]]) {
  for (const width of [300, 240, 220]) {
    const { browser, page, errs } = await open(engine, width);
    const at = `${label} ${width}px`;
    const rs = await rows(page);
    const by = Object.fromEntries(rs.map((r) => [r.slug, r]));
    const wide = width >= 280;
    // §5.2: the indent ladder and its stop.
    const step = wide ? 14 : 8;
    ok(by["deep-2"].x - by["deep-1"].x === step && by["deep-3"].x - by["deep-2"].x === step, `${at}: one ${step}px step per level`);
    ok(by["deep-6"].x === by[wide ? "deep-5" : "deep-4"].x, `${at}: indent stops at level ${wide ? 4 : 3}`);
    // deep-1 is level 0, so deep-6 is level 5.
    ok(by["deep-6"].text === "5deep-6" && !/^\d/.test(by["deep-4"].text) && (wide ? by["deep-5"].text === "deep-5" : by["deep-5"].text === "4deep-5"),
      `${at}: the level shows as a number past the stop (${by["deep-4"].text}, ${by["deep-5"].text}, ${by["deep-6"].text})`);
    ok(by["deep-6"].w >= (width >= 240 ? 60 : 40), `${at}: the deepest name keeps ${by["deep-6"].w}px`);
    ok(rs.every((r) => r.right <= 0), `${at}: no lane row runs past the nav (${rs.filter((r) => r.right > 0).map((r) => r.slug)})`);
    // §5.1: carets, hit-tested.
    const carets = rs.filter((r) => r.hit !== null);
    ok(carets.length >= 8, `${at}: carets drawn (${carets.length})`);
    ok(carets.every((r) => r.hit), `${at}: every caret is hit at its centre (${carets.filter((r) => !r.hit).map((r) => r.slug)})`);
    ok(carets.every((r) => r.ch === 0), `${at}: a caret is its row's full height`);
    // §5.4: ghosts.
    ok(by["view-combined"].ghost && by["view-combined"].tier === "Closed" && !by["view-combined"].slot, `${at}: a closed starter is a ghost, labelled, not a slot`);
    ok(by["critique"].lvl === "2" && by["probe-fonts"].lvl === "3", `${at}: lanes under a ghost keep their depth`);
    ok(!by["adopted"].lvl || by["adopted"].lvl === "0", `${at}: an adopted lane is drawn at the top`);
    ok(!by["from-elsewhere"].lvl || by["from-elsewhere"].lvl === "0", `${at}: a lane started in another project is at the top`);
    if (width === 300) {
      // §5.3: fold, pill.
      await page.click('[data-testid="lane-caret|orchestrator"]');
      await page.waitForTimeout(200);
      const f = await rows(page);
      ok(f.find((r) => r.slug === "orchestrator")?.pill === "5", `${at}: the fold's pill counts the 5 real lanes it hides`);
      ok(!f.some((r) => r.slug === "critique"), `${at}: a fold hides every depth`);
      await page.click('[data-testid="lane-caret|orchestrator"]');
      // §5.6: the filter keeps a match's starters, dim.
      await page.fill('input[placeholder^="filter"]', "probe");
      await page.waitForTimeout(250);
      const q = await rows(page);
      ok(q.map((r) => `${r.ghost ? "~" : ""}${r.slug}`).join(",") === "~orchestrator,~view-combined,~critique,probe-fonts", `${at}: filter context (${q.map((r) => r.slug)})`);
    }
    ok(!errs.length, `${at}: page errors ${errs.join(" | ")}`);
    await browser.close();
  }
  // §5.6: manual drag under nesting.
  const { browser, page } = await open(engine, 300, "manual");
  // First, from the starting order: the case that once refused itself (the
  // gap after the family puts the next family's row under the pointer).
  await drag(page, "feat-docs", "handed", 0.8);
  let o = await order(page);
  ok(o.indexOf("feat-docs") > o.indexOf("handed") && o.indexOf("feat-docs") < o.indexOf("design-chats"), `${label}: a lane drops at the end of its family (${o.slice(0, 10)})`);
  await drag(page, "feat-docs", "view-kimi", 0.2);
  o = await order(page);
  ok(o.indexOf("feat-docs") < o.indexOf("view-kimi"), `${label}: a lane reorders among its siblings (${o.slice(0, 10)})`);
  const why = await drag(page, "handed", "deep-3", 0.5);
  ok(/moves among the lanes of orchestrator only/.test(why ?? ""), `${label}: a drop on another family is refused in words (${why})`);
  await drag(page, "deep-1", "adopted", 0.2);
  o = await order(page);
  const d = o.indexOf("deep-1");
  ok(o.slice(d, d + 6).join(",") === "deep-1,deep-2,deep-3,deep-4,deep-5,deep-6" && d < o.indexOf("adopted"), `${label}: a row's subtree travels with it`);
  await browser.close();
}

if (bad) { console.error(`lanetree-browser: ${bad} failure(s)`); process.exit(1); }
console.log("lanetree-browser: the lane tree draws, folds, filters and drags as §5 says, in Chromium and WebKit");
