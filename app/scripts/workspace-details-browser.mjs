// Browser gate for the workspace view's small things a review proved wrong, in
// Chromium AND WebKit against the mock: an image's preview is never another file's;
// the bar between chat and documents says what it is; no button for what does not
// exist (Tidy… only over the budget, its words in its title and name); a removed workspace's folder stays said; a document's name keeps its floor
// in its header; what is not shown in a memory's list is named.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/workspace-details-browser.mjs
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const call = (page, cmd, args) => page.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
const isDoc = (page, id) => until(page, (id) => { const d = document.querySelector("[data-testid=main-doc]"); return !!d && !d.hidden && d.dataset.doc === id; }, id);
const file = (page, p) => page.locator(`[data-testid=memory-file][data-path='${p}']`).first();
const svg = (c) => `data:image/svg+xml;base64,${Buffer.from(`<svg xmlns="http://www.w3.org/2000/svg" width="80" height="60"><rect width="80" height="60" fill="${c}"/></svg>`).toString("base64")}`;

async function boot(page, betas = ["beta_workspace_chats"], query = "", more = {}) {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", ...Object.fromEntries(betas.map((b) => [b, true])), ...more });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0&memtick=100${query}`);
  await page.waitForSelector("[data-testid=ws-group]");
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-workspace-panel=memory]");
  return (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
}

