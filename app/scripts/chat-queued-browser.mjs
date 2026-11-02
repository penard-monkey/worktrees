// Headless drive of the isolated mock harness on port 1451. Set
// PLAYWRIGHT_MODULE to a local Playwright installation before running.
// The query holds a busy Claude place and writes the first send as a
// queued_command absorbed into its current turn; the second is a normal turn.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");

const base = process.env.CHAT_QUEUED_URL ?? "http://localhost:1451";
const first = "Layer these two images for the tile.";
const second = "Give me more options.";

function check(ok, message) {
  if (!ok) throw new Error(message);
  console.log(`ok — ${message}`);
}

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(12_000);
    await page.goto(`${base}/?turns=queued&activity=1&agentprompt=off`);
    await page.locator('[data-testid="places-rail"]').click();
    await page.locator('li.row[data-slug="messaging"]').click();
    await page.locator('[data-testid="chat"]').waitFor();

    async function send(text) {
      await page.locator('[data-testid="chat-field"]').fill(text);
      await page.locator('[data-testid="chat-send"]').click();
    }

    await send(first);
    await page.locator('[data-testid="chat-user"]', { hasText: first }).waitFor();
    await page.locator('[data-testid="chat-agent"]', { hasText: "On it. (A scripted reply" }).last().waitFor();
    check(await page.locator('[data-testid="chat-user"]', { hasText: first }).count() === 1, `${name}: absorbed message has one user bubble`);
    check(await page.getByText("Sent while Claude was working").count() === 1, `${name}: absorbed bubble names when it was sent`);
    check(await page.locator('[data-testid="chat-pending"]').count() === 0, `${name}: absorbed message has no pending duplicate`);

    await send(second);
    await page.locator('[data-testid="chat-user"]', { hasText: second }).waitFor();
    await page.locator('[data-testid="chat-agent"]', { hasText: "On it. (A scripted reply" }).nth(1).waitFor();
    check(await page.locator('[data-testid="chat-user"]', { hasText: second }).count() === 1, `${name}: normal turn has one user bubble`);
    check(await page.locator('[data-testid="chat-pending"]').count() === 0, `${name}: normal turn has no pending duplicate`);
    const rows = await page.locator('[data-testid="chat-list"] > li').allTextContents();
    const positions = [first, second].map((s) => rows.findIndex((r) => r.includes(s)));
    check(positions[0] >= 0 && positions[1] > positions[0], `${name}: messages stay in terminal order`);
    check(!rows.join(" ").includes("waiting for Claude"), `${name}: no stale waiting status remains`);
  } finally {
    await browser.close();
  }
}
