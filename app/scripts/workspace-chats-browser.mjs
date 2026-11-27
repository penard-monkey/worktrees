// Browser gate for a workspace's chats on screen (docs/proposals/
// workspace-view-design.md §2, §3.2, §5: WorkspaceChat.tsx, useWsChats.ts), in
// Chromium AND WebKit against the mock. The view they sit in is
// workspace-view-browser.mjs's.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/workspace-chats-browser.mjs
//
// What it holds: the rows in the card, in their order, with their words and
// dots; the card's and the switch's amber over working lanes; the filter and ⌘K;
// a chat in the middle with the documents beside it, its draft its own; a prompt
// answered there; Chat | Terminal; Close, ended, Resume, Delete; a start, its
// name from the first message, a refusal said in the chat's side; the Chats tab;
// one side under 760 px; Back and Forward; six themes; nothing of it with the
// beta off. And a chat row's right-click menu, in the card and in the Chats tab:
// the header's acts by the chat's state, refused ones said, by key too. And
// New chat's ▾: the harness and model a chat starts on.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const group = (page, name) => page.locator("[data-testid=ws-group]", { has: page.locator("[data-testid=ws-name]", { hasText: name }) }).first();
const row = (page, title) => group(page, "Everything").locator("[data-testid=ws-chat-row]", { hasText: title });
const colOf = (page, title) => until(page, (t) => document.querySelector("[data-testid=ws-chat-col] .wsc-name-t")?.textContent === t, title);
const tab = (page, t) => page.click(`[data-testid=ws-tab][data-tab=${t}]`);
const isTab = (page, t) => until(page, (t) => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === t, t, 2000);
const sends = (page) => page.evaluate(() => window.__mock.chatSends());
const draft = (page) => page.locator("[data-testid=ws-chat-col] textarea").first();

