// Browser gate for the workspace view's small things a review proved wrong, in
// Chromium AND WebKit against the mock: an image's preview is never another file's;
// the bar between chat and documents says what it is; no button for what does not
// exist; a removed workspace's folder stays said; a document's name keeps its floor
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

async function boot(page, betas = ["beta_workspace_chats"], query = "") {
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { places_side: "left", dock_side: "right", settings_rev: 5, nav_pinned: true, nav_view: "workspaces", ...Object.fromEntries(betas.map((b) => [b, true])) });
  await page.goto(`${BASE}/?ideas=seed&workspaces=1&folding=0&memtick=100${query}`);
  await page.waitForSelector("[data-testid=ws-group]");
  await page.locator("[data-testid=ws-name]", { hasText: "Everything" }).first().click();
  await page.waitForSelector("[data-testid=ws-tab][data-tab=memory]");
  return (await call(page, "workspaces_list", {})).workspaces.find((w) => w.name === "Everything").id;
}

async function picture(page, tag) {
  await boot(page);
  await page.click("[data-testid=ws-tab][data-tab=memory]");
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
  await page.waitForSelector("[data-testid=ws-split]");
  const bar = page.locator("[data-testid=ws-split]");
  const at = () => page.evaluate(() => { const b = document.querySelector("[data-testid=ws-split]"), d = document.querySelector(".wsf-docs").getBoundingClientRect().width, w = document.querySelector(".wsf-body").getBoundingClientRect().width; return { now: Number(b.getAttribute("aria-valuenow")), min: Number(b.getAttribute("aria-valuemin")), max: Number(b.getAttribute("aria-valuemax")), real: Math.round((d / w) * 100), docs: d, mid: document.querySelector("[data-testid=ws-mid]").getBoundingClientRect().width, stored: Number(localStorage.getItem("wt.workspace.split")) }; });
  for (const [key, n] of [["ArrowRight", 30], ["ArrowLeft", 45]]) {
    await bar.focus();
    for (let i = 0; i < n; i++) await page.keyboard.press(key);
    await page.waitForTimeout(80);
    const a = await at();
    ok(Math.abs(a.now - a.real) <= 1, `${tag} ${key}: it says what is on screen (${a.now}, ${a.real})`);
    ok(a.docs >= 339 && a.mid >= 419, `${tag} ${key}: and both keep their floor (${a.docs.toFixed(0)}, ${a.mid.toFixed(0)})`);
    ok(Math.abs(a.stored * 100 - a.real) <= 1 && a.now >= a.min && a.now <= a.max, `${tag} ${key}: what is kept is that too (${a.stored}, ${a.min} to ${a.max})`);
    // One press the other way moves it at once: it did not run on past the edge.
    await page.keyboard.press(key === "ArrowRight" ? "ArrowLeft" : "ArrowRight");
    await page.waitForTimeout(60);
    ok(Math.abs((await at()).real - a.real) >= 1, `${tag} ${key}: one press back moves it`);
  }
  const b = await bar.boundingBox();
  await page.mouse.move(b.x + 1, b.y + 200); await page.mouse.down(); await page.mouse.move(b.x + 900, b.y + 200, { steps: 5 }); await page.mouse.up();
  const d = await at();
  ok(Math.abs(d.now - d.real) <= 1 && d.docs >= 339, `${tag} the pointer stops at the same place (${d.now}, ${d.real})`);
  // A window made narrower takes it in again.
  for (let i = 0; i < 45; i++) await page.keyboard.press("ArrowLeft");
  await page.setViewportSize({ width: 1250, height: 900 });
  await page.waitForTimeout(150);
  const s = await at();
  ok(Math.abs(s.now - s.real) <= 1 && s.mid >= 419, `${tag} and in a narrower window (${s.now}, ${s.real}, ${s.mid.toFixed(0)})`);
}

async function tidy(page, tag) {
  const id = await boot(page);
  await page.click("[data-testid=ws-tab][data-tab=memory]");
  await isDoc(page, "MEMORY.md");
  await page.evaluate((id) => window.__mock.memoryEdit(id, "MEMORY.md", "# Memory\n\n" + "A line of the index, long enough to count.\n".repeat(560)), id);
  ok(await until(page, () => /The index is long \(\d+ KB\)\. Every chat reads all of it\./.test(document.querySelector("[data-testid=memory-long]")?.textContent ?? "")), `${tag} a long index says so`);
  ok(await page.locator("[data-testid=memory-long] button").count() === 0 && !/Tidy|Not yet/.test(await page.locator("body").innerText()), `${tag} …with no button for what does not exist yet`);
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
  await page.click("[data-testid=ws-tab][data-tab=memory]");
  await isDoc(page, "MEMORY.md");
  await file(page, "topics/nas-layout.md").click().catch(async () => { await page.click("[data-testid=ws-side-toggle]"); await file(page, "topics/nas-layout.md").click(); });
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
  await page.click("[data-testid=ws-tab][data-tab=memory]");
  const says = () => page.evaluate(() => document.querySelector("[data-testid=memory-skipped]")?.innerText ?? "");
  ok(await until(page, () => /video\.mov/.test(document.querySelector("[data-testid=memory-skipped]")?.innerText ?? "")), `${tag} what is not shown is named`);
  ok(/4 files not shown/.test(await says()) && /video\.mov\s*: over 16 MB/.test(await says()) && /\.git\s*: a repository inside the memory/.test(await says()), `${tag} …each with why (${JSON.stringify(await says())})`);
}

const only = process.env.ONLY?.split(",");
for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const browser = await engine.launch();
  for (const part of [picture, split, tidy, removed, header, skipped]) {
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
