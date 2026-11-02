// Run against `pnpm dev:mock --port 1454 --force` in Chromium and WebKit.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const base = process.env.CHAT_MULTIFORM_URL ?? "http://localhost:1454";

function check(ok, message) {
  if (!ok) throw new Error(message);
  console.log(`ok — ${message}`);
}

async function open(page, variant) {
  await page.goto(`${base}/?agentprompt=${variant}`);
  await page.locator('[data-testid="places-rail"]').click();
  await page.locator('li.row[data-slug="messaging"]').click();
  await page.locator('[data-testid="chat-prompt"]').waitFor();
}

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(12_000);
    await open(page, "multi-form");
    check(await page.locator('[data-testid="chat-boxes"] input[type="checkbox"]').count() === 4, `${name}: all boxes show in the form`);
    check(await page.locator('[data-testid="chat-send-answer"]').count() === 0, `${name}: no single-answer button for the boxes`);
    check((await page.locator('[data-testid="chat-prompt-lines"]').innerText()).includes("☐ Toppings  ☐ Size"), `${name}: form tabs stay above the choices`);
    check(await page.locator('[data-testid="chat-box-text"]').count() === 1, `${name}: free-text box has a field`);
    await page.locator('[data-testid="chat-box-1"]').click();
    await page.locator('[data-testid="chat-prompt"]').click({ position: { x: 8, y: 8 } });
    await page.keyboard.press("2");
    await page.waitForFunction(() => document.querySelector('[data-testid="chat-box-2"]')?.checked === true);
    check(await page.locator('[data-testid="chat-box-1"]').isChecked() && await page.locator('[data-testid="chat-box-2"]').isChecked(), `${name}: number key toggles a second box`);
    check((await page.locator('[data-testid="chat-send-many"]').innerText()) === "Send 2 choices", `${name}: send button counts ticks`);
    await page.locator('[data-testid="chat-send-many"]').click();
    await page.getByText("Which size do you want?").waitFor();
    check(await page.locator('[data-testid="chat-boxes"]').count() === 0, `${name}: Next advanced to the next question`);

    await open(page, "multi-form");
    await page.locator('[data-testid="chat-box-text"]').fill("pineapple");
    check(await page.locator('[data-testid="chat-box-4"]').isChecked(), `${name}: typing ticks the free-text box`);
    await page.locator('[data-testid="chat-send-many"]').click();
    await page.getByText("Which size do you want?").waitFor();
    check(await page.locator('[data-testid="chat-send-answer"]').count() === 0, `${name}: field sent through the multi-select path`);

    await open(page, "multi-form-typed");
    check(await page.locator('[data-testid="chat-box-4"]').isChecked(), `${name}: held terminal text stays checked`);
    check(await page.getByText("Typed in the terminal: sent as it reads").count() === 1, `${name}: held terminal text is explained`);
    check(await page.locator('[data-testid="chat-send-answer"]').count() === 0, `${name}: held text is never offered as a single answer`);
  } finally {
    await browser.close();
  }
}
