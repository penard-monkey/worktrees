// Optional browser gate for the named workspaces view (idea board phase 2),
// in Chromium AND WebKit (the app is WKWebView; the harness is Chrome —
// AGENTS.md). Against the mock, seeded with `?workspaces=1` and the Ideas and
// workspaces beta on (beta.ts): "Everything"
// holds two projects and one root that is not registered, "Tools" shares one
// of them.
//
//   VITE_MOCK=1 vite --port 1467 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     WS_MOCK_URL=http://localhost:1467 node app/scripts/workspaces-browser.mjs
//
// What it holds (workspaces-check.mjs holds the rules and the wiring; this is
// what the running nav DRAWS and what its controls DO):
//   - the switch is there, reachable (elementFromPoint), and All projects is
//     today's nav; the Workspaces view lists each workspace, its projects, and
//     every place of each — a shared project under both, the unregistered one
//     as "not registered";
//   - the layout: the workspace header is sticky and its projects' headers
//     stick UNDER it, not over it (getComputedStyle);
//   - the menus, each a drag's twin: Add to workspace adds, Remove from
//     workspace takes out and Undo puts back, Move down reorders, Rename, a
//     New workspace field, Remove workspace asks first;
//   - a place selected under a workspace is the same selection in All projects;
//   - the view is remembered across a reload (the per-viewer blob);
//   - a synthetic pointer drag: a project onto a workspace that holds it lights
//     nothing and does nothing, onto one that does not lights it and adds it;
//     Escape cancels; a workspace dragged above another reorders.
//     (A synthetic drag bypasses hit-testing on the way IN — AGENTS.md — so
//     these only drive what happens after the press; the real drag is a
//     hand test.)
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.WS_MOCK_URL ?? "http://localhost:1467";

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

const groups = (page) => page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-group]")].map((g) => ({
  name: g.querySelector(".ws-name")?.textContent ?? g.querySelector("input")?.value ?? "",
  projects: [...g.querySelectorAll(".ws-kids > .project:not(.ws-gone) .pname")].map((n) => n.textContent),
  gone: [...g.querySelectorAll("[data-testid=ws-gone] .pgone")].map((n) => n.textContent),
  places: g.querySelectorAll("li.row[data-slug]").length,
  lit: g.classList.contains("drop-on"),
})));
const reachable = (page, sel) => page.evaluate((s) => {
  const el = document.querySelector(s);
  if (!el) return false;
  const r = el.getBoundingClientRect();
  const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
  return !!hit && (hit === el || el.contains(hit));
}, sel);
/** `ws:<name>|<selector>` addresses a workspace group by its NAME (an open gap
 *  is a div too, so `:nth-of-type` shifts under a drag); anything else is a
 *  plain selector. Passed into the page as data and resolved there. */
const where = (sel) => (sel.startsWith("ws:") ? { name: sel.slice(3).split("|")[0], rest: sel.split("|")[1] ?? "" } : { css: sel });
/** One step of a pointer drag, then a frame for navdrag's rAF to resolve. */
const pointer = (page, type, sel, dy = 0) => page.evaluate(async ([t, w, d]) => {
  const g = w.css ? null : [...document.querySelectorAll("[data-testid=ws-group]")].find((x) => x.querySelector(".ws-name")?.textContent === w.name);
  const el = w.css ? document.querySelector(w.css) : w.rest ? g.querySelector(w.rest) : g;
  const r = el.getBoundingClientRect();
  const x = r.left + Math.min(40, r.width / 2), y = r.top + r.height / 2 + d;
  const target = t === "pointerdown" ? el : window;
  target.dispatchEvent(new PointerEvent(t, { bubbles: true, clientX: x, clientY: y, button: 0, buttons: t === "pointerup" ? 0 : 1, pointerId: 1 }));
  await new Promise((res) => requestAnimationFrame(() => requestAnimationFrame(res)));
}, [type, where(sel), dy]);
const ctxOn = async (page, sel) => {
  await page.evaluate((w) => {
    const g = w.css ? null : [...document.querySelectorAll("[data-testid=ws-group]")].find((x) => x.querySelector(".ws-name")?.textContent === w.name);
    const el = w.css ? document.querySelector(w.css) : w.rest ? g.querySelector(w.rest) : g;
    const r = el.getBoundingClientRect();
    el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 30, clientY: r.top + 5 }));
  }, where(sel));
  await page.waitForTimeout(80);
};

