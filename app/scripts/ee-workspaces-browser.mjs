// Optional browser gate for "Show Everything Else in Workspaces" (Settings →
// Layout, `nav_ee_in_workspaces`), in Chromium AND WebKit, against the mock.
// ee-workspaces-check.mjs holds the key and the wiring; this is what the nav
// DRAWS in each position and what its controls then do.
//
//   VITE_MOCK=1 vite --port 1489 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     EE_MOCK_URL=http://localhost:1489 node app/scripts/ee-workspaces-browser.mjs
//
//   - on (the default): Everything Else, its header, count and fold, as before;
//   - the checkbox in Settings flips it live, and the choice survives a reload;
//   - off: the Workspaces view draws the named workspaces and nothing else;
//     All projects lists every project, unchanged;
//   - off: a waiting place in a project in no workspace puts an amber dot on
//     the switch's All projects side (only while Workspaces shows), and a
//     waiting place inside a named workspace does not;
//   - off: taking a project out of its last workspace says where it went;
//   - off: a selected place in a hidden project: no row is lit, nothing
//     breaks, the place still shows, Back and Forward still move;
//   - off: the filter's Enter selects the first row the view shows;
//   - with no named workspace: on, the group as today; off, an EMPTY view
//     (the owner's choice) with its line, New workspace and All projects,
//     and the All projects mark when a place there waits.
// Screenshots (both positions, 240px and 300px nav, dark and light) go to
// EE_SHOTS (default: the place's .worktrees-attachments/agent/).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const { chromium, webkit } = pw.chromium ? pw : pw.default;
const BASE = process.env.EE_MOCK_URL ?? "http://localhost:1489";
const SHOTS = process.env.EE_SHOTS ?? fileURLToPath(new URL("../../.worktrees-attachments/agent/", import.meta.url));
fs.mkdirSync(SHOTS, { recursive: true });
const only = process.argv.find((a) => a.startsWith("--engine="))?.slice(9);

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

// `restore_last: false`: a boot at the view, no place selected (restore is on by
// default since 2026-11-25, and selecting the waiting place would ack its mark).
const SEED = { places_side: "left", dock_side: "left", settings_rev: 4, nav_pinned: true, nav_view: "workspaces", restore_last: false };
/** `free`: take the first workspace's first project out (it is in no other),
 *  so a project with real places is in no workspace — the mock's Everything
 *  Else otherwise holds only a "repo gone" project with none. */
async function open(browser, extra = {}, query = "workspaces=1&activity=2", { viewport = { width: 1280, height: 1400 }, free = true } = {}) {
  const page = await browser.newPage({ viewport });
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { ...SEED, ...extra });
  await page.goto(`${BASE}/?${query}`);
  await page.waitForSelector("[data-testid=nav-view-workspaces]");
  await page.waitForTimeout(600);
  if (free && query.includes("workspaces=1")) page.freed = await freeOne(page);
  return page;
}
async function freeOne(page) {
  const root = await page.evaluate(() => {
    const h = document.querySelector("[data-testid=ws-group] .ws-kids > .project[data-project-root]");
    const r = h.querySelector(".project-h").getBoundingClientRect();
    h.querySelector(".project-h").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 30, clientY: r.top + 5 }));
    return h.dataset.projectRoot;
  });
  await page.waitForTimeout(100);
  await page.click("[data-testid=ctx-ws-remove]");
  await page.waitForTimeout(300);
  return root;
}
const view = (page) => page.evaluate(() => {
  const g = document.querySelector("[data-testid=ee-group]");
  return {
    ee: !!g,
    header: g?.querySelector("[data-testid=ee-header]")?.textContent ?? null,
    eeProjects: g ? [...g.querySelectorAll(".project[data-project-root]")].map((r) => r.dataset.projectRoot) : [],
    named: [...document.querySelectorAll("[data-testid=ws-group] .ws-name")].map((n) => n.textContent),
    roots: [...new Set([...document.querySelectorAll(".nav-scroll .project[data-project-root]")].map((r) => r.dataset.projectRoot))],
    title: document.querySelector(".nav-scroll")?.textContent.includes("Everything Else"),
    mark: document.querySelector("[data-testid=nav-all-needs]")?.className ?? null,
    undo: document.querySelector(".undo")?.textContent ?? null,
  };
});
const reachable = (page, sel) => page.evaluate((s) => [...document.querySelectorAll(s)].map((el) => {
  el.scrollIntoView({ block: "nearest" });
  const r = el.getBoundingClientRect();
  const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
  return !!hit && (hit === el || el.contains(hit));
}), sel);
const toggle = async (page) => {
  await page.click("[data-track=settings]");
  await page.click(".settings-cat[data-cat=layout]");
  await page.waitForSelector(".settings-body[data-page=layout]");
  const box = page.locator("label.setting-check", { hasText: "Show Everything Else in Workspaces" }).locator("input[type=checkbox]");
  const was = (await box.count()) === 1 ? await box.isChecked() : null;
  await box.click();
  await page.waitForTimeout(150);
  const now = await box.isChecked();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(250);
  return { was, now };
};