async function boot(page, betas = ["beta_workspace_chats"], extra = {}, query = "") {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", chat_view: true, ...Object.fromEntries(betas.map((b) => [b, true])), ...extra });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0${query}`);
  await page.waitForSelector("[data-testid=ws-group]");
  if (betas.includes("beta_workspace_chats")) await page.waitForSelector("[data-testid=ws-chat-row]");
  return (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
}

async function nav(page, tag, name) {
  const ws = await boot(page);
  const rows = await group(page, "Everything").locator("[data-testid=ws-chat-row]").evaluateAll((b) => b.map((x) => [x.dataset.state, x.querySelector(".wsc-title").textContent, x.querySelector(".wsc-age").textContent]));
  ok(JSON.stringify(rows.map((r) => r.slice(0, 2))) === JSON.stringify([["waiting", "Who can see what"], ["running", "Release order"], ["ended", "Storage layout"], ["ended", "First look"], ["ended", "Naming"]]), `${tag} live first, the waiting one first, then three closed (${JSON.stringify(rows)})`);
  ok(/^waiting 1\dm$/.test(rows[0][2]) && rows[2][2] === "1d", `${tag} a waiting row says so in words, a closed one its age`);
  ok(await row(page, "Who can see what").locator(".wsc-age").evaluate((e) => { const d = document.createElement("i"); d.style.color = "var(--txt-hi)"; e.append(d); const same = getComputedStyle(d).color === getComputedStyle(e).color; d.remove(); return same; }), `${tag} …in --txt-hi`);
  ok(await group(page, "Everything").locator(".ws-icon").first().evaluate((e) => e.classList.contains("waiting")), `${tag} its card's dot is waiting`);
  ok(!!(await page.$("[data-testid=nav-ws-waits]")), `${tag} the Workspaces switch has the amber dot`);
  await page.click("[data-testid=nav-view-projects]");
  ok(!!(await page.$("[data-testid=nav-ws-waits]")) && !(await page.$("[data-testid=ws-chat-row]")), `${tag} …from All projects too, which lists no chat`);
  await page.click("[data-testid=nav-view-workspaces]");
  ok(await group(page, "Everything").locator("[data-testid=ws-chats-all]").count() === 0, `${tag} three closed are all of them: no All chats`);
  ok(await page.evaluate(() => { const g = document.querySelector("[data-testid=ws-group]"); const c = g.querySelector("[data-testid=ws-chats]"), p = g.querySelector(".project"); const F = Node.DOCUMENT_POSITION_FOLLOWING; return !g.querySelector("[data-testid=ws-docs]") && !!(c.compareDocumentPosition(p) & F); }), `${tag} above the projects, with no documents row`);
  // Folds, as the event says them.
  const id = async (t) => row(page, t).getAttribute("data-chat");
  const [rel, sto] = [await id("Release order"), await id("Storage layout")];
  await page.evaluate(([w, c]) => window.__mock.wsFold(w, c, "folding"), [ws, rel]);
  ok(await until(page, (c) => document.querySelector(`[data-chat='${c}']`)?.dataset.state === "folding", rel) && /folding…/.test(await row(page, "Release order").innerText()), `${tag} a fold the event names is on its row`);
  await page.evaluate(([w, c]) => window.__mock.wsFold(w, c, "failed", "no note"), [ws, sto]);
  ok(await until(page, (c) => document.querySelector(`[data-chat='${c}']`)?.dataset.state === "fold-failed", sto), `${tag} and a failed one, closed or not`);
  await page.evaluate(([w, a, b]) => { window.__mock.wsFold(w, a, null); window.__mock.wsFold(w, b, null); }, [ws, rel, sto]);
  // A fourth closed one.
  await page.evaluate(([w, c]) => window.__mock.endChat(w, c), [ws, rel]);
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chats-all]")) && /All chats · 5/.test(await page.locator("[data-testid=ws-chats-all]").innerText()), `${tag} a fourth closed brings All chats · 5`);
  await page.click("[data-testid=ws-chats-all]");
  ok(await isTab(page, "chats"), `${tag} …which opens the Chats tab`);

  // ── chats that could not be read ──
  await page.evaluate(() => { const t = window.__TAURI_INTERNALS__, i = t.invoke; window.__badChats = true; t.invoke = (c, a) => (c === "workspace_chats" && window.__badChats ? Promise.reject("not a workspace id: x") : i(c, a)); });
  await page.evaluate(([w, c]) => window.__mock.wsFold(w, c, "folding"), [ws, rel]);
  ok(await until(page, () => /could not be read: not a workspace id: x\./.test(document.querySelector("[data-testid=ws-chats-error]")?.textContent ?? "")), `${tag} chats that could not be read say why`);
  await page.evaluate(([w, c]) => { window.__badChats = false; window.__mock.wsFold(w, c, null); }, [ws, rel]);
  await until(page, () => !document.querySelector("[data-testid=ws-chats-error]"));

  // ── the filter ──
  const box = page.locator(".nav-filter input, input[placeholder^='filter places']").first();
  await box.fill("storage");
  ok(await until(page, () => [...document.querySelectorAll("[data-testid=ws-chat-row]")].map((r) => r.querySelector(".wsc-title").textContent).join() === "Storage layout"), `${tag} the filter matches chat titles, and hides the rest`);
  await box.press("Enter");
  ok(await colOf(page, "Storage layout"), `${tag} Enter lands on the chat`);
  await box.fill("zzz-nothing");
  ok(await until(page, () => !document.querySelector("[data-testid=ws-chats]")), `${tag} no match, no group`);
  await box.fill("");

  // ── ⌘K ──
  await page.keyboard.press("Meta+k");
  await page.waitForSelector(".qs-input");
  ok(!(await page.$("[data-testid=switch-more]")), `${tag} ⌘K opens on places`);
  await page.fill(".qs-input", "who can");
  ok(await until(page, () => /Who can see what/.test(document.querySelector("[data-testid=switch-more]")?.textContent ?? "")), `${tag} ⌘K finds a chat`);
  await page.keyboard.press("Enter");
  ok(await colOf(page, "Who can see what"), `${tag} …and opens it`);
  await page.keyboard.press("Meta+k");
  await page.fill(".qs-input", "everything memory");
  ok(await until(page, () => /Memory/.test(document.querySelector("[data-testid=switch-more]")?.textContent ?? "")), `${tag} ⌘K finds a workspace's document`);
  await page.keyboard.press("Enter");
  ok(await isTab(page, "memory"), `${tag} …and opens its tab`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsc-nav-${name}.png` });
}

async function chat(page, tag, name) {
  const ws = await boot(page, ["beta_workspace_chats", "beta_ideas"]);
  await row(page, "Release order").click();
  ok(await colOf(page, "Release order") && await isTab(page, "memory"), `${tag} a chat row opens it in the middle, beside Memory`);
  const g = await page.evaluate(() => { const m = document.querySelector("[data-testid=ws-mid]").getBoundingClientRect(), d = document.querySelector(".wsf-docs").getBoundingClientRect(), b = document.querySelector(".wsf-body").getBoundingClientRect(); return { left: m.left <= b.left + 1, right: d.left >= m.right - 1, share: d.width / b.width, mid: m.width, docs: d.width }; });
  ok(g.left && g.right && Math.abs(g.share - 0.46) < 0.01 && g.mid >= 420 && g.docs >= 340, `${tag} chat left, documents on the dock's side, at 46 percent (${JSON.stringify(g)})`);
  ok(await row(page, "Release order").evaluate((e) => e.classList.contains("on")) && !(await page.evaluate(() => document.querySelector(".home-item").classList.contains("on"))), `${tag} its row is marked, Home is not lit`);
  ok(/A chat of Everything\. It reads the memory index first, and it can read \d of 3 projects\. It is told not to write in a project\. The rules are set when the chat starts\. They stop Claude's file tools, not every program it runs\./.test(await page.locator("[data-testid=chat-intro]").innerText()), `${tag} its first line says what it is`);
  const reads = await page.locator("[data-testid=ws-chat-readsb]").innerText();
  ok(/^reads \d of 3/.test(reads), `${tag} the header counts what it reads (${reads})`);
  await page.click("[data-testid=ws-chat-readsb]");
  ok(/not read: not registered/.test(await page.locator("[data-testid=ws-chat-reads]").innerText()), `${tag} …and names which, and why not`);
  ok(/They stop Claude's file tools, not every program it runs\./.test(await page.locator("[data-testid=ws-chat-limits]").innerText()) && !(await page.$("[data-testid=ws-chat-stale]")) && !(await page.$("[data-testid=ws-chat-stale-chip]")), `${tag} …with what its rules stop, in the engine's words`);
  // ── its rules are not what a launch now would get ──
  await page.keyboard.press("Escape");
  const stale = await page.evaluate(() => { const c = document.querySelector("[data-testid=ws-chat-col]").dataset.chat, w = document.querySelector("[data-testid=ws-frame]").dataset.ws; window.__mock.wsStale(w, c, [{ kind: "project", name: "valleos", path: "/w/valleos", change: "private" }, { kind: "document", name: "Plan", path: "/i/p.md", change: "unlinked" }]); return [w, c]; });
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-stale-chip]")) && await page.evaluate(() => { const b = document.querySelector("[data-testid=ws-chat-stale-chip]").getBoundingClientRect(); const h = document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2); return !!h.closest("[data-testid=ws-chat-stale-chip]"); }), `${tag} a chat whose rules changed says so in its header`);
  await page.click("[data-testid=ws-chat-stale-chip]");
  ok(/valleos is private now\s+“Plan” is not linked any more[\s\S]*Close and start a new chat to apply/.test(await page.locator("[data-testid=ws-chat-stale]").innerText()), `${tag} …and what, and what to do about it`);
  await page.waitForTimeout(300);
  ok(await page.evaluate(() => { const b = document.querySelector("[data-testid=ws-chat-stale]")?.getBoundingClientRect(); return !!b && b.width > 0 && !!document.elementFromPoint(b.x + 20, b.y + 10)?.closest("[data-testid=ws-chat-stale]"); }), `${tag} …and it stays open to be read`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsc-stale-${name}.png` });
  await page.keyboard.press("Escape");
  ok(!!(await page.$("[data-testid=ws-chat-col] textarea")), `${tag} nothing is ended: it goes on`);
  await page.evaluate(([w, c]) => window.__mock.wsStale(w, c, null), stale);
  ok(await until(page, () => !document.querySelector("[data-testid=ws-chat-stale-chip]")), `${tag} and when they are the same again, it is gone`);
  await page.click("[data-testid=ws-chat-readsb]");
  await page.keyboard.press("Escape");
  ok(await until(page, () => /Which ships first\?/.test(document.querySelector("[data-testid=chat-list]")?.textContent ?? "")), `${tag} its transcript`);
  ok(!(await page.$("[data-testid=ws-chat-col] .chat-attach")) && !/for a command|\/ for/.test(await page.locator("[data-testid=ws-chat-col]").innerText()) && (await draft(page).getAttribute("placeholder")) === "Message this chat", `${tag} no attachments and no slash commands in the composer`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsc-chat-${name}.png` });
  await call(page, "workspace_chat_rename", { id: ws, conv: await row(page, "Release order").getAttribute("data-chat"), title: "Release order, a name much too long for any header there is to hold whole" });
  await page.locator("[data-testid=ws-chat-menu]").waitFor();
  await page.evaluate(() => window.__mock.wsFold && window.dispatchEvent(new Event("focus")));
  await row(page, "Who can see what").click(); await row(page, "Release order").click();
  ok(await page.evaluate(() => { const r = document.querySelector("[data-testid=ws-chat-readsb]"); return r.scrollWidth <= r.clientWidth + 1; }), `${tag} "reads n of m" stays whole beside a long name`);
  await call(page, "workspace_chat_rename", { id: ws, conv: await row(page, "Release order").getAttribute("data-chat"), title: "Release order" });
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));

  // ── a draft in each ──
  await draft(page).fill("half a thought");
  await tab(page, "plan");
  ok(await colOf(page, "Release order") && await draft(page).inputValue() === "half a thought", `${tag} another document keeps the chat, and its draft`);
  await row(page, "Who can see what").click();
  ok(await colOf(page, "Who can see what") && await isTab(page, "plan"), `${tag} another chat keeps the document's tab`);
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-col] .chat-card")), `${tag} a waiting one shows its prompt`);
  await page.locator("[data-testid=ws-chat-col] textarea").last().fill("and here");
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  ok(await draft(page).inputValue() === "half a thought", `${tag} each has its own draft`);
  await row(page, "Who can see what").click();
  await colOf(page, "Who can see what");
  ok(await page.locator("[data-testid=ws-chat-col] textarea").last().inputValue() === "and here", `${tag} …both ways`);

  // ── its prompt ──
  await page.locator("[data-testid=ws-chat-col] .chat-card").getByText("Yes", { exact: true }).first().click();
  const go = page.locator("[data-testid=ws-chat-col] .chat-card button", { hasText: /^(Answer|Send|Confirm|Yes)/ }).first();
  if (await go.count()) await go.click().catch(() => {});
  ok(await until(page, () => !document.querySelector("[data-testid=ws-chat-col] .chat-card"), null, 8000), `${tag} it is answered there`);
  ok(await until(page, () => !document.querySelector("[data-testid=nav-ws-waits]"), null, 8000), `${tag} …and the switch's dot goes`);

  // ── a message ──
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  await draft(page).fill("What is next?");
  await draft(page).press("Enter");
  ok(await until(page, () => window.__mock.chatSends().some((s) => s.text === "What is next?")), `${tag} a message goes by workspace_send`);
  ok((await sends(page)).at(-1).id === ws, `${tag} …to this workspace's chat`);
  ok(await until(page, () => /Noted: “What is next\?”/.test(document.querySelector("[data-testid=chat-list]")?.textContent ?? ""), null, 10000), `${tag} and its reply arrives`);
  ok((await page.locator("[data-testid=ws-chat-col] .wsc-name-t").innerText()) === "Release order", `${tag} a chat with a name keeps it`);

  // ── Chat | Terminal ──
  await page.click("[data-testid=ws-chat-view-terminal]");
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-term] .xterm")) && !(await page.$("[data-testid=chat]")), `${tag} Terminal shows its session, in the chat's place`);
  await page.click("[data-testid=ws-chat-view-chat]");
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-col] [data-testid=chat]") && !document.querySelector("[data-testid=ws-chat-term]")), `${tag} and Chat is back`);

  // ── rename ──
  await page.click("[data-testid=ws-chat-menu]");
  ok(await page.locator("[data-testid=ws-chat-list-row]").count() === 5, `${tag} the title's menu has the workspace's chats`);
  await page.click("[data-testid=ws-chat-rename]");
  await page.fill("[data-testid=ws-chat-rename-input]", "Order of release");
  await page.keyboard.press("Enter");
  ok(await colOf(page, "Order of release") && await until(page, () => [...document.querySelectorAll("[data-testid=ws-chat-row] .wsc-title")].some((e) => e.textContent === "Order of release")), `${tag} Rename renames it, here and in the card`);

  // ── Close without folding, ended, Resume ── (Close, the fold: workspace-fold-browser.mjs)
  ok(await page.locator("[data-testid=ws-chat-close]").evaluate((e) => getComputedStyle(e).backgroundColor !== getComputedStyle(document.querySelector("[data-testid=ws-frame-new-chat]")).backgroundColor), `${tag} its Close is not a filled button`);
  await page.click("[data-testid=ws-chat-close-menu]");
  ok(await page.locator(".ctxmenu .pop-item").evaluateAll((b) => b.map((x) => x.textContent).join()) === "Close without folding,Delete", `${tag} its menu: Close without folding, Delete (and the terminal app, when there is one)`);
  await page.click("[data-testid=ws-chat-close-item]");
  ok(await until(page, () => /Closed, not folded\. Started /.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? "")) && !(await page.$("[data-testid=ws-chat-view-terminal]")) && await page.locator("[data-testid=ws-chat-col] textarea").count() === 0, `${tag} closed, it reads, with no composer and no terminal`);
  ok(await until(page, () => !!document.querySelector("[data-testid=chat-list] li")), `${tag} …its transcript still there`);
  await page.click("[data-testid=ws-chat-resume]");
  ok(await until(page, () => !document.querySelector("[data-testid=ws-chat-ended]") && !!document.querySelector("[data-testid=ws-chat-view-terminal]")), `${tag} Resume brings it back`);
  // A working one asks first.
  await draft(page).fill("Something long");
  await draft(page).press("Enter");
  await until(page, () => document.querySelector("[data-testid=ws-chat-col]")?.dataset.state === "busy");
  await page.click("[data-testid=ws-chat-close-menu]");
  await page.click("[data-testid=ws-chat-close-item]");
  ok(/It is working\. Close\?/.test(await page.locator("[data-testid=ws-chat-close-item]").innerText().catch(() => "")), `${tag} closing one that is working, with no fold, asks first`);
  await page.keyboard.press("Escape");
  await until(page, () => document.querySelector("[data-testid=ws-chat-col]")?.dataset.state === "running", null, 10000);
  // ── Delete ──
  await page.click("[data-testid=ws-chat-close-menu]");
  await page.click("[data-testid=ws-chat-delete]");
  ok(/Delete this chat\?/.test(await page.locator("[data-testid=ws-chat-delete]").innerText()) && (await page.evaluate((w) => window.__mock.wsChats(w).length, ws)) === 5, `${tag} Delete asks first`);
  await page.click("[data-testid=ws-chat-delete]");
  ok(await until(page, (w) => window.__mock.wsChats(w).length === 4, ws) && await until(page, () => !document.querySelector("[data-testid=ws-mid]")), `${tag} the second press deletes it, and the documents take the room`);
  ok(await page.evaluate(() => Math.abs(document.querySelector(".wsf-docs").getBoundingClientRect().width - document.querySelector(".wsf-body").getBoundingClientRect().width) < 1), `${tag} …all of it`);
}

