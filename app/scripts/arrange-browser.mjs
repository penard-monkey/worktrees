// Optional browser gate for drag to arrange (arrange.ts + the grips in
// App.tsx), in Chromium AND WebKit (the app is WKWebView; the harness is
// Chrome — AGENTS.md), with REAL pointer input: Playwright's mouse goes
// through hit-testing, which a dispatched PointerEvent does not.
//
// Install Playwright in scratch space with both engines, then run against the
// mock harness on a port that is not 1420:
//   VITE_MOCK=1 vite --port 1477 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     ARRANGE_MOCK_URL=http://localhost:1477 node app/scripts/arrange-browser.mjs
//
// What it holds, and why each needs a browser rather than layout-check.mjs:
//   - pointer capture, the click a browser derives from a captured release,
//     and Esc-then-release — event ORDER, which drifts with an engine;
//   - the bug only this run found: a drop that unmounts its own grip (the
//     Terminal's, into the tabs) gets no click, and a click-suppression flag
//     left standing then swallowed the next grip click;
//   - the resize budget the gesture promises, counted at the invoke: 0
//     `term_resize`/`shell_resize` during a drag, at most 1 per pane per drop
//     (the mock answers instantly, so this is the count, not the timing —
//     ROADMAP's flip check is the real-app half);
//   - reachability (elementFromPoint, not a rect): the Terminal grip must not
//     cover the strip's `+` or a tab's × when the strip wraps;
//   - Reset's Undo beside the button (the banner is under the Settings scrim),
//     focus return after a move that unmounts the grip, and the live region
//     re-announcing the same words.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.ARRANGE_MOCK_URL ?? "http://localhost:1477";

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
// Two page errors the mock produces on main too, unrelated to arranging:
// xterm's Viewport.syncScrollArea on place select in a headless browser, and
// the mock's missing event-plugin internals when a terminal unmounts.
const KNOWN = /dimensions|unregisterListener/;

async function open(browser, extra = {}, size = { width: 1280, height: 800 }) {
  const ctx = await browser.newContext({ viewport: size });
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => { if (!KNOWN.test(String(e))) errors.push(String(e).slice(0, 200)); });
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "left", settings_rev: 4, nav_pinned: true, ...extra });
  await page.goto(BASE + "/");
  await page.bringToFront();
  await page.waitForSelector("li.row[data-slug]", { state: "attached" });
  await page.locator("li.row[data-slug]:visible").nth(1).click();
  await page.locator('[data-track="dock.files"]').click();
  await page.waitForSelector(".dock");
  await page.waitForTimeout(400);
  // Count resizes per pane at the bridge the app actually calls.
  await page.evaluate(() => {
    const T = window.__TAURI_INTERNALS__, o = T.invoke;
    window.__rz = [];
    T.invoke = (c, a) => { if (c === "term_resize" || c === "shell_resize") window.__rz.push(`${c}:${a?.id ?? a?.index ?? "?"}`); return o(c, a); };
  });
  return { ctx, page, errors };
}
const box = (page, q) => page.evaluate((q) => {
  const e = document.querySelector(q);
  if (!e) return null;
  const b = e.getBoundingClientRect();
  return { l: b.left, r: b.right, t: b.top, b: b.bottom, cx: b.left + b.width / 2, cy: b.top + b.height / 2 };
}, q);
const sides = (page) => page.evaluate(() => ({
  p: document.documentElement.dataset.placesSide, d: document.documentElement.dataset.dockSide, lower: !!document.querySelector(".dock-lower"),
}));
const resizes = (page) => page.evaluate(() => window.__rz.slice());
const perPaneMax = (list) => Math.max(0, ...Object.values(list.reduce((a, k) => ({ ...a, [k]: (a[k] ?? 0) + 1 }), {})));
async function drag(page, gripQ, x, y, { mid, esc } = {}) {
  const g = await box(page, gripQ);
  await page.mouse.move(g.cx, g.cy);
  await page.mouse.down();
  await page.mouse.move(g.cx + 20, g.cy + 10, { steps: 3 });
  await page.mouse.move(x, y, { steps: 12 });
  if (mid) await mid();
  if (esc) await page.keyboard.press("Escape");
  await page.mouse.up();
  await page.waitForTimeout(500);
}