async function run(engine, name) {
  const browser = await engine.launch();
  const tag = `[${name}]`;
  try {
    // ── the harness's own contract first: serve hands a socket that declares a
    // SNAPSHOT event (`scope::SNAPSHOT`: `sessions:busy` among them) its last
    // push at once, so a listener added after the poll's push still has the
    // waiting set. The mock's `?activity=<n>` pushes once at 400 ms; on a cold
    // vite App listens later than that, and every waiting place read idle
    // (the integrator's "wsc-dot done", 2026-11-25).
    {
      const p = await open(browser, {}, "workspaces=1&activity=2", { free: false });
      const late = await p.evaluate(() => new Promise((done) => {
        const T = window.__TAURI_INTERNALS__;
        const handler = T.transformCallback((e) => done(e.payload?.waiting ?? null));
        T.invoke("plugin:event|listen", { event: "sessions:busy", target: { kind: "Any" }, handler });
        setTimeout(() => done("nothing"), 500);
      }));
      ok(Array.isArray(late) && late.length > 0, `${tag} the mock replays the held sessions:busy to a listener added after its push (${JSON.stringify(late)})`);
      await p.close();
    }

    // ── on: the default, as before ──
    let page = await open(browser);
    const on = await view(page);
    ok(on.ee && /Everything Else\s*\d/.test(on.header ?? "") && on.eeProjects.includes(page.freed), `${tag} on (default): Everything Else with its header holds the freed project`);
    ok(on.mark === null, `${tag} …and the switch's All projects side carries no mark`);
    await page.click("[data-testid=ee-header] .ws-name");
    await page.waitForTimeout(150);
    ok((await view(page)).eeProjects.length === 0, `${tag} …its fold folds`);
    await page.click("[data-testid=ee-header] .ws-name");
    await page.waitForTimeout(150);

    // ── the checkbox flips it live ──
    const t = await toggle(page);
    ok(t.was === true && t.now === false, `${tag} Settings → Layout: "Show Everything Else in Workspaces", ticked, then unticked`);
    const off = await view(page);
    ok(!off.ee && !off.title, `${tag} off: no Everything Else group and no "Everything Else" anywhere in the view`);
    ok(off.named.join() === on.named.join(), `${tag} …the named workspaces as they were: ${JSON.stringify(off.named)}`);
    ok(!off.roots.includes(page.freed), `${tag} …and the project in no workspace is not in the view`);
    ok(off.mark && off.mark.includes("waiting"), `${tag} …its waiting place marks All projects: ${off.mark}`);
    ok((await reachable(page, "[data-testid=nav-all-needs]")).length === 1, `${tag} …the dot is drawn (inside the segment)`);
    await page.click("[data-testid=nav-view-projects]");
    await page.waitForTimeout(200);
    const all = await view(page);
    ok(all.roots.includes(page.freed) && all.roots.length >= 3 && all.mark === null, `${tag} All projects lists every project (${all.roots.length}), the freed one too, and drops the mark while it shows`);
    // the filter's Enter in Workspaces: the first row the view shows
    await page.click("[data-testid=nav-view-workspaces]");
    await page.waitForTimeout(200);
    const first = await page.$eval(".nav-scroll li.row[data-slug]", (r) => r.dataset.slug);
    await page.fill("input.search", "");
    await page.focus("input.search");
    await page.keyboard.press("Enter");
    await page.waitForTimeout(250);
    // (a project two workspaces hold lights its row in both)
    ok(await page.$eval(".nav-scroll li.row[data-slug]", (r) => r.classList.contains("sel")), `${tag} the filter's Enter selects the first row the view shows (${first})`);

    // a selected place in a hidden project
    await page.click("[data-testid=nav-view-projects]");
    await page.waitForTimeout(150);
    const hiddenSlug = await page.$eval(`.nav-scroll .project[data-project-root="${page.freed}"] li.row[data-slug]`, (r) => r.dataset.slug);
    await page.click(`.nav-scroll .project[data-project-root="${page.freed}"] li.row[data-slug="${hiddenSlug}"]`);
    await page.waitForTimeout(250);
    const errs = [];
    // xterm in a headless browser throws on `dimensions` when a place's
    // terminal mounts unsized: the harness's noise, not this feature's.
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errs.push(String(e)); });
    await page.click("[data-testid=nav-view-workspaces]");
    await page.waitForTimeout(250);
    const sel = await page.evaluate(() => ({ lit: document.querySelectorAll(".nav-scroll li.row.sel").length, id: document.querySelector(".identity")?.textContent ?? "" }));
    ok(sel.lit === 0 && sel.id.length > 0, `${tag} a place selected in a hidden project: no row lit in Workspaces, and the place still shows (${JSON.stringify(sel.id.slice(0, 40))})`);
    const backOn = await page.$eval("[data-testid=nav-back]", (b) => !b.disabled);
    if (backOn) { await page.click("[data-testid=nav-back]"); await page.waitForTimeout(250); }
    const afterBack = await page.evaluate(() => document.querySelector(".identity")?.textContent ?? "");
    ok(backOn && afterBack !== sel.id, `${tag} …Back moves to where it was before`);
    await page.click("[data-testid=nav-forward]").catch(() => {});
    await page.waitForTimeout(250);
    ok((await page.evaluate(() => document.querySelector(".identity")?.textContent ?? "")) === sel.id && errs.length === 0, `${tag} …Forward comes back to it, with no page error ${JSON.stringify(errs).slice(0, 300)}`);

    // taking a project out of a workspace, with the group hidden: out of one
    // of two, no word; out of its last, the Undo says where it went
    const shared = await page.evaluate(() => {
      const all = [...document.querySelectorAll("[data-testid=ws-group] .ws-kids > .project[data-project-root]")].map((p) => p.dataset.projectRoot);
      return all.find((r) => all.filter((x) => x === r).length === 2) ?? null;
    });
    const takeOut = async (wsIdx) => {
      await page.evaluate(([r, i]) => {
        const g = document.querySelectorAll("[data-testid=ws-group]")[i];
        const h = g.querySelector(`.ws-kids > .project[data-project-root="${r}"] .project-h`);
        const b = h.getBoundingClientRect();
        h.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: b.left + 30, clientY: b.top + 5 }));
      }, [shared, wsIdx]);
      await page.waitForTimeout(100);
      await page.click("[data-testid=ctx-ws-remove]");
      await page.waitForTimeout(300);
      return view(page);
    };
    ok(!!shared, `${tag} a project two workspaces hold: ${shared}`);
    let v = await takeOut(1);
    ok(v.roots.includes(shared) && /taken out of/.test(v.undo ?? "") && !/All projects/.test(v.undo ?? ""), `${tag} out of one of its two workspaces: still in the view, the Undo says no more (${JSON.stringify(v.undo)})`);
    v = await takeOut(0);
    ok(!v.roots.includes(shared) && /now only in All projects/.test(v.undo ?? ""), `${tag} out of its last: it leaves the view, and the Undo says where (${JSON.stringify(v.undo)})`);
    await page.click(".undo .undo-btn");
    await page.waitForTimeout(300);
    ok((await view(page)).roots.includes(shared), `${tag} …Undo brings it back`);

    // the choice survives a reload
    await page.reload();
    await page.waitForSelector("[data-testid=nav-view-workspaces]");
    await page.waitForTimeout(300);
    ok(!(await view(page)).ee, `${tag} off survives a reload`);
    await page.close();

    // off, but the only waiting places are inside named workspaces: no mark
    page = await open(browser, { nav_ee_in_workspaces: false }, "workspaces=1&activity=0");
    const quiet = await view(page);
    ok(!quiet.ee && !(quiet.mark ?? "").includes("waiting"), `${tag} off, waiting only inside a named workspace: no amber on All projects (${quiet.mark})`);
    await page.close();

    // ── no named workspace ──
    page = await open(browser, { nav_ee_in_workspaces: true }, "activity=2", { free: false });
    let g = await view(page);
    ok(g.ee && g.header && g.eeProjects.length > 0 && (await page.$$("[data-testid=ws-empty]")).length === 1 && !(await page.$("[data-testid=ws-empty-all]")) && g.mark === null,
      `${tag} no named workspace, on: "No workspaces yet" and Everything Else over every project, as today`);
    await page.close();
    page = await open(browser, { nav_ee_in_workspaces: false }, "activity=2", { free: false });
    g = await view(page);
    const emptyText = await page.$eval("[data-testid=ws-empty]", (e) => e.textContent).catch(() => "");
    ok(!g.ee && g.roots.length === 0 && /No workspaces yet/.test(emptyText), `${tag} no named workspace, off: an empty view that says "No workspaces yet"`);
    const btns = await reachable(page, "[data-testid=ws-new-first], [data-testid=ws-empty-all]");
    ok(btns.length === 2 && btns.every(Boolean), `${tag} …with New workspace and All projects, both reachable`);
    ok(g.mark?.includes("waiting"), `${tag} …and the waiting place marks All projects (${g.mark})`);
    await page.click("[data-testid=ws-empty-all]");
    await page.waitForTimeout(200);
    ok(await page.$eval("[data-testid=nav-view-projects]", (b) => b.getAttribute("aria-selected") === "true") && (await view(page)).roots.length >= 3,
      `${tag} …All projects opens All projects, every project in it`);
    await page.click("[data-testid=nav-view-workspaces]");
    await page.waitForTimeout(150);
    await page.click("[data-testid=ws-new-first]");
    await page.waitForTimeout(150);
    ok(!!(await page.$(".ws-new-row input")), `${tag} …New workspace opens its name field`);
    await page.close();

    // ── screenshots: 240px and 300px nav, dark and light, both positions ──
    if (name === "webkit") {
      for (const theme of ["tokyo-night", "tokyo-day"]) {
        for (const w of [240, 300]) {
          for (const v of [true, false]) {
            page = await open(browser, { nav_ee_in_workspaces: v, nav_width: w, theme }, "workspaces=1&activity=2", { viewport: { width: 1100, height: 900 } });
            if (w === 240) {
              const segs = await reachable(page, ".nav-view-seg");
              ok(segs.length === 2 && segs.every(Boolean), `${tag} ${theme} ${w}px ${v ? "on" : "off"}: both switch segments are reachable`);
              if (!v) {
                const fit = await page.evaluate(() => {
                  const d = document.querySelector("[data-testid=nav-all-needs]")?.getBoundingClientRect();
                  const s = document.querySelector("[data-testid=nav-view-projects]").getBoundingClientRect();
                  return !!d && d.width > 0 && d.left >= s.left && d.right <= s.right;
                });
                ok(fit, `${tag} ${theme} ${w}px off: the mark sits inside the All projects segment`);
              }
            }
            await page.$eval(".nav-scroll", (n) => { n.scrollTop = 0; });
            await page.waitForTimeout(100);
            await (await page.$(".nav")).screenshot({ path: path.join(SHOTS, `ee-workspaces-${v ? "on" : "off"}-${w}-${theme}.png`) });
            await page.close();
          }
        }
        page = await open(browser, { nav_ee_in_workspaces: false, nav_width: 240, theme }, "activity=2", { viewport: { width: 1100, height: 900 }, free: false });
        await (await page.$(".nav")).screenshot({ path: path.join(SHOTS, `ee-workspaces-off-empty-240-${theme}.png`) });
        await page.close();
      }
    }
  } finally {
    await browser.close();
  }
}

for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  if (only && only !== name) continue;
  await run(engine, name);
}
console.log(failed ? `\n${failed} failed` : "\nall passed");
process.exit(failed ? 1 : 0);