/** A row's right-click menu: the header's acts, by the chat's state (wschats.ts `chatActs`). */
async function rowmenu(page, tag) {
  const ws = await boot(page);
  const items = () => page.locator(".ctxmenu .pop-item").evaluateAll((b) => b.map((x) => (x.disabled ? "~" : "") + x.querySelector("span span").textContent));
  const ctx = async (t) => { await row(page, t).click({ button: "right" }); return until(page, () => !!document.querySelector(".ctxmenu")); };
  ok(await ctx("Release order"), `${tag} a chat row has a right-click menu`);
  ok(JSON.stringify(await items()) === JSON.stringify(["Close", "Close without folding", "Delete"]), `${tag} …a live chat's: Close, Close without folding, Delete (${JSON.stringify(await items())})`);
  ok((await page.locator(".ctxmenu .wsc-menu-h").innerText()) === "Release order" && await page.evaluate(() => document.querySelector(".ctxmenu").getAttribute("role") === "menu"), `${tag} …a menu, named by the chat`);
  ok(!(await page.$("[data-testid=ws-chat-col]")), `${tag} …and opening it opens no chat`);
  await page.keyboard.press("ArrowDown");
  ok(await page.evaluate(() => document.activeElement?.dataset.act === "close"), `${tag} ↓ goes into it`);
  await page.keyboard.press("Escape");
  ok(await until(page, () => !document.querySelector(".ctxmenu")), `${tag} Escape closes it`);
  // By key: the menu key on a focused row, its first act focused.
  await row(page, "Release order").focus();
  await page.keyboard.press("Shift+F10");
  ok(await until(page, () => document.activeElement?.dataset.act === "close"), `${tag} Shift+F10 on a row opens it, its first act focused`);
  await page.keyboard.press("Escape");
  // Close without folding, from the row.
  await ctx("Release order");
  await page.click("[data-testid=ws-chat-act-close-none]");
  ok(await until(page, () => [...document.querySelectorAll("[data-testid=ws-chat-row]")].find((r) => r.querySelector(".wsc-title").textContent === "Release order")?.dataset.state === "ended"), `${tag} Close without folding ends it`);
  await ctx("Release order");
  ok(JSON.stringify(await items()) === JSON.stringify(["Fold now", "Resume", "Delete"]), `${tag} an ended one: Fold now, Resume, Delete (${JSON.stringify(await items())})`);
  await page.click("[data-testid=ws-chat-act-resume]");
  ok(await until(page, () => [...document.querySelectorAll("[data-testid=ws-chat-row]")].find((r) => r.querySelector(".wsc-title").textContent === "Release order")?.dataset.state === "running"), `${tag} Resume starts it again`);
  // Another chat folds: its fold would be refused, and says why.
  const rel = await row(page, "Release order").getAttribute("data-chat");
  await page.evaluate(([w, c]) => window.__mock.wsFold(w, c, "folding"), [ws, rel]);
  await until(page, (c) => document.querySelector(`[data-chat='${c}']`)?.dataset.state === "folding", rel);
  await ctx("Release order");
  ok(await page.locator("[data-testid=ws-chat-act-close]").isDisabled() && /It is folding into memory\./.test(await page.locator("[data-testid=ws-chat-act-close]").innerText()), `${tag} a folding chat's Close is refused, with the reason`);
  await page.keyboard.press("Escape");
  await ctx("Naming");
  ok(await page.locator("[data-testid=ws-chat-act-fold-now]").isDisabled() && /“Release order” is folding\. Close this one after\./.test(await page.locator("[data-testid=ws-chat-act-fold-now]").innerText()), `${tag} …and another chat's Fold now, with the engine's words`);
  await page.keyboard.press("Escape");
  await page.evaluate(([w, c]) => window.__mock.wsFold(w, c, null), [ws, rel]);
  // Close, the fold, from the row.
  await until(page, (c) => document.querySelector(`[data-chat='${c}']`)?.dataset.state !== "folding", rel);
  await ctx("Storage layout");
  ok(JSON.stringify(await items()) === JSON.stringify(["Resume", "Delete"]), `${tag} a folded one: Resume, Delete (${JSON.stringify(await items())})`);
  await page.keyboard.press("Escape");
  await ctx("First look");
  await page.click("[data-testid=ws-chat-act-fold-now]");
  ok(await until(page, () => /folding…/.test([...document.querySelectorAll("[data-testid=ws-chat-row]")].find((r) => r.querySelector(".wsc-title").textContent === "First look")?.textContent ?? "")), `${tag} Fold now folds it`);
  // Delete asks first.
  await ctx("Storage layout");
  await page.click("[data-testid=ws-chat-act-delete]");
  ok(/Delete this chat\?/.test(await page.locator("[data-testid=ws-chat-act-delete]").innerText()) && (await page.evaluate((w) => window.__mock.wsChats(w).length, ws)) === 5, `${tag} Delete asks first`);
  await page.click("[data-testid=ws-chat-act-delete]");
  ok(await until(page, (w) => window.__mock.wsChats(w).length === 4, ws) && !(await page.$(".ctxmenu")), `${tag} …and the second press deletes it`);
  // The Chats tab's rows have it too, once the fold has landed.
  await until(page, () => ![...document.querySelectorAll("[data-testid=ws-chat-row]")].some((r) => r.dataset.state === "folding"), null, 8000);
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-testid=ws-overview]");
  await tab(page, "chats");
  await page.locator("[data-testid=ws-chats-row]", { hasText: "Naming" }).locator(".wsc-item-b").click({ button: "right" });
  ok(await until(page, () => !!document.querySelector(".ctxmenu")) && JSON.stringify(await items()) === JSON.stringify(["Fold now", "Resume", "Delete"]), `${tag} the Chats tab's rows have it too (${JSON.stringify(await items())})`);
}

