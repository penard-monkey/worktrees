// Browser gate for the fold on screen (docs/proposals/workspace-view-design.md
// §5.1 to §5.3: WorkspaceChat.tsx), in Chromium AND WebKit against the mock.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/workspace-fold-browser.mjs
//
// What it holds: Close folds, at once on screen; one line in the composer's place,
// with Show the terminal and Stop the fold; what the engine refuses, in its words;
// a fold that waits, its prompt card; landing: the receipt among the turns, the diff
// beside it unless the person edits (then the dot); no memory changes; failed, with
// what it holds, Try again and Close without folding; checking; the ended lines, Fold
// now, a transcript that is gone; Undo and Redo; the nav row; a change's live chats;
// what a chat reads of the plan and the ideas; a fold twice; six themes.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 6000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const row = (page, title) => page.locator("[data-testid=ws-chat-row]", { hasText: title }).first();
const colOf = (page, title) => until(page, (t) => document.querySelector("[data-testid=ws-chat-col] .wsc-name-t")?.textContent === t, title);
const tab = (page, t) => page.click(`[data-testid=ws-tab][data-tab=${t}]`);
const isTab = (page, t) => until(page, (t) => document.querySelector("[data-testid=ws-tab].on")?.dataset.tab === t, t, 2000);
const text = (page, id) => page.evaluate((id) => document.querySelector(`[data-testid=${id}]`)?.textContent ?? "", id);
const fold = (page, ws, conv, st, why) => page.evaluate(([w, c, s, y]) => window.__mock.wsFold(w, c, s, y), [ws, conv, st, why ?? null]);

async function boot(page, query = "&folding=0&foldms=900", betas = ["beta_workspace_chats", "beta_ideas"]) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", chat_view: true, ...Object.fromEntries(betas.map((b) => [b, true])) });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&memtick=150${query}`);
  await page.waitForSelector("[data-testid=ws-chat-row]");
  const ws = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
  const id = async (t) => row(page, t).getAttribute("data-chat");
  return { ws, id };
}

/** A chat nobody has written to, folded: the opener is the engine's, not a message. */
async function unsent(page, tag) {
  const { ws } = await boot(page);
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.click("[data-testid=ws-frame-new-chat]");
  const named = () => page.evaluate(() => document.querySelector("[data-testid=ws-chat-col] .wsc-name-t")?.textContent ?? "");
  await until(page, () => /^Chat \d+$/.test(document.querySelector("[data-testid=ws-chat-col] .wsc-name-t")?.textContent ?? "") && document.querySelector("[data-testid=ws-chat-col]").dataset.state === "running", null, 10000);
  const was = await named();
  await page.click("[data-testid=ws-chat-close]");
  ok(await until(page, () => /^Closed and folded/.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? ""), null, 10000), `${tag} a chat with no message is folded`);
  await page.waitForTimeout(400);
  ok(await named() === was && /^Chat \d+$/.test(was), `${tag} …and keeps its name (${await named()})`);
  ok(!/chats\/|fold\.md/.test(await page.locator("[data-testid=ws-chats], [data-testid=nav]").first().innerText().catch(() => "")) && !(await call(page, "workspace_chats", { id: ws })).chats.some((c) => /^Read /.test(c.title)), `${tag} …in the list too`);
  const list = await page.locator("[data-testid=chat-list]").innerText();
  ok(/The chat was asked to fold into memory\./.test(list) && !/fold\.md|\/Users\//.test(list) && await page.locator("[data-testid=chat-user]").count() === 0, `${tag} the opener is a quiet line: no bubble, no path, no id`);
  // The engine's word is its mark: the same words from a person are their message.
  await page.click("[data-testid=ws-chat-resume]");
  await until(page, () => document.querySelector("[data-testid=ws-chat-col]").dataset.state === "running", null, 6000);
  const conv = await page.evaluate(() => document.querySelector("[data-testid=ws-chat-col]").dataset.chat);
  const box = page.locator("[data-testid=ws-chat-col] textarea").first();
  await box.fill(`Read chats/${conv}/start.md and begin.`);
  await box.press("Enter");
  ok(await until(page, () => document.querySelectorAll("[data-testid=ws-chat-col] [data-testid=chat-user]").length === 1 && /start\.md and begin/.test(document.querySelector("[data-testid=ws-chat-col] [data-testid=chat-user]").textContent), null, 8000), `${tag} the same words from a person are their message, in a bubble`);
  ok(await until(page, () => document.querySelector("[data-testid=ws-chat-col]").dataset.state === "running", null, 8000) && /^Chat \d+$/.test(await named()), `${tag} …which the engine will not take for a name (${await named()})`);
  ok(!/Could not name/.test(await page.locator("body").innerText()), `${tag} …and nobody is told of that`);
}

