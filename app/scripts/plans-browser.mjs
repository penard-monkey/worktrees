// Optional browser gate for what a plan DOES (idea board phase 5), in Chromium
// AND WebKit, against the mock. plans-check.mjs holds the rules and wiring.
//
//   VITE_MOCK=1 vite --port 1471 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     PLANS_MOCK_URL=http://localhost:1471 node app/scripts/plans-browser.mjs
// PLANS_SCREENSHOTS=<dir> saves each window per engine.
//
// What it holds, in the Ideas window and in the overview:
//   - Review… opens the brief as text; nothing has started; an edited brief is
//     what plan_start sends; one Start makes one place, its line is written,
//     and the other items stay "not started";
//   - a double press starts once; a branch in another place and an empty brief
//     are refused before Start; a refused launch keeps the place and offers
//     Launch anyway;
//   - busy / there / removed follow the snapshot, and the line stays; with a
//     slow list (?slowlist) a place just started is never called removed;
//   - a place dragged onto a free item is recorded, and lights nothing on an
//     item that has one; Link a place… does the same without a drag;
//   - Clone… and Create… open the main window's dialogs filled in and waiting,
//     with the Documents warning; after Create the line reads registered and
//     the workspace holds the project;
//   - every button is reachable (elementFromPoint) at 1200 and 720 px, and the
//     review's words read ≥ 4.5:1 in all six themes.
// The synthetic drag bypasses hit-testing on the way IN (AGENTS.md): the real
// drag is a hand test.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.PLANS_MOCK_URL ?? "http://localhost:1471";
const SHOTS = process.env.PLANS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
const JELLY = "01JC8Z4K3M9QX2T7VB00000901";
const ROOT = "/Users/demo/workspace/worktrees";

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const BODY = `# Plan: Phase five

## Projects
- worktrees (registered)
- ans (clone: https://github.com/demo/ans.git)
- cfg (new: ~/Documents/cfg)

## Work
### Role
project: worktrees
branch: feat/p5-role
brief: |
  Goal: the role.

  Stop when it is green.

### Second
project: worktrees
branch: feat/p5-second
brief: Second brief.

### Held
project: worktrees
branch: feat/p5-role
brief: x

### dead launch
project: worktrees
branch: feat/p5-dead
brief: y

### Later
project: ans
branch: later
brief: z
`;
const text = (page, sel) => page.evaluate((s) => document.querySelector(s)?.textContent ?? null, sel);
const rows = (page, sel) => page.evaluate((s) => [...document.querySelectorAll(s)].map((r) => ({ kind: r.dataset.place, text: r.innerText.replace(/\s+/g, " ") })), sel);
const reachable = (page, sel) => page.evaluate((s) => [...document.querySelectorAll(s)].map((el) => {
  el.scrollIntoView({ block: "nearest" });
  const r = el.getBoundingClientRect(), hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
  return r.width > 0 && !!hit && (hit === el || el.contains(hit));
}), sel);
/** Press on `from`, move onto `to`, look, release. */
async function drag(page, from, to, look) {
  const pt = (sel) => page.evaluate((s) => { const el = document.querySelector(s); el.scrollIntoView({ block: "nearest" }); const r = el.getBoundingClientRect(); return { x: r.left + Math.min(30, r.width / 2), y: r.top + r.height / 2 }; }, sel);
  const a = await pt(from), b = await pt(to);
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await page.mouse.move(a.x + 8, a.y + 8);
  await page.mouse.move(b.x, b.y, { steps: 4 });
  await page.waitForTimeout(120);
  const seen = await page.evaluate(look);
  await page.mouse.up();
  await page.waitForTimeout(300);
  return seen;
}
const contrast = (page, sels) => page.evaluate(([sels, themes]) => {
  const parse = (s) => {
    let m = s.match(/^rgba?\(([^)]+)\)/);
    if (m) { const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return { r: p[0], g: p[1], b: p[2], a: p[3] ?? 1 }; }
    m = s.match(/^color\(srgb ([^)]+)\)/);
    const p = m[1].split(/[ /]+/).filter(Boolean).map(Number);
    return { r: p[0] * 255, g: p[1] * 255, b: p[2] * 255, a: p[3] ?? 1 };
  };
  const over = (t, u) => ({ r: t.r * t.a + u.r * (1 - t.a), g: t.g * t.a + u.g * (1 - t.a), b: t.b * t.a + u.b * (1 - t.a), a: 1 });
  const bg = (el) => { const st = []; for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c.a > 0) st.push(c); if (c.a >= 1) break; } return st.reverse().reduce((x, c) => (x ? over(c, x) : c), null); };
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b); };
  const before = document.documentElement.dataset.theme;
  let worst = [99, ""];
  for (const t of themes) {
    document.documentElement.dataset.theme = t;
    for (const s of sels) for (const el of document.querySelectorAll(s)) {
      const [x, y] = [lum(parse(getComputedStyle(el).color)), lum(bg(el))].sort((p, q) => q - p);
      const r = (x + 0.05) / (y + 0.05);
      if (r < worst[0]) worst = [r, `${t} ${s}`];
    }
  }
  document.documentElement.dataset.theme = before ?? "";
  return worst;
}, [sels, THEMES]);