/** New chat's ▾: the harness and its model (workspace-chat-provider.md §3). */
async function picker(page, tag) {
  const ws = await boot(page);
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-testid=ws-overview]");
  await page.click("[data-testid=ws-frame-new-chat-more]");
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-pick]")), `${tag} New chat's ▾ opens the pick`);
  const hs = await page.locator("[data-testid=ws-chat-pick-harness]").evaluateAll((b) => b.map((x) => (x.disabled ? "~" : "") + x.textContent));
  ok(JSON.stringify(hs) === JSON.stringify(["Claude", "~Codex", "~pi", "~Kimi"]), `${tag} every harness, the ones that cannot be a chat disabled (${JSON.stringify(hs)})`);
  ok(/Codex cannot be a workspace chat yet\. Its sandbox does not stop reads, so a private project would be readable\./.test(await page.locator("[data-testid=ws-chat-pick-why]").innerText()), `${tag} …each with the engine's reason`);
  ok(await page.locator("[data-testid=ws-chat-pick-harness][data-harness=codex]").getAttribute("title") !== null, `${tag} …on the button too`);
  await until(page, () => document.querySelectorAll("[data-testid=ws-chat-pick-model] option").length > 2);
  await page.selectOption("[data-testid=ws-chat-pick-model]", "sonnet");
  await page.click("[data-testid=ws-chat-pick-start]");
  ok(await until(page, () => window.__mock.chatStarts().length === 1) && JSON.stringify((await page.evaluate(() => window.__mock.chatStarts()))[0]) === JSON.stringify({ harness: "claude", model: "sonnet" }), `${tag} Start starts Claude on that model`);
  ok(await until(page, () => /^Claude sonnet$/.test(document.querySelector("[data-testid=ws-chat-model]")?.textContent ?? "")), `${tag} its header names the model it was started on (${await page.locator("[data-testid=ws-chat-model]").innerText().catch(() => "none")})`);
  const conv = await page.locator("[data-testid=ws-chat-col]").getAttribute("data-chat");
  ok((await call(page, "workspace_chats", { id: ws })).chats.find((c) => c.id === conv)?.model === "sonnet", `${tag} …and its record holds it`);
  ok(/· Claude · sonnet$/.test(await group(page, "Everything").locator(`[data-testid=ws-chat-row][data-chat='${conv}']`).getAttribute("title")), `${tag} its row says what it runs on`);
  // The plain press starts the last pick, with no step.
  await until(page, () => !document.querySelector("[data-testid=ws-frame-new-chat]")?.disabled);
  await page.click("[data-testid=ws-frame-new-chat]");
  ok(await until(page, () => window.__mock.chatStarts().length === 2) && JSON.stringify((await page.evaluate(() => window.__mock.chatStarts()))[1]) === JSON.stringify({ harness: "claude", model: "sonnet" }), `${tag} New chat's plain press starts the workspace's last pick`);
  ok(!(await page.$("[data-testid=ws-chat-pick]")), `${tag} …with no step`);
}