async function land(page, tag, name) {
  const { ws, id } = await boot(page);
  const rel = await id("Release order");
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  await tab(page, "plan");
  // (That the list's own word is drawn before a slower push is wschats-check.mjs's:
  // the mock pushes at once.)
  await page.click("[data-testid=ws-chat-close]");
  ok(await until(page, () => /Folding into memory…/.test(document.querySelector("[data-testid=ws-chat-folding]")?.textContent ?? ""), null, 700), `${tag} Close folds, and says so at once`);
  ok(await page.locator("[data-testid=ws-chat-col] textarea").count() === 0 && !!(await page.$("[data-testid=chat-list] li")) && await page.locator("[data-testid=ws-chat-folding] button").evaluateAll((b) => b.map((x) => x.textContent).join()) === "Show the terminal,Stop the fold", `${tag} the composer gives way to one line, the steps stay`);
  ok(/folding…/.test(await row(page, "Release order").innerText()) && await row(page, "Release order").getAttribute("data-state") === "folding" && await row(page, "Release order").locator(".wsc-dot.folding").evaluate((e) => getComputedStyle(e).boxShadow !== "none"), `${tag} its row says folding, with the halo`);
  ok(!/\d+ ?%|usually|under a minute/i.test(await page.locator("[data-testid=ws-chat-col]").innerText()), `${tag} no percent, and no promise of how long`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsf-folding-${name}.png` });
  ok(await until(page, () => /^Closed and folded into memory\./.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? "")), `${tag} it lands: closed and folded`);
  ok(await isTab(page, "memory") && await until(page, () => !!document.querySelector("[data-testid=memory-change] .wsm-change-h.fold") && !!document.querySelector("[data-testid=memory-diff]")), `${tag} the documents' side shows the diff`);
  ok(/Changed by the fold of “Release order”/.test(await text(page, "memory-change")), `${tag} …of that fold`);
  ok(await until(page, () => !!document.querySelector("[data-testid=chat-list] [data-testid=ws-chat-receipt]")) && /Folded into memory.*2 files \+\d+( −[1-9]\d*)?Show/.test(await text(page, "ws-chat-receipt")), `${tag} the receipt ends the chat (${await text(page, "ws-chat-receipt")})`);
  ok(await page.evaluate(() => { const l = document.querySelector("[data-testid=chat-list]"); return l.lastElementChild.contains(document.querySelector("[data-testid=ws-chat-receipt]")); }), `${tag} …after its turns`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsf-landed-${name}.png` });
  // ── the note: named by the fold, called by its heading, its path the hover text ──
  const note = (await call(page, "workspace_chats", { id: ws })).chats.find((c) => c.id === rel).note.replace(/^memory\//, "");
  const cf = await page.locator(`[data-testid=memory-change-file][data-path='${note}']`).evaluate((b) => [b.querySelector(".wsm-cfile-n").textContent, b.title]);
  ok(/^notes\/\d{4}-\d\d-\d\d-release-order-\w{6}\.md$/.test(note) && cf[1] === note && !/^notes\//.test(cf[0]) && cf[0].length > 0, `${tag} the diff's file list says the note's heading (${cf.join(" | ")})`);
  await page.click(`[data-testid=memory-change-file][data-path='${note}']`);
  ok(await page.evaluate((n) => { const s = document.querySelector(".wsm-one-h span"); return s.title === n && !s.textContent.startsWith("notes/"); }, note), `${tag} …and so does its diff`);
  await page.click("[data-testid=memory-hist-back]");
  ok(!(await page.locator("[data-testid=memory-history]").innerText()).includes(note), `${tag} …and History`);
  // ── the receipt's acts ──
  await tab(page, "plan");
  await page.click("[data-testid=ws-receipt-diff]");
  ok(await isTab(page, "memory") && await until(page, () => !!document.querySelector("[data-testid=memory-change]")), `${tag} Show the diff`);
  await page.click("[data-testid=ws-receipt-note]");
  ok(await until(page, (c) => document.querySelector("[data-testid=main-doc]")?.dataset.doc === c, (await call(page, "workspace_chats", { id: ws })).chats.find((c) => c.id === rel).note.replace(/^memory\//, "")), `${tag} Open the note`);
  ok(!(await page.locator("[data-testid=main-doc] .ix-title").first().innerText()).includes(note.slice(6)) && (await page.locator("[data-testid=main-doc] .ix-title").first().innerText()) === cf[0], `${tag} …under its heading`);
  // ── Undo, Redo ──
  const undo = page.locator("[data-testid=ws-chat-receipt] [data-testid=memory-revert]");
  await undo.click();
  ok(/Undo this fold\?/.test(await undo.innerText()) && !(await call(page, "workspace_chats", { id: ws })).chats.find((c) => c.id === rel).fold.reverted, `${tag} Undo takes two presses`);
  await undo.click();
  ok(await until(page, () => /Folded into memory, and undone/.test(document.querySelector("[data-testid=ws-chat-receipt]")?.textContent ?? "")), `${tag} the second undoes it, and the receipt says so`);
  ok(await until(page, () => /^Closed\. Its fold was undone\./.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? "") && !!document.querySelector("[data-testid=ws-chat-fold-now]")), `${tag} the chat's line too, with Fold now`);
  ok((await undo.innerText()) === "Redo", `${tag} and it has Redo`);
  await undo.click(); await undo.click();
  ok(await until(page, () => /^Closed and folded/.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? "")), `${tag} which puts it back`);
  // ── the same chat, folded twice ──
  const log = async () => (await call(page, "workspace_memory_log", { id: ws })).changes.filter((c) => /^fold: Release order$/.test(c.subject)).length;
  const first = await log();
  await page.click("[data-testid=ws-chat-resume]");
  await until(page, () => !!document.querySelector("[data-testid=ws-chat-col] textarea"));
  await page.locator("[data-testid=ws-chat-col] textarea").fill("One more thing");
  await page.locator("[data-testid=ws-chat-col] textarea").press("Enter");
  ok(await until(page, () => /Noted: “One more thing”/.test(document.querySelector("[data-testid=chat-list]")?.textContent ?? ""), null, 10000), `${tag} resumed, it goes on`);
  ok(await page.evaluate(() => { const r = document.querySelector("[data-testid=ws-chat-receipt]"), all = [...document.querySelectorAll("[data-testid=chat-list] [data-testid=chat-user]")]; return !!(r.compareDocumentPosition(all.at(-1)) & Node.DOCUMENT_POSITION_FOLLOWING); }), `${tag} …after the receipt, which stays where the fold was`);
  await until(page, () => document.querySelector("[data-testid=ws-chat-col]")?.dataset.state === "running", null, 8000);
  await page.click("[data-testid=ws-chat-close]");
  await page.waitForSelector("[data-testid=ws-chat-folding]");
  await page.evaluate((w) => window.__mock.memoryEdit(w, "topics/second.md", "# What the second fold wrote\n"), ws);
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-ended]"), null, 8000) && await log() === first + 1, `${tag} closed again, it folds again: one more change, and no double`);
}

