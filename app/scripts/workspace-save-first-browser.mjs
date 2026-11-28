// Browser gate: an act of a workspace's chats or of its memory's history starts once
// the document that is open has reached its file (Kimi's K1), in Chromium AND WebKit
// against the mock. Its write is HELD and then let go, or refused; the engine's own
// calls are recorded.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/workspace-save-first-browser.mjs
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const isDoc = (page, id) => until(page, (id) => { const d = document.querySelector("[data-testid=main-doc]"); return !!d && !d.hidden && d.dataset.doc === id; }, id);
const row = (page, t) => page.locator("[data-testid=ws-chat-row]", { hasText: t }).first();
const ACTS = ["workspace_chat_fold", "workspace_chat_close", "workspace_chat_start", "workspace_chat_resume", "workspace_memory_revert"];

async function boot(page) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", beta_workspace_chats: true, beta_ideas: true });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0&memtick=150`);
  await page.waitForSelector("[data-testid=ws-chat-row]");
  const id = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
  await page.evaluate((acts) => {
    const t = window.__TAURI_INTERNALS__, real = t.invoke.bind(t);
    window.__h = { hold: false, fail: null, waiting: [], calls: [] };
    t.invoke = async (c, a) => {
      if (c === "workspace_memory_write") {
        if (window.__h.fail) throw window.__h.fail;
        if (window.__h.hold) await new Promise((r) => window.__h.waiting.push(r));
        const out = await real(c, a);
        window.__h.calls.push("saved");
        return out;
      }
      if (acts.includes(c)) window.__h.calls.push(c);
      return real(c, a);
    };
  }, ACTS);
  return id;
}
/** A chat on screen with a memory file beside it, in Edit, and text typed. */
async function edit(page, chat, how) {
  await row(page, chat).click();
  await page.waitForSelector("[data-testid=ws-chat-col]");
  const memory = page.locator("[data-workspace-panel=memory]");
  if (await memory.getAttribute("aria-pressed") !== "true") await memory.click();
  await isDoc(page, "MEMORY.md");
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.waitForSelector("[data-testid=main-doc] .cm-content");
  await page.evaluate((how) => { Object.assign(window.__h, { calls: [], hold: how === "held", fail: how === "refused" ? "the disk is full" : null }); }, how);
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.type(" MINE");
}
const calls = (page) => page.evaluate(() => window.__h.calls.slice());
/** One act, with the write held and then let go: saved, then the act. */
async function after(page, tag, what, cmd, press) {
  await press();
  await until(page, () => window.__h.waiting.length > 0, null, 2500);
  await page.waitForTimeout(250);
  const early = await calls(page);
  ok(!early.includes(cmd), `${tag} ${what} waits for the save (${early})`);
  await page.evaluate(() => { window.__h.hold = false; for (const r of window.__h.waiting.splice(0)) r(); });
  ok(await until(page, (cmd) => window.__h.calls.includes(cmd), cmd), `${tag} …and then starts`);
  const all = await calls(page);
  ok(all.indexOf("saved") >= 0 && all.indexOf("saved") < all.indexOf(cmd), `${tag} …after it (${all})`);
  await page.waitForTimeout(700);
  ok(!(await page.$("[data-testid=ideas-conflict]")), `${tag} …and no conflict of the person's own making`);
}
/** …and with a write that is refused: no act, and the text is there, said. */
async function not(page, tag, what, cmd, press) {
  await press();
  await page.waitForTimeout(500);
  ok(!(await calls(page)).includes(cmd), `${tag} ${what} does not start while it cannot be saved`);
  ok(/MINE/.test(await page.locator("[data-testid=main-doc] .cm-content").textContent()) && !!(await page.$("[data-testid=ideas-error], [data-testid=ideas-notice]")), `${tag} …and the text is there, with why`);
}
const PRESS = {
  Close: ["Release order", "workspace_chat_fold", async (p) => { await p.getByTestId("ws-chat-menu").click(); await p.getByTestId("ws-chat-close").click(); }],
  "Fold now": ["First look", "workspace_chat_fold", (p) => p.click("[data-testid=ws-chat-fold-now]")],
  Resume: ["First look", "workspace_chat_resume", (p) => p.click("[data-testid=ws-chat-resume]")],
  "New chat": ["Release order", "workspace_chat_start", (p) => p.click("[data-testid=ws-frame-new-chat]")],
  "Close without folding": ["Release order", "workspace_chat_close", async (p) => { await p.click("[data-testid=ws-chat-menu]"); await p.click("[data-testid=ws-chat-close-item]"); if (await p.$("[data-testid=ws-chat-close-item]")) await p.click("[data-testid=ws-chat-close-item]"); }],
};
async function lifecycle(page, tag) {
  for (const [what, [chat, cmd, press]] of Object.entries(PRESS)) {
    for (const how of ["held", "refused"]) {
      const p = await page.context().newPage();
      await boot(p);
      await edit(p, chat, how);
      await (how === "held" ? after : not)(p, `${tag} [${what}]`, what, cmd, () => press(p));
      await p.close();
    }
  }
}
/** Undo, then Redo, of a change in History (which covers the editor, not its text). */
async function history(p, nth) {
  if (!(await p.$("[data-testid=memory-history]"))) {
    await p.click("[data-testid=ideas-versions-toggle], [data-testid=doc-menu-open]");
    if (!(await p.$("[data-testid=memory-history]"))) await p.click("[data-testid=doc-menu-history]");
  }
  await p.click("[data-testid=memory-hist-all]").catch(() => {});
  await nth.click();
  await p.waitForSelector("[data-testid=memory-change] [data-testid=memory-revert]:not(:disabled)");
}
async function revert(page, tag) {
  for (const how of ["held", "refused"]) {
    const p = await page.context().newPage();
    await boot(p);
    const undo = p.locator("[data-testid=memory-change] [data-testid=memory-revert]");
    const twice = async () => { await undo.click(); await undo.click(); };
    await edit(p, "Storage layout", how);
    await history(p, p.locator("[data-testid=memory-log-row][data-kind=fold]").first());
    await (how === "held" ? after : not)(p, `${tag} [Undo]`, "Undo", "workspace_memory_revert", twice);
    if (how === "held") {
      await p.click("[data-testid=memory-hist-close]");
      await edit(p, "Storage layout", how);
      await history(p, p.locator("[data-testid=memory-log-row][data-kind=revert], [data-testid=memory-log-row][data-kind=undo]").first());
      await after(p, `${tag} [Redo]`, "Redo", "workspace_memory_revert", twice);
    }
    await p.close();
  }
  // The receipt's.
  const p = await page.context().newPage();
  await boot(p);
  await edit(p, "Storage layout", "held");
  if (await p.$("[data-testid=ws-chat-receipt] [data-testid=memory-revert]")) {
    const r = p.locator("[data-testid=ws-chat-receipt] [data-testid=memory-revert]");
    await after(p, `${tag} [a receipt]`, "Undo", "workspace_memory_revert", async () => { await r.click(); await r.click(); });
  } else ok(false, `${tag} there is a receipt with Undo`);
  await p.close();
}
async function other(page, tag) {
  // A document of ANOTHER workspace that cannot be saved is not this one's business.
  await boot(page);
  await edit(page, "Release order", "refused");
  await page.locator("[data-testid=ws-name]", { hasText: "Tools" }).first().click();
  await page.click("[data-testid=ws-frame-new-chat], [data-testid=ws-first-chat], [data-testid=ws-new-chat]").catch(() => {});
  await page.locator("button", { hasText: /^New chat$/ }).first().click().catch(() => {});
  ok(await until(page, () => window.__h.calls.includes("workspace_chat_start")), `${tag} another workspace's chat starts all the same`);
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [lifecycle, revert, other]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1600, height: 900 } })).newPage();
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 4).join(" ")}`); if (process.env.IDEAS_SCREENSHOTS) for (const [i, p] of page.context().pages().entries()) await p.screenshot({ path: `${process.env.IDEAS_SCREENSHOTS}/sf-${name}-${part.name}-${i}-FAIL.png` }).catch(() => {}); }
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\nworkspace-save-first-browser: ${failed} failure(s)` : "\nworkspace-save-first-browser: all good");
process.exit(failed ? 1 : 0);