async function start(page, tag) {
  const ws = await boot(page, ["beta_workspace_chats"]);
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-testid=ws-overview]");
  ok(await page.locator("[data-testid=ws-ov-card-chats] [data-testid=ws-chat-line]").count() === 5 && /2 chats live/.test(await page.locator(".wsf-counts").innerText()), `${tag} the Overview lists them, and its bar counts the live`);
  ok(/“Who can see what” is waiting for an answer\./.test(await page.locator("[data-testid=ws-attention][data-kind=chat]").innerText()), `${tag} a waiting one is the attention line`);
  await page.click("[data-testid=ws-attention][data-kind=chat] [data-testid=ws-attention-go]");
  ok(await colOf(page, "Who can see what"), `${tag} …which opens it`);
  await page.click("[data-testid=ws-frame-new-chat]");
  ok(await until(page, () => /^Chat \d+$/.test(document.querySelector("[data-testid=ws-chat-col] .wsc-name-t")?.textContent ?? "")), `${tag} New chat starts one, in the middle, with the engine's name`);
  ok(await until(page, () => [...document.querySelectorAll("[data-testid=ws-chat-row] .wsc-title")].some((e) => /^Chat \d+$/.test(e.textContent))), `${tag} …and its row is there`);
  await until(page, () => /What would you like to settle/.test(document.querySelector("[data-testid=chat-list]")?.textContent ?? ""), null, 10000);
  ok(/^Chat \d+$/.test(await page.locator("[data-testid=ws-chat-col] .wsc-name-t").innerText()), `${tag} the engine's opener names nothing`);
  ok((await page.locator("[data-testid=chat-quiet]").innerText()) === "The chat was started." && await page.locator("[data-testid=chat-user]").count() === 0, `${tag} …and is a quiet line, not something the person said`);
  await draft(page).fill("Which box should the media server run on?");
  await draft(page).press("Enter");
  ok(await colOf(page, "Which box should the media server run…"), `${tag} the first message names it, cut at a word`);
  await until(page, () => document.querySelector("[data-testid=ws-chat-col]")?.dataset.state === "running", null, 8000);
  await draft(page).fill("And the second?");
  await draft(page).press("Enter");
  await page.waitForTimeout(1500);
  ok(await colOf(page, "Which box should the media server run…"), `${tag} …once`);

  // ── a refusal ──
  await page.evaluate(() => { const t = window.__TAURI_INTERNALS__, i = t.invoke; window.__refuse = true; t.invoke = (c, a) => (c === "workspace_chat_start" && window.__refuse ? Promise.reject("5 agents are running and the limit is 5") : i(c, a)); });
  const before = await page.locator("[data-testid=ws-chat-row]").count();
  await group(page, "Everything").locator("[data-testid=ws-chat-new]").click();
  ok(await until(page, () => /The chat was not started[\s\S]*5 agents are running and the limit is 5/.test(document.querySelector("[data-testid=ws-chat-refused]")?.textContent ?? "")), `${tag} a refused start is said in the chat's side, in the engine's words`);
  ok(await page.locator("[data-testid=ws-chat-row]").count() === before && !(await page.$("[data-testid=ws-chat-starting]")), `${tag} …and leaves no row`);
  await page.evaluate(() => { window.__refuse = false; });
  await page.click("[data-testid=ws-chat-again]");
  ok(await until(page, () => /^Chat \d+$/.test(document.querySelector("[data-testid=ws-chat-col] .wsc-name-t")?.textContent ?? "")), `${tag} Try again starts it`);

  // ── the Chats tab ──
  await tab(page, "chats");
  ok(await page.locator("[data-testid=ws-chats-row]").count() === 7, `${tag} the Chats tab has every chat`);
  const words = await page.locator("[data-testid=ws-chats-row]").evaluateAll((r) => r.map((x) => x.querySelector(".ws-ov-detail").textContent));
  ok(words.includes("waiting on you") && words.includes("closed · folded into memory") && words.includes("closed · its fold was undone") && words.includes("closed, not folded"), `${tag} …their state in words (${words.join("; ")})`);
  ok(/started \w+ \d+, 20\d\d/.test(await page.locator("[data-testid=ws-chats-row] .wsc-when").first().innerText()), `${tag} …and dates`);
  await page.fill("[data-testid=ws-chats-filter]", "naming");
  ok(await page.locator("[data-testid=ws-chats-row]").count() === 1, `${tag} …and a filter`);
  await page.locator("[data-testid=ws-chats-row] button").click();
  ok(await colOf(page, "Naming") && /Closed\. Its fold was undone\./.test(await page.locator("[data-testid=ws-chat-ended]").innerText()), `${tag} a row opens the chat, ended`);
  // A workspace with none.
  const other = await page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-name]")].map((e) => e.textContent).find((n) => !n.startsWith("Everything")));
  await page.locator("[data-testid=ws-name]", { hasText: other }).first().click();
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-ov-card-chats] [data-testid=ws-no-chats]")) && !(await page.$("[data-testid=ws-mid]")), `${tag} a workspace with none says so, with New chat, and no chat follows you there`);
  // Reloaded with both running.
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  ok(await until(page, () => !document.querySelector("[data-testid=ws-mid]")), `${tag} a workspace's name is its Overview, in the whole room`);
  void ws;
}

