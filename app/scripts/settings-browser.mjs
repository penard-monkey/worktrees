// Optional browser gate for the Settings regroup and its modes
// (docs/proposals/settings-reorg.md), in Chromium AND WebKit (the app is
// WKWebView; the harness is Chrome — AGENTS.md).
//
//   VITE_MOCK=1 vite --port 1458 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=/tmp/pw/node_modules/playwright/index.mjs \
//     SETTINGS_MOCK_URL=http://localhost:1458 node app/scripts/settings-browser.mjs
// SETTINGS_SCREENSHOTS=<dir> saves a page per mode and engine.
//
// What it holds, and why each needs a browser (settingsmap-check.mjs holds
// the same rules on the map itself; this is what the running sheet DRAWS):
//   - a fresh window opens in Beginner, lists six groups, hides the Advanced
//     pages, and says how many settings Advanced would add — and the switch
//     persists the mode in the blob;
//   - a setting changed in Expert stays drawn in Beginner, with its tier and
//     why (rule 2);
//   - an offer's deep link into a section the mode hides reveals it, with the
//     switch beside it (§4.3);
//   - the confirms: Full access asks first and its arm expires; Reset arms,
//     expires, and resets only its scope;
//   - an available agent CLI update draws Agent CLIs in any mode (rule 3);
//   - a remote window: computer pages name the host, a console-only control is
//     drawn disabled with why, an admin-only read is a GrantNote, Restore
//     window is not drawn, and the settings file is this machine's;
//   - every button on every page is REACHABLE (elementFromPoint at its
//     centre), and the new words — group headers, scope sentences, chips, the
//     footer — read at ≥ 4.5:1 in all six themes (composited backgrounds);
//   - a group heading stands apart from its pages in all six themes (brighter
//     and heavier, a rule the rail does not swallow), and at the narrowest
//     window, in every mode, here and remote: its pages sit indented under it,
//     nothing is clipped, and the mode switch sits right, by esc and close.
//   - the header's search: small with its glass, wider on focus without moving
//     the switch, esc or close; a list as you type (a combobox: arrows, Enter,
//     a click), a hit the mode hides marked with its tier and REVEALED when
//     opened, the mode untouched; Escape closes the list, then clears the
//     box, then the sheet; a press outside closes the list; ⌘F focuses it; a
//     remote window never lists what it cannot draw; at the narrowest window
//     the list sits inside the modal and every row is reachable; in all six
//     themes the field is lighter than the header and its words read.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
import fs from "node:fs";
const BASE = process.env.SETTINGS_MOCK_URL ?? "http://localhost:1458";
const SHOTS = process.env.SETTINGS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

async function openSheet(page, query = "") {
  await page.goto(`${BASE}/${query}`);
  await page.waitForSelector("[data-track=settings]");
  await page.click("[data-track=settings]");
  await page.waitForSelector(".settings-body .page-title");
}
const mode = (page) => page.evaluate(() => document.querySelector("[data-testid=settings-mode] button.on")?.dataset.mode ?? null);
const setMode = async (page, m) => { await page.click(`[data-testid=settings-mode] [data-mode=${m}]`); await page.waitForTimeout(150); };
const go = async (page, cat) => { await page.click(`.settings-cat[data-cat=${cat}]`); await page.waitForSelector(`.settings-body[data-page=${cat}]`); await page.waitForTimeout(250); };
const listed = (page) => page.evaluate(() => [...document.querySelectorAll(".settings-cat")].map((b) => b.dataset.cat));
const drawn = (page) => page.evaluate(() => [...document.querySelectorAll(".settings-body [data-sec]")].map((s) => s.dataset.sec));
const blob = (page, key = "wt-mock-ui-state") => page.evaluate((k) => JSON.parse(sessionStorage.getItem(k) ?? "null"), key);

/** Every button in the body: reachable at its centre. */
const unreachable = (page) => page.evaluate(() => [...document.querySelectorAll(".settings-body button")].filter((b) => {
  if (b.closest("details:not([open])")) return false;
  b.scrollIntoView({ block: "center" });
  const q = b.getBoundingClientRect();
  if (q.width === 0 || q.height === 0) return false;
  const hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2);
  return !(hit && (b === hit || b.contains(hit)));
}).map((b) => b.textContent.trim().slice(0, 40)));

