// Headless drive of the isolated mock harness on port 1451. Set
// PLAYWRIGHT_MODULE to a local Playwright installation before running.
// The query holds a busy Claude place and writes the first send as a
// queued_command absorbed into its current turn; the second is a normal turn.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");

const base = process.env.CHAT_QUEUED_URL ?? "http://localhost:1451";
const first = "Layer these two images for the tile.";
const second = "Give me more options.";
const place = "/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo/.worktrees/messaging"; // its mock path, for the draft push

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

    // A draft ending in a space: Claude records the line trimmed, and the
    // pending copy has to settle against that (chatsend-check has the orders).
    const spaced = "make it a PR";
    await send(`${spaced} `);
    await page.locator('[data-testid="chat-user"]', { hasText: spaced }).waitFor();
    await page.locator('[data-testid="chat-agent"]', { hasText: "On it. (A scripted reply" }).nth(2).waitFor();
    check(await page.locator('[data-testid="chat-user"]', { hasText: spaced }).count() === 1, `${name}: a draft ending in a space has one user bubble`);
    check(await page.locator('[data-testid="chat-pending"]').count() === 0, `${name}: a draft ending in a space leaves no pending copy`);

    // The composer's sample caught that line between the paste and Enter: no
    // notice. Anything else in the terminal's input still gets one.
    const notice = page.locator('[data-testid="chat-pane-draft"]');
    await page.evaluate(([path, text]) => window.__mock.setDrafts([{ path, text, queued: false }]), [place, spaced]);
    await page.waitForTimeout(150);
    check(await notice.count() === 0, `${name}: a sample of the line just sent shows no unsent-text notice`);
    await page.evaluate(([path, text]) => window.__mock.setDrafts([{ path, text, queued: false }]), [place, "something typed in the terminal"]);
    await notice.waitFor();
    check((await notice.textContent()).includes("something typed in the terminal"), `${name}: other text in the terminal still shows the notice`);
  } finally {
    await browser.close();
  }
}