async function narrow(page, tag) {
  await boot(page, ["beta_workspace_chats", "beta_ideas"]);
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  await page.setViewportSize({ width: 1100, height: 800 }); // 1100 − 88 − 300 = 712 of room
  const vis = () => page.evaluate(() => ["[data-testid=ws-mid]", ".wsf-docs"].map((s) => (document.querySelector(s)?.getBoundingClientRect().width ?? 0) > 0));
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-side-chat]")) && JSON.stringify(await vis()) === "[true,false]", `${tag} under 760 of room, one side, with Chat | Documents in the bar`);
  await draft(page).fill("kept");
  await page.click("[data-testid=ws-side-docs]");
  ok(JSON.stringify(await vis()) === "[false,true]", `${tag} Documents shows the other`);
  await row(page, "Who can see what").click();
  ok(JSON.stringify(await vis()) === "[true,false]", `${tag} a chat row goes to the chat`);
  await page.click("[data-testid=ws-side-docs]");
  await tab(page, "memory");
  ok(JSON.stringify(await vis()) === "[false,true]" && await isTab(page, "memory"), `${tag} a document's tab is on the documents side`);
  await row(page, "Release order").click();
  ok(await draft(page).inputValue() === "kept", `${tag} and the draft was kept`);
  await page.setViewportSize({ width: 1440, height: 900 });
  ok(await until(page, () => !document.querySelector("[data-testid=ws-side-chat]")) && JSON.stringify(await vis()) === "[true,true]", `${tag} with room, both`);
  // The width of the documents.
  const bar = await page.locator("[data-testid=ws-split]").boundingBox();
  await page.mouse.move(bar.x, bar.y + 200); await page.mouse.down(); await page.mouse.move(bar.x - 120, bar.y + 200, { steps: 6 }); await page.mouse.up();
  const w = await page.evaluate(() => document.querySelector(".wsf-docs").getBoundingClientRect().width);
  await page.mouse.move(bar.x - 120, bar.y + 200); await page.mouse.down(); await page.mouse.move(40, bar.y + 200, { steps: 6 }); await page.mouse.up();
  const m = await page.evaluate(() => document.querySelector("[data-testid=ws-mid]").getBoundingClientRect().width);
  ok(Math.abs(w - (1052 * 0.46 + 120)) < 8 && m >= 418, `${tag} the bar between them drags, and stops at the chat's floor (${w.toFixed(0)}, ${m.toFixed(0)})`);
}

async function back(page, tag) {
  await boot(page, ["beta_workspace_chats", "beta_ideas"]);
  const step = () => page.waitForTimeout(700);
  const go = async (b) => { await step(); await page.locator(`[data-testid=nav-${b}]:visible`).first().click(); };
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await step(); await row(page, "Release order").click(); await colOf(page, "Release order");
  await step(); await row(page, "Who can see what").click(); await colOf(page, "Who can see what");
  await step(); await tab(page, "plan");
  await step(); await page.locator("[data-testid=ws-group] li.row[data-slug]").first().click();
  await go("back");
  ok(await colOf(page, "Who can see what") && await isTab(page, "plan"), `${tag} Back from a place is the chat and the tab`);
  await go("back");
  ok(await colOf(page, "Who can see what") && await isTab(page, "overview"), `${tag} then the tab before`);
  await go("back");
  ok(await colOf(page, "Release order"), `${tag} then the chat before`);
  ok(/Everything/.test(await page.locator("[data-testid=nav-back]:visible").first().getAttribute("title")) && /Forward to Everything · Who can see what · Overview/.test(await page.locator("[data-testid=nav-forward]:visible").first().getAttribute("title")), `${tag} the buttons name the chat`);
  await go("back");
  ok(await isTab(page, "overview") && !(await page.$("[data-testid=ws-mid]")), `${tag} then the Overview, with no chat`);
  await go("forward");
  ok(await colOf(page, "Release order"), `${tag} and Forward is the chat again`);
}

