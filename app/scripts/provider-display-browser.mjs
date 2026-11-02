// Drive the real App against the isolated mock on port 1452 in both engines.
// A provider switch leaves a canonical shell signal and Claude history while
// Codex runs in its sidecar; every visible identity must follow Codex.
import assert from "node:assert/strict";

const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.PROVIDER_DISPLAY_URL ?? "http://localhost:1452";

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(12_000);
    await page.goto(`${base}/?activity=1&agentprompt=off&agentsend=gone`);
    await page.locator('[data-testid="places-rail"]').click();
    const codex = page.locator('li.row[data-slug="billing-refactor"]');
    await codex.click();
    await codex.locator('.g-agent-codex').waitFor();
    await page.locator('[data-testid="chat-agent"][aria-label^="Codex"]').first().waitFor();
    await page.locator('[data-testid="chat-field"]').fill("Are you there?");
    await page.locator('[data-testid="chat-send"]').click();
    await page.locator('[data-testid="chat-line"]', { hasText: "No agent is running in this place." }).waitFor();

    const row = page.locator('li.row[data-slug="messaging"]');
    await row.click();
    await page.locator('[data-testid="chat"]').waitFor();
    await row.locator('.g-agent-claude').waitFor();
    assert.match(await page.locator('[data-testid="chat-agent"]').first().getAttribute("aria-label"), /^Claude/);

    const repo = await page.evaluate(async () => {
      const ws = await window.__TAURI_INTERNALS__.invoke("list_workspace");
      return ws.projects.find((p) => p.snapshot?.places.some((pl) => pl.slug === "messaging")).root;
    });
    await page.evaluate(async (root) => window.__mock.switchAgent(root, "messaging", "codex", true), repo);
    await row.locator('.g-agent-codex').waitFor();
    assert.match(await page.locator('.agent-cell .agent-name').first().textContent(), /Codex/);
    await page.locator('[data-testid="chat-agent"][aria-label^="Codex"]').first().waitFor();
    assert.equal(await row.locator('.g-agent-claude').count(), 0);

    await page.locator('[data-testid="chat-field"]').fill("Are you there?");
    await page.locator('[data-testid="chat-send"]').click();
    await page.locator('[data-testid="chat-line"]', { hasText: "No agent is running in this place." }).waitFor();
    assert.equal(await page.locator('[data-testid="chat-line"]').textContent(), "No agent is running in this place.");
    console.log(`ok — ${name}: Codex mark, cell, turns and neutral no-agent line after Claude switch`);

    // A delayed snapshot can still name Claude while the transcript reader
    // already found Codex. The turn label follows the reader's harness.
    const delayed = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    delayed.setDefaultTimeout(12_000);
    await delayed.goto(`${base}/?activity=1&agentprompt=off&turnsharness=codex`);
    await delayed.locator('[data-testid="places-rail"]').click();
    const stale = delayed.locator('li.row[data-slug="messaging"]');
    await stale.click();
    await stale.locator('.g-agent-claude').waitFor();
    await delayed.locator('[data-testid="chat-agent"][aria-label^="Codex"]').first().waitFor();
    console.log(`ok — ${name}: turn label follows agent_turns.harness during a delayed snapshot`);
  } finally {
    await browser.close();
  }
}