/** Contrast of each matched element's text against its composited background. */
const contrast = (page, sel) => page.evaluate((sel) => {
  const parse = (c) => {
    let m = /^rgba?\(([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; }
    m = /^color\(srgb ([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; }
    return [0, 0, 0, 0];
  };
  const over = (top, under) => { const a = top[3]; return [0, 1, 2].map((i) => top[i] * a + under[i] * (1 - a)).concat(1); };
  const bgOf = (el) => {
    const stack = [];
    for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) stack.push(c); if (c[3] >= 1) break; }
    let acc = [255, 255, 255, 1];
    for (const c of stack.reverse()) acc = over(c, acc);
    return acc;
  };
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
  return [...document.querySelectorAll(sel)].filter((e) => e.textContent.trim()).map((e) => {
    const bg = bgOf(e);
    const fg = over(parse(getComputedStyle(e).color), bg);
    const [a, b] = [lum(fg), lum(bg)].sort((x, y) => y - x);
    return { text: e.textContent.trim().slice(0, 30), ratio: (a + 0.05) / (b + 0.05) };
  });
}, sel);

/** The rail and the header as drawn (the owner's review of the first build:
 *  the group headings read as one more page). What sets a heading apart from
 *  its pages — colour against them, weight, the pages indented under it, a
 *  rule and room above every later group — and where the mode switch sits. */
const railLook = (page) => page.evaluate(() => {
  const parse = (c) => {
    let m = /^rgba?\(([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; }
    m = /^color\(srgb ([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; }
    return [0, 0, 0, 0];
  };
  const over = (top, under) => { const a = top[3]; return [0, 1, 2].map((i) => top[i] * a + under[i] * (1 - a)).concat(1); };
  const bgOf = (el) => {
    const stack = [];
    for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) stack.push(c); if (c[3] >= 1) break; }
    let acc = [255, 255, 255, 1];
    for (const c of stack.reverse()) acc = over(c, acc);
    return acc;
  };
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
  const ratio = (x, y) => { const [a, b] = [lum(x), lum(y)].sort((p, q) => q - p); return (a + 0.05) / (b + 0.05); };
  const textX = (el) => { const r = document.createRange(); r.selectNodeContents(el.firstChild); return r.getBoundingClientRect().left; };
  const rail = document.querySelector(".settings-cats");
  const surface = bgOf(rail);
  const heads = [...document.querySelectorAll(".settings-group-h")];
  const items = [...document.querySelectorAll(".settings-cat:not(.on)")];
  const on = document.querySelector(".settings-cat.on");
  const groups = [...document.querySelectorAll(".settings-group")];
  const hc = over(parse(getComputedStyle(heads[0]).color), surface);
  const ic = over(parse(getComputedStyle(items[0]).color), bgOf(items[0]));
  const rules = groups.slice(1).map((g) => {
    const cs = getComputedStyle(g);
    return parseFloat(cs.borderTopWidth) >= 1 ? ratio(over(parse(cs.borderTopColor), surface), surface) : 0;
  });
  const mid = (r) => r.top + r.height / 2;
  const box = (sel) => document.querySelector(sel).getBoundingClientRect();
  const [title, search, seg, keys] = [box(".settings-h b"), box(".settings-search-input"), box(".settings-mode"), box(".settings-h .modal-keys")];
  return {
    headingVsItem: ratio(hc, ic),
    heavier: +getComputedStyle(heads[0]).fontWeight > +getComputedStyle(items[0]).fontWeight,
    minRule: Math.min(...rules),
    indent: Math.min(...items.map(textX)) - Math.max(...heads.map(textX)),
    pillShift: on ? on.getBoundingClientRect().left - Math.max(...heads.map(textX)) : 0,
    minRoom: Math.min(...groups.slice(1).map((g, i) => g.querySelector(".settings-group-h").getBoundingClientRect().top - groups[i].getBoundingClientRect().bottom)),
    // A page row does not wrap (nowrap), so it overflows; a heading is a
    // block, so it WRAPS instead — "Remote access" on two lines is the miss.
    clipped: [...rail.querySelectorAll(".settings-cat, .settings-group-h")].filter((e) => {
      if (e.scrollWidth > e.clientWidth + 0.5) return true;
      const r = document.createRange(); r.selectNodeContents(e.firstChild);
      return new Set([...r.getClientRects()].map((q) => Math.round(q.top))).size > 1;
    }).map((e) => e.textContent),
    titleGap: search.left - title.right,
    searchToSeg: seg.left - search.right,
    segToKeys: keys.left - seg.right,
    centre: Math.max(Math.abs(mid(seg) - mid(title)), Math.abs(mid(search) - mid(title))),
  };
});

const SEARCH = "[data-testid=settings-search-input]";
/** The header's search as drawn: the box and its neighbours, the list, its
 *  rows (each hit-tested at its centre: the list floats over the page inside
 *  a modal whose overflow is hidden), and the combobox's ARIA. */
const searchLook = (page) => page.evaluate(() => {
  const R = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); return { left: b.left, right: b.right, top: b.top, bottom: b.bottom, width: b.width, height: b.height }; };
  const q = (s) => document.querySelector(s);
  const input = q("[data-testid=settings-search-input]");
  const rows = [...document.querySelectorAll("[data-testid=settings-search-row]")];
  const pop = R(q(".settings-search-pop")), modal = R(q(".settings-modal"));
  return {
    box: R(input), icon: R(q(".settings-search-icon")), title: R(q(".settings-h b")), seg: R(q(".settings-mode")),
    keys: R(q(".settings-h .modal-keys")), close: R(q(".settings-h .icon-btn")), pop, modal,
    inside: pop && modal ? pop.left >= modal.left && pop.right <= modal.right && pop.top >= modal.top && pop.bottom <= modal.bottom - 1 : null,
    rows: rows.map((r) => ({
      id: r.dataset.id, optId: r.id, selected: r.getAttribute("aria-selected"),
      tier: r.querySelector("[data-testid=settings-search-tier]")?.textContent ?? null,
      label: r.querySelector(".settings-search-label")?.textContent, where: r.querySelector(".settings-search-where")?.textContent,
    })),
    unhit: rows.filter((r) => {
      const b = r.getBoundingClientRect();
      const hit = document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2);
      return !(hit && r.contains(hit));
    }).map((r) => r.dataset.id),
    none: !!q("[data-testid=settings-search-none]"),
    role: input?.getAttribute("role"), expanded: input?.getAttribute("aria-expanded"),
    controls: input?.getAttribute("aria-controls") ?? null, active: input?.getAttribute("aria-activedescendant") ?? null,
    listbox: q(".settings-search-pop [role=listbox]")?.id ?? null,
    focused: document.activeElement === input, value: input?.value, sheet: !!q(".settings-modal"),
    selection: input ? [input.selectionStart, input.selectionEnd] : null,
  };
});
/** Type into the box from empty, as a person would (it is focused first). */
const searchFor = async (page, text) => {
  await page.click(SEARCH);
  await page.fill(SEARCH, "");
  await page.keyboard.type(text);
  await page.waitForTimeout(150);
};
const near = (a, b) => Math.abs(a - b) <= 1;
/** The field against the header, and the words in the list, composited. */
const searchColours = (page) => page.evaluate(() => {
  const parse = (c) => {
    let m = /^rgba?\(([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; }
    m = /^color\(srgb ([^)]+)\)/.exec(c);
    if (m) { const p = m[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; }
    return [0, 0, 0, 0];
  };
  const over = (t, u) => { const a = t[3]; return [0, 1, 2].map((i) => t[i] * a + u[i] * (1 - a)).concat(1); };
  const bgOf = (el) => { const st = []; for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) st.push(c); if (c[3] >= 1) break; } let acc = [255, 255, 255, 1]; for (const c of st.reverse()) acc = over(c, acc); return acc; };
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
  const ratio = (x, y) => { const [a, b] = [lum(x), lum(y)].sort((p, q) => q - p); return (a + 0.05) / (b + 0.05); };
  const text = (el) => ratio(over(parse(getComputedStyle(el).color), bgOf(el)), bgOf(el));
  const input = document.querySelector(".settings-search-input");
  const head = bgOf(document.querySelector(".settings-h"));
  const fill = bgOf(input);
  // The floor the placeholder is held to: --txt-dim on --bg-elev, the
  // lowest the app already puts it on (a hovered row), in this theme.
  const root = getComputedStyle(document.documentElement);
  const tok = (n) => { const d = document.createElement("i"); d.style.color = root.getPropertyValue(n); document.body.append(d); const c = parse(getComputedStyle(d).color); d.remove(); return c; };
  const hiRow = document.querySelector(".settings-search-row.hi");
  const pop = document.querySelector(".settings-search-pop");
  return {
    fillDelta: Math.max(...[0, 1, 2].map((i) => Math.abs(fill[i] - head[i]))), lighter: lum(fill) > lum(head),
    placeholder: ratio(over(parse(getComputedStyle(input, "::placeholder").color), fill), fill),
    icon: ratio(over(parse(getComputedStyle(document.querySelector(".settings-search-icon")).color), fill), fill),
    border: ratio(over(parse(getComputedStyle(input).borderTopColor), head), head),
    floor: ratio(tok("--txt-dim"), tok("--bg-elev")),
    words: [...document.querySelectorAll(".settings-search-label, .settings-search-where, .settings-search-tier, .settings-search-none")]
      .map((e) => ({ text: e.textContent.trim().slice(0, 24), ratio: text(e) })),
    // the highlighted row: its accent bar against the row, as a non-text mark
    bar: hiRow ? ratio(tok("--accent"), bgOf(hiRow)) : null,
    popVsHead: pop ? ratio(bgOf(pop), head) : null,
  };
});

async function run(engine, name) {
  const browser = await engine.launch();
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `[${name}] page error: ${e.message}`));

  // ── a fresh window: Beginner ─────────────────────────────────────────────
  await openSheet(page);
  ok(await mode(page) === "beginner", `[${name}] a fresh window opens Settings in Beginner`);
  const groups = await page.evaluate(() => [...document.querySelectorAll(".settings-group-h")].map((h) => h.textContent));
  ok(groups.join("|") === "Workspace|Agents|Agent policy|Projects|Remote access|System", `[${name}] six groups: ${groups.join(", ")}`);
  const beg = await listed(page);
  ok(!beg.includes("reference") && !beg.includes("guidance"), `[${name}] Beginner lists no Advanced page that holds nothing changed (${beg.join(", ")})`);
  // A page above the mode is listed only for a section that is drawn anyway,
  // and such a section says why (rule 2: the mock has a profile).
  if (beg.includes("profiles")) {
    await go(page, "profiles");
    ok(await page.locator("[data-testid=why-profiles]").count() === 1, `[${name}] Profiles is listed in Beginner only because it holds a profile, and says so`);
    await go(page, "general");
  }
  ok(beg.includes("policy") && beg.includes("general") && beg.includes("shortcuts"), `[${name}] Beginner lists General, Agent policy, Shortcuts`);
  const gen = await drawn(page);
  ok(gen.includes("restore-last") && gen.includes("editor") && gen.includes("pull-requests") && !gen.includes("terminal-command") && !gen.includes("auto-fetch"),
    `[${name}] General in Beginner: ${gen.join(", ")}`);
  const more = await page.locator("[data-testid=page-more]").textContent().catch(() => null);
  ok(/^\d+ more settings? in Advanced ›$/.test(more ?? ""), `[${name}] General says what Advanced adds: "${more}"`);
  ok(await page.locator("[data-testid=page-scope]").textContent() === "These settings are this window's.", `[${name}] General says whose its settings are`);
  if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-beginner-general.png` });

  // the footer switches the mode in place, and the blob keeps it
  await page.click("[data-testid=page-more]");
  await page.waitForTimeout(300);
  ok(await mode(page) === "advanced", `[${name}] "N more settings in Advanced ›" switches to Advanced`);
  await page.waitForTimeout(400);
  ok((await blob(page))?.settings_mode === "advanced", `[${name}] the mode is saved in the window's blob`);
  ok((await listed(page)).includes("reference"), `[${name}] Advanced lists Reference panels`);
  ok((await drawn(page)).includes("terminal-command"), `[${name}] Advanced draws Terminal command`);

  // ── rule 2: changed in Expert, still drawn in Beginner ───────────────────
  await setMode(page, "expert");
  await go(page, "appearance");
  await page.locator(".sec[data-sec=density] .seg button", { hasText: "compact" }).click();
  await page.waitForTimeout(200);
  await setMode(page, "beginner");
  const why = await page.locator("[data-testid=why-density]").textContent().catch(() => null);
  ok(!!why && why.includes("Advanced setting") && why.includes("not at its default"), `[${name}] a changed Advanced setting stays drawn in Beginner: "${why}"`);
  ok(!(await drawn(page)).includes("ui-font"), `[${name}] …while an unchanged one stays hidden`);
  if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-beginner-changed.png` });

  // ── confirms: Full access asks, and its arm expires ──────────────────────
  await go(page, "policy");
  const pol = await drawn(page);
  ok(pol.includes("codex-permissions") && pol.includes("main-guard") && pol.includes("plan-limits"), `[${name}] Agent policy in Beginner: ${pol.join(", ")}`);
  // Every section drawn ABOVE the mode says why (rules 2 and 3).
  const unexplained = await page.evaluate(() => [...document.querySelectorAll(".settings-body .sec[data-tier]")]
    .filter((s) => s.dataset.tier !== "beginner" && !s.querySelector(".sec-why")).map((s) => s.dataset.sec));
  ok(unexplained.length === 0, `[${name}] every Advanced/Expert section drawn in Beginner says why${unexplained.length ? ` — not: ${unexplained.join(", ")}` : ""}`);
  const onMode = () => page.evaluate(() => document.querySelector("[data-testid=codex-permissions] button.on")?.dataset.testid);
  const before = await onMode();
  await page.click("[data-testid=codex-permissions-full]");
  await page.waitForTimeout(150);
  ok(await page.locator("[data-testid=codex-full-confirm]").count() === 1 && await onMode() === before, `[${name}] Full access asks first, and changes nothing yet`);
  await page.waitForTimeout(4300);
  ok(await page.locator("[data-testid=codex-full-confirm]").count() === 0, `[${name}] …and the arm expires after 4 s`);
  await page.evaluate(() => { const b = document.querySelector("[data-testid=codex-permissions-full]"); b.click(); });
  await page.waitForTimeout(100);
  await page.evaluate(() => { document.querySelector("[data-testid=codex-permissions-full]").click(); });
  await page.waitForTimeout(400);
  ok(await onMode() === "codex-permissions-full", `[${name}] two clicks set Full access`);
  await page.click("[data-testid=codex-permissions-auto-review]");
  await page.waitForTimeout(300);
  ok(await onMode() === "codex-permissions-auto-review", `[${name}] going back to Auto-review needs no confirm`);
  if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-beginner-policy.png` });

  // ── reset: the default scope takes the look, not a silenced suggestion ───
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  // silence one offer, from the offers list
  await page.click("[data-track=offers]");
  await page.waitForSelector(".wn-offer");
  await page.locator(".wn-offer", { hasText: "Let Worktrees keep your agents' plans" }).locator(".mcp-dismiss").click();
  await page.waitForTimeout(600);
  const silenced = Object.keys((await blob(page))?.offers_dismissed ?? {});
  ok(silenced.includes("planning"), `[${name}] an offer silenced (${silenced.join(", ")})`);

  // ── a deep link into a section the mode hides ────────────────────────────
  await page.locator(".wn-offer", { hasText: "Let agents see your other projects" }).locator(".enter-btn").click();
  await page.waitForSelector(".settings-body[data-page=policy]");
  await page.waitForTimeout(400);
  ok(await mode(page) === "beginner", `[${name}] the link does not change the mode`);
  ok((await drawn(page)).includes("cross-project"), `[${name}] an offer's link into an Advanced section reveals it in Beginner`);
  ok(await page.locator("[data-testid=reveal-switch-cross-project]").count() === 1, `[${name}] …with the switch to Advanced beside it`);
  ok(await page.evaluate(() => document.querySelector(".settings-cat.on")?.dataset.cat) === "policy", `[${name}] …on the page it moved to (Agent policy)`);
  if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-beginner-revealed.png` });
  await page.click("[data-testid=reveal-switch-cross-project]");
  await page.waitForTimeout(200);
  ok(await mode(page) === "advanced", `[${name}] the reveal's switch moves to Advanced`);

  await go(page, "data");
  await page.click("[data-testid=reset-go]");
  ok((await page.locator("[data-testid=reset-go]").textContent()) === "Confirm reset?", `[${name}] Reset arms first`);
  await page.waitForTimeout(4300);
  ok((await page.locator("[data-testid=reset-go]").textContent()) !== "Confirm reset?", `[${name}] …and its arm expires`);
  await page.evaluate(() => { document.querySelector("[data-testid=reset-go]").click(); });
  await page.waitForTimeout(80);
  await page.evaluate(() => { document.querySelector("[data-testid=reset-go]").click(); });
  await page.waitForTimeout(500);
  const after = await blob(page);
  ok(after?.density === "comfortable", `[${name}] Reset (Appearance & layout) put density back`);
  ok(Object.keys(after?.offers_dismissed ?? {}).includes("planning"), `[${name}] …and kept the silenced suggestion`);
  ok(after?.settings_mode === "advanced", `[${name}] …and kept the mode`);

  // ── every page in Expert: buttons reachable, nothing throws ──────────────
  await setMode(page, "expert");
  for (const cat of await listed(page)) {
    await go(page, cat);
    const lost = await unreachable(page);
    ok(lost.length === 0, `[${name}] ${cat}: every button is reachable${lost.length ? ` — not: ${lost.join(" | ")}` : ""}`);
  }

  // ── contrast of the new words, in every theme ────────────────────────────
  await go(page, "general");
  for (const theme of THEMES) {
    await page.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
    await page.waitForTimeout(60);
    const rows = [
      ...await contrast(page, ".settings-group-h"), ...await contrast(page, ".page-scope"),
      ...await contrast(page, ".sec-chip"), ...await contrast(page, ".page-title"),
    ];
    const low = rows.filter((r) => r.ratio < 4.5);
    ok(low.length === 0, `[${name}] ${theme}: group headers, scope, chips ≥ 4.5:1${low.length ? ` — ${low.map((r) => `"${r.text}" ${r.ratio.toFixed(2)}`).join(", ")}` : ` (min ${Math.min(...rows.map((r) => r.ratio)).toFixed(2)})`}`);
    // A heading reads as a heading, not one more page: brighter than its pages
    // and heavier, and a rule above each later group that the surface does
    // not swallow (`--line` measured 1.13:1 on tokyo-day's rail).
    const r = await railLook(page);
    ok(r.headingVsItem >= 1.5 && r.heavier, `[${name}] ${theme}: a group heading stands apart from its pages (${r.headingVsItem.toFixed(2)}:1 against them, heavier: ${r.heavier})`);
    ok(r.minRule >= 1.4, `[${name}] ${theme}: the rule between groups shows on the rail (${r.minRule.toFixed(2)}:1)`);
  }
  await ctx.close();

  // ── presets (§5): status, apply, the marker, back to preset, Compare ───────
  const pctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const pp = await pctx.newPage();
  pp.on("pageerror", (e) => ok(false, `[${name}] preset page error: ${e.message}`));
  await openSheet(pp);
  await go(pp, "policy");
  await pp.waitForSelector("[data-testid=preset-seg] button.on");
  const onPreset = () => pp.evaluate(() => document.querySelector("[data-testid=preset-seg] button.on")?.dataset.preset);
  const codexOn = () => pp.evaluate(() => document.querySelector("[data-testid=codex-permissions] button.on")?.dataset.testid);
  ok(await onPreset() === "balanced", `[${name}] a fresh host reads as Balanced (today's defaults)`);
  ok(await pp.locator("[data-testid=preset-modified]").count() === 0, `[${name}] …unmodified`);
  await pp.click("[data-testid=preset-compare]");
  const cols = await pp.evaluate(() => [...document.querySelectorAll("[data-testid=preset-table] thead th")].map((t) => [t.textContent, t.className]));
  ok(cols.map((c) => c[0]).join("|") === "|Cautious|Balanced|Autonomous" && cols[2][1] === "cur", `[${name}] Compare shows all three, Balanced highlighted`);
  await pp.click("[data-preset=cautious]");
  await pp.waitForTimeout(500);
  ok(await onPreset() === "cautious", `[${name}] Cautious applies in one click`);
  ok(await codexOn() === "codex-permissions-ask", `[${name}] …and the section below re-reads what it wrote (Codex: Ask)`);
  await pp.click("[data-testid=codex-permissions-auto-review]");
  await pp.waitForTimeout(500);
  await go(pp, "general"); await go(pp, "policy");
  await pp.waitForSelector("[data-testid=preset-modified]");
  const marker = await pp.locator("[data-testid=preset-modified]").textContent();
  ok(marker.includes("Cautious (modified:") && marker.includes("Codex permissions"), `[${name}] a changed value marks the preset modified: "${marker.slice(0, 60)}"`);
  ok(await pp.locator("[data-testid=preset-back-codex_access]").count() === 1, `[${name}] …and its control offers the way back`);
  await pp.click("[data-testid=preset-back-codex_access] button");
  await pp.waitForTimeout(500);
  ok(await codexOn() === "codex-permissions-ask" && await pp.locator("[data-testid=preset-modified]").count() === 0, `[${name}] back to preset restores that one value and clears the marker`);
  await pp.click("[data-preset=autonomous]");
  await pp.waitForTimeout(150);
  ok(await pp.locator("[data-testid=preset-full-confirm]").count() === 1 && await onPreset() === "cautious", `[${name}] Autonomous asks first, because it sets Full access`);
  await pp.click("[data-preset=autonomous]");
  await pp.waitForTimeout(500);
  ok(await onPreset() === "autonomous" && await codexOn() === "codex-permissions-full", `[${name}] …and a second click applies it`);
  if (SHOTS) await pp.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-preset.png` });

  // ── Limits (§6 item 1) ───────────────────────────────────────────────────
  await setMode(pp, "advanced");
  await pp.locator("[data-testid=limit-cap-all]").scrollIntoViewIfNeeded();
  const capBefore = await pp.locator("[data-testid=limit-cap-all]").textContent();
  await pp.click("[data-testid=limit-inc-all]");
  await pp.waitForTimeout(300);
  ok(Number(await pp.locator("[data-testid=limit-cap-all]").textContent()) === Number(capBefore) + 1, `[${name}] Limits: + raises the live-agent cap (${capBefore} → ${await pp.locator("[data-testid=limit-cap-all]").textContent()})`);
  ok(await pp.locator("[data-testid=limit-reset-all]").count() === 1, `[${name}] …and offers the way back to the machine default`);
  await pp.click("[data-testid=limit-reset-all]");
  await pp.waitForTimeout(300);
  ok(await pp.locator("[data-testid=limit-reset-all]").count() === 0, `[${name}] …which removes the key`);
  await pctx.close();

  // ── Notifications (§6 item 9): drawn in Beginner; permission only on a click
  {
    const nctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
    const np = await nctx.newPage();
    await openSheet(np);
    await go(np, "agents");
    const on = await np.evaluate(() => document.querySelector("[data-testid=notify-mode] button.on")?.dataset.mode);
    ok(on === "waiting", `[${name}] Notifications is a Beginner section, on "when an agent waits" by default (${on})`);
    ok(await np.locator("[data-testid=notify-allow]").count() === 1, `[${name}] …and offers to ask the system, rather than asking on load`);
    await np.click("[data-testid=notify-allow]");
    await np.waitForTimeout(200);
    ok(await np.locator("[data-testid=notify-allow]").count() === 0, `[${name}] …and the button goes once allowed`);
    ok((await np.evaluate(() => window.__mock.notifications().length)) === 0, `[${name}] nothing was sent while the window is visible`);
    await nctx.close();
  }

  // ── "set by the environment" (§6 item 4) ─────────────────────────────────
  const ectx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const ep = await ectx.newPage();
  await openSheet(ep, "?env=codex,pi,guidance");
  await setMode(ep, "expert");
  await go(ep, "policy");
  await ep.waitForSelector(".sec[data-sec=codex-permissions] [data-testid=env-origin]");
  const codexEnv = await ep.locator(".sec[data-sec=codex-permissions] [data-testid=env-origin]").textContent();
  ok(codexEnv.includes("WORKTREES_CODEX_PERMISSIONS=full") && codexEnv.includes("wins over it from then on"), `[${name}] Codex: the environment is named, and a choice here is said to win`);
  ok(await ep.evaluate(() => !document.querySelector("[data-testid=codex-permissions-ask]").disabled), `[${name}] …so the control stays usable`);
  ok(await ep.locator(".sec[data-sec=pi-trust] [data-testid=env-origin]").count() === 1, `[${name}] pi trust names its variable too`);
  await ep.click("[data-testid=codex-permissions-ask]");
  await ep.waitForTimeout(400);
  ok(await ep.evaluate(() => document.querySelector(".sec[data-sec=codex-permissions] [data-testid=env-origin]")?.dataset.source) === "host", `[${name}] after a choice, the host's own setting is said to win over the variable`);
  await go(ep, "guidance");
  const gEnv = await ep.locator(".sec[data-sec=agent-guidance] [data-testid=env-origin]").textContent();
  ok(gEnv.includes("WORKTREES_AGENT_GUIDANCE=off") && gEnv.includes("wins over this switch"), `[${name}] Guidance: the variable wins over the switch, and says so`);
  ok(await ep.evaluate(() => document.querySelector("[data-testid=guidance-enabled]").disabled), `[${name}] …and the switch it outranks is disabled`);
  await ectx.close();

  // ── an available agent CLI update is never hidden (rule 3) ───────────────
  // The update bubble and the rail's "upd" badge both say a harness has an
  // update; Agent CLIs, where it is installed, is Advanced. In Beginner the
  // bubble's own destination drew nothing about it (harness-updates-browser
  // found that, after the rebase onto #605–#612's main).
  const uctx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
  const up = await uctx.newPage();
  up.on("pageerror", (e) => ok(false, `[${name}] update page error: ${e.message}`));
  await up.goto(`${BASE}/?harnessupdates=1`);
  const bubbleShown = await up.locator("[data-testid=update-bubble]").waitFor({ timeout: 12_000 }).then(() => true, () => false);
  if (bubbleShown) await up.locator("[data-testid=update-bubble] .enter-btn").click();
  const onUpdates = await up.waitForSelector(".settings-body[data-page=updates]", { timeout: 5000 }).then(() => true, () => false);
  await up.waitForTimeout(250);
  ok(bubbleShown && onUpdates && await mode(up) === "beginner", `[${name}] the update bubble opens Settings → Updates (a fresh window: Beginner)`);
  const clis = await up.locator("[data-testid=why-agent-clis]").textContent().catch(() => null);
  ok((await drawn(up)).includes("agent-clis") && !!clis && clis.includes("needs attention"),
    `[${name}] …where Agent CLIs (Advanced) is drawn while an update is available, saying why: "${clis}"`);
  await uctx.close();

  // ── a remote window ──────────────────────────────────────────────────────
  const rctx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
  const rp = await rctx.newPage();
  rp.on("pageerror", (e) => ok(false, `[${name}] remote page error: ${e.message}`));
  await openSheet(rp, "?remote=noadmin");
  ok(await mode(rp) === "beginner", `[${name}] a remote window keeps its own mode (fresh device blob: Beginner)`);
  await setMode(rp, "expert");
  await rp.waitForTimeout(500);
  // The harness's `?remote` marks the window remote without the shim's door,
  // so its blob is the harness's one key; the real door routes it to
  // remote-ui-state.json (ipc.ts REMOTE_SETTINGS).
  ok(((await blob(rp, "wt-mock-ui-state:remote")) ?? (await blob(rp)))?.settings_mode === "expert", `[${name}] …and saves it in the window's blob`);
  ok(!(await drawn(rp)).includes("restore-window"), `[${name}] Restore window is not drawn on a remote window`);
  await go(rp, "policy");
  const scope = await rp.locator("[data-testid=page-scope]").textContent();
  ok(scope.includes("Studio"), `[${name}] a computer page names the host: "${scope}"`);
  const codex = await rp.evaluate(() => {
    const s = document.querySelector(".sec[data-sec=codex-permissions]");
    return { disabled: s?.disabled, line: !!s?.querySelector("[data-testid=grant-line]"), btn: s?.querySelector("button")?.matches(":disabled") };
  });
  ok(codex.disabled && codex.line && codex.btn, `[${name}] Codex permissions: read-only, controls disabled, and says why`);
  const presetRemote = await rp.evaluate(() => {
    const s = document.querySelector(".sec[data-sec=policy-preset]");
    return { disabled: s?.disabled, line: !!s?.querySelector("[data-testid=grant-line]"), named: !!s?.querySelector("[data-testid=preset-seg] button.on") };
  });
  ok(presetRemote.disabled && presetRemote.line && presetRemote.named, `[${name}] the preset is read on a remote window, and applied only at the host`);
  await go(rp, "profiles");
  ok(await rp.locator(".sec[data-sec=profiles] [data-testid=grant-note]").count() === 1, `[${name}] Profiles (an admin read) is a GrantNote for a device without admin`);
  await go(rp, "general");
  ok(await rp.locator(".sec[data-sec=auto-fetch] [data-testid=grant-line]").count() === 1, `[${name}] Auto-fetch (admin) says why it cannot be changed here`);
  await go(rp, "data");
  const file = await rp.locator("[data-testid=settings-file-path]").textContent();
  ok(file.includes("remote-ui-state.json"), `[${name}] the settings file is this machine's: "${file}"`);
  if (SHOTS) await rp.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-remote-data.png` });
  await rctx.close();

  // ── the header's search ──────────────────────────────────────────────────
  const sctx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
  const sp = await sctx.newPage();
  sp.on("pageerror", (e) => ok(false, `[${name}] search page error: ${e.message}`));
  await openSheet(sp);
  await sp.waitForTimeout(300); // the modal's pop animation scales every rect
  const s0 = await searchLook(sp);
  ok(s0.role === "combobox" && s0.expanded === "false" && s0.box.width < 110,
    `[${name}] search: a small combobox at rest (${s0.box.width.toFixed(0)}px), collapsed`);
  ok(s0.icon.left >= s0.box.left && s0.icon.right <= s0.box.right && s0.icon.top >= s0.box.top && s0.icon.bottom <= s0.box.bottom,
    `[${name}] search: the magnifying glass sits inside the box`);
  ok(s0.seg.left - s0.box.right >= 8 && s0.seg.left - s0.box.right <= 16, `[${name}] search: immediately left of the mode switch (${(s0.seg.left - s0.box.right).toFixed(0)}px)`);
  await sp.click(SEARCH);
  await sp.waitForTimeout(300);
  const s1 = await searchLook(sp);
  ok(s1.box.width >= s0.box.width + 80 && near(s1.box.right, s0.box.right),
    `[${name}] search: focused, it widens LEFTWARD (${s0.box.width.toFixed(0)} → ${s1.box.width.toFixed(0)}px, right edge kept)`);
  ok(near(s1.seg.left, s0.seg.left) && near(s1.keys.left, s0.keys.left) && near(s1.close.left, s0.close.left) && near(s1.title.left, s0.title.left),
    `[${name}] search: …and the title, switch, esc and close do not move`);

  await sp.keyboard.type("dark mode");
  await sp.waitForTimeout(150);
  let sl = await searchLook(sp);
  ok(sl.rows.length >= 1 && sl.rows[0].id === "theme" && sl.rows[0].where === "Workspace › Appearance" && sl.rows[0].tier === null,
    `[${name}] search: "dark mode" lists Theme, in Workspace › Appearance, no tier chip (Beginner shows it): ${sl.rows.map((r) => r.id).join(", ")}`);
  ok(!sl.rows.some((r) => r.id === "theme-pair"), `[${name}] search: …and not the light/dark pair, which this theme setting never draws`);
  ok(sl.expanded === "true" && sl.listbox && sl.controls === sl.listbox && sl.active === sl.rows[0].optId && sl.rows[0].selected === "true",
    `[${name}] search: combobox semantics — expanded, controls the listbox, the active option is announced`);
  ok(sl.inside && sl.unhit.length === 0, `[${name}] search: the list sits inside the modal and every row is reachable at its centre${sl.unhit.length ? ` — not: ${sl.unhit.join(", ")}` : ""}`);

  await searchFor(sp, "zzqqxx");
  sl = await searchLook(sp);
  ok(sl.none && sl.rows.length === 0 && sl.controls === null, `[${name}] search: nothing matches → "No settings match", and no listbox to point at`);

  // arrows wrap, Enter opens: "font" is UI font size (Advanced, hidden in
  // Beginner) then Terminal font.
  await searchFor(sp, "font");
  sl = await searchLook(sp);
  ok(sl.rows.slice(0, 2).map((r) => `${r.id}:${r.tier}`).join() === "ui-font:Advanced,term-font:null",
    `[${name}] search: "font" → both font sections, the hidden one marked Advanced (${sl.rows.map((r) => `${r.id}:${r.tier}`).join(", ")})`);
  const actives = [];
  for (const k of ["ArrowDown", "ArrowDown", "ArrowUp", "ArrowUp"]) {
    await sp.keyboard.press(k);
    await sp.waitForTimeout(60);
    const now = await searchLook(sp);
    actives.push(now.rows.findIndex((r) => r.optId === now.active && r.selected === "true"));
  }
  const n = sl.rows.length;
  ok(actives.join() === [1 % n, 2 % n, 1 % n, 0].join(), `[${name}] search: ↓↓↑↑ moves the active row and wraps (${actives.join(" → ")} of ${n})`);
  await sp.keyboard.press("ArrowDown");
  await sp.keyboard.press("Enter");
  // A wait that a broken box would leave hanging reports, never throws: the
  // rules after it still run (a mutation of the list's outside handler lost
  // the row's click, and the bare wait took the whole run down with it).
  const reached = (page, cat) => page.waitForSelector(`.settings-body[data-page=${cat}]`, { timeout: 3000 }).then(() => true, () => false);
  const toTerminal = await reached(sp, "terminal");
  await sp.waitForTimeout(120);
  sl = await searchLook(sp);
  const flashed = await sp.evaluate(() => document.querySelector(".settings-body [data-sec=term-font]")?.classList.contains("focus-flash"));
  ok(toTerminal && flashed && !sl.pop && sl.value === "font" && sl.focused, `[${name}] search: Enter opens Terminal at Terminal font and flashes it; the list closes, the words stay, focus stays in the box`);
  ok(await mode(sp) === "beginner", `[${name}] search: …in Beginner still`);

  // a hit the mode hides is REVEALED by opening it, by a click, and the mode stays
  await searchFor(sp, "cap agents");
  sl = await searchLook(sp);
  ok(sl.rows[0]?.id === "limits" && sl.rows[0].tier === "Advanced" && sl.rows[0].where === "Agent policy",
    `[${name}] search: "cap agents" → Limits first, marked Advanced, in Agent policy`);
  await sp.locator("[data-testid=settings-search-row][data-id=limits]").click({ timeout: 3000 }).catch(() => {});
  const toPolicy = await reached(sp, "policy");
  await sp.waitForTimeout(250);
  ok(toPolicy && (await drawn(sp)).includes("limits") && await sp.locator("[data-testid=reveal-switch-limits]").count() === 1,
    `[${name}] search: clicking it opens Agent policy with Limits revealed, and the switch to Advanced beside it`);
  ok(await mode(sp) === "beginner" && ((await blob(sp))?.settings_mode ?? "beginner") === "beginner", `[${name}] search: …and the mode is still Beginner, in the blob too`);
  ok((await searchLook(sp)).focused, `[${name}] search: a click on a row keeps focus in the box`);

  // Escape: the list, then the text, then the sheet
  await searchFor(sp, "theme");
  const escs = [];
  for (let i = 0; i < 3; i++) {
    await sp.keyboard.press("Escape");
    await sp.waitForTimeout(120);
    const e = await searchLook(sp);
    escs.push(e.sheet ? `${e.pop ? "list" : "no list"}/${e.value ? "text" : "empty"}` : "closed");
  }
  ok(escs.join(" · ") === "no list/text · no list/empty · closed", `[${name}] search: Escape closes the list, then clears the box, then the sheet (${escs.join(" · ")})`);

  // a press outside closes the list; an empty box collapses on blur
  await openSheet(sp);
  await sp.waitForTimeout(300);
  await searchFor(sp, "plan");
  await sp.click(".settings-h b"); // outside the box and its list, which covers the page title
  await sp.waitForTimeout(250);
  sl = await searchLook(sp);
  ok(!sl.pop && sl.value === "plan" && sl.box.width >= s1.box.width - 1, `[${name}] search: a press outside closes the list; holding text, the box stays wide`);
  await sp.click(SEARCH);
  await sp.waitForTimeout(150);
  ok(!!(await searchLook(sp)).pop, `[${name}] search: focusing a box that holds text opens its list again`);
  await sp.fill(SEARCH, "");
  await sp.click(".settings-h b"); // outside the box and its list, which covers the page title
  await sp.waitForTimeout(300);
  sl = await searchLook(sp);
  ok(!sl.focused && near(sl.box.width, s0.box.width), `[${name}] search: empty and blurred, it collapses (${sl.box.width.toFixed(0)}px)`);

  // ⌘F, with Settings the only dialog up, focuses the box (and selects its text)
  await sp.fill(SEARCH, "logs");
  await sp.click(".settings-h b"); // outside the box and its list, which covers the page title
  await sp.keyboard.press("Meta+f");
  await sp.waitForTimeout(150);
  sl = await searchLook(sp);
  ok(sl.focused && sl.selection?.join() === "0,4", `[${name}] search: ⌘F focuses the box and selects what it holds`);

  // the field and the list in every theme: the field LIGHTER than the header
  // (the owner's review), its glass and placeholder no fainter than --txt-dim
  // on a hovered row, the list's words ≥ 4.5:1, the active row marked.
  await sp.fill(SEARCH, "");
  await sp.click(".settings-h b"); // outside the box and its list, which covers the page title
  await sp.waitForTimeout(250);
  for (const theme of THEMES) {
    await sp.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
    await sp.waitForTimeout(60);
    const f = await searchColours(sp);
    ok(f.lighter && f.fillDelta >= 15 && f.border >= 1.4,
      `[${name}] ${theme}: the search field is lighter than the header (${f.fillDelta} RGB units), border ${f.border.toFixed(2)}:1`);
    ok(f.placeholder >= f.floor - 0.01 && f.icon >= f.floor - 0.01,
      `[${name}] ${theme}: its placeholder and glass read (${f.placeholder.toFixed(2)}:1, --txt-dim on a hovered row is ${f.floor.toFixed(2)}:1)`);
  }
  await searchFor(sp, "a");
  for (const theme of THEMES) {
    await sp.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
    await sp.waitForTimeout(60);
    const f = await searchColours(sp);
    const low = f.words.filter((w) => w.ratio < 4.5);
    ok(f.words.length >= 16 && low.length === 0,
      `[${name}] ${theme}: the list's labels, places and tier chips ≥ 4.5:1${low.length ? ` — ${low.map((w) => `"${w.text}" ${w.ratio.toFixed(2)}`).join(", ")}` : ` (min ${Math.min(...f.words.map((w) => w.ratio)).toFixed(2)})`}`);
    ok(f.bar >= 3, `[${name}] ${theme}: the active row's mark shows (${f.bar.toFixed(2)}:1)`);
    if (SHOTS && ["tokyo-day", "nord"].includes(theme)) await sp.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-search-${theme}.png` });
  }
  await searchFor(sp, "zzqqxx");
  for (const theme of THEMES) {
    await sp.evaluate((t) => { document.documentElement.dataset.theme = t; }, theme);
    await sp.waitForTimeout(60);
    const w = (await searchColours(sp)).words;
    ok(w.length === 1 && w[0].ratio >= 4.5, `[${name}] ${theme}: "No settings match" reads (${w[0]?.ratio.toFixed(2)}:1)`);
  }
  await sctx.close();

  // a remote window lists nothing it can never draw, and opens a GrantNote
  const srctx = await browser.newContext({ viewport: { width: 1280, height: 860 } });
  const srp = await srctx.newPage();
  srp.on("pageerror", (e) => ok(false, `[${name}] remote search page error: ${e.message}`));
  await openSheet(srp, "?remote=noadmin");
  await searchFor(srp, "restore");
  const rIds = (await searchLook(srp)).rows.map((r) => r.id);
  ok(rIds.includes("restore-last") && !rIds.includes("restore-window"), `[${name}] remote search: "restore" lists Startup, never Restore window (${rIds.join(", ")})`);
  await searchFor(srp, "ai profiles");
  const prof = (await searchLook(srp)).rows[0];
  ok(prof?.id === "profiles" && prof.tier === "Advanced", `[${name}] remote search: "ai profiles" → AI profiles, marked Advanced`);
  await srp.keyboard.press("Enter");
  const toProfiles = await srp.waitForSelector(".settings-body[data-page=profiles]", { timeout: 3000 }).then(() => true, () => false);
  await srp.waitForTimeout(250);
  ok(toProfiles && await srp.locator(".sec[data-sec=profiles] [data-testid=grant-note]").count() === 1, `[${name}] remote search: opening it shows the GrantNote that says why`);
  await srctx.close();

  // ── the rail and the header at the app's narrowest window ────────────────
  // (tauri.conf.json's minWidth × minHeight), in every mode, here and remote.
  for (const remote of [false, true]) {
    const nctx = await browser.newContext({ viewport: { width: 900, height: 560 } });
    const np = await nctx.newPage();
    np.on("pageerror", (e) => ok(false, `[${name}] narrow page error: ${e.message}`));
    await openSheet(np, remote ? "?remote=noadmin" : "");
    const where = remote ? "remote, 900px" : "900px";
    for (const m of ["beginner", "advanced", "expert"]) {
      await setMode(np, m);
      const r = await railLook(np);
      ok(r.indent >= 8 && r.pillShift >= -1, `[${name}] ${where}, ${m}: pages sit indented under their heading, the selected pill too (${r.indent.toFixed(0)}px, pill ${r.pillShift.toFixed(0)}px)`);
      ok(r.minRoom >= 16, `[${name}] ${where}, ${m}: room above every later group (${r.minRoom.toFixed(0)}px)`);
      ok(r.clipped.length === 0, `[${name}] ${where}, ${m}: no heading or page clipped or wrapped${r.clipped.length ? ` — ${r.clipped.join(", ")}` : ""}`);
      ok(r.segToKeys >= 0 && r.segToKeys <= 24 && r.searchToSeg >= 8 && r.searchToSeg <= 16 && r.titleGap >= 16 && r.centre <= 1,
        `[${name}] ${where}, ${m}: the search then the mode switch sit right, beside esc and close (${r.searchToSeg.toFixed(0)}px, ${r.segToKeys.toFixed(0)}px), clear of the title (${r.titleGap.toFixed(0)}px), centred with it (${r.centre.toFixed(1)}px)`);
      // Wide and listing its eight rows, nothing to its right moves and the
      // list stays inside the modal, every row reachable.
      const before = await searchLook(np);
      await searchFor(np, "a");
      const w = await searchLook(np);
      const wr = await railLook(np);
      ok(w.rows.length === 8 && w.inside && w.unhit.length === 0 && near(w.seg.left, before.seg.left) && near(w.close.left, before.close.left) && wr.titleGap >= 16 && wr.centre <= 1,
        `[${name}] ${where}, ${m}: the open search fits — 8 rows inside the modal (bottom ${(w.modal.bottom - w.pop.bottom).toFixed(0)}px clear), all reachable, switch and close unmoved, title ${wr.titleGap.toFixed(0)}px clear`);
      if (SHOTS && m === "beginner") await np.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-narrow${remote ? "-remote" : ""}-search.png` });
      // Cleared and blurred directly, not by Escape: Escape has its own rule
      // above, and a broken one must not close the sheet under this loop.
      await np.fill(SEARCH, "");
      await np.evaluate(() => document.activeElement?.blur());
      await np.waitForTimeout(250);
    }
    if (SHOTS) await np.locator(".settings-modal").screenshot({ path: `${SHOTS}/${name}-narrow${remote ? "-remote" : ""}-expert.png` });
    await nctx.close();
  }
  await browser.close();
}

if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });
await run(chromium, "chromium");
await run(webkit, "webkit");
console.log(failed ? `\n${failed} failure(s)` : "\nall good");
process.exit(failed ? 1 : 0);