/** The folder-trust question, first of a workspace: taller than a short pane. */
async function trust(page, tag, name) {
  await boot(page);
  await page.setViewportSize({ width: 1440, height: 560 });
  await page.evaluate(() => {
    const t = window.__TAURI_INTERNALS__, i = t.invoke;
    const lines = ["Accessing workspace:", "", " /Users/demo/.local/share/worktrees/workspaces/01M8884X2QGG0A2ZJHEP1FKQ7J", "", ...Array.from({ length: 14 }, (_, n) => ` Quick safety check, line ${n}: is this a project you created or one you trust?`), "", " Claude Code'll be able to read, edit, and execute files here.", "", " Security guide"];
    t.invoke = (c, a) => (c === "workspace_prompt" ? Promise.resolve({ waiting: true, readable: true, harness: "claude", session: "s", kind: "permission", fingerprint: "fp-trust", lines: window.__trust === false ? ["Edit file"] : lines, choices: [{ n: 1, label: "No, exit", highlighted: true }, { n: 2, label: "Yes, I trust this folder", highlighted: false }], note: "" })
      : c === "workspace_answer" ? (window.__answered = a, Promise.resolve({ answered: true, confirmed: true, harness: "claude", session: "s", choice: a.choice, label: "Yes" })) : i(c, a));
  });
  await row(page, "Who can see what").click();
  await page.waitForSelector("[data-testid=ws-chat-col] .chat-card");
  await page.waitForTimeout(300);
  ok(/This is the workspace's own folder\. Worktrees made it/.test(await page.locator("[data-testid=ws-chat-trust]").innerText()), `${tag} the folder-trust question says what the folder is`);
  const tall = await page.evaluate(() => document.querySelector(".chat-card").getBoundingClientRect().height > document.querySelector(".chat-scroll").getBoundingClientRect().height);
  ok(tall, `${tag} the card is taller than the pane`);
  await page.locator(".chat-scroll").evaluate((e) => { e.scrollTop = e.scrollTop - 60; });
  await page.locator("[data-testid=ws-chat-col] .chat-choice", { hasText: "Yes, I trust this folder" }).first().evaluate((e) => e.click());
  await page.waitForTimeout(200);
  for (const at of ["where it was", "its top"]) {
    if (at === "its top") await page.locator(".chat-scroll").evaluate((e) => { const c = e.querySelector(".chat-card"); e.scrollTop = c.offsetTop - 10; });
    await page.waitForTimeout(150);
    ok(await page.evaluate(() => { const b = document.querySelector("[data-testid=chat-send-answer]"); if (!b) return false; const q = b.getBoundingClientRect(), hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2); return !!hit && (hit === b || b.contains(hit)); }), `${tag} a choice clicked, its Send answer is in view (scrolled to ${at})`);
  }
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsc-trust-${name}.png` });
  await page.click("[data-testid=chat-send-answer]");
  ok(await until(page, () => window.__answered?.choice === 2 && window.__answered?.fingerprint === "fp-trust"), `${tag} …and answers`);
}

async function themes(page, tag, name) {
  await boot(page, ["beta_workspace_chats", "beta_ideas"]);
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  for (const theme of THEMES) {
    await page.evaluate((t) => { if (t === "tokyo-night") delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = t; }, theme);
    for (const t of ["overview", "chats"]) {
      await tab(page, t);
      await page.waitForTimeout(120);
      const low = await page.evaluate(() => {
        const parse = (c) => { let x = /^rgba?\(([^)]+)\)/.exec(c); if (x) { const p = x[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; } x = /^color\(srgb ([^)]+)\)/.exec(c); if (x) { const p = x[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; } return [0, 0, 0, 0]; };
        const over = (a, b) => [0, 1, 2].map((i) => a[i] * a[3] + b[i] * (1 - a[3])).concat(1);
        const bgOf = (el) => { const st = []; for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) st.push(c); if (c[3] >= 1) break; } let acc = [255, 255, 255, 1]; for (const c of st.reverse()) acc = over(c, acc); return acc; };
        const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
        const ratio = (el) => { const bg = bgOf(el); const [a, b] = [lum(over(parse(getComputedStyle(el).color), bg)), lum(bg)].sort((p, q) => q - p); return (a + 0.05) / (b + 0.05); };
        return [...document.querySelectorAll(".wsc-title, .wsc-age.strong, .wsc-all, .wsc-name-t, .wsc-readsb, .wsc-head .ctrl:not(:disabled), .chat-intro, .wsc-model, .wsc-when, .wsc-line .ws-ov-name, .wsc-item .ws-ov-name, .wsf-attn-t")]
          .filter((e) => e.textContent.trim() && e.getBoundingClientRect().width).map((e) => [e.className + ": " + e.textContent.trim().slice(0, 24), ratio(e)]).filter(([, r]) => r < 4.5);
      });
      ok(low.length === 0, `${tag} ${theme} ${t}: its words read ≥ 4.5:1${low.length ? `: ${low.map(([a, r]) => `${a} ${r.toFixed(2)}`).join("; ")}` : ""}`);
    }
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsc-${theme}-${name}.png` });
  }
  // Every control of the column is reachable.
  for (const w of [1600, 1440, 1250]) {
    await page.setViewportSize({ width: w, height: 900 });
    await page.waitForTimeout(250);
    const bad = await page.evaluate(() => [...document.querySelectorAll("[data-testid=ws-chat-col] .wsc-head button, .wsf-top button")].filter((b) => { if (b.disabled) return false; const q = b.getBoundingClientRect(); const hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2); return !q.width || !(hit && (b === hit || b.contains(hit))); }).map((b) => b.dataset.testid));
    const head = await page.evaluate(() => document.querySelector(".wsc-head").getBoundingClientRect().height);
    ok(bad.length === 0 && head < 42, `${tag} ${w}px: the chat's header is one row, every control reachable${bad.length ? `: ${bad}` : ""} (${head})`);
  }
}

