// Browser gate: the app's menus and the workspace's tabs by keyboard alone (Kimi's K5),
// in Chromium AND WebKit against the mock. The shared menu (CtxMenu.tsx, its cascading
// lists) as a chat, a workspace and a PLACE open it, the document's own menu, and the
// tabs. No pointer is used but where it says so.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/menu-keys-browser.mjs
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 4000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const focused = (page) => page.evaluate(() => { const e = document.activeElement; return { id: e?.dataset?.testid ?? null, text: (e?.textContent ?? "").trim().slice(0, 40), role: e?.getAttribute?.("role") ?? null, inMenu: !!e?.closest?.(".ctxmenu, .ix-menu"), tag: e?.tagName }; });
const press = async (page, ...ks) => { for (const k of ks) { await page.keyboard.press(k); await page.waitForTimeout(40); } };
const menus = (page) => page.evaluate(() => document.querySelectorAll(".ctxmenu, .ix-menu").length);

async function boot(page) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", beta_workspace_chats: true, beta_ideas: true });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0`);
  await page.waitForSelector("[data-testid=ws-group]");
}
/** Open from `trigger` with Enter, walk, leave with Escape, then with Tab. */
async function menu(page, tag, what, trigger, { open = "Enter" } = {}) {
  const t = page.locator(trigger).first();
  await t.focus();
  await press(page, open);
  await until(page, () => !!document.querySelector(".ctxmenu, .ix-menu"));
  const a = await focused(page);
  ok(a.inMenu && a.role === "menuitem", `${tag} ${what}: opened with a key, its first act has the focus (${JSON.stringify(a)})`);
  ok(await page.evaluate(() => { const m = document.querySelector(".ctxmenu, .ix-menu"); return m.getAttribute("role") === "menu" && [...m.querySelectorAll("button")].filter((b) => b.matches(".pop-item, .ix-pop-item")).every((b) => b.getAttribute("role") === "menuitem"); }), `${tag} ${what}: it is a menu, of menu items`);
  await press(page, "ArrowDown");
  const b = await focused(page);
  const many = await page.evaluate(() => document.querySelectorAll(".ctxmenu [role=menuitem]:not(:disabled), .ix-menu [role=menuitem]:not(:disabled)").length);
  ok(b.inMenu && (many < 2 || b.text !== a.text), `${tag} ${what}: ↓ is the next one (${b.text})`);
  await press(page, "ArrowUp");
  ok((await focused(page)).text === a.text, `${tag} ${what}: ↑ the one before`);
  await press(page, "ArrowUp");
  const last = await focused(page);
  await press(page, "Home");
  const home = await focused(page);
  await press(page, "End");
  ok(home.text === a.text && (await focused(page)).text === last.text, `${tag} ${what}: it wraps, Home and End are its ends (${last.text})`);
  await press(page, "Escape");
  ok(await menus(page) === 0 && await t.evaluate((e) => e === document.activeElement), `${tag} ${what}: Escape closes it, the focus is on what opened it`);
  await press(page, open);
  await until(page, () => !!document.querySelector(".ctxmenu, .ix-menu"));
  await press(page, "Tab");
  ok(await menus(page) === 0 && await t.evaluate((e) => e === document.activeElement), `${tag} ${what}: Tab does not go behind it: it closes, the focus is back`);
}

async function chat(page, tag) {
  await boot(page);
  await page.locator("[data-testid=ws-chat-row]", { hasText: "Release order" }).first().click();
  await page.waitForSelector("[data-testid=ws-chat-col]");
  await menu(page, tag, "a chat's menu", "[data-testid=ws-chat-menu]");
  await menu(page, tag, "its close menu", "[data-testid=ws-chat-close-menu]");
  // An act of it, by key.
  await page.locator("[data-testid=ws-chat-menu]").focus();
  await press(page, "Enter");
  await press(page, "Enter");
  ok(await until(page, () => document.activeElement?.dataset?.testid === "ws-chat-rename-input"), `${tag} Enter on Rename… runs it, and its own field has the focus`);
  await press(page, "Escape");
  // What it reads is no menu: a panel, named, and Escape and Tab leave it the same way.
  const r = page.locator("[data-testid=ws-chat-readsb]");
  await r.focus();
  await press(page, "Enter");
  ok(await until(page, () => { const m = document.querySelector(".ctxmenu"); return m?.getAttribute("role") === "dialog" && !!m.getAttribute("aria-label") && m.contains(document.activeElement); }), `${tag} what it reads is a panel with a name, focused`);
  await press(page, "Escape");
  ok(await menus(page) === 0 && await r.evaluate((e) => e === document.activeElement), `${tag} …and Escape returns the focus`);
}