async function companion(page, tag, name) {
  await page.goto(`${BASE}/companion/index.html?ideas=seed`);
  await page.waitForSelector("[data-testid=ideas-rows] li");
  const id = (await call(page, "plan_make", { ideas: [JELLY], draft: BODY })).id;
  await page.waitForSelector(`[data-idea='${id}'] button.ix-row-main`);
  await page.click(`[data-idea='${id}'] button.ix-row-main`);
  await page.waitForSelector("[data-testid=plan-work]");
  const doc = () => page.evaluate((i) => window.__mock.ideaFile(i).content, id);
  const work = () => rows(page, "[data-testid=plan-work]");
  const review = async (n) => { await page.locator("[data-testid=plan-work]").nth(n).locator("[data-testid=plan-review]").click(); await page.waitForSelector("[data-testid=start-review]"); };
  ok((await work()).every((w) => w.kind === "none" && w.text.includes("not started")), `${tag} five items, none started`);
  ok((await reachable(page, "[data-testid=plan-review], [data-testid=plan-link-place], [data-testid=plan-bring]")).every(Boolean), `${tag} every button in the panel is reachable at 1200`);

  // ── the review, then Start ──
  await review(0);
  ok(await page.inputValue("[data-testid=start-brief]") === "Goal: the role.\n\nStop when it is green.", `${tag} Review… shows the brief as it would be written`);
  ok(Object.keys(await page.evaluate(() => window.__mock.placeBriefs())).length === 0 && !(await doc()).includes("started:"), `${tag} reviewing starts nothing`);
  const facts = await page.evaluate(() => ["start-project", "start-branch", "start-agent"].map((t) => document.querySelector(`[data-testid=${t}]`).textContent));
  ok(facts[0].includes(ROOT) && facts[1].includes("feat/p5-role") && facts[1].includes("feat-p5-role") && facts[2].includes("Claude"), `${tag} …and where it goes: ${facts.join(" | ")}`);
  await page.fill("[data-testid=start-brief]", "   ");
  ok(await page.isDisabled("[data-testid=start-go]") && (await text(page, "[data-testid=start-problem]")).includes("empty"), `${tag} an empty brief cannot start`);
  await page.fill("[data-testid=start-brief]", "Goal: reviewed.\n- and edited");
  const w = await contrast(page, [".sw-head", ".sw-facts dt", ".sw-facts dd", ".sw-quiet", ".sw-label", ".sw-brief", ".sw-btn:not(:disabled)"]);
  ok(w[0] >= 4.5, `${tag} the review reads ≥ 4.5:1 in every theme (worst ${w[0].toFixed(2)} ${w[1]})`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/plan-review-${name}.png` });
  for (const width of [720, 1200]) {
    await page.setViewportSize({ width, height: 800 });
    ok((await reachable(page, "[data-testid=start-go], [data-testid=start-close], [data-testid=start-brief]")).every(Boolean), `${tag} the review's controls are reachable at ${width}`);
  }
  await page.evaluate(() => { const b = document.querySelector("[data-testid=start-go]"); b.click(); b.click(); });
  ok(await until(page, () => document.querySelector("[data-testid=start-done]")), `${tag} Start says it started`);
  const briefs = await page.evaluate(() => window.__mock.placeBriefs());
  ok(JSON.stringify(briefs) === JSON.stringify({ [`${ROOT}|feat-p5-role`]: "Goal: reviewed.\n- and edited" }), `${tag} one place, with the edited text, though it was pressed twice: ${JSON.stringify(briefs)}`);
  await page.click("[data-testid=start-close]");
  ok((await doc()).includes(`  Stop when it is green.\nstarted: ${ROOT} · feat-p5-role · `) && (await doc()).match(/started:/g).length === 1, `${tag} its started: line is in the plan, and only its`);
  ok((await doc()).includes("  Goal: the role."), `${tag} the plan's own brief is as it was`);
  await until(page, () => document.querySelector("[data-testid=plan-work]")?.dataset.place === "there");
  let rs = await work();
  ok(rs[0].text.includes("open") && rs[0].kind === "there" && rs.slice(1).every((r) => r.kind === "none"), `${tag} the item is open, the others are not started: ${rs.map((r) => r.kind)}`);
  ok(await page.locator("[data-testid=plan-work]").nth(0).locator("[data-testid=plan-review]").count() === 0, `${tag} a started item offers no second Start`);

  // ── refused before Start ──
  await review(2);
  ok(await page.isDisabled("[data-testid=start-go]") && (await text(page, "[data-testid=start-problem]")).includes("already in the place feat-p5-role"), `${tag} a branch in another place is refused, said`);
  await page.keyboard.press("Escape");
  ok(await page.locator("[data-testid=start-review]").count() === 0, `${tag} Escape closes the review`);
  await review(4);
  ok(await page.isDisabled("[data-testid=start-go]") && (await text(page, "[data-testid=start-problem]")).includes("not cloned yet"), `${tag} an item whose project is not cloned cannot start`);
  await page.click("[data-testid=start-close]");

  // ── a refused launch ──
  await review(3);
  await page.click("[data-testid=start-go]");
  ok(await until(page, () => document.querySelector("[data-testid=start-refused]")) && (await text(page, "[data-testid=start-refused]")).includes("5 agents"), `${tag} a refused launch says the place is ready and why`);
  await page.click("[data-testid=start-anyway]");
  ok(await until(page, () => document.querySelector("[data-testid=start-done]")), `${tag} Launch anyway starts its agent`);
  await page.click("[data-testid=start-close]");
  ok((await doc()).match(/started:/g).length === 2, `${tag} its line was written when the place was made, once`);

  // ── removed, then a place linked ──
  await page.evaluate(([r, s]) => window.__mock.placeRemoveElsewhere(r, s), [ROOT, "feat-p5-role"]);
  ok(await until(page, () => document.querySelector("[data-testid=plan-work]")?.dataset.place === "gone"), `${tag} the place removed: the item says so`);
  ok((await work())[0].text.includes("place removed") && (await doc()).includes(`${ROOT} · feat-p5-role · `), `${tag} …and keeps its line`);
  await page.locator("[data-testid=plan-work]").nth(1).locator("[data-testid=plan-link-place]").click();
  await page.waitForSelector("[data-testid=pick-place-row]");
  const slug = await page.getAttribute(`[data-testid=pick-place-row][data-root='${ROOT}']`, "data-slug");
  const src = `[data-testid=pick-place-row][data-root='${ROOT}'][data-slug='${slug}']`;
  const line = async (n) => `[data-drop-work='${await page.locator("[data-testid=plan-work]").nth(n).getAttribute("data-drop-work")}']`;
  ok(await drag(page, src, await line(3), () => document.querySelectorAll(".ix-plan-work.ix-drop").length) === 0, `${tag} a place over an item that has one lights nothing`);
  ok(!(await doc()).includes(`· ${slug} ·`), `${tag} …and dropping it there writes nothing`);
  ok(await drag(page, src, await line(1), () => document.querySelectorAll(".ix-plan-work.ix-drop").length) === 1, `${tag} over a free item it lights that item`);
  await until(page, () => document.querySelectorAll("[data-testid=plan-work]")[1]?.dataset.place !== "none");
  ok((await doc()).includes(`brief: Second brief.\nstarted: ${ROOT} · ${slug} · `), `${tag} …and dropping it writes the item's line`);
  // The gone item takes a place through the picker's click.
  await page.locator("[data-testid=plan-work]").nth(0).locator("[data-testid=plan-link-place]").click();
  const other = await page.evaluate(([r, s]) => [...document.querySelectorAll(`[data-testid=pick-place-row][data-root='${r}']`)].map((x) => x.dataset.slug).find((x) => x !== s && x !== "feat-p5-dead"), [ROOT, slug]);
  await page.click(`[data-testid=pick-place-row][data-root='${ROOT}'][data-slug='${other}']`);
  await until(page, () => document.querySelector("[data-testid=plan-work]")?.dataset.place !== "gone");
  ok((await doc()).includes(`started: ${ROOT} · ${other} · `) && !(await doc()).includes("feat-p5-role · "), `${tag} Link a place… replaces the line of a place that is gone`);

  // ── Clone… asks the main window ──
  await page.click("[data-testid=plan-bring][data-name=ans]");
  const asked = await page.evaluate(() => JSON.parse(localStorage.getItem("wt.main.ask") ?? "null"));
  ok(asked?.act === "project" && asked.plan === id && asked.name === "ans", `${tag} Clone… asks the main window, naming the project (${JSON.stringify(asked)})`);
  const dots = await page.evaluate(() => ["there", "none"].map((k) => { const el = document.querySelector(`[data-testid=plan-work][data-place=${k}] .ix-plan-state`); return el && getComputedStyle(el, "::before").backgroundColor; }));
  ok(dots[0] && dots[1] && dots[0] !== dots[1], `${tag} a started item's dot is not a not-started one's (${dots})`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/plan-panel-${name}.png` });
  await page.setViewportSize({ width: 720, height: 800 });
  ok((await reachable(page, "[data-testid=plan-review], [data-testid=plan-link-place], [data-testid=plan-bring], [data-testid=plan-show-place]")).every(Boolean), `${tag} every button in the panel is reachable at 720`);
  await page.setViewportSize({ width: 1200, height: 800 });
}

async function main(page, tag, name) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", beta_ideas: true });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1`);
  await page.waitForSelector("[data-testid=nav-view-workspaces]");
  if (await page.locator("[data-testid=ws-group]").count() === 0) await page.click("[data-testid=nav-view-workspaces]");
  await page.waitForSelector("[data-testid=ws-group]");
  const id = (await call(page, "plan_make", { ideas: [JELLY], draft: BODY })).id;
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).click();
  await page.click("[data-testid=ws-ov-link-plan]");
  await page.click(`[data-testid=ws-ov-plan-pick-row][data-doc='${id}']`);
  const wsId = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.plan === id)?.id;
  await page.waitForSelector("[data-testid=ws-ov-work]");
  const doc = () => page.evaluate((i) => window.__mock.ideaFile(i).content, id);
  const work = () => rows(page, "[data-testid=ws-ov-work]");
  ok((await reachable(page, "[data-testid=ws-ov-review], [data-testid=ws-ov-link-place], [data-testid=ws-ov-bring]")).every(Boolean), `${tag} overview: every button is reachable`);

  // ── Create…: the dialog, filled in and waiting ──
  await page.click("[data-testid=ws-ov-bring][data-name=cfg]");
  await page.waitForSelector("[data-testid=new-project-dialog]");
  ok(await page.inputValue("[data-testid=np-name]") === "cfg" && await page.inputValue("[data-testid=np-location]") === "~/Documents", `${tag} Create… opens New project with the plan's path`);
  const mac = await page.evaluate(() => /Mac/.test(navigator.platform));
  ok(!mac || (await text(page, "[data-testid=dialog-privacy]") ?? "").includes("Documents"), `${tag} …and warns about Documents`);
  await page.fill("[data-testid=np-location]", "/Users/demo/workspace");
  ok(await page.locator("[data-testid=dialog-privacy]").count() === 0, `${tag} …not elsewhere`);
  await page.click("[data-testid=np-create]");
  ok(await until(page, () => [...document.querySelectorAll("[data-testid=ws-ov-project]")].some((r) => r.textContent.includes("cfg") && r.dataset.state === "registered")), `${tag} once created, the plan says registered`);
  ok((await doc()).includes("- cfg (registered)\n"), `${tag} …in its own text`);
  const roots = await call(page, "workspaces_list", {}).then((l) => l.workspaces.find((w) => w.id === wsId).projects.map((p) => p.root));
  ok(roots.includes("/Users/demo/workspace/cfg"), `${tag} …and its workspace references it`);

  // ── Clone…, asked by the Ideas window ──
  await page.evaluate((i) => { localStorage.setItem("wt.main.ask", JSON.stringify({ act: "project", plan: i, name: "ans", at: Date.now() })); window.dispatchEvent(new Event("focus")); }, id);
  await page.waitForSelector("[data-testid=clone-dialog]");
  ok(await page.inputValue("[data-testid=clone-url]") === "https://github.com/demo/ans.git" && await page.locator("[data-testid=clone-prog]").count() === 0, `${tag} an ask opens Clone with the plan's URL, waiting`);
  ok(await page.evaluate(() => localStorage.getItem("wt.main.ask")) === null, `${tag} …and is taken once`);
  await page.click("[data-testid=clone-go]");
  ok(await until(page, () => [...document.querySelectorAll("[data-testid=ws-ov-project]")].some((r) => r.textContent.includes("ans") && r.dataset.state === "registered"), null, 15000), `${tag} once cloned, the plan says registered, and the overview is still shown`);

  // ── the review is read-only; Start ──
  await page.locator("[data-testid=ws-ov-work]").nth(0).locator("[data-testid=ws-ov-review]").click();
  await page.waitForSelector("[data-testid=start-review]");
  ok(await page.evaluate(() => document.querySelector("[data-testid=start-brief]").readOnly) && await page.locator("[data-testid=start-edit]").count() === 1, `${tag} overview: the brief is read-only, Edit in Ideas beside it`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/overview-review-${name}.png` });
  await page.click("[data-testid=start-go]");
  ok(await until(page, () => document.querySelector("[data-testid=start-done]")), `${tag} overview: Start starts it`);
  await page.click("[data-testid=start-close]");
  ok((await page.evaluate(() => window.__mock.placeBriefs()))[`${ROOT}|feat-p5-role`] === "Goal: the role.\n\nStop when it is green.", `${tag} …with the plan's brief`);
  await until(page, () => document.querySelector("[data-testid=ws-ov-work]")?.dataset.place === "there");

  // ── a nav place onto an item ──
  const place = await page.evaluate((r) => { const li = [...document.querySelectorAll(".ws-group li.row[data-slug], [data-testid=ws-group] li.row[data-slug]")].find((x) => x.dataset.slug !== "(main)" && x.dataset.slug !== "feat-p5-role" && x.closest("[data-repo]")?.dataset.repo === r && x.offsetParent); return li?.dataset.slug ?? null; }, ROOT);
  ok(!!place, `${tag} a nav place to drag (${place})`);
  const src = `[data-testid=ws-group] [data-repo='${ROOT}'] li.row[data-slug='${place}']`;
  const item = async (n) => `[data-work-line='${await page.locator("[data-testid=ws-ov-work]").nth(n).getAttribute("data-work-line")}']`;
  ok(await drag(page, src, await item(0), () => document.querySelectorAll(".ws-ov-work.on").length) === 0, `${tag} a nav place over a started item lights nothing`);
  ok(await drag(page, src, await item(1), () => document.querySelectorAll(".ws-ov-work.on").length) === 1, `${tag} over a free item it lights that item`);
  await until(page, () => document.querySelectorAll("[data-testid=ws-ov-work]")[1]?.dataset.place !== "none");
  ok((await doc()).includes(`started: ${ROOT} · ${place} · `), `${tag} …and is recorded as its place`);
  ok(await page.locator("[data-testid=ws-overview]").count() === 1, `${tag} the drop did not enter the place`);
  ok(await page.evaluate(() => getSelection().type !== "Range"), `${tag} …and left no text selected`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/overview-${name}.png` });
  const w = await contrast(page, ["[data-testid=ws-ov-work] .ws-ov-state", "[data-testid=ws-ov-work] .ws-ov-name"]);
  ok(w[0] >= 4.5, `${tag} the work list reads ≥ 4.5:1 in every theme (worst ${w[0].toFixed(2)} ${w[1]})`);
  await page.locator("[data-testid=ws-ov-work]").nth(0).locator("[data-testid=ws-ov-show-place]").click();
  ok(await until(page, () => !document.querySelector("[data-testid=ws-overview]")), `${tag} Show ▸ selects the place`);
}

/** The list of places is slow (a git fan-out) and the plan's line is not: the
 *  item just started must never read "place removed" in between. */
async function slow(page, tag) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", beta_ideas: true });
  for (const [where, sel, go] of [
    ["overview", "ws-ov", async () => {
      await page.goto(`${BASE}/?ideas=seed&workspaces=1&slowlist=900`);
      await page.waitForSelector("[data-testid=ws-name]");
      const id = (await call(page, "plan_make", { ideas: [JELLY], draft: BODY })).id;
      await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).click();
      await page.click("[data-testid=ws-ov-link-plan]");
      await page.click(`[data-testid=ws-ov-plan-pick-row][data-doc='${id}']`);
    }],
    ["Ideas", "plan", async () => {
      await page.goto(`${BASE}/companion/index.html?ideas=seed&slowlist=900`);
      await page.waitForSelector("[data-testid=ideas-rows] li");
      const id = (await call(page, "plan_make", { ideas: [JELLY], draft: BODY })).id;
      await page.click(`[data-idea='${id}'] button.ix-row-main`);
    }],
  ]) {
    await go();
    await page.waitForSelector(`[data-testid=${sel}-work]`);
    await page.waitForTimeout(1500);
    await page.evaluate((s) => {
      window.__kinds = [];
      new MutationObserver(() => { const k = document.querySelector(`[data-testid=${s}-work]`)?.dataset.place; if (k && window.__kinds.at(-1) !== k) window.__kinds.push(k); })
        .observe(document.body, { subtree: true, childList: true, attributes: true });
    }, sel);
    await page.locator(`[data-testid=${sel}-work]`).nth(0).locator(`[data-testid=${sel}-review]`).click();
    await page.click("[data-testid=start-go]");
    await until(page, () => document.querySelector("[data-testid=start-done]"), null, 15000);
    const there = await until(page, (s) => document.querySelector(`[data-testid=${s}-work]`)?.dataset.place === "there", sel, 15000);
    const kinds = await page.evaluate(() => window.__kinds);
    ok(there && !kinds.includes("gone"), `${tag} ${where}: a place just started is never called removed while the list is slow (${kinds})`);
    await page.evaluate(([r, s]) => window.__mock.placeRemoveElsewhere(r, s), [ROOT, "feat-p5-role"]);
    ok(await until(page, (s) => document.querySelector(`[data-testid=${s}-work]`)?.dataset.place === "gone", sel, 15000), `${tag} ${where}: …and one that is gone still says so`);
  }
}

for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [companion, main, slow]) {
    const page = await (await browser.newContext({ viewport: { width: 1200, height: 800 } })).newPage();
    const errors = [];
    // xterm throws this on ANY place selected in the dev harness (StrictMode's
    // throwaway mount); a plain nav click on main does too.
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag, name); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.close();
  }
  await browser.close();
}
console.log(failed ? `\nplans-browser: ${failed} failure(s)` : "\nplans-browser: all ok");
process.exit(failed ? 1 : 0);
