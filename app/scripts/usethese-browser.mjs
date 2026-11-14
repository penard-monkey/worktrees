// Headless drive of *Use these* (group chat §5) as the owner, in WebKit AND
// Chromium, against the mock: the note is the panel's first field and has
// focus; a send with an empty note asks once ("Send without telling the agent
// what to do with them?"); "Don't ask again" is kept in settings
// (`use_these_ask_note`) and the next empty send goes without asking.
//
//   app/: VITE_MOCK=1 ./node_modules/.bin/vite --port 1437 --force
//   PLAYWRIGHT_MODULE=<a local playwright> node app/scripts/usethese-browser.mjs [port]
//
// usethese-check.mjs holds the same rules on the source.
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = Number(process.argv[2] ?? 1437);
const URL0 = `http://localhost:${port}/?beta=group_chat`;
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));
const stored = (page) => page.evaluate(() => JSON.parse(sessionStorage.getItem("wt-mock-ui-state") ?? "null")?.use_these_ask_note);

for (const engine of ["webkit", "chromium"]) {
  const browser = await pw[engine].launch();
  const page = await browser.newPage({ viewport: { width: 1365, height: 1000 } });
  const tag = engine;
  await page.addInitScript((s) => {
    if (!sessionStorage.getItem("wt-mock-ui-state")) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify(s));
  }, { settings_rev: 5, settings_mode: "advanced", nav_pinned: true, beta_group_chat: true });
  await page.goto(URL0);
  await page.waitForSelector("[data-track=settings]");
  await page.waitForTimeout(1500);
  // A place with a live Claude, shared with Bret, and a post of his.
  const slug = await page.evaluate(async () => {
    const ws = await window.__TAURI_INTERNALS__.invoke("list_workspace", {});
    for (const pv of ws.projects) for (const p of pv.snapshot?.places ?? []) if (p.agent_sessions?.claude?.up) return p.slug;
    return null;
  });
  if (!slug) { fail(`${tag}: the mock has a place with a live Claude`); await browser.close(); continue; }
  await page.evaluate(() => window.__mock.guestDevice());
  await page.evaluate((slug) => document.querySelector(`.nav-scroll [data-slug="${CSS.escape(slug)}"]`)?.click(), slug);
  await page.waitForSelector("[data-testid=share-with]", { timeout: 6000 }).catch(() => {});
  await page.click("[data-testid=share-with]");
  await page.waitForSelector(".ls-modal .ls-pick input[type=checkbox]", { timeout: 6000 }).catch(() => {});
  await page.evaluate(() => { const b = document.querySelector(".ls-modal .ls-pick input[type=checkbox]"); if (b && !b.checked) b.click(); });
  await page.waitForTimeout(200);
  await page.click("[data-testid=share-confirm]");
  await page.waitForSelector("[data-testid=lane-thread]", { timeout: 6000 }).catch(() => {});
  await page.evaluate(() => window.__mock.guestPost("try the --json flag"));
  await page.waitForTimeout(1200);
  const opened = await page.$("[data-track=use-these-open]");
  if (!opened) { fail(`${tag}: Use these… is offered`); await browser.close(); continue; }
  await opened.click();
  await page.waitForTimeout(300);
  const m = await page.evaluate(() => {
    const u = document.querySelector("[data-testid=use-these]");
    const note = u?.querySelector("[data-testid=use-these-note]");
    const first = u?.querySelector("input:not([type=checkbox]), textarea, pre");
    return { focused: document.activeElement === note, first: first === note, placeholder: note?.placeholder, spell: note?.getAttribute("spellcheck") };
  });
  check(m.first, `${tag}: the note is the panel's first field`);
  check(m.focused, `${tag}: the note has focus`);
  check(m.placeholder === "Fold these in and show me the result", `${tag}: the note's placeholder is an instruction (${m.placeholder})`);
  check(m.spell === "false", `${tag}: the note turns spellcheck (WebKit's substitutions) off`);
  // Empty note: Send asks first, and sends nothing.
  await page.click("[data-testid=use-these-send]");
  await page.waitForTimeout(300);
  const ask = await page.$("[data-testid=use-these-ask]");
  check(!!ask && /Send without telling the agent what to do with them\?/.test(await ask.textContent()), `${tag}: an empty note asks before the send`);
  check(!(await page.$("[data-testid=use-these-words]")), `${tag}: …and nothing was sent yet`);
  await page.click("[data-testid=use-these-never]");
  await page.click("[data-testid=use-these-send-anyway]");
  await page.waitForTimeout(1200);
  const words = await page.evaluate(() => document.querySelector("[data-testid=use-these] [role=status]")?.textContent ?? null);
  check(!!words, `${tag}: Send without a note sends (${words})`);
  check((await stored(page)) === false, `${tag}: Don't ask again is kept in settings (use_these_ask_note: false)`);
  // The next empty send does not ask (the mock's agent refused the first,
  // so the panel is still open with its selection).
  if (!(await page.$("[data-testid=use-these-send]"))) await (await page.$("[data-track=use-these-open]"))?.click();
  await page.waitForTimeout(300);
  await page.click("[data-testid=use-these-send]");
  await page.waitForTimeout(400);
  check(!(await page.$("[data-testid=use-these-ask]")), `${tag}: after Don't ask again, an empty send does not ask`);
  await browser.close();
}
console.log(failed ? `\nusethese-browser: ${failed} failure(s)` : "\nusethese-browser: all good");
process.exit(failed ? 1 : 0);