async function run(engine, name) {
  const browser = await engine.launch();
  // Tall enough that every group's header is on screen: a synthetic pointer
  // over a header scrolled out of the nav hits nothing (the drags below).
  const page = await browser.newPage({ viewport: { width: 1280, height: 2400 } });
  // A pinned nav: unpinned, it hides itself on Home (arrange-browser's seed).
  // Workspaces are a beta (beta.ts), off by default: the seed turns it on.
  // `beta-browser.mjs` holds what the nav draws with it off.
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "left", settings_rev: 4, nav_pinned: true, beta_ideas: true });
  await page.goto(`${BASE}/?workspaces=1`);
  await page.waitForSelector("[data-testid=nav-view-projects]");
  const tag = `[${name}]`;

  ok(await reachable(page, "[data-testid=nav-view-workspaces]"), `${tag} the Workspaces switch is reachable`);
  ok(await page.$eval("[data-testid=nav-view-projects]", (b) => b.textContent === "All projects" && b.getAttribute("aria-selected") === "true"),
    `${tag} All projects is the default and says so`);
  const allRows = await page.$$eval(".nav-scroll li.row[data-slug]", (r) => r.length);
  ok(allRows > 0 && (await page.$$("[data-testid=ws-group]")).length === 0, `${tag} All projects is today's nav (${allRows} rows, no groups)`);

  await page.click("[data-testid=nav-view-workspaces]");
  await page.waitForSelector("[data-testid=ws-group]");
  let g = await groups(page);
  ok(g.length === 2 && g[0].name === "Everything" && g[1].name === "Tools", `${tag} two workspaces, in order: ${JSON.stringify(g.map((x) => x.name))}`);
  ok(g[0].projects.length === 2 && g[0].gone.join() === "not registered", `${tag} Everything: two projects and one "not registered"`);
  ok(g[1].projects.length === 1 && g[0].projects.includes(g[1].projects[0]), `${tag} Tools shares a project with Everything`);
  ok(g[0].places > 0 && g[1].places > 0, `${tag} every place of each project is drawn (${g[0].places}, ${g[1].places})`);

  // layout
  const lay = await page.evaluate(() => {
    const wh = document.querySelector(".ws-h"), ph = document.querySelector(".ws-kids .project-h");
    const a = getComputedStyle(wh), b = getComputedStyle(ph);
    return { whPos: a.position, whTop: a.top, phPos: b.position, phTop: b.top, whH: wh.getBoundingClientRect().height, z: [a.zIndex, b.zIndex] };
  });
  ok(lay.whPos === "sticky" && lay.whTop === "0px", `${tag} the workspace header sticks at the top`);
  ok(lay.phPos === "sticky" && parseFloat(lay.phTop) === lay.whH, `${tag} its projects' headers stick under it (${lay.phTop} vs ${lay.whH}px)`);
  ok(Number(lay.z[0]) > Number(lay.z[1]), `${tag} …and the workspace header paints over them`);
  await page.$eval("[data-testid=ws-new]", (b) => b.scrollIntoView({ block: "nearest" }));
  ok(await reachable(page, "[data-testid=ws-new]"), `${tag} New workspace is reachable (at the list's end)`);
  await page.$eval(".nav-scroll", (n) => { n.scrollTop = 0; });

  // a place selected through a workspace is the place selected in All projects
  const slug = await page.$eval("[data-testid=ws-group] li.row[data-slug]", (r) => r.dataset.slug);
  await page.click(`[data-testid=ws-group] li.row[data-slug="${slug}"]`);
  await page.waitForTimeout(150);
  await page.click("[data-testid=nav-view-projects]");
  await page.waitForTimeout(150);
  ok(await page.$$eval(`.nav-scroll li.row.sel[data-slug="${slug}"]`, (r) => r.length === 1), `${tag} the place selected under a workspace is selected in All projects`);

  // menu: Add to workspace (from All projects)
  const firstRoot = await page.$eval(".nav-scroll .project[data-project-root]", (p) => p.dataset.projectRoot);
  await ctxOn(page, `.project[data-project-root="${firstRoot}"] .project-h`);
  await page.click("[data-testid=ctx-ws-add]");
  const offers = await page.$$eval("[data-testid=ws-add-item]", (b) => b.map((x) => x.textContent.trim()));
  ok(!offers.includes("Everything"), `${tag} Add to workspace offers only the ones that do not hold it: ${JSON.stringify(offers)}`);
  await page.click("[data-testid=ws-add-item]");
  await page.waitForTimeout(100);
  await page.click("[data-testid=nav-view-workspaces]");
  await page.waitForTimeout(100);
  g = await groups(page);
  ok(g[1].projects.length === 2, `${tag} …and adds it (Tools: ${g[1].projects.length})`);

  // menu: Remove from workspace, then Undo
  await ctxOn(page, "ws:Tools|.ws-kids > .project:not(.ws-gone) .project-h");
  ok((await page.$$("[data-testid=ctx-ws-remove]")).length === 1, `${tag} a project under a workspace has Remove from workspace`);
  await page.click("[data-testid=ctx-ws-remove]");
  await page.waitForSelector(".undo");
  g = await groups(page);
  ok(g[1].projects.length === 1, `${tag} …which takes it out`);
  await page.click(".undo .undo-btn");
  await page.waitForTimeout(100);
  g = await groups(page);
  ok(g[1].projects.length === 2, `${tag} …and Undo puts it back`);
  await page.click("[data-testid=nav-view-projects]");
  await page.waitForTimeout(80);
  ok((await page.$$eval(".nav-scroll .project[data-project-root]", (p) => p.length)) >= 2, `${tag} All projects is unchanged by it`);
  await page.click("[data-testid=nav-view-workspaces]");
  await page.waitForTimeout(80);

  // menu: Move down
  await page.click("[data-testid=ws-group] [data-testid=ws-menu]");
  await page.click("[data-testid=ws-move-down]");
  await page.waitForTimeout(100);
  g = await groups(page);
  ok(g.map((x) => x.name).join() === "Tools,Everything", `${tag} Move down reorders`);

  // rename
  await page.click("[data-testid=ws-group] [data-testid=ws-menu]");
  await page.click("[data-testid=ws-rename]");
  await page.fill("[data-testid=ws-name-input]", "Toolbox");
  await page.press("[data-testid=ws-name-input]", "Enter");
  await page.waitForTimeout(100);
  g = await groups(page);
  ok(g[0].name === "Toolbox", `${tag} Rename renames`);

  // new
  await page.click("[data-testid=ws-new]");
  await page.fill("[data-testid=ws-name-input]", "Phone");
  await page.press("[data-testid=ws-name-input]", "Enter");
  await page.waitForTimeout(100);
  g = await groups(page);
  ok(g.length === 3 && g[2].name === "Phone" && g[2].projects.length === 0, `${tag} New workspace makes an empty one at the end`);
  ok(g[2].name === "Phone" && (await page.$$(".ws-drop-hint")).length === 1, `${tag} …which says how to fill it`);

  await page.$eval(".nav-scroll", (n) => { n.scrollTop = 0; });
  // drag: a project from Everything onto Toolbox (which holds it: nothing), then onto Phone (adds)
  const from = "ws:Everything|.ws-kids > .project:not(.ws-gone) .project-h";
  await pointer(page, "pointerdown", from);
  await pointer(page, "pointermove", from, 6);
  await pointer(page, "pointermove", "ws:Toolbox|.ws-h");
  g = await groups(page);
  const holds = g[0].projects.includes(g[1].projects[0]);
  ok(holds ? !g[0].lit : g[0].lit, `${tag} a workspace that already holds it lights ${holds ? "nothing" : "up"}`);
  await pointer(page, "pointermove", "ws:Phone|.ws-h");
  g = await groups(page);
  ok(g[2].lit, `${tag} one that does not hold it lights up`);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(60);
  await pointer(page, "pointerup", "ws:Phone|.ws-h");
  g = await groups(page);
  ok(g[2].projects.length === 0 && !g[2].lit, `${tag} Escape cancels the drop`);
  await page.waitForTimeout(250);
  await pointer(page, "pointerdown", from);
  await pointer(page, "pointermove", from, 6);
  await pointer(page, "pointermove", "ws:Phone|.ws-h");
  await pointer(page, "pointerup", "ws:Phone|.ws-h");
  await page.waitForTimeout(100);
  g = await groups(page);
  ok(g[2].projects.length === 1, `${tag} dropping it there adds it`);
  ok(g[1].projects.length === 2, `${tag} …and leaves it where it came from (a project can be in many)`);
  await page.waitForTimeout(250);

  // drag: Phone above Toolbox (from the top: a header scrolled away is not
  // under any pointer)
  await page.$eval(".nav-scroll", (n) => { n.scrollTop = 0; });
  await pointer(page, "pointerdown", "ws:Phone|.ws-name");
  await pointer(page, "pointermove", "ws:Phone|.ws-name", -6);
  await pointer(page, "pointermove", "ws:Toolbox|.ws-h", -8);
  ok((await page.$$(".drop-gap.ws")).length === 1, `${tag} a workspace drag opens a gap`);
  await pointer(page, "pointerup", "ws:Toolbox|.ws-h", -8);
  await page.waitForTimeout(150);
  g = await groups(page);
  ok(g.map((x) => x.name).join() === "Phone,Toolbox,Everything", `${tag} …and drops it there: ${g.map((x) => x.name).join()}`);
  await page.waitForTimeout(250);

  // remove workspace asks first
  await page.click("[data-testid=ws-group] [data-testid=ws-menu]");
  await page.click("[data-testid=ws-remove]");
  ok((await groups(page)).length === 3, `${tag} Remove workspace arms first`);
  await page.click("[data-testid=ws-remove]");
  await page.waitForTimeout(100);
  ok((await groups(page)).length === 2, `${tag} …and removes on the second click`);

  // remembered
  await page.reload();
  await page.waitForSelector("[data-testid=nav-view-workspaces]");
  ok(await page.$eval("[data-testid=nav-view-workspaces]", (b) => b.getAttribute("aria-selected") === "true"), `${tag} the view is remembered across a reload`);

  // drag from All projects: held over the Workspaces switch, the view springs
  // open; dropped on a workspace, the project is added. (The reload reseeded
  // the mock: Everything, Tools.)
  await page.click("[data-testid=nav-view-projects]");
  await page.waitForTimeout(100);
  const r0 = await page.$eval(".nav-scroll .project[data-project-root]", (p) => p.dataset.projectRoot);
  const head = `.project[data-project-root="${r0}"] .project-h`;
  await pointer(page, "pointerdown", head);
  await pointer(page, "pointermove", head, 6);
  await pointer(page, "pointermove", "[data-testid=nav-view-workspaces]");
  await page.waitForTimeout(700);
  ok(await page.$eval("[data-testid=nav-view-workspaces]", (b) => b.getAttribute("aria-selected") === "true"), `${tag} a project held over Workspaces springs the view open`);
  await pointer(page, "pointermove", "ws:Tools|.ws-h");
  ok((await groups(page))[1].lit, `${tag} …and Tools, which lacks it, lights up`);
  await pointer(page, "pointerup", "ws:Tools|.ws-h");
  await page.waitForTimeout(150);
  g = await groups(page);
  ok(g[1].projects.length === 2, `${tag} …and takes it on the drop`);
  await page.waitForTimeout(250);

  // drag it out of Tools onto "All projects": out of Tools, Undo offered,
  // still registered
  const out = `ws:Tools|.project[data-project-root="${r0}"] .project-h`;
  await pointer(page, "pointerdown", out);
  await pointer(page, "pointermove", out, 6);
  await pointer(page, "pointermove", "[data-testid=nav-view-projects]");
  ok(await page.$eval("[data-testid=nav-view-projects]", (b) => b.classList.contains("drop-on")), `${tag} a project from a workspace lights All projects`);
  await pointer(page, "pointerup", "[data-testid=nav-view-projects]");
  await page.waitForSelector(".undo");
  g = await groups(page);
  ok(g[1].projects.length === 1, `${tag} …and dropped there, leaves the workspace (Undo offered)`);
  await page.click("[data-testid=nav-view-projects]");
  await page.waitForTimeout(100);
  ok((await page.$$(`.nav-scroll .project[data-project-root="${r0}"]`)).length === 1, `${tag} …and is still in All projects`);
  await browser.close();
}

await run(chromium, "chromium");
await run(webkit, "webkit");
if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("\nworkspaces-browser: all ok");
