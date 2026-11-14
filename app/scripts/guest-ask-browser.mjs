// Headless drive of the mock harness as a GUEST (group chat §7, §2.2), in
// WebKit AND Chromium: a place name's *Ask David to share* form takes the
// row's full width, stays open and keeps its note while the app re-renders,
// and its send reaches the host.
//
//   app/: VITE_MOCK=1 ./node_modules/.bin/vite --port 1437 --force
//   PLAYWRIGHT_MODULE=<a local playwright> node app/scripts/guest-ask-browser.mjs [port]
//
// Why: on the owner's guest VM (2026-11-14, 1365×904) the form drew one word
// per line, and after a note and a press of the send nothing reached the host
// and Ask would not open again. Two causes, both measured here:
//  - `.row` is a three-column grid (dot, name, glyphs), so the form, its
//    fourth child, wrapped into the 14px DOT column: input and send fail
//    `elementFromPoint`, and the form ran 345px down past the window;
//  - the rows under a local project are drawn by App's `ProjectNode`, which
//    was a component DEFINED inside App, so every App render (the 3 s poll)
//    remounted them and the form's state, note included, was gone.
// Rects alone say nothing about reachability (AGENTS.md): every part is
// hit-tested, and the form must survive a poll tick with its note and focus.
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = Number(process.argv[2] ?? 1437);
const URL0 = `http://localhost:${port}/?guest&beta=group_chat`;
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));

for (const engine of ["webkit", "chromium"]) {
  const browser = await pw[engine].launch();
  for (const pin of [true, false]) {
    const page = await browser.newPage({ viewport: { width: 1365, height: 904 } });
    const tag = `${engine} 1365×904${pin ? " sidebar pinned" : " sidebar overlay"}`;
    await page.goto(URL0);
    await page.waitForSelector("[data-testid=guest-name-row]", { state: "attached", timeout: 15000 });
    if (pin) { await page.keyboard.press("Meta+b"); await page.waitForTimeout(400); }
    else { await page.mouse.move(20, 300); await page.waitForTimeout(600); }
    // The first name row under the local `worktrees` project: (main).
    const sel = ".project:not(.guest-project) [data-testid=guest-name-row]";
    const btn = await page.$(`${sel} [data-testid=guest-ask]`);
    if (!btn) { fail(`${tag}: a name row under the local project has an Ask button`); await page.close(); continue; }
    await btn.scrollIntoViewIfNeeded();
    await btn.click();
    await page.waitForTimeout(250);
    const m = await page.evaluate((sel) => {
      const f = document.querySelector(`${sel} .guest-askform`);
      if (!f) return null;
      const li = f.closest("li");
      const r = (e) => { const b = e.getBoundingClientRect(); return { x: b.x, y: b.y, w: b.width, h: b.height }; };
      const hit = (e) => { const b = e.getBoundingClientRect(); const x = document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2); return !!x && (x === e || e.contains(x)); };
      const inp = f.querySelector("input"), send = f.querySelector("[data-testid=guest-ask-send]");
      const cs = getComputedStyle(li);
      const inner = li.getBoundingClientRect().width - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
      return { inner, li: r(li), form: r(f), input: r(inp), send: r(send), hitInput: hit(inp), hitSend: hit(send), focused: document.activeElement === inp, vh: innerHeight };
    }, sel);
    if (!m) { fail(`${tag}: Ask opens the form`); await page.close(); continue; }
    check(m.form.w >= m.inner - 1, `${tag}: the form takes the row's width (${Math.round(m.form.w)} of ${Math.round(m.inner)} inside its padding)`);
    check(m.form.h <= 160 && m.form.y + m.form.h <= m.vh, `${tag}: the form is short and in the window (${Math.round(m.form.y)}+${Math.round(m.form.h)} of ${m.vh})`);
    check(m.hitInput && m.input.w >= 120, `${tag}: the note field takes a click (${Math.round(m.input.w)}px wide)`);
    check(m.hitSend, `${tag}: the send takes a click`);
    check(m.focused, `${tag}: the note field has focus`);
    await page.keyboard.type("could I watch?");
    // A poll tick or two: the app re-renders, the form must not.
    await page.waitForTimeout(4000);
    const after = await page.evaluate((sel) => {
      const inp = document.querySelector(`${sel} .guest-askform input`);
      return inp ? { value: inp.value, focused: document.activeElement === inp } : null;
    }, sel);
    check(after && after.value === "could I watch?", `${tag}: the form and its note survive a re-render (${after ? JSON.stringify(after.value) : "form gone"})`);
    check(after && after.focused, `${tag}: the note keeps its focus through a re-render`);
    const before = (await page.evaluate(() => window.__mock.guestCalls())).filter((c) => c === "share_request").length;
    const sendBtn = await page.$(`${sel} [data-testid=guest-ask-send]`);
    if (sendBtn) await sendBtn.click();
    await page.waitForTimeout(400);
    const sent = (await page.evaluate(() => window.__mock.guestCalls())).filter((c) => c === "share_request").length;
    check(sent === before + 1, `${tag}: the send reaches the host (${sent - before} share_request)`);
    check(!!(await page.$(`${sel} [data-testid=guest-asked-mark]`)), `${tag}: the row says Asked`);
    await page.close();
  }
  await browser.close();
}
console.log(failed ? `\nguest-ask-browser: ${failed} failure(s)` : "\nguest-ask-browser: all good");
process.exit(failed ? 1 : 0);
