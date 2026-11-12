// Browser gate for the Ideas window (app/companion/), in Chromium AND WebKit
// (the app is WKWebView; the harness is Chrome — AGENTS.md), against the mock.
//
//   VITE_MOCK=1 vite --port 1466 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1466 node app/scripts/ideas-browser.mjs
// IDEAS_SCREENSHOTS=<dir> saves the window per engine and theme.
//
// What it holds, typed with real key events into CodeMirror:
//   - the list: newest edited first, the title filter, a hand-made file with
//     Adopt, a bad id with its reason and no way to open it, and the skipped
//     entry mentioned;
//   - byte-exact: a document with frontmatter, an unknown key, an HTML
//     comment, a table and a mermaid fence (LF), and a CRLF one: one
//     character typed, the saved file differs by that character only; Enter
//     in the CRLF file writes CRLF;
//   - a save with no change sends NO idea_write (opened, clicked, blurred,
//     Read and back, switched away; typed and deleted back);
//   - the frontmatter: a keystroke in the id line changes nothing, and
//     select-all + delete keeps the three guarded lines;
//   - create (the new idea is listed and takes typing), rename (file and
//     heading, same id), editing the first heading renames the file on its
//     save (header, list row and mock file follow; no conflict), remove (two
//     clicks, says .history), adopt;
//   - an edit elsewhere: reloaded when clean; a conflict when dirty, with
//     the typed text kept, then Keep mine and Reload; a `removed` that the
//     read finds (a save by rename) reloads, one that it does not shows the
//     text kept and Save as a new idea;
//   - Read: the body rendered (heading, table), no frontmatter, no comment,
//     the mermaid fence as code and no diagram;
//   - layout: the editor fills its pane, every header and list button is
//     reachable (elementFromPoint) at 1200 and 720 px, words read ≥ 4.5:1 and
//     the dimmed markers stay visible in all six themes, and the list and
//     the document are different surfaces in each.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1466";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const THEMES = ["tokyo-night", "tokyo-day", "catppuccin-mocha", "catppuccin-latte", "nord", "gruvbox-dark"];
const JELLY = "01JC8Z4K3M9QX2T7VB00000901";
const GPU = "01JC8Z4K3M9QX2T7VB00000902";
const TIDY = "01JC8Z4K3M9QX2T7VB00000903";

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

