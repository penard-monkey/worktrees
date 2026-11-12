// Browser gate for the Ideas window's conversations (phase S, app/companion/
// IdeaChat.tsx), in Chromium AND WebKit (the app is WKWebView; the harness is
// Chrome — AGENTS.md), against the mock.
//
//   VITE_MOCK=1 vite --port 1477 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1477 node app/scripts/ideas-sessions-browser.mjs
// IDEAS_SCREENSHOTS=<dir> saves the window per engine.
//
// What it holds:
//   - an idea with no conversation offers New conversation; starting one
//     opens the chat beside the editor, and Claude's first turn shows;
//   - the list dots the idea (`ideas:sessions`): busy, then waiting on an
//     edit prompt; approving it in the chat changes the idea's file and the
//     editor shows the new line (clean: reloaded; dirty: "changed under you");
//   - every call went to an `idea_*` row, never an `agent_*` one; the
//     composer has no attach button and no slash menu;
//   - a second conversation on the same idea; the menu lists both, newest
//     first, each with its state and start date;
//   - close ends it: read-only, with Resume, and the transcript still shows;
//     resume brings the composer back; rename; remove asks first;
//   - the header and the menu are reachable (elementFromPoint) at 1200 px.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1477";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const JELLY = "01JC8Z4K3M9QX2T7VB00000901";

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 6000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const reachable = (page, sel) => page.evaluate((sel) => {
  const el = document.querySelector(sel);
  if (!el) return false;
  const r = el.getBoundingClientRect();
  const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
  return !!hit && (hit === el || el.contains(hit));
}, sel);