async function picture(page, tag) {
  await boot(page);
  await page.click("[data-workspace-panel=memory]");
  await isDoc(page, "MEMORY.md");
  // Two pictures; the second one's bytes are held.
  await page.evaluate(([a, b]) => {
    const t = window.__TAURI_INTERNALS__, real = t.invoke.bind(t);
    window.__pics = { "files/rack.svg": a, "files/other.svg": b }; window.__hold = null;
    const file = (p) => ({ path: p, name: p.slice(6), size: 90, mtime_ms: 5, kind: "image", title: null });
    t.invoke = async (c, x) => {
      if (c === "workspace_memory_image") { if (window.__hold) await window.__hold; return { b64: window.__pics[x.path].split(",")[1], unchanged: false, etag: `${x.path}${Math.random()}`, mime: "image/svg+xml", size: 90, truncated: false }; }
      const out = await real(c, x);
      if (c === "workspace_memory" && out.made) { const f = out.folders.find((d) => d.name === "files"); if (f && !f.files.some((y) => y.path === "files/other.svg")) f.files.push(file("files/other.svg")); }
      return out;
    };
  }, [svg("red"), svg("blue")]);
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/x.md", "# x\n"), (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id);
  await file(page, "files/other.svg").waitFor();
  await file(page, "files/rack.svg").click();
  ok(await until(page, () => document.querySelector("[data-testid=memory-preview] img")?.naturalWidth > 0), `${tag} a picture is shown`);
  await page.evaluate(() => { window.__hold = new Promise((r) => { window.__free = r; }); });
  await file(page, "files/other.svg").click();
  await until(page, () => document.querySelector("[data-testid=memory-preview]")?.dataset.path === "files/other.svg");
  ok(!(await page.$("[data-testid=memory-preview] img")), `${tag} another's name never stands under its picture while its own is read`);
  await page.evaluate(() => window.__free());
  ok(await until(page, () => /Ymx1Z|Ymx1ZS/.test(document.querySelector("[data-testid=memory-preview] img")?.src ?? "") || !!document.querySelector("[data-testid=memory-preview] img")), `${tag} …then its own`);
  // Read again for the same file: it stays on screen.
  await page.evaluate(() => { window.__hold = new Promise((r) => { window.__free = r; }); });
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/x.md", "# x2\n"), (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id);
  await page.waitForTimeout(400);
  ok(!!(await page.$("[data-testid=memory-preview] img")), `${tag} the same file read again does not blink`);
  await page.evaluate(() => { window.__free(); window.__hold = null; });
  // In Read: an image whose source changed under it.
  await file(page, "topics/nas-layout.md").click();
  await isDoc(page, "topics/nas-layout.md");
  ok(await until(page, () => document.querySelector("[data-testid=doc-image]")?.naturalWidth > 0), `${tag} a picture in a text`);
  await page.evaluate(() => { window.__hold = new Promise((r) => { window.__free = r; }); });
  const id = (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
  const was = (await call(page, "workspace_memory_file", { id, path: "topics/nas-layout.md" })).content;
  await page.evaluate(([id, t]) => window.__mock.memoryEdit(id, "topics/nas-layout.md", t), [id, was.replace("rack.svg", "other.svg")]);
  ok(await until(page, () => !document.querySelector("[data-testid=doc-image]") && !!document.querySelector("[data-testid=ideas-read] .ix-pic-note")), `${tag} …and there too: its words, until the new one is read`);
  await page.evaluate(() => window.__free());
  ok(await until(page, () => !!document.querySelector("[data-testid=doc-image]")), `${tag} …and then it`);
}

async function split(page, tag) {
  await boot(page);
  await page.locator("[data-testid=ws-chat-row]").first().click();
  const dock = page.getByTestId("ws-dock");
  await dock.waitFor();
  const width = async () => (await dock.boundingBox()).width;
  const start = await width();
  const grip = await dock.locator(".dock-resizer").boundingBox();
  await page.mouse.move(grip.x + 1, grip.y + 200); await page.mouse.down();
  await page.mouse.move(grip.x - 49, grip.y + 200, { steps: 5 }); await page.mouse.up();
  const changed = await width();
  ok(Math.abs(changed - start - 50) <= 2, `${tag} the common dock handle resizes the workspace panel`);
  await page.locator("[data-workspace-panel=memory]").click();
  ok(Math.abs(await width() - changed) <= 1, `${tag} another panel uses the same width`);
}

const TIDY_WHAT = "Tidy the index: start a chat with Claude that moves detail out of MEMORY.md into topic files, keeping one line and a link for each. It is told to delete nothing, and each change it makes is in History, where you can undo it.";
const AGENT = process.env.AGENT_SHOTS; // .worktrees-attachments/agent, for the report
async function tidy(page, tag, name) {
  const id = await boot(page);
  await page.click("[data-workspace-panel=memory]");
  await isDoc(page, "MEMORY.md");
  ok(await page.locator("[data-testid=memory-tidy]").count() === 0, `${tag} an index under its budget offers no Tidy…`);
  await page.evaluate((id) => window.__mock.memoryEdit(id, "MEMORY.md", "# Memory\n\n" + "A line of the index, long enough to count.\n".repeat(560)), id);
  ok(await until(page, () => /The index is long \(\d+ KB\)\. Every chat reads all of it\./.test(document.querySelector("[data-testid=memory-long]")?.textContent ?? "")), `${tag} a long index says so`);
  const b = page.locator("[data-testid=memory-long] [data-testid=memory-tidy]");
  ok(await b.count() === 1 && (await b.innerText()).trim() === "Tidy…", `${tag} …with Tidy… beside it`);
  ok(await b.getAttribute("title") === TIDY_WHAT && await b.getAttribute("aria-label") === TIDY_WHAT, `${tag} its tooltip and its accessible name say what it does`);
  ok(await page.getByRole("button", { name: TIDY_WHAT, exact: true }).count() === 1, `${tag} …and a reader finds it by those words`);
  const hit = async () => b.evaluate((el) => { const r = el.getBoundingClientRect(); const at = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); return { hit: !!at && (at === el || el.contains(at)), w: r.width, right: r.right, vw: window.innerWidth }; });
  const h = await hit();
  ok(h.hit && h.right <= h.vw, `${tag} the button is reachable (${JSON.stringify(h)})`);
  if (AGENT && name === "webkit") await page.screenshot({ path: `${AGENT}/tidy-webkit-1440-night.png` });
  // At the limit: the start is refused in the engine's words, and Try again tidies again.
  await page.evaluate(() => { const t = window.__TAURI_INTERNALS__, i = t.invoke.bind(t); window.__calls = []; window.__refuse = true; t.invoke = (c, a) => { window.__calls.push(c); return c === "workspace_chat_tidy" && window.__refuse ? Promise.reject("5 agents are running and the limit is 5") : i(c, a); }; });
  await b.click();
  ok(await until(page, () => /limit is 5/.test(document.querySelector("[data-testid=ws-chat-refused]")?.textContent ?? "")), `${tag} at the limit it says why`);
  ok(await page.evaluate(() => window.__calls.includes("workspace_chat_tidy") && !window.__calls.includes("workspace_chat_start")), `${tag} …and it asked for a tidy, not a chat`);
  await page.evaluate(() => { window.__refuse = false; window.__calls = []; });
  await page.click("[data-testid=ws-chat-again]");
  ok(await until(page, () => window.__calls.includes("workspace_chat_tidy")) && await page.evaluate(() => !window.__calls.includes("workspace_chat_start")), `${tag} Try again tidies again`);
  ok(await until(page, () => [...document.querySelectorAll("[data-testid=ws-chat-line], .wsc-title, [data-testid=ws-nav-chat]")].some((e) => /Tidy the index/.test(e.textContent ?? "")) || /Tidy the index/.test(document.body.innerText)), `${tag} the tidy chat is open, by its name`);
  const chats = await call(page, "workspace_chats", { id });
  ok(chats.chats.filter((c) => c.tidy).length === 1, `${tag} one tidy chat in the record`);
  // While it runs, the index's line opens it instead of a second.
  await page.click("[data-workspace-panel=memory]");
  ok(await until(page, () => !!document.querySelector("[data-testid=memory-tidy-open]") && /A tidy chat is running\./.test(document.querySelector("[data-testid=memory-long]")?.textContent ?? "")), `${tag} a running tidy is said, with Open it`);
  ok(await page.locator("[data-testid=memory-tidy]").count() === 0, `${tag} …and no second Tidy…`);
  if (AGENT && name === "webkit") await page.screenshot({ path: `${AGENT}/tidy-webkit-1440-running.png` });
}

