// Headless drive of the mock harness in Chromium AND WebKit (the app's engine):
// a chat opened on a Claude that has written no session file yet says it is
// starting, never "Couldn't read the conversation", and shows the turns once
// the file is there, with no click. Run against `VITE_MOCK=1 vite --port 1473
// --force` in app/, with PLAYWRIGHT_MODULE pointing at a local Playwright
// installation. `?claudestart=<ms>` is the mock's starting window, counted
// from the page load. CHAT_START_SHOTS names a directory for screenshots.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.CHAT_START_URL ?? "http://localhost:1473";
const shots = process.env.CHAT_START_SHOTS;
let failures = 0;
const check = (ok, m) => { console.log(`${ok ? "ok" : "not ok"} — ${m}`); if (!ok) failures++; };

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(12_000);
    // Every state the pane passes through, in order, as the page sees it.
    await page.addInitScript(() => {
      window.__seen = [];
      const note = () => {
        const s = document.querySelector('[data-testid="chat-starting"]') ? "starting"
          : document.querySelector('[data-testid="chat-failed"]') ? "failed"
          : document.querySelector('[data-testid="chat-list"] > li') ? "turns" : null;
        if (s && window.__seen[window.__seen.length - 1] !== s) window.__seen.push(s);
      };
      new MutationObserver(note).observe(document, { subtree: true, childList: true });
    });
    await page.goto(`${base}/?turns=tools&agentprompt=off&claudestart=2500`);
    await page.locator('[data-testid="places-rail"]').click();
    await page.locator('li.row[data-slug="messaging"]').click();
    const starting = page.locator('[data-testid="chat-starting"]');
    await starting.waitFor();
    check(await starting.getAttribute("role") === "status", `${name}: the note is a status, not an alert`);
    check((await starting.textContent())?.includes("is starting"), `${name}: it says the agent is starting: ${JSON.stringify(await starting.textContent())}`);
    if (shots) await page.screenshot({ path: `${shots}/${name}-starting.png` });
    await page.locator('[data-testid="chat-list"] > li').first().waitFor();
    if (shots) await page.screenshot({ path: `${shots}/${name}-turns.png` });
    const seen = await page.evaluate(() => window.__seen);
    check(JSON.stringify(seen) === JSON.stringify(["starting", "turns"]), `${name}: starting, then the turns, and never the failure: ${JSON.stringify(seen)}`);
    check(await page.locator('[data-testid="chat-failed"]').count() === 0, `${name}: no "Couldn't read the conversation"`);
  } catch (e) {
    check(false, `${name}: ${e.message.split("\n")[0]}`);
  } finally {
    await browser.close();
  }
}
console.log(failures ? `\n${failures} failure(s)` : "\nall ok");
process.exit(failures ? 1 : 0);