async function run(engine, name) {
  const tag = `[${name}]`;
  const browser = await engine.launch();
  const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  // Every row the page asks, from the mock's door.
  const calls = [];
  await page.exposeFunction("__noteCall", (c) => calls.push(c));
  await page.addInitScript(() => {
    const wrap = () => {
      const t = window.__TAURI_INTERNALS__;
      if (!t?.invoke || t.__wrapped) return false;
      const inner = t.invoke.bind(t);
      t.invoke = (cmd, args, opts) => { window.__noteCall(cmd); return inner(cmd, args, opts); };
      t.__wrapped = true;
      return true;
    };
    const id = setInterval(() => { if (wrap()) clearInterval(id); }, 5);
  });
  await page.goto(`${BASE}/companion/index.html?ideas=seed&ideareply=250&ideastick=150`);
  await page.waitForSelector("[data-testid=ideas-rows] li");
  await page.click(`[data-idea='${JELLY}'] button.ix-row-main`);
  await until(page, () => document.querySelector(".cm-content"));

  // ── the panel, empty ──
  await page.click("[data-testid=ideas-chat-toggle]");
  ok(await until(page, () => document.querySelector("[data-testid=idea-conv-first]")), `${tag} an idea with no conversation offers New conversation`);
  ok(await reachable(page, "[data-testid=ideas-chat-toggle]") && await reachable(page, "[data-testid=idea-conv-first]"), `${tag} the toggle and New conversation are reachable`);
  const editorW = await page.evaluate(() => document.querySelector(".ix-editor").getBoundingClientRect().width);
  ok(editorW > 300, `${tag} the editor keeps room beside the chat (${Math.round(editorW)} px)`);

  // ── start ──
  await page.click("[data-testid=idea-conv-first]");
  ok(await until(page, () => document.querySelector("[data-testid=chat] .chat-bubble") && [...document.querySelectorAll(".chat-bubble")].some((b) => b.textContent.includes("What would you like to sharpen first"))), `${tag} the conversation starts and Claude's first turn shows`);
  ok(await until(page, (id) => document.querySelector(`[data-idea='${id}'] [data-testid=ideas-dot]`), JELLY), `${tag} the idea's list row has a dot while it has a live conversation`);
  ok(!(await page.$("[data-testid=chat-attach]")), `${tag} no attach button in an idea conversation`);
  const ph = await page.getAttribute("[data-testid=chat-field]", "placeholder");
  ok(ph && !ph.includes("/ for a command"), `${tag} the composer offers no slash commands ("${ph}")`);

  // ── ask for an edit, approve it in the chat ──
  await page.fill("[data-testid=chat-field]", "please add a line about GPU transcoding");
  await page.press("[data-testid=chat-field]", "Enter");
  ok(await until(page, (id) => document.querySelector(`[data-idea='${id}'] [data-testid=ideas-dot]`)?.dataset.state === "busy", JELLY, 3000), `${tag} the dot is busy while Claude works`);
  ok(await until(page, () => document.querySelector("[data-testid=chat-prompt]")), `${tag} the edit's permission prompt shows in the chat`);
  ok(await until(page, (id) => document.querySelector(`[data-idea='${id}'] [data-testid=ideas-dot]`)?.dataset.state === "waiting", JELLY), `${tag} the dot is amber while the conversation waits`);
  const sends = await page.evaluate(() => window.__mock.ideaSends());
  ok(sends.length === 1 && sends[0].text === "please add a line about GPU transcoding" && sends[0].id === "01JC8Z4K3M9QX2T7VB00000901", `${tag} the line went through idea_send`);
  await page.click("[data-testid=chat-choice-1]");
  await page.click("[data-testid=chat-send-answer]");
  ok(await until(page, () => document.querySelector(".cm-content")?.innerText.includes("please add a line about GPU transcoding")), `${tag} the approved edit reaches the editor`);
  ok(await until(page, () => [...document.querySelectorAll(".chat-bubble")].some((b) => b.textContent.includes("Done: I added it"))), `${tag} Claude's reply after the edit shows`);

  // ── an edit while the person has unsaved text: changed under you ──
  // A click in the chat saves the editor (blur is a save point), so the edit
  // that lands under unsaved text is one approved while typing goes on: the
  // row is answered from here, the editor keeping the keyboard.
  await page.fill("[data-testid=chat-field]", "please add a second line");
  await page.press("[data-testid=chat-field]", "Enter");
  ok(await until(page, () => document.querySelector("[data-testid=chat-prompt]")), `${tag} a second edit prompt`);
  await page.locator(".cm-line", { hasText: "which" }).first().click();
  await page.keyboard.press("End");
  await page.keyboard.type(" typed");
  await page.evaluate(async (id) => {
    const t = window.__TAURI_INTERNALS__;
    const conv = window.__mock.ideaConvs(id)[0].id;
    const p = await t.invoke("idea_prompt", { id, conv });
    await t.invoke("idea_answer", { id, conv, choice: 1, fingerprint: p.fingerprint });
  }, JELLY);
  ok(await until(page, () => document.querySelector("[data-testid=ideas-conflict]")), `${tag} Claude's edit under unsaved text is the "changed under you" banner`);
  ok((await page.evaluate(() => document.querySelector(".cm-content")?.innerText ?? "")).includes(" typed"), `${tag} the typed text is kept`);
  await page.click("[data-testid=ideas-keep-mine]");
  await until(page, () => !document.querySelector("[data-testid=ideas-conflict]"));

  // ── a second conversation ──
  await page.click("[data-testid=idea-conv-menu]");
  ok(await reachable(page, "[data-testid=idea-conv-new]"), `${tag} New conversation is reachable in the menu`);
  await page.click("[data-testid=idea-conv-new]");
  ok(await until(page, () => document.querySelector("[data-testid=idea-conv-menu]")?.textContent.includes("Conversation 2")), `${tag} the second conversation opens`);
  ok(await until(page, () => [...document.querySelectorAll(".chat-bubble")].some((b) => b.textContent.includes("What would you like to sharpen first"))), `${tag} the second conversation starts on the idea too`);
  await page.click("[data-testid=idea-conv-menu]");
  const listed = await page.evaluate(() => [...document.querySelectorAll(".ix-chat-row")].map((r) => ({ title: r.querySelector(".ix-row-title").textContent, state: r.dataset.state, words: r.textContent })));
  ok(listed.length === 2 && listed[0].title === "Conversation 2" && listed[1].title === "Conversation 1", `${tag} the menu lists both, newest first (${listed.map((l) => l.title).join(", ")})`);
  ok(listed.every((l) => /running|working|waiting|ended/.test(l.words) && /\d/.test(l.words)), `${tag} each with its state and start date`);
  await page.keyboard.press("Escape");
  ok(await until(page, () => !document.querySelector("[data-testid=idea-conv-list]")), `${tag} Escape closes the menu`);

  // ── close: read-only, transcript still there; resume ──
  await page.click("[data-testid=idea-conv-close]");
  ok(await until(page, () => document.querySelector("[data-testid=idea-conv-ended]") && !document.querySelector("[data-testid=chat-field]")), `${tag} a closed conversation is read-only, with Resume`);
  ok(await until(page, () => [...document.querySelectorAll(".chat-bubble")].some((b) => b.textContent.includes("What would you like to sharpen first"))), `${tag} its transcript still reads`);
  await page.click("[data-testid=idea-conv-resume]");
  ok(await until(page, () => document.querySelector("[data-testid=chat-field]") && !document.querySelector("[data-testid=idea-conv-ended]")), `${tag} Resume brings the composer back`);

  // ── rename, remove (asks first) ──
  await page.click("[data-testid=idea-conv-menu]");
  await page.click("[data-testid=idea-conv-rename]");
  await page.fill("[data-testid=idea-conv-rename-input]", "Who can see what");
  await page.press("[data-testid=idea-conv-rename-input]", "Enter");
  ok(await until(page, () => document.querySelector("[data-testid=idea-conv-menu]")?.textContent.includes("Who can see what")), `${tag} rename`);
  await page.click("[data-testid=idea-conv-menu]");
  await page.evaluate(() => { const b = document.querySelector("[data-testid=idea-conv-remove]"); b.click(); });
  ok((await page.textContent("[data-testid=idea-conv-remove]")).includes("?"), `${tag} remove asks first`);
  await page.evaluate(() => document.querySelector("[data-testid=idea-conv-remove]").click());
  ok(await until(page, (id) => window.__mock.ideaConvs(id).length === 1, JELLY), `${tag} the second click removes it`);

  // ── every call was an idea row ──
  const agentRows = [...new Set(calls.filter((c) => c.startsWith("agent_")))];
  ok(agentRows.length === 0, `${tag} no agent_* row was called (${agentRows.join(", ") || "none"})`);
  const ideaRows = new Set(calls.filter((c) => c.startsWith("idea_")));
  ok(["idea_conversations", "idea_conversation_start", "idea_turns", "idea_prompt", "idea_send", "idea_answer", "idea_conversation_close", "idea_conversation_resume", "idea_conversation_rename", "idea_conversation_remove"].every((c) => ideaRows.has(c)), `${tag} the panel drives the idea rows (${[...ideaRows].sort().join(", ")})`);

  // ── the main window: the lightbulb ambers while a conversation waits ──
  const main = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await main.addInitScript(() => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify({ settings_rev: 5, beta_ideas: true, places_side: "left", dock_side: "left" }));
  });
  await main.goto(`${BASE}/?ideas=seed&ideareply=150`);
  await main.waitForSelector("[data-testid=ideas-rail]");
  ok(!(await main.$("[data-testid=ideas-rail][data-waiting]")), `${tag} main window: the lightbulb is plain while nothing waits`);
  const fp = await main.evaluate(async (id) => {
    const t = window.__TAURI_INTERNALS__;
    const r = await t.invoke("idea_conversation_start", { id });
    await new Promise((res) => setTimeout(res, 400));
    await t.invoke("idea_send", { id, conv: r.conversation.id, text: "please add a line" });
    await new Promise((res) => setTimeout(res, 600));
    const p = await t.invoke("idea_prompt", { id, conv: r.conversation.id });
    return { conv: r.conversation.id, fp: p.fingerprint };
  }, JELLY);
  ok(await until(main, () => document.querySelector("[data-testid=ideas-rail]")?.dataset.waiting === "true"), `${tag} main window: the lightbulb ambers while a conversation waits`);
  const navSlugs = await main.evaluate(() => [...document.querySelectorAll(".nav-scroll li.row[data-slug]")].map((r) => r.dataset.slug));
  ok(!navSlugs.some((sl) => sl.includes("idea")), `${tag} main window: no place for the conversation in the nav`);
  await main.evaluate(async ({ id, conv, fp }) => window.__TAURI_INTERNALS__.invoke("idea_answer", { id, conv, choice: 1, fingerprint: fp }), { id: JELLY, ...fp });
  ok(await until(main, () => !document.querySelector("[data-testid=ideas-rail]")?.dataset.waiting), `${tag} main window: and plain again once it is answered`);
  await main.close();

  if (SHOTS) await page.screenshot({ path: `${SHOTS}/ideas-sessions-${name}.png` });
  ok(errors.length === 0, `${tag} no page errors${errors.length ? `: ${errors.join(" | ")}` : ""}`);
  await browser.close();
}

await run(chromium, "chromium");
await run(webkit, "webkit");
console.log(failed ? `\nideas-sessions-browser: ${failed} failure(s)` : "\nideas-sessions-browser: all good");
process.exit(failed ? 1 : 0);