// The model in the chat's header, as a place's agent label names it, and a
// `/model` switch shown by the next push with nothing reloaded. Measured at
// the narrowest nav and a normal one, in a light theme too.
async function model(page, tag, name) {
  const ws = await boot(page, ["beta_workspace_chats", "beta_ideas"], { nav_width: 240 });
  const conv = await row(page, "Release order").getAttribute("data-chat");
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  const words = () => page.evaluate(() => document.querySelector("[data-testid=ws-chat-model]")?.textContent ?? null);
  ok(await until(page, () => document.querySelector("[data-testid=ws-chat-model]")?.textContent === "Claude Opus 5.5"), `${tag} the header names the chat's model (${await words()})`);
  await page.evaluate(([w, c]) => window.__mock.wsChatModel(w, c, "Fable 5.2"), [ws, conv]);
  ok(await until(page, () => document.querySelector("[data-testid=ws-chat-model]")?.textContent === "Claude Fable 5.2"), `${tag} a /model switch shows at the next push, with no restart`);
  for (const [theme, width] of [["tokyo-night", 1440], ["tokyo-day", 1440], ["tokyo-day", 1100], ["catppuccin-latte", 900]]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate((t) => { if (t === "tokyo-night") delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = t; }, theme);
    await page.waitForTimeout(200);
    const m = await page.evaluate(() => {
      const e = document.querySelector("[data-testid=ws-chat-model]"), head = document.querySelector(".wsc-head");
      if (!e) return { gone: true, head: head.getBoundingClientRect().height };
      const q = e.getBoundingClientRect(), hit = q.width ? document.elementFromPoint(q.x + Math.min(q.width / 2, 8), q.y + q.height / 2) : null;
      const nav = document.querySelector(".nav")?.getBoundingClientRect().width ?? 0;
      const ctrls = [...head.querySelectorAll("button")].filter((b) => !b.disabled).every((b) => { const r = b.getBoundingClientRect(); const h = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2); return r.width > 0 && h && (b === h || b.contains(h)); });
      return { w: q.width, hit: !!hit && e.contains(hit), head: head.getBoundingClientRect().height, nav, ctrls, color: getComputedStyle(e).color };
    });
    ok((m.gone || (m.hit && m.w > 0)) && m.head < 42 && m.ctrls !== false, `${tag} ${theme} ${width}px (nav ${m.nav}): the model is on one header row and hit-tests, every control still reachable (${JSON.stringify(m)})`);
    if (SHOTS && name === "webkit") await page.screenshot({ path: `${SHOTS}/wsc-model-${theme}-${width}.png` });
  }
  // Ended: no model.
  await page.evaluate(([w, c]) => window.__mock.endChat(w, c), [ws, conv]);
  ok(await until(page, () => !document.querySelector("[data-testid=ws-chat-model]")), `${tag} an ended chat names no model`);
}

// A chat, or its fold, that waits is told by a system notification while the
// window is hidden, under the place agents' setting; never while it shows,
// never in Off, never from the first push, and once.
async function notified(page, tag) {
  await page.addInitScript(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (window.__hidden ? "hidden" : "visible") });
  });
  const ws = await boot(page, ["beta_workspace_chats", "beta_ideas"], { notify_hidden: "waiting" });
  const sent = () => page.evaluate(() => window.__mock.notifications());
  const hide = (h) => page.evaluate((h) => { window.__hidden = h; document.dispatchEvent(new Event("visibilitychange")); }, h);
  await page.waitForTimeout(400);
  ok((await sent()).length === 0, `${tag} what already waits at load is a baseline, not news`);
  const rel = await row(page, "Release order").getAttribute("data-chat");
  await hide(true);
  await call(page, "workspace_send", { id: ws, conv: rel, text: "edit the index" });
  ok(await until(page, () => window.__mock.notifications().length === 1, null, 4000), `${tag} a chat that stops on a prompt while the window is hidden is told`);
  const [n] = await sent();
  ok(n?.title === "A workspace chat is waiting for you" && n?.body === "“Release order” in Everything stopped on a question.", `${tag} …in words that name it and its workspace (${JSON.stringify(n)})`);
  await page.evaluate(([w, c]) => window.__mock.wsChatModel(w, c, "Fable 5.2"), [ws, rel]);
  await page.waitForTimeout(300);
  ok((await sent()).length === 1, `${tag} a push that changes something else tells nothing again`);
  const sto = await row(page, "Storage layout").getAttribute("data-chat");
  await page.evaluate(([w, c]) => window.__mock.wsFold(w, c, "waiting"), [ws, sto]);
  ok(await until(page, () => window.__mock.notifications().length === 2, null, 3000) && /^The fold of “Storage layout” in Everything stopped on a question\.$/.test((await sent())[1]?.body ?? ""), `${tag} a fold stopped on a prompt is told as the fold's (${JSON.stringify((await sent())[1])})`);
  await page.evaluate(([w, c]) => window.__mock.wsFold(w, c, "failed", "no note in five minutes"), [ws, sto]);
  ok(await until(page, () => window.__mock.notifications().length === 3, null, 3000) && (await sent())[2]?.title === "A fold did not finish", `${tag} and a fold that failed (${JSON.stringify((await sent())[2])})`);
  await page.evaluate(([w, c]) => window.__mock.wsFold(w, c, null), [ws, sto]);
  await hide(false);
  const first = await row(page, "First look").getAttribute("data-chat");
  await call(page, "workspace_chat_resume", { id: ws, conv: first });
  await call(page, "workspace_send", { id: ws, conv: first, text: "change a line" });
  await until(page, (c) => document.querySelector(`[data-chat='${c}']`)?.dataset.state === "waiting", first, 4000);
  ok((await sent()).length === 3, `${tag} nothing while the window shows`);
}

async function off(page, tag) {
  await boot(page, ["beta_ideas"]);
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-testid=ws-overview]");
  ok(!(await page.$("[data-testid=ws-chats]")) && !(await page.$("[data-testid=ws-frame-new-chat]")) && !(await page.$("[data-testid=nav-ws-waits]")) && !(await page.$("[data-testid=ws-attention][data-kind=chat]")), `${tag} with Ideas alone there is nothing of chats, though one waits`);
  await page.keyboard.press("Meta+k");
  await page.fill(".qs-input", "who can");
  ok(await until(page, () => !!document.querySelector(".qs-empty")), `${tag} …nor in ⌘K`);
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [nav, chat, rowmenu, picker, start, narrow, back, trust, themes, model, notified, off]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    page.on("console", (m) => { if (m.type() === "error" && /Maximum update depth|Cannot update a component|unique "key"/.test(m.text())) errors.push(m.text().slice(0, 200)); });
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag, name); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsc-${name}-${part.name}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\nworkspace-chats-browser: ${failed} failure(s)` : "\nworkspace-chats-browser: all good");
process.exit(failed ? 1 : 0);