async function nav(page, tag) {
  await boot(page);
  await menu(page, tag, "a workspace's menu", "[data-testid=ws-menu]");
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-testid=ws-frame-menu]");
  await menu(page, tag, "the view's menu", "[data-testid=ws-frame-menu]");
  await page.locator("[data-testid=ws-menu]").first().focus();
  await press(page, "Enter");
  // A cascading row: → opens it and goes in, ← comes back.
  for (let i = 0; i < 20 && !(await page.evaluate(() => document.activeElement?.dataset.testid === "ws-color")); i++) await press(page, "ArrowDown");
  if (await page.evaluate(() => document.activeElement?.dataset.testid === "ws-color")) {
    await press(page, "ArrowRight");
    ok(await until(page, () => !!document.activeElement?.closest(".ctx-flyout")), `${tag} → opens a cascading list and goes in`);
    const x = await focused(page);
    await press(page, "ArrowDown");
    ok(await page.evaluate(() => !!document.activeElement?.closest(".ctx-flyout") && document.activeElement.getAttribute("role") === "menuitem"), `${tag} ↓ walks it (${x.text} → ${(await focused(page)).text})`);
    await press(page, "ArrowLeft");
    ok(await until(page, () => !document.querySelector(".ctx-flyout") && document.activeElement?.getAttribute("aria-haspopup") === "menu"), `${tag} ← closes it, and is on its row`);
  } else ok(false, `${tag} a workspace's menu has a cascading row`);
}

/** Places' menus: as they were by pointer, and by key once they are open. */
async function place(page, tag) {
  await boot(page);
  const row = page.locator("[data-testid=ws-group] li.row[data-slug]").nth(1);
  await page.locator(".nav-filter, [data-testid=nav-filter], input[placeholder*='filter']").first().focus();
  await row.click({ button: "right" });
  await until(page, () => !!document.querySelector(".ctxmenu"));
  const had = await focused(page);
  ok(!had.inMenu, `${tag} opened with the pointer, the focus stays where it was`);
  ok(await page.evaluate(() => document.querySelector(".ctxmenu").getAttribute("role") === "menu"), `${tag} it is a menu`);
  await press(page, "ArrowDown");
  const a = await focused(page);
  ok(a.inMenu, `${tag} ↓ goes into it (${a.text})`);
  await press(page, "Escape");
  ok(await menus(page) === 0 && (await focused(page)).tag === had.tag, `${tag} Escape closes it, the focus is where it was`);
  // By pointer alone, as ever.
  await row.click({ button: "right" });
  await until(page, () => !!document.querySelector(".ctxmenu"));
  await page.mouse.click(900, 500);
  ok(await until(page, () => !document.querySelector(".ctxmenu")), `${tag} a press elsewhere closes it`);
  await row.click({ button: "right" });
  await until(page, () => !!document.querySelector(".ctxmenu"));
  await page.locator(".ctxmenu .pop-item", { hasText: /^Enter/ }).first().click();
  ok(await until(page, () => !!document.querySelector(".space > .topbar .slug") && !document.querySelector(".ctxmenu")), `${tag} and its acts run`);
}

async function doc(page, tag) {
  await boot(page);
  await page.locator("[data-testid=ws-doc-memory]").first().click();
  await until(page, () => document.querySelector("[data-testid=main-doc]")?.dataset.doc === "MEMORY.md");
  await page.locator("[data-testid=ws-chat-row]").first().click();
  await page.waitForSelector("[data-testid=doc-menu-open]");
  await page.waitForTimeout(400);
  await menu(page, tag, "a document's menu", "[data-testid=doc-menu-open]");
}

async function tabs(page, tag) {
  await boot(page);
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-testid=ws-tab]");
  const st = () => page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-tab]")].map((t) => ({ tab: t.dataset.tab, stop: t.tabIndex === 0, on: t.getAttribute("aria-selected") === "true", focus: t === document.activeElement, panel: t.getAttribute("aria-controls") })));
  const s = await st();
  ok(s.filter((t) => t.stop).length === 1 && s.find((t) => t.stop).on, `${tag} one Tab stop: the tab that is on`);
  ok(await page.evaluate(() => { const on = document.querySelector("[data-testid=ws-tab][aria-selected=true]"), p = document.getElementById(on.getAttribute("aria-controls")); return !!p && p.getAttribute("role") === "tabpanel" && p.getAttribute("aria-labelledby") === on.id && p.checkVisibility(); }), `${tag} its panel is named, and names it`);
  await page.locator("[data-testid=ws-tab][aria-selected=true]").focus();
  await press(page, "ArrowRight");
  let n = await st();
  ok(n[1].focus && n[0].on, `${tag} → is the next tab's, and does not open it`);
  await press(page, "End");
  ok((await st()).at(-1).focus, `${tag} End the last`);
  await press(page, "ArrowRight");
  ok((await st())[0].focus, `${tag} → wraps`);
  await press(page, "ArrowLeft", "Home");
  ok((await st())[0].focus, `${tag} Home the first`);
  await press(page, "ArrowRight", "Enter");
  ok(await until(page, () => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === "memory"), `${tag} Enter opens it`);
  n = await st();
  ok(n[1].on && n[1].stop && n[1].focus && n.filter((t) => t.stop).length === 1, `${tag} …and it has the stop and the focus`);
  ok(await page.evaluate(() => { const on = document.querySelector("[data-testid=ws-tab][aria-selected=true]"), p = document.getElementById(on.getAttribute("aria-controls")); return !!p && p.checkVisibility() && p.getAttribute("aria-labelledby") === on.id; }), `${tag} …and a panel of its own`);
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [chat, nav, place, doc, tabs]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (process.env.IDEAS_SCREENSHOTS) await page.screenshot({ path: `${process.env.IDEAS_SCREENSHOTS}/mk-${name}-${part.name}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\nmenu-keys-browser: ${failed} failure(s)` : "\nmenu-keys-browser: all good");
process.exit(failed ? 1 : 0);
