// Headless drive of the mock harness in Chromium AND WebKit: a place whose
// tmux session is up with NO agent in it (pane 0 is still the launch shell,
// stopped on an rc file's question: oh-my-zsh's update prompt, 2026-11-26)
// shows its terminal and says the agent did not start, and a key typed there
// reaches the session. v0.54.0 drew "No live session … Enter to start" with no
// terminal at all, so the question could not be answered from the app.
//
//   VITE_MOCK=1 vite --port 1479 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     NO_AGENT_URL=http://localhost:1479 node app/scripts/place-no-agent-browser.mjs
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.NO_AGENT_URL ?? "http://localhost:1479";
let failures = 0;
const check = (ok, m) => { console.log(`${ok ? "ok" : "not ok"} — ${m}`); if (!ok) failures++; };
const SLUG = "messaging";

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(12_000);
    await page.goto(`${base}/?agentprompt=off`);
    await page.locator('[data-testid="places-rail"]').click();
    // The place's session stays up; its agent never came.
    await page.evaluate(async (slug) => {
      const t = window.__TAURI_INTERNALS__;
      const ws = await t.invoke("list_workspace", {});
      const p = ws.projects.find((p) => p.snapshot?.places.some((x) => x.slug === slug));
      window.__mock.shellOnly(p.root, slug);
      window.__writes = [];
      const i = t.invoke;
      t.invoke = (c, a) => { if (c === "term_write") window.__writes.push(a.data); return i(c, a); };
    }, SLUG);
    await page.locator(`li.row[data-slug="${SLUG}"]`).click();
    const cell = page.locator('[data-testid="place-no-agent"]');
    await cell.waitFor();
    check(/did not start/.test(await cell.innerText()), `${name}: the place says the agent did not start`);
    check(await page.getByText("No live session for").count() === 0, `${name}: not "No live session"`);
    const term = cell.locator(".xterm");
    await term.waitFor();
    check(await term.count() === 1, `${name}: its terminal is there`);
    await term.click();
    await page.keyboard.type("y");
    const wrote = await page.waitForFunction(() => window.__writes.some((d) => d.includes(121)), null, { timeout: 4000 }).then(() => true, () => false);
    check(wrote, `${name}: a key typed there reaches the session`);
  } catch (e) {
    check(false, `${name}: ${e.message.split("\n")[0]}`);
  } finally {
    await browser.close();
  }
}
console.log(failures ? `\n${failures} failure(s)` : "\nall ok");
process.exit(failures ? 1 : 0);