for (const [engine, type] of Object.entries({ chromium, webkit })) {
  console.log(`── ${engine}`);
  const browser = await type.launch({ headless: true });

  // ── the reference panels: drag, budget, Undo, Esc ──
  {
    const { ctx, page, errors } = await open(browser);
    const space = await box(page, ".space-body");
    const dock0 = await box(page, ".dock");
    await page.evaluate(() => { window.__rz.length = 0; });
    let mid = null;
    await drag(page, ".dock-tabs .grip", space.r - 150, space.cy, {
      mid: async () => {
        mid = await page.evaluate(() => ({
          zones: [...document.querySelectorAll(".arrange-zone")].map((z) => z.dataset.zone + (z.classList.contains("hover") ? "*" : "") + (z.classList.contains("here") ? "=" : "")),
          dockL: document.querySelector(".dock").getBoundingClientRect().left,
          rz: window.__rz.length,
        }));
      },
    });
    ok(mid?.zones.join() === "ref-left=,ref-right*" && mid.dockL === dock0.l, `mid-drag: zones ${mid?.zones.join(" ")}, nothing moved`);
    ok(mid?.rz === 0, `0 pty resizes during the drag (${mid?.rz})`);
    const after = await resizes(page);
    ok(perPaneMax(after) <= 1, `at most 1 resize per pane for the drop (${JSON.stringify(after)})`);
    const s = await sides(page);
    const dock = await box(page, ".dock"), main = await box(page, ".main");
    ok(s.d === "right" && dock.l >= main.r - 1, `dropped right of the agent (dock_side ${s.d})`);
    const undo = await page.evaluate(() => document.querySelector(".float-stack .undo")?.textContent ?? "");
    const live = await page.evaluate(() => document.querySelector('[data-testid="arrange-live"]').textContent);
    ok(/Reference panels moved to the right of the agent/.test(undo) && /right of the agent/.test(live), `Undo banner + announcement (${live})`);
    ok(!(await page.evaluate(() => !!document.querySelector(".ctxmenu"))), "a drag does not also open the Move menu");
    await page.locator(".float-stack .undo .undo-btn").click();
    await page.waitForTimeout(300);
    ok((await sides(page)).d === "left", "Undo puts it back");
    await drag(page, ".dock-tabs .grip", space.r - 150, space.cy, { esc: true });
    ok((await sides(page)).d === "left" && !(await page.evaluate(() => !!document.querySelector(".arrange-catch, .ctxmenu"))), "Esc mid-drag cancels: no move, no overlay, no menu");
    ok(errors.length === 0, `no page errors ${errors.join(" | ")}`);
    await ctx.close();
  }

  // ── keyboard: the grip's Move menu, focus in and back ──
  {
    const { ctx, page } = await open(browser);
    await page.locator(".dock-tabs .grip").focus();
    await page.keyboard.press("Enter");
    await page.waitForSelector(".ctxmenu");
    const items = await page.evaluate(() => [...document.querySelectorAll(".ctxmenu .pop-item")].map((b) => `${b.dataset.dest}${b.disabled ? "=" : ""}`));
    ok(items.join() === "ref-left=,ref-right,term-tab=,term-below", `menu: ${items.join(" ")}`);
    ok((await page.evaluate(() => document.activeElement?.dataset?.dest)) === "ref-right", "focus lands on the first destination you can pick");
    await page.keyboard.press("Enter");
    await page.waitForTimeout(300);
    const s = await sides(page);
    ok(s.d === "right" && (await page.evaluate(() => document.activeElement?.dataset?.grip)) === "reference", "Enter moves it and focus returns to the grip");
    await page.keyboard.press("Enter");
    await page.waitForSelector(".ctxmenu");
    await page.keyboard.press("Escape");
    await page.waitForTimeout(200);
    ok(!(await page.evaluate(() => !!document.querySelector(".ctxmenu"))) && (await sides(page)).d === "right", "Esc closes the menu, nothing moves");
    await ctx.close();
  }

  // ── Places and the agent by their own grips ──
  {
    const { ctx, page } = await open(browser);
    const app = await box(page, ".app");
    await drag(page, ".nav-head .grip", app.r - 60, app.cy);
    ok((await sides(page)).p === "right", "Places dragged to the right edge");
    const space = await box(page, ".space-body");
    await drag(page, ".agent-label .grip", space.l + 100, space.cy);
    const s = await sides(page), dock = await box(page, ".dock"), main = await box(page, ".main");
    ok(s.d === "right" && main.r <= dock.l + 1, "the agent dragged to the left half puts the reference on its right");
    // with the column closed the agent has nothing to be left or right of
    await page.keyboard.press("Meta+j");
    await page.waitForTimeout(300);
    ok((await page.locator(".agent-label .grip").count()) === 0, "no agent grip while the reference column is closed");
    await ctx.close();
  }

  // ── the Terminal: grip over a wrapping strip, the unmounting drop ──
  {
    const { ctx, page } = await open(browser, { dock_lower: "terminal", dock_width: 360 });
    await page.waitForSelector(".dock-lower .grip");
    for (let i = 0; i < 4; i++) { await page.keyboard.press("Meta+t"); await page.waitForTimeout(250); }
    const reach = await page.evaluate(() => {
      const hit = (e) => { if (!e) return "absent"; const b = e.getBoundingClientRect(); return e.contains(document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2)) ? "hit" : "covered"; };
      const xs = [...document.querySelectorAll(".dock-lower .termtab-x")];
      // Where the strip's items may END: its content box. The mock's tabs are
      // short ("sh 5"), so where the `+` happens to land says little; a
      // renamed tab is wider. The rule is that the corner is RESERVED.
      const strip = document.querySelector(".dock-lower .termtab-strip"), cs = getComputedStyle(strip);
      const contentRight = strip.getBoundingClientRect().right - parseFloat(cs.paddingRight) - parseFloat(cs.borderRightWidth);
      const gripLeft = document.querySelector(".dock-lower > .grip").getBoundingClientRect().left;
      return { tabs: document.querySelectorAll(".dock-lower .termtab").length, add: hit(document.querySelector(".dock-lower .termtab-add")), lastX: hit(xs[xs.length - 1]), grip: hit(document.querySelector(".dock-lower > .grip")),
        contentRight: Math.round(contentRight), gripLeft: Math.round(gripLeft) };
    });
    ok(reach.tabs >= 5 && reach.add === "hit" && reach.lastX === "hit" && reach.grip === "hit", `${reach.tabs} shells in a 360px column: + ${reach.add}, last × ${reach.lastX}, grip ${reach.grip}`);
    ok(reach.contentRight <= reach.gripLeft, `the strip's items end at ${reach.contentRight}, left of the grip at ${reach.gripLeft} — no tab, × or + can sit under it`);
    const dock = await box(page, ".dock");
    await drag(page, ".dock-lower .grip", dock.cx, dock.t + 80);
    ok(!(await sides(page)).lower, "the Terminal dragged into the upper half becomes a tab again");
    // The grip that started that drop is gone. The next grip click must open
    // the menu (the flag-left-standing bug).
    await page.locator(".dock-tabs .grip").click();
    await page.waitForSelector(".ctxmenu", { timeout: 3000 }).catch(() => {});
    ok(await page.evaluate(() => !!document.querySelector(".ctxmenu")), "after a drop that unmounted its grip, the next grip click opens the menu");
    await page.locator('.ctxmenu [data-dest="term-below"]').click();
    await page.waitForTimeout(400);
    ok((await sides(page)).lower, "…and the reference grip's menu puts the Terminal back below");
    // keyboard move from the Terminal's own menu into the tabs: focus must land on a grip that exists
    await page.locator(".dock-lower .grip").focus();
    await page.keyboard.press("Enter");
    await page.waitForSelector(".ctxmenu");
    await page.locator('.ctxmenu [data-dest="term-tab"]').focus();
    await page.keyboard.press("Enter");
    await page.waitForTimeout(400);
    ok((await page.evaluate(() => document.activeElement?.dataset?.grip)) === "reference", "a move that unmounts the grip returns focus to the reference grip");
    await ctx.close();
  }

  // ── "below" when the slot cannot show says so ──
  {
    const { ctx, page } = await open(browser, {}, { width: 1280, height: 370 });
    await page.locator(".dock-tabs .grip").click();
    await page.waitForSelector(".ctxmenu");
    await page.locator('.ctxmenu [data-dest="term-below"]').click();
    await page.waitForTimeout(300);
    const live = await page.evaluate(() => document.querySelector('[data-testid="arrange-live"]').textContent);
    ok(/set to sit below/.test(live) && /tall enough/.test(live) && !(await sides(page)).lower, `a window too short for the slot: "${live}"`);
    await ctx.close();
  }

  // ── Settings → Layout → Reset, with its own Undo; re-announcing ──
  {
    const { ctx, page } = await open(browser, { places_side: "right", dock_lower: "terminal" });
    await page.locator('[data-track="settings"]').click();
    await page.getByText("Navigation", { exact: true }).first().click();
    await page.locator('[data-testid="layout-reset"]').click();
    await page.waitForTimeout(300);
    let s = await sides(page);
    ok(s.p === "left" && s.d === "right" && !s.lower && await page.locator('[data-testid="layout-reset"]').isDisabled(), "Reset gives Classic, tabs only, and greys out");
    const undoHit = await page.evaluate(() => { const e = document.querySelector('[data-testid="layout-reset-undo"]'); if (!e) return "absent"; const b = e.getBoundingClientRect(); return e.contains(document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2)) ? "hit" : "covered"; });
    ok(undoHit === "hit", `Reset's Undo is beside the button and reachable over the sheet (${undoHit})`);
    const n0 = await page.evaluate(() => document.querySelector('[data-testid="arrange-live"] span')?.textContent);
    await page.locator('[data-testid="layout-reset-undo"]').click();
    await page.waitForTimeout(300);
    s = await sides(page);
    ok(s.p === "right" && s.d === "left" && s.lower, `Undo restores all three (${JSON.stringify(s)}; was "${n0}")`);
    // the same words twice must be two announcements: the span is re-keyed
    const span1 = await page.evaluate(() => { const e = document.querySelector('[data-testid="arrange-live"] span'); e.dataset.mark = "1"; return e.textContent; });
    await page.locator('[data-testid="layout-reset"]').click();
    await page.waitForTimeout(200);
    await page.locator('[data-testid="layout-reset-undo"]').click();
    await page.waitForTimeout(300);
    const span2 = await page.evaluate(() => { const e = document.querySelector('[data-testid="arrange-live"] span'); return { text: e.textContent, fresh: !e.dataset.mark }; });
    ok(span1 === "Layout restored" && span2.text === "Layout restored" && span2.fresh, "\"Layout restored\" twice is announced twice (a fresh node each time)");
    await ctx.close();
  }
  await browser.close();
}
console.log(failed ? `\narrange-browser: ${failed} failure(s)` : "\narrange-browser: all passed");
process.exit(failed ? 1 : 0);