/** Narrow, and in a light theme: the line wraps, the button stays whole and reachable. */
async function tidyNarrow(page, tag, name) {
  for (const [w, theme] of [[900, "tokyo-day"], [1440, "catppuccin-latte"], [760, "tokyo-night"]]) {
    const ctx = await page.context().browser().newContext({ viewport: { width: w, height: 800 } });
    page = await ctx.newPage();
    const id = await boot(page, ["beta_workspace_chats"], "", { theme });
    await page.click("[data-workspace-panel=memory]");
    await isDoc(page, "MEMORY.md");
    await page.evaluate((id) => window.__mock.memoryEdit(id, "MEMORY.md", "# Memory\n\n" + "A line of the index, long enough to count.\n".repeat(560)), id);
    const b = page.locator("[data-testid=memory-tidy]");
    await b.waitFor();
    const m = await b.evaluate((el) => { const r = el.getBoundingClientRect(); const at = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); const s = getComputedStyle(el); return { hit: !!at && (at === el || el.contains(at)), w: r.width, h: r.height, right: r.right, vw: document.documentElement.clientWidth, wrap: el.scrollWidth <= el.clientWidth + 1, bg: s.backgroundColor, fg: s.color, theme: document.documentElement.dataset.theme ?? "" }; });
    ok(m.hit && m.right <= m.vw && m.wrap && m.h < 40, `${tag} ${w}px ${theme}: whole and reachable (${JSON.stringify(m)})`);
    const sx = await page.evaluate(() => document.scrollingElement.scrollWidth <= document.documentElement.clientWidth);
    ok(sx, `${tag} ${w}px: no sideways scroll`);
    if (AGENT && name === "webkit") await page.screenshot({ path: `${AGENT}/tidy-webkit-${w}-${theme}.png` });
    await ctx.close();
  }
}