async function edits(page, tag) {
  const { ws, id } = await boot(page);
  const rel = await id("Release order");
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  await tab(page, "memory");
  await page.waitForSelector("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.click("[data-testid=ws-side-toggle]");
  await page.click("[data-testid=memory-file][data-path='topics/nas-layout.md']");
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await until(page, () => !!document.querySelector("[data-testid=main-doc] .cm-content"));
  await page.click("[data-testid=ws-chat-close]");
  ok(await until(page, () => /^Closed and folded/.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? "")), `${tag} a fold lands while you edit`);
  await page.waitForTimeout(500);
  ok(!(await page.$("[data-testid=memory-history]")) && await page.evaluate(() => document.querySelector("[data-testid=main-doc]").dataset.doc === "topics/nas-layout.md"), `${tag} …and your document stays: no diff over it`);
  await tab(page, "plan");
  await fold(page, ws, rel, "folding");
  await until(page, () => !!document.querySelector("[data-testid=ws-chat-folding]"));
  // Away from the chat, it lands with the dot alone.
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.evaluate(() => localStorage.getItem("x"));
  await fold(page, ws, rel, "landed");
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-docs] [data-testid=memory-fresh]")), `${tag} one that lands elsewhere is the Memory chip's dot`);
  ok(await isTab(page, "overview"), `${tag} …and moves nothing`);
  // ── closed without a fold, with changes left ──
  await row(page, "Who can see what").click();
  await colOf(page, "Who can see what");
  await tab(page, "overview");
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/guests.md", "# Guests\n\nLeft behind.\n"), ws);
  await page.click("[data-testid=ws-chat-close-menu]");
  await page.click("[data-testid=ws-chat-close-item]");
  ok(await until(page, () => /^Closed, not folded\./.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? "")) && await until(page, () => /Changed: Chat closed: Who can see what/.test(document.querySelector("[data-testid=memory-change]")?.textContent ?? "")), `${tag} closed without a fold, what it left is shown the same way`);
}