const file = (page, id) => page.evaluate((id) => window.__mock.ideaFile(id), id);
const writes = (page, id) => page.evaluate((id) => window.__mock.ideaWrites().filter((w) => !id || w.id === id), id);
const saveState = (page) => page.evaluate(() => document.querySelector("[data-testid=ideas-save]")?.dataset.state ?? null);
const editorText = (page) => page.evaluate(() => document.querySelector(".cm-content")?.innerText ?? null);
const until = (page, fn, arg, ms = 5000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const openIdea = async (page, id) => {
  await page.click(`[data-idea='${id}'] button.ix-row-main`);
  await until(page, (id) => document.querySelector(`[data-idea='${id}']`)?.classList.contains("on") && document.querySelector(".cm-content"), id);
};
/** Put the cursor at the end of the line holding `text` (a click on it, then End). */
const caretAfter = async (page, text) => {
  await page.locator(".cm-line", { hasText: text }).first().click();
  await page.keyboard.press("End");
};
/** Wait for the open idea to be saved and the save to settle. */
const saved = (page) => until(page, () => document.querySelector("[data-testid=ideas-save]")?.dataset.state === "clean" && !document.querySelector("[data-testid=ideas-save]")?.textContent.startsWith("Saving"));
const MOD = process.platform === "darwin" ? "Meta" : "Control";

async function run(engine, name) {
  const browser = await engine.launch();
  const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${BASE}/companion/index.html?ideas=seed`);
  await page.waitForSelector("[data-testid=ideas-rows] li");
  const tag = `[${name}]`;

  // ── the list ──
  const rows = await page.evaluate(() => [...document.querySelectorAll(".ix-row")].map((r) => ({ id: r.dataset.idea ?? null, file: r.dataset.file, problem: r.dataset.problem ?? null, title: r.querySelector(".ix-row-title").textContent, button: !!r.querySelector("button.ix-row-main"), adopt: !!r.querySelector("[data-testid=ideas-adopt]") })));
  ok(JSON.stringify(rows.map((r) => r.title)) === JSON.stringify(["Jellyfin on the home server", "Home server GPU", "Notes from the train", "Media library tidy", "Broken id"]), `${tag} the list is newest edited first (${rows.map((r) => r.title).join(", ")})`);
  const handmade = rows.find((r) => r.file === "notes-from-the-train.md");
  ok(handmade?.problem === "no-id" && handmade.adopt && !handmade.button, `${tag} a hand-made file is listed with Adopt and cannot be opened as it is`);
  const bad = rows.find((r) => r.file === "broken.md");
  ok(bad?.problem === "bad-id" && !bad.button && !bad.adopt && (await page.locator(".ix-row[data-file='broken.md'] .ix-row-why").textContent()).includes("not valid"), `${tag} a bad id is listed with its reason and cannot be opened`);
  ok((await page.locator("[data-testid=ideas-skipped]").textContent()).includes("a-link.md"), `${tag} the skipped entry is mentioned, not hidden`);
  await page.fill("[data-testid=ideas-filter]", "gpu");
  ok(JSON.stringify(await page.evaluate(() => [...document.querySelectorAll(".ix-row-title")].map((e) => e.textContent))) === '["Home server GPU"]', `${tag} the filter matches titles`);
  await page.fill("[data-testid=ideas-filter]", "");

  // ── byte-exact, LF ──
  await openIdea(page, JELLY);
  const lf0 = (await file(page, JELLY)).content;
  ok(lf0.includes("<!--") && lf0.includes("| box |") && lf0.includes("```mermaid") && lf0.includes("related:"), `${tag} the LF fixture has an unknown key, a comment, a table and a mermaid fence`);
  await caretAfter(page, "Jellyfin on the home server");
  await page.keyboard.type("!");
  ok(await until(page, (id) => window.__mock.ideaWrites().some((w) => w.id === id), JELLY, 3000), `${tag} typing saves on its own after the pause`);
  await saved(page);
  const lf1 = (await file(page, JELLY)).content;
  ok(lf1 === lf0.replace("# Jellyfin on the home server", "# Jellyfin on the home server!"), `${tag} LF: the saved file differs from the original by the one typed character`);

  // ── byte-exact, CRLF ──
  await openIdea(page, GPU);
  const cr0 = (await file(page, GPU)).content;
  ok(cr0.includes("\r\n"), `${tag} the GPU fixture is CRLF`);
  await caretAfter(page, "Home server GPU");
  await page.keyboard.type("!");
  await until(page, (id) => window.__mock.ideaWrites().some((w) => w.id === id), GPU, 3000);
  await saved(page);
  ok((await file(page, GPU)).content === cr0.replace("# Home server GPU", "# Home server GPU!"), `${tag} CRLF: the saved file differs by the one typed character`);
  await page.keyboard.press("Enter");
  await page.keyboard.type("x");
  await page.locator(".ix-title").click(); // blur: a save point
  await until(page, (c) => window.__mock.ideaFile(c.id).content !== c.before, { id: GPU, before: (await file(page, GPU)).content }, 3000);
  ok((await file(page, GPU)).content === cr0.replace("# Home server GPU", "# Home server GPU!\r\nx"), `${tag} CRLF: Enter writes CRLF, and blur saved it without waiting`);

  // ── no change, no write ──
  await openIdea(page, TIDY);
  const tidy0 = (await file(page, TIDY)).content;
  await caretAfter(page, "Media library tidy");
  await page.keyboard.press("ArrowUp");
  await page.locator(".ix-title").click();
  await page.click("[data-testid=ideas-mode-read]");
  await page.click("[data-testid=ideas-mode-edit]");
  await caretAfter(page, "Media library tidy");
  await page.keyboard.type("q");
  await page.keyboard.press("Backspace");
  await page.waitForTimeout(1300);
  await page.locator(".ix-title").click();
  await openIdea(page, JELLY);
  await page.waitForTimeout(400);
  ok((await writes(page, TIDY)).length === 0 && (await file(page, TIDY)).content === tidy0, `${tag} opened, moved in, blurred, Read and back, typed and deleted back, switched away: no idea_write`);

  const attrs = await page.evaluate(() => { const c = document.querySelector(".cm-content"); return [c.getAttribute("autocorrect"), c.getAttribute("autocapitalize"), c.getAttribute("spellcheck")]; });
  ok(JSON.stringify(attrs) === '["off","off","true"]', `${tag} the source editor turns autocorrect and autocapitalize off, spellcheck on (${JSON.stringify(attrs)})`);

  // ── the frontmatter guard ──
  await openIdea(page, TIDY);
  await caretAfter(page, `id: ${TIDY}`);
  await page.keyboard.type("Z");
  await page.keyboard.press("Home");
  await page.keyboard.type("Z");
  await page.waitForTimeout(1100);
  // (A ULID holds Z's of its own: read the id line, not the whole text.)
  const gw = await writes(page, TIDY), idLine = await page.locator(".cm-line", { hasText: "id:" }).first().textContent();
  ok(gw.length === 0 && idLine === `id: ${TIDY}`, `${tag} a keystroke at the end or head of the id line changes nothing (writes ${gw.length}; ${JSON.stringify(idLine)})`);
  await page.locator(".cm-content").click();
  await page.keyboard.press(`${MOD}+a`);
  await page.keyboard.press("Backspace");
  await until(page, (id) => window.__mock.ideaWrites().some((w) => w.id === id), TIDY, 3000);
  await saved(page);
  ok((await file(page, TIDY)).content === `---\nid: ${TIDY}\n---\n` && !(await page.$("[data-testid=ideas-error]")), `${tag} select-all and delete keeps the three guarded lines, and the save is accepted`);

  // ── create ──
  await page.click("[data-testid=ideas-new]");
  await until(page, () => document.querySelector(".ix-row.on .ix-row-title")?.textContent === "Untitled");
  const newId = await page.evaluate(() => document.querySelector(".ix-row.on").dataset.idea);
  ok(await until(page, () => !!document.activeElement?.closest(".cm-content")), `${tag} a new idea opens with the cursor in the editor`);
  await page.keyboard.type("Hello from a new idea");
  await until(page, (id) => window.__mock.ideaFile(id)?.content.includes("Hello from a new idea"), newId, 3000);
  const created = (await file(page, newId))?.content ?? "";
  ok(/^---\nid: [0-9A-Z]{26}\ncreated: \d{4}-\d\d-\d\d\n---\n\n# Untitled\nHello from a new idea$/.test(created), `${tag} New idea is listed, takes typing at once at the end of its text, and saves (${JSON.stringify(created.slice(-40))})`);

  // ── rename ──
  await page.click("[data-testid=ideas-rename]");
  await page.fill("[data-testid=ideas-rename-input]", "Greetings plan");
  await page.keyboard.press("Enter");
  await until(page, (id) => window.__mock.ideaFile(id)?.file === "greetings-plan.md", newId, 3000);
  const ren = await file(page, newId);
  ok(ren?.file === "greetings-plan.md" && ren.content.includes("# Greetings plan") && ren.content.includes(`id: ${newId}`), `${tag} Rename moves the file name and the heading; the id stays`);
  ok(await until(page, () => document.querySelector(".cm-content")?.innerText.includes("# Greetings plan")), `${tag} and the editor shows the renamed heading`);
  await page.waitForTimeout(500); // the rename's own tick
  ok(!(await page.$("[data-testid=ideas-conflict]")), `${tag} the rename's own ideas:changed is not taken for an edit elsewhere`);

  // ── the first heading names the file ──
  await caretAfter(page, "# Greetings plan");
  await page.keyboard.type(" shared");
  ok(await until(page, (id) => window.__mock.ideaFile(id)?.file === "greetings-plan-shared.md", newId, 3000), `${tag} a save whose heading changed renames the file (${(await file(page, newId))?.file})`);
  ok(await until(page, (id) => document.querySelector(".ix-title")?.textContent === "Greetings plan shared"
    && document.querySelector(`[data-idea='${id}']`)?.dataset.file === "greetings-plan-shared.md", newId, 3000),
    `${tag} the header and the list row follow the save's answer`);
  await page.waitForTimeout(500); // the rename's own tick
  await page.keyboard.type("!");
  await saved(page);
  ok(!(await page.$("[data-testid=ideas-conflict]")) && !(await page.$("[data-testid=ideas-error]")) && (await file(page, newId))?.content.includes("# Greetings plan shared!"),
    `${tag} the renamed file's next save goes through: no conflict, no error`);

  // ── remove ──
  await page.click("[data-testid=ideas-remove]");
  ok((await page.locator("[data-testid=ideas-remove]").textContent()).includes(".history") && (await file(page, newId)) !== null, `${tag} Remove asks first and says the text is kept in .history`);
  await page.click("[data-testid=ideas-remove]");
  await until(page, (id) => !document.querySelector(`[data-idea='${id}']`), newId, 3000);
  ok((await file(page, newId)) === null && (await page.locator("[data-testid=ideas-notice]").textContent()).includes(".history"), `${tag} the second click removes it, and the notice names .history`);

  // ── adopt ──
  await page.click(".ix-row[data-file='notes-from-the-train.md'] [data-testid=ideas-adopt]");
  await until(page, () => document.querySelector(".ix-row.on")?.dataset.file === "notes-from-the-train.md" && document.querySelector(".ix-row.on")?.dataset.idea);
  const adoptedId = await page.evaluate(() => document.querySelector(".ix-row.on")?.dataset.idea);
  const adopted = adoptedId && (await file(page, adoptedId));
  ok(adopted && adopted.content.startsWith(`---\nid: ${adoptedId}\n`) && adopted.content.includes("Written in another editor."), `${tag} Adopt gives the hand-made file an id and opens it`);

  // ── an edit elsewhere: clean ──
  await openIdea(page, JELLY);
  const theirs = (await file(page, JELLY)).content.replace("which *box*", "which *box* — edited in vim");
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: JELLY, c: theirs });
  ok(await until(page, () => document.querySelector(".cm-content")?.innerText.includes("edited in vim")), `${tag} an edit elsewhere with the editor clean is reloaded`);
  ok(!(await page.$("[data-testid=ideas-conflict]")), `${tag} without a conflict`);

  // ── an edit elsewhere: dirty → Keep mine ──
  await caretAfter(page, "edited in vim");
  await page.keyboard.type(" MINE");
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: JELLY, c: theirs.replace("transcode", "THEIRS transcode") });
  ok(await until(page, () => !!document.querySelector("[data-testid=ideas-conflict]")), `${tag} an edit elsewhere while typing is a conflict`);
  ok((await editorText(page)).includes(" MINE") && (await file(page, JELLY)).content.includes("THEIRS"), `${tag} the typed text is kept, and nothing was written over theirs`);
  await page.click("[data-testid=ideas-keep-mine]");
  await until(page, (id) => window.__mock.ideaFile(id).content.includes(" MINE"), JELLY, 3000);
  const kept = (await file(page, JELLY)).content;
  ok(kept.includes(" MINE") && !kept.includes("THEIRS") && !(await page.$("[data-testid=ideas-conflict]")), `${tag} Keep mine writes the editor's text`);
  const hist = await page.evaluate((id) => window.__mock.ideaHistory(id), JELLY);
  ok(hist.length === 1 && hist[0].content.includes("THEIRS"), `${tag} and the text it overwrote went to history (keep_history)`);
  ok((await writes(page, JELLY)).filter((w) => w.keep_history).length === 1, `${tag} only Keep mine's write asked for a history copy`);

  // ── dirty → Reload ──
  await caretAfter(page, "MINE");
  await page.keyboard.type(" again");
  await page.evaluate((a) => window.__mock.ideaEditElsewhere(a.id, a.c), { id: JELLY, c: kept.replace("transcode", "OTHER transcode") });
  await until(page, () => !!document.querySelector("[data-testid=ideas-conflict]"));
  await page.click("[data-testid=ideas-reload]");
  ok((await editorText(page)).includes(" again"), `${tag} Reload asks first (one click drops nothing)`);
  await page.click("[data-testid=ideas-reload]");
  ok(await until(page, () => document.querySelector(".cm-content")?.innerText.includes("OTHER transcode") && !document.querySelector(".cm-content")?.innerText.includes(" again")), `${tag} the second click takes the file's text`);

  // ── removed: a save by rename, then a real removal ──
  const flick = (await file(page, JELLY)).content.replace("OTHER", "RENAMED-SAVE");
  await page.evaluate((a) => window.__mock.ideaFlickerElsewhere(a.id, a.c), { id: JELLY, c: flick });
  ok(await until(page, () => document.querySelector(".cm-content")?.innerText.includes("RENAMED-SAVE")) && !(await page.$("[data-testid=ideas-removed]")),
    `${tag} a \`removed\` that idea_read finds (a save by rename) is a change, and the editor stays open`);
  await caretAfter(page, "RENAMED-SAVE");
  await page.keyboard.type(" unsaved");
  await page.evaluate((id) => window.__mock.ideaRemoveElsewhere(id), JELLY);
  ok(await until(page, () => !!document.querySelector("[data-testid=ideas-removed]")), `${tag} a removal elsewhere, confirmed by idea_read, says so`);
  ok((await editorText(page)).includes(" unsaved"), `${tag} and the unsaved text stays in the editor`);
  await page.click("[data-testid=ideas-save-as-new]");
  await until(page, (old) => { const on = document.querySelector(".ix-row.on"); return on && on.dataset.idea !== old; }, JELLY, 3000);
  const reborn = await page.evaluate(() => document.querySelector(".ix-row.on")?.dataset.idea);
  const rf = reborn && (await file(page, reborn));
  ok(rf && rf.content.includes(" unsaved") && rf.content.includes(`id: ${reborn}`) && !rf.content.includes(JELLY.slice(-6) + "\n"), `${tag} Save as a new idea keeps the text under a new id`);

  // ── Read ──
  await openIdea(page, GPU);
  await page.click("[data-testid=ideas-mode-read]");
  const read = await page.evaluate(() => {
    const r = document.querySelector("[data-testid=ideas-read]");
    return { h1: r?.querySelector(".md-h1")?.textContent, text: r?.textContent ?? "", editorShown: getComputedStyle(document.querySelector(".ix-editor")).display !== "none" };
  });
  ok(read.h1 === "Home server GPU!" && !read.text.includes("id:") && !read.editorShown, `${tag} Read renders the body (h1 ${JSON.stringify(read.h1)}), without the frontmatter, in place of the editor`);
  await page.click(`[data-idea='${reborn}'] button.ix-row-main`);
  await until(page, () => !!document.querySelector("[data-testid=ideas-read] .md-h1"));
  const read2 = await page.evaluate(() => {
    const r = document.querySelector("[data-testid=ideas-read]");
    return { table: !!r?.querySelector("table"), comment: (r?.textContent ?? "").includes("7k2q"), mermaid: [...(r?.querySelectorAll("pre, code") ?? [])].some((e) => e.textContent.includes("graph TD")), svg: !!r?.querySelector("svg") };
  });
  ok(read2.table && !read2.comment && read2.mermaid && !read2.svg, `${tag} Read draws the table, no comment, the mermaid fence as code and no diagram (${JSON.stringify(read2)})`);
  await page.click("[data-testid=ideas-mode-edit]");

  // ── layout ──
  const lay = await page.evaluate(() => {
    const b = document.querySelector(".ix-body").getBoundingClientRect();
    const e = document.querySelector(".ix-editor").getBoundingClientRect();
    const side = document.querySelector(".ix-side").getBoundingClientRect();
    return { fill: Math.abs(b.width - e.width) < 1 && Math.abs(b.height - e.height) < 1 && b.height > 400, side: side.width, scroll: document.documentElement.scrollWidth <= window.innerWidth };
  });
  ok(lay.fill && lay.side === 0 && lay.scroll, `${tag} the editor fills its pane, the side column is empty, and the page never scrolls sideways (${JSON.stringify(lay)})`);
  for (const w of [1200, 720]) {
    await page.setViewportSize({ width: w, height: 700 });
    await page.waitForTimeout(100);
    const miss = await page.evaluate(() => [...document.querySelectorAll(".ix-head button, .ix-list-head button, .ix-list-head input, [data-testid=ideas-adopt], .ix-list-foot button")].filter((b) => {
      const q = b.getBoundingClientRect();
      if (!q.width || !q.height) return true;
      const hit = document.elementFromPoint(q.x + q.width / 2, q.y + q.height / 2);
      return !(hit && (b === hit || b.contains(hit)));
    }).map((b) => b.textContent.trim() || b.placeholder));
    ok(miss.length === 0, `${tag} at ${w}px every header and list control is reachable${miss.length ? `: missing ${miss.join(", ")}` : ""}`);
  }
  await page.setViewportSize({ width: 1200, height: 800 });

  // ── the six themes ──
  await openIdea(page, reborn);
  await page.click("[data-testid=ideas-mode-edit]").catch(() => {});
  for (const theme of THEMES) {
    await page.evaluate((t) => { if (t === "tokyo-night") delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = t; }, theme);
    const m = await page.evaluate(() => {
      const parse = (c) => {
        let x = /^rgba?\(([^)]+)\)/.exec(c);
        if (x) { const p = x[1].split(/[ ,/]+/).filter(Boolean).map(Number); return [p[0], p[1], p[2], p[3] ?? 1]; }
        x = /^color\(srgb ([^)]+)\)/.exec(c);
        if (x) { const p = x[1].split(/[ /]+/).filter(Boolean).map(Number); return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1]; }
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
      const ratio = (el) => { const bg = bgOf(el); const fg = over(parse(getComputedStyle(el).color), bg); const [a, b] = [lum(fg), lum(bg)].sort((p, q) => q - p); return (a + 0.05) / (b + 0.05); };
      const words = [...document.querySelectorAll(".ix-row-title, .ix-row-why > span, .ix-btn, .ix-seg > button, .ix-state, .ix-title, .ix-list-foot .ix-quiet, .ix-row-age")]
        .filter((e) => e.textContent.trim() && !e.disabled).map((e) => ({ t: e.textContent.trim().slice(0, 24), r: ratio(e) }));
      // A heading's `#` is a marker: dimmed, and still readable as a mark.
      const hashes = [...document.querySelectorAll(".cm-line span")].filter((s) => s.textContent.trim() === "#");
      const body = [...document.querySelectorAll(".cm-line")].find((l) => l.textContent.includes("unsaved"));
      const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
      return {
        low: words.filter((w) => w.r < 4.5),
        marker: hashes.length ? ratio(hashes[0]) : null,
        dimmer: hashes.length && body ? lum(over(parse(getComputedStyle(hashes[0]).color), bgOf(hashes[0]))) !== lum(over(parse(getComputedStyle(body).color), bgOf(body))) : false,
        surfaces: dist(bgOf(document.querySelector(".ix-list")), bgOf(document.querySelector(".ix-body"))),
      };
    });
    ok(m.low.length === 0, `${tag} ${theme}: every word in the list and the header reads ≥ 4.5:1${m.low.length ? `: ${m.low.map((w) => `${w.t} ${w.r.toFixed(2)}`).join("; ")}` : ""}`);
    ok(m.marker !== null && m.marker >= 2 && m.dimmer, `${tag} ${theme}: a heading's # is dimmed and still visible (${m.marker?.toFixed(2)}:1)`);
    ok(m.surfaces >= 6, `${tag} ${theme}: the list and the document are different surfaces (distance ${m.surfaces.toFixed(1)})`);
    if (SHOTS) await page.screenshot({ path: `${SHOTS}/ideas-${name}-${theme}.png` });
  }

  ok(errors.length === 0, `${tag} no page errors${errors.length ? `: ${errors.join(" | ")}` : ""}`);
  await browser.close();
}

await run(chromium, "chromium");
await run(webkit, "webkit");
console.log(failed ? `\nideas-browser: ${failed} failure(s)` : "\nideas-browser: all good");
process.exit(failed ? 1 : 0);
