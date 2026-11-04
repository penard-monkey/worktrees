// Headless drive of the mock harness for slash commands in the chat
// (docs/proposals/chat-slash-commands.md), in Chromium AND WebKit. Run the
// harness on its own port (`VITE_MOCK=1 vite --port 1456`) and set
// PLAYWRIGHT_MODULE to a local Playwright installation:
//
//   CHAT_SLASH_URL=http://localhost:1456 PLAYWRIGHT_MODULE=… node app/scripts/chat-slash-browser.mjs
//
// The menu (filtering, arrows, Esc), a model list answered from its card
// (this session only), a dialog closed, inline output as a Command notice,
// the two-press confirm, a refusal, a busy agent's queued command, and
// Codex's model then effort lists and its printed output.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");

const base = process.env.CHAT_SLASH_URL ?? "http://localhost:1456";

function check(ok, message) {
  if (!ok) throw new Error(message);
  console.log(`ok — ${message}`);
}

async function open(page, query, slug) {
  await page.goto(`${base}/?${query}`);
  await page.locator('[data-testid="places-rail"]').click();
  await page.locator(`li.row[data-slug="${slug}"]`).click();
  await page.locator('[data-testid="chat"]').waitFor();
}

const field = (page) => page.locator('[data-testid="chat-field"]');
const line = (page) => page.locator('[data-testid="chat-line"]');
const menuRows = (page) => page.locator('[data-testid="chat-slash"] [role="option"]');

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(12_000);

    // ── the menu ──────────────────────────────────────────────────────────
    await open(page, "agentprompt=off", "messaging");
    await field(page).click();
    await page.keyboard.type("/");
    await menuRows(page).first().waitFor();
    check(await menuRows(page).count() >= 10, `${name}: / opens the menu of the agent's commands`);
    await page.keyboard.type("sta");
    await page.waitForFunction(() => document.querySelectorAll('[data-testid="chat-slash"] [role="option"]').length < 5);
    check((await menuRows(page).first().textContent()).includes("/status"), `${name}: typing filters it, the name that starts with it first`);
    const hit = await page.evaluate(() => {
      const r = document.querySelector('[data-testid="chat-slash"] [role="option"]').getBoundingClientRect();
      return document.elementFromPoint(r.x + 10, r.y + r.height / 2)?.closest('[role="option"]') !== null;
    });
    check(hit, `${name}: the menu's row is reachable (hit-tested), not clipped`);
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => !document.querySelector('[data-testid="chat-slash"]'));
    check(await field(page).inputValue() === "/sta", `${name}: Esc closes the menu and keeps the draft`);
    await field(page).fill("");

    // ── a dialog, opened and closed ─────────────────────────────────────────
    await field(page).fill("/sta");
    await page.keyboard.press("Enter");
    check(await field(page).inputValue() === "/status", `${name}: Enter on a partial name fills it`);
    await page.keyboard.press("Enter");
    const card = page.locator('[data-testid="chat-prompt"]');
    await card.waitFor();
    check((await card.textContent()).includes("Settings  Status"), `${name}: /status opens its dialog as a prompt card`);
    await card.getByText("Close", { exact: true }).click();
    await card.getByRole("button", { name: /Send answer/ }).click();
    await page.waitForFunction(() => !document.querySelector('[data-testid="chat-prompt"]'));
    check(true, `${name}: its one choice, Close, closes it`);

    // ── a model list, answered for this session ────────────────────────────
    await field(page).fill("/model");
    await page.keyboard.press("Enter");
    await card.waitFor();
    check(await page.locator('[data-testid="chat-session-only"]').count() === 1, `${name}: the model card says it picks for this session only`);
    await card.getByText("Opus 5.5 ✔", { exact: true }).click();
    await card.getByRole("button", { name: /Send answer/ }).click();
    const notice = page.locator('[data-testid="chat-notice-command"]', { hasText: "/model" });
    await notice.waitFor();
    check(true, `${name}: the pick lands as the /model notice`);

    // ── inline output ──────────────────────────────────────────────────────
    await field(page).fill("/context");
    await page.keyboard.press("Enter");
    const ctx = page.locator('[data-testid="chat-notice-command"]', { hasText: "/context" });
    await ctx.waitFor();
    await ctx.getByText("Show command output").click();
    check((await ctx.locator("pre").textContent()).includes("Context Usage"), `${name}: /context's output is its notice's, in lines`);

    // ── the confirm ────────────────────────────────────────────────────────
    const before = await page.locator('[data-testid="chat-notice-command"]').count();
    await field(page).fill("/clear");
    await page.keyboard.press("Enter");
    check((await line(page).textContent()).startsWith("Press Enter again to run /clear"), `${name}: /clear asks for a second press`);
    check(await field(page).inputValue() === "/clear", `${name}: nothing ran on the first`);
    await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector('[data-testid="chat-field"]').value === "");
    check(true, `${name}: the second press runs it`);
    await field(page).fill("/login");
    await page.keyboard.press("Enter");
    check((await line(page).textContent()).includes("A sensitive command"), `${name}: a sensitive command confirms with why`);
    void before;

    // ── refusals ───────────────────────────────────────────────────────────
    await field(page).fill("/model sonnet");
    check((await line(page).textContent()).startsWith("/model takes no argument from the chat"), `${name}: /model with an argument is refused before the host is asked`);
    check(await page.locator('[data-testid="chat-send"]').isDisabled(), `${name}: and Send is off`);
    await open(page, "agentprompt=off&agentcmd=device", "messaging");
    await field(page).fill("/login");
    await page.keyboard.press("Enter");
    await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector('[data-testid="chat-line"]').textContent.includes("admin permission"));
    check(true, `${name}: the host's refusal says why (a phone without admin)`);

    // ── a busy agent ───────────────────────────────────────────────────────
    await open(page, "agentprompt=off&activity=1", "messaging");
    await field(page).fill("/context");
    await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector('[data-testid="chat-line"]').textContent.includes("queued behind the running turn"));
    check(true, `${name}: over a running turn the command is queued, and says so`);

    // ── Codex ──────────────────────────────────────────────────────────────
    await open(page, "agentprompt=off", "billing-refactor");
    await field(page).fill("/model");
    await page.keyboard.press("Enter");
    await card.waitFor();
    check((await card.textContent()).includes("Select Model and Effort"), `${name}: Codex's model list`);
    check((await card.textContent()).includes("Choosing it would make it your default"), `${name}: a model that would be saved is the terminal's`);
    await card.getByText("GPT-6-Sol (current)", { exact: true }).click();
    await card.getByRole("button", { name: /Send answer/ }).click();
    await page.waitForFunction(() => document.querySelector('[data-testid="chat-prompt"]')?.textContent.includes("Select Reasoning Level"));
    check(true, `${name}: then its effort list`);
    const effort = page.locator('[data-testid="chat-prompt"]');
    await effort.getByText("Medium (default) (current)", { exact: true }).click();
    await effort.getByRole("button", { name: /Send answer/ }).click();
    await page.waitForFunction(() => !document.querySelector('[data-testid="chat-prompt"]'));
    await field(page).fill("/status");
    await page.keyboard.press("Enter");
    const cx = page.locator('[data-testid="chat-notice-command"]', { hasText: "/status" });
    await cx.waitFor();
    await cx.getByText("Show command output").click();
    check((await cx.locator("pre").textContent()).includes("GPT-6-Sol"), `${name}: Codex's /status output is a notice of this window's`);
  } finally {
    await browser.close();
  }
}