async function states(page, tag, name) {
  const { ws, id } = await boot(page);
  const [rel, who] = [await id("Release order"), await id("Who can see what")];
  // ── refused ──
  await row(page, "Who can see what").click();
  await colOf(page, "Who can see what");
  await page.click("[data-testid=ws-chat-close]");
  ok(await until(page, () => /^It is waiting on you: answer it first\./.test(document.querySelector("[data-testid=ws-chat-said]")?.textContent ?? "")), `${tag} a chat that waits is not folded, in the engine's words`);
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  ok(!(await page.$("[data-testid=ws-chat-said]")), `${tag} …which are that chat's`);
  await page.locator("[data-testid=ws-chat-col] textarea").fill("Work on it");
  await page.locator("[data-testid=ws-chat-col] textarea").press("Enter");
  await until(page, () => document.querySelector("[data-testid=ws-chat-col]")?.dataset.state === "busy");
  await page.click("[data-testid=ws-chat-close]");
  ok(await until(page, () => /^It is working: stop it or wait\./.test(document.querySelector("[data-testid=ws-chat-said]")?.textContent ?? "")) && !(await page.$("[data-testid=ws-chat-folding]")), `${tag} nor is one that works: nothing closes`);
  await page.click("[data-testid=ws-chat-said-stop]");
  ok(await until(page, () => document.querySelector("[data-testid=ws-chat-col]")?.dataset.state === "running", null, 8000), `${tag} …with Stop beside it`);
  // ── waiting ──
  await fold(page, ws, rel, "waiting");
  ok(await until(page, () => /The fold waits on your answer\. Its clock has stopped\./.test(document.querySelector("[data-testid=ws-chat-folding]")?.textContent ?? "") && !!document.querySelector("[data-testid=ws-chat-col] .chat-card")), `${tag} a fold that waits: its prompt card, and the clock has stopped`);
  ok(/^waiting/.test(await row(page, "Release order").locator(".wsc-age").innerText()) && !!(await page.$("[data-testid=nav-ws-waits]")), `${tag} its row waits, and the switch's dot`);
  await page.locator("[data-testid=ws-chat-col] .chat-choice").first().click();
  await page.click("[data-testid=chat-send-answer]");
  ok(await until(page, () => /Folding into memory…/.test(document.querySelector("[data-testid=ws-chat-folding]")?.textContent ?? ""), null, 8000), `${tag} answered, it folds on`);
  // ── checking ──
  await fold(page, ws, rel, "checking");
  ok(await until(page, () => /Checking the fold…/.test(document.querySelector("[data-testid=ws-chat-folding]")?.textContent ?? "")) && /checking…/.test(await row(page, "Release order").innerText()), `${tag} after a restart: Checking the fold…`);
  // ── one at a time ──
  await page.evaluate(([w, c]) => { window.__mock.endChat(w, c); }, [ws, who]);
  await row(page, "Who can see what").click();
  await colOf(page, "Who can see what");
  await page.waitForSelector("[data-testid=ws-chat-fold-now]");
  await page.click("[data-testid=ws-chat-fold-now]");
  ok(await until(page, () => /“Release order” is folding\. Close this one after\./.test(document.querySelector("[data-testid=ws-chat-said]")?.textContent ?? "")), `${tag} one fold at a time, naming the other`);
  // ── failed ──
  await page.evaluate((w) => window.__mock.memoryEdit(w, "topics/half.md", "# Half\n"), ws);
  await fold(page, ws, rel, "failed");
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  ok(await until(page, () => /^The fold did not finish: it wrote no note in five minutes, and the chat is still open\. Memory may hold part of it: 1 file changed\./.test(document.querySelector("[data-testid=ws-chat-fold-failed]")?.textContent ?? "")), `${tag} failed: what memory holds, as the engine counted (${await text(page, "ws-chat-fold-failed")})`);
  ok(await row(page, "Release order").locator(".wsc-dot.fold-failed").count() === 1 && /fold failed/.test(await row(page, "Release order").innerText()) && await page.locator("[data-testid=ws-chat-col] textarea").count() === 1, `${tag} red in the nav; the chat is open, and takes a message`);
  ok(await page.locator("[data-testid=ws-chat-fold-failed] button").evaluateAll((b) => b.map((x) => x.textContent).join()) === "See memory,Try again,Close without folding", `${tag} with Try again and Close without folding`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsf-failed-${name}.png` });
  await fold(page, ws, rel, "failed", "its Claude exited and wrote no note");
  ok(await until(page, () => /^The fold did not finish: its Claude exited and wrote no note\./.test(document.querySelector("[data-testid=ws-chat-fold-failed]")?.textContent ?? "")) && !/unchanged/i.test(await text(page, "ws-chat-fold-failed")), `${tag} it never says memory is unchanged on its own`);
  await page.click("[data-testid=ws-chat-fold-again]");
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-folding]") && !document.querySelector("[data-testid=ws-chat-fold-failed]")), `${tag} Try again folds`);
  ok(await until(page, () => !!document.querySelector("[data-testid=ws-chat-ended]"), null, 8000), `${tag} …and lands`);
  // ── nothing to keep ──
  await page.click("[data-testid=ws-chat-resume]");
  await until(page, () => !!document.querySelector("[data-testid=ws-chat-col] textarea"));
  await fold(page, ws, rel, "folding");
  await until(page, () => !!document.querySelector("[data-testid=ws-chat-folding]"));
  await fold(page, ws, rel, "nothing");
  ok(await until(page, () => /^Closed\. No memory changes\./.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? "") && !document.querySelector("[data-testid=ws-chat-receipt]")), `${tag} nothing to keep: No memory changes, and no receipt`);
  // ── Stop ──
  await page.click("[data-testid=ws-chat-resume]");
  await until(page, () => !!document.querySelector("[data-testid=ws-chat-col] textarea"));
  await page.click("[data-testid=ws-chat-close]");
  await page.waitForSelector("[data-testid=ws-chat-fold-stop]");
  await page.click("[data-testid=ws-chat-fold-stop]");
  ok(await until(page, () => /^Closed, not folded\./.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? "")), `${tag} Stop the fold ends it: closed, not folded`);
  // ── a transcript that is gone ──
  await page.evaluate(() => { const t = window.__TAURI_INTERNALS__, i = t.invoke; t.invoke = (c, a) => (c === "workspace_turns" ? Promise.resolve({ harness: "claude", session: "s", turns: [], more: false, ended: true, reason: "this conversation's transcript is no longer on disk", note: "" }) : i(c, a)); });
  await tab(page, "chats");
  await page.locator("[data-testid=ws-chats-row] .wsc-item-b", { hasText: "First look" }).click();
  ok(await until(page, () => /^Closed\. Its transcript is gone\./.test(document.querySelector("[data-testid=ws-chat-ended]")?.textContent ?? "")) && !(await page.$("[data-testid=ws-chat-resume]")) && !(await page.$("[data-testid=ws-chat-fold-now]")), `${tag} a transcript that is gone: said, with neither Resume nor Fold now`);
}

/** The terminal under a fold: its session is replaced, so it attaches again. */
async function terminal(page, tag) {
  const { } = await boot(page, "&folding=0&foldms=4000");
  await page.evaluate(() => { window.__opens = 0; const t = window.__TAURI_INTERNALS__, i = t.invoke; t.invoke = (c, a) => { if (c === "workspace_term_open") window.__opens++; return i(c, a); }; });
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  await page.click("[data-testid=ws-chat-view-terminal]");
  await until(page, () => window.__opens > 0 && !!document.querySelector("[data-testid=ws-chat-term] .xterm"));
  const before = await page.evaluate(() => window.__opens);
  await page.click("[data-testid=ws-chat-close]");
  ok(await until(page, (n) => window.__opens > n, before) && !!(await page.$("[data-testid=ws-chat-term] .xterm")), `${tag} the terminal stays on, and attaches again`);
}

async function lists(page, tag) {
  const { ws, id } = await boot(page, "&foldms=900&memquiet=200");
  ok(await row(page, "Subtitles pipeline").getAttribute("data-state") === "folding", `${tag} the seeded folding chat is one`);
  await call(page, "workspace_link_idea", { id: ws, idea: "01JC8Z4K3M9QX2T7VB00000901" });
  const plan = (await call(page, "plan_make", { ideas: ["01JC8Z4K3M9QX2T7VB00000901"], title: "Jelly" })).id;
  await call(page, "workspace_set_plan", { id: ws, plan });
  await fold(page, ws, await id("Subtitles pipeline"), null);
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  await page.click("[data-testid=ws-chat-readsb]");
  ok(await until(page, () => [...document.querySelectorAll("[data-testid=ws-chat-reads-doc]")].map((l) => l.textContent).join("|") === "Plan: Jellyplan · read|Jellyfin on the home serveridea · read"), `${tag} its reads menu lists the plan and the linked ideas, under the projects (${await page.locator("[data-testid=ws-chat-reads-doc]").allInnerTexts()})`);
  await page.keyboard.press("Escape");
  // ── a change's live chats ──
  await tab(page, "memory");
  await page.waitForSelector("[data-testid=ws-side-toggle]");
  await page.click("[data-testid=ws-side-toggle]");
  await page.evaluate((w) => window.__mock.memoryEdit(w, "topics/live.md", "# Live\n\nWritten.\n"), ws);
  await page.click("[data-testid=memory-file][data-path='MEMORY.md']");
  await page.waitForTimeout(1200);
  if (await page.$("[data-testid=main-doc] [data-testid=ideas-versions-toggle]")) await page.click("[data-testid=main-doc] [data-testid=ideas-versions-toggle]"); else { await page.click("[data-testid=main-doc] [data-testid=doc-menu-open]"); await page.click("[data-testid=doc-menu-history]"); }
  await page.waitForSelector("[data-testid=memory-history]");
  await page.click("[data-testid=memory-hist-all]");
  ok(await until(page, () => /while “.*Release order.*” .*were live/.test([...document.querySelectorAll("[data-testid=memory-log-row]")].map((r) => r.textContent).join("\n")) && !/ by “/.test(document.querySelector("[data-testid=memory-history]").textContent)), `${tag} History says while which chats were live, never by`);
  // One of them deleted since.
  await call(page, "workspace_chat_remove", { id: ws, conv: await id("Who can see what") });
  await page.evaluate(([w, c]) => window.__mock.wsFold(w, c, null), [ws, await id("Release order")]);
  ok(await until(page, () => /a chat that was deleted/.test(document.querySelector("[data-testid=memory-history]")?.textContent ?? "")), `${tag} …and one deleted since is that`);
}

async function themes(page, tag, name) {
  const { ws, id } = await boot(page);
  const rel = await id("Release order");
  await row(page, "Release order").click();
  await colOf(page, "Release order");
  for (const st of ["folding", "failed", "landed"]) {
    await fold(page, ws, rel, st === "landed" ? "folding" : st);
    if (st === "landed") { await page.waitForTimeout(300); await fold(page, ws, rel, "landed"); await page.waitForSelector("[data-testid=ws-chat-receipt]"); }
    await page.waitForTimeout(400);
    for (const theme of THEMES) {
      await page.evaluate((t) => { if (t === "tokyo-night") delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = t; }, theme);
      const low = await page.evaluate(() => {
        const parse = (c) => { let x = /^rgba?\(([^)]+)\)/.exec(c); if (x) { const p = x[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; } x = /^color\(srgb ([^)]+)\)/.exec(c); if (x) { const p = x[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; } return [0, 0, 0, 0]; };
        const over = (a, b) => [0, 1, 2].map((i) => a[i] * a[3] + b[i] * (1 - a[3])).concat(1);
        const bgOf = (el) => { const st = []; for (let e = el; e; e = e.parentElement) { const c = parse(getComputedStyle(e).backgroundColor); if (c[3] > 0) st.push(c); if (c[3] >= 1) break; } let acc = [255, 255, 255, 1]; for (const c of st.reverse()) acc = over(c, acc); return acc; };
        const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
        const ratio = (el) => { const bg = bgOf(el); const [a, b] = [lum(over(parse(getComputedStyle(el).color), bg)), lum(bg)].sort((p, q) => q - p); return (a + 0.05) / (b + 0.05); };
        return [...document.querySelectorAll(".wsc-ended > span, .wsc-said > span, .wsc-receipt b, .wsc-age.strong, .wsc-ended .ctrl:not(:disabled), .wsc-said .ctrl:not(:disabled), .wsc-receipt .ctrl:not(:disabled)")]
          .filter((e) => e.textContent.trim() && e.getBoundingClientRect().width).map((e) => [e.className + ": " + e.textContent.trim().slice(0, 24), ratio(e)]).filter(([, r]) => r < 4.5);
      });
      ok(low.length === 0, `${tag} ${theme} ${st}: its words read ≥ 4.5:1${low.length ? `: ${low.map(([a, r]) => `${a} ${r.toFixed(2)}`).join("; ")}` : ""}`);
    }
    for (const w of [1440, 900]) {
      await page.setViewportSize({ width: w, height: 800 });
      await page.waitForTimeout(250);
      if (await page.$("[data-testid=ws-side-chat]")) await page.click("[data-testid=ws-side-chat]");
      const bad = await page.evaluate(() => [...document.querySelectorAll(".wsc-ended button:not(:disabled), .wsc-said button:not(:disabled), .wsc-receipt button:not(:disabled), .wsc-head button:not(:disabled)")].filter((b) => { b.scrollIntoView({ block: "nearest" }); const q = b.getBoundingClientRect(), hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2); return !q.width || !(hit && (b === hit || b.contains(hit))); }).map((b) => b.dataset.testid));
      ok(bad.length === 0, `${tag} ${w}px ${st}: every control is reachable${bad.length ? `: ${bad}` : ""}`);
    }
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.evaluate(() => { delete document.documentElement.dataset.theme; });
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsf-${st}-theme-${name}.png` });
  }
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [unsent, land, edits, states, terminal, lists, themes]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    page.on("console", (m) => { if (m.type() === "error" && /Maximum update depth|Cannot update a component|unique "key"/.test(m.text())) errors.push(m.text().slice(0, 200)); });
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag, name); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsf-${name}-${part.name}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\nworkspace-fold-browser: ${failed} failure(s)` : "\nworkspace-fold-browser: all good");
process.exit(failed ? 1 : 0);