async function removed(page, tag) {
  await boot(page);
  await page.locator("[data-testid=ws-group]", { has: page.locator("[data-testid=ws-name]", { hasText: "Everything" }) }).first().locator("[data-testid=ws-menu]").click();
  await page.click("[data-testid=ws-remove]");
  await page.click("[data-testid=ws-remove]");
  ok(await until(page, () => /Its memory and its chats were kept/.test(document.querySelector("[data-testid=ws-moved]")?.textContent ?? "") && /\.removed\//.test(document.querySelector("[data-testid=ws-moved]")?.textContent ?? "")), `${tag} a removed workspace's folder is said`);
  await page.evaluate(() => { window.__rev = []; const t = window.__TAURI_INTERNALS__, real = t.invoke.bind(t); t.invoke = (c, a) => { if (/reveal/.test(c)) { window.__rev.push(JSON.stringify(a)); return Promise.resolve(null); } return real(c, a); }; });
  await page.click("[data-testid=ws-moved-reveal]");
  ok(await until(page, () => window.__rev.some((x) => /\.removed\//.test(x))), `${tag} …Show in Finder shows it`);
  await page.locator("[data-testid=ws-group] li.row[data-slug]").first().click();
  await page.waitForTimeout(600);
  ok(!!(await page.$("[data-testid=ws-moved]")), `${tag} it stays through going elsewhere`);
  await page.click("[data-testid=ws-moved-dismiss]");
  ok(await until(page, () => !document.querySelector("[data-testid=ws-moved]")), `${tag} until it is dismissed`);
}

async function header(page, tag, name) {
  await boot(page, ["beta_workspace_chats", "beta_ideas"]);
  await page.click("[data-workspace-panel=memory]");
  await isDoc(page, "MEMORY.md");
  await file(page, "topics/nas-layout.md").click();
  await isDoc(page, "topics/nas-layout.md");
  const at = () => page.evaluate(() => {
    const h = document.querySelector("[data-testid=main-doc] .ix-head"), t = h.querySelector(".ix-title"), r = document.createRange();
    r.selectNodeContents(t);
    const vis = (s) => { const e = h.querySelector(s); return !!e && e.checkVisibility() && e.getBoundingClientRect().width > 8; };
    return { w: Math.round(h.getBoundingClientRect().width), name: t.clientWidth, full: Math.ceil(r.getBoundingClientRect().width), floor: parseFloat(getComputedStyle(t).fontSize) * 0.6 * 12, crumb: vis(".wsm-crumb"), state: vis(".ix-state"), seg: vis(".ix-seg"), float: vis("[data-testid=main-doc-float]"), more: vis("[data-testid=doc-menu-open]"), out: h.scrollWidth - h.clientWidth, h: h.getBoundingClientRect().height };
  });
  const seen = [];
  for (const w of [1440, 1300, 1200, 1120, 1060, 1000, 940, 900]) {
    await page.setViewportSize({ width: w, height: 800 });
    await page.waitForTimeout(120);
    const m = await at();
    seen.push(m);
    ok(m.name >= Math.min(m.floor, m.full) - 1 || m.w < 400, `${tag} ${m.w}px: the name keeps twelve characters, or all of itself (${m.name} of ${m.full})`);
    ok(m.out <= 0 && m.h <= 46 && m.seg && m.float && (m.more || m.w >= 640), `${tag} ${m.w}px: one row, with its acts (${JSON.stringify(m)})`);
    ok(!m.crumb || m.name >= Math.min(m.floor, m.full) - 1, `${tag} ${m.w}px: the folder goes before the name is cut`);
    ok(m.state || !m.crumb, `${tag} ${m.w}px: …and before the word of its state`);
  }
  ok(seen.some((m) => m.crumb && m.state) && seen[0].crumb, `${tag} with room, all of it (${seen.map((m) => `${m.w}:${m.crumb ? "c" : ""}${m.state ? "s" : ""}`).join(" ")})`);
  // With a chat beside it, as it was seen.
  await page.setViewportSize({ width: 1440, height: 800 });
  await page.locator("[data-testid=ws-chat-row]").first().click();
  await page.waitForSelector("[data-testid=ws-chat-col]");
  await page.waitForTimeout(250);
  const c = await at();
  ok(c.name >= Math.min(c.floor, c.full) - 1, `${tag} beside a chat (${c.w}px): ${c.name} of ${c.full}`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsd-header-${name}.png` });
  // A state that needs somebody is never folded.
  await page.evaluate(() => { const t = window.__TAURI_INTERNALS__, real = t.invoke.bind(t); t.invoke = async (c, a) => { if (c === "workspace_memory_write") throw "the disk is full"; return real(c, a); }; });
  await page.click("[data-testid=main-doc] [data-testid=ideas-mode-edit]");
  await page.locator("[data-testid=main-doc] .cm-content").click();
  await page.keyboard.type("x");
  ok(await until(page, () => { const e = document.querySelector("[data-testid=main-doc] .ix-state"); return /Not saved/.test(e?.textContent ?? "") && e.checkVisibility() && e.getBoundingClientRect().width > 20; }), `${tag} "Not saved" always shows`);
}

async function skipped(page, tag) {
  await boot(page);
  await page.evaluate(() => {
    const t = window.__TAURI_INTERNALS__, real = t.invoke.bind(t);
    t.invoke = async (c, x) => { const out = await real(c, x); if (c === "workspace_memory" && out.made) out.skipped = [{ path: "video.mov", why: "over 16 MB" }, { path: ".git", why: "a repository inside the memory" }, { path: "a", why: "w1" }, { path: "b", why: "w2" }]; return out; };
  });
  await page.evaluate((id) => window.__mock.memoryEdit(id, "topics/y.md", "# y\n"), (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id);
  await page.click("[data-workspace-panel=memory]");
  const says = () => page.evaluate(() => document.querySelector("[data-testid=memory-skipped]")?.innerText ?? "");
  ok(await until(page, () => /video\.mov/.test(document.querySelector("[data-testid=memory-skipped]")?.innerText ?? "")), `${tag} what is not shown is named`);
  ok(/4 files not shown/.test(await says()) && /video\.mov\s*: over 16 MB/.test(await says()) && /\.git\s*: a repository inside the memory/.test(await says()), `${tag} …each with why (${JSON.stringify(await says())})`);
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [picture, split, tidy, tidyNarrow, removed, header, skipped]) {
    if (only && !only.includes(part.name)) continue;
    const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
    const errors = [];
    page.on("pageerror", (e) => { if (!/dimensions/.test(String(e))) errors.push(String(e)); });
    const tag = `[${name} ${part.name}]`;
    try { await part(page, tag, name); } catch (e) { ok(false, `${tag} ${String(e).split("\n").slice(0, 3).join(" ")}`); if (SHOTS) await page.screenshot({ path: `${SHOTS}/wsd-${name}-${part.name}-FAIL.png` }).catch(() => {}); }
    ok(errors.length === 0, `${tag} no page error${errors.length ? `: ${errors[0]}` : ""}`);
    await page.context().close();
  }
  await browser.close();
}
console.log(failed ? `\nworkspace-details-browser: ${failed} failure(s)` : "\nworkspace-details-browser: all good");
process.exit(failed ? 1 : 0);
