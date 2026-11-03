// Settings → Updates and the shared announcement, in Chromium and WebKit.
// Run only against VITE_MOCK=1 on port 1456; update_harness is then a mock.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");

const BASE = process.env.HARNESS_MOCK_URL ?? "http://localhost:1456";
let failures = 0;
const ok = (condition, label) => {
  console.log(`${condition ? "ok  " : "FAIL"} ${label}`);
  if (!condition) failures++;
};

for (const [engine, type] of Object.entries({ chromium, webkit })) {
  const browser = await type.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1120, height: 800 } });
  const query = "?harnessupdates=1&harnessfail=pi&harnessslow=claude";
  await page.goto(`${BASE}/${query}`);
  const bubble = page.locator("[data-testid=update-bubble]");
  await bubble.waitFor({ timeout: 12_000 });
  const announcement = await bubble.textContent();
  ok(announcement.includes("Claude 2.1.320") && announcement.includes("Kimi 0.44.0") && announcement.includes("Pi 0.100.0"),
    `${engine}: bubble lists available harness versions`);
  ok(!announcement.includes("Worktrees"), `${engine}: harness-only bubble has no worktrees release`);
  await bubble.locator(".enter-btn").click();
  const section = page.locator("[data-testid=harness-updates]");
  await section.waitFor();
  ok(await section.locator(".harness-update").count() === 4, `${engine}: four CLI rows`);

  const claude = section.locator("[data-testid=harness-update-claude]");
  const codex = section.locator("[data-testid=harness-update-codex]");
  const kimi = section.locator("[data-testid=harness-update-kimi]");
  const pi = section.locator("[data-testid=harness-update-pi]");
  ok((await claude.textContent()).includes("2.1.317 → 2.1.320"), `${engine}: installed and latest Claude versions`);
  ok((await codex.textContent()).includes("0.166.0 → 0.166.0"), `${engine}: current Codex version`);
  ok(await codex.locator("button").count() === 0, `${engine}: current Codex has no update button`);
  ok((await kimi.textContent()).includes("can't tell how this Kimi Code was installed"), `${engine}: unknown Kimi method says why it is manual`);
  ok(await kimi.getByRole("button", { name: "Update", exact: true }).count() === 0, `${engine}: unknown Kimi method has no update button`);
  const steps = kimi.locator("[data-testid=harness-steps-kimi]");
  ok(await steps.evaluate((d) => d.open), `${engine}: steps are open while a newer Kimi is out`);
  ok(await steps.locator("pre").first().textContent() === "kimi upgrade", `${engine}: the first step is Kimi's own updater`);
  await steps.getByRole("button", { name: "Copy" }).first().click();
  await steps.getByRole("button", { name: "Copied" }).waitFor();
  ok(true, `${engine}: a step's command copies`);
  ok(await codex.locator("[data-testid=harness-steps-codex]").count() === 0, `${engine}: an updatable Codex shows no steps`);
  ok(await claude.locator("button").count() === 1, `${engine}: available Claude has an Update button`);

  await page.getByRole("button", { name: "Check for updates", exact: true }).click();
  ok((await page.locator(".settings-body").textContent()).includes("updates available"), `${engine}: manual check reports harness updates`);

  await claude.locator("button").click();
  ok((await claude.locator("button").textContent()).includes("Updating"), `${engine}: slow update shows progress`);
  await claude.locator("[data-testid=update-output-claude]").getByText("$ claude update").waitFor();
  await page.waitForFunction(() => document.querySelector("[data-testid=harness-update-claude]")?.textContent?.includes("2.1.320 → 2.1.320"));
  ok(await claude.locator("button").count() === 0 && (await claude.textContent()).includes("✓ done"),
    `${engine}: success refreshes the installed version and hides Update`);

  await pi.locator("button").click();
  await page.waitForFunction(() => document.querySelector("[data-testid=update-output-pi]")?.textContent?.includes("failed (exit 42)"));
  ok((await pi.textContent()).includes("0.99.1 → 0.100.0") && await pi.locator("button").count() === 1,
    `${engine}: failure leaves Pi update available and displays output`);
  await page.close();

  const both = await browser.newPage();
  await both.goto(`${BASE}/?latest=v9.9.9&harnessupdates=1`);
  const all = both.locator("[data-testid=update-bubble]");
  await all.waitFor({ timeout: 12_000 });
  const text = await all.textContent();
  ok(text.includes("Worktrees 9.9.9") && text.includes("Claude 2.1.320"),
    `${engine}: shared bubble lists worktrees and harness updates`);
  await both.close();
  await browser.close();
}

if (failures) { console.error(`${failures} harness update browser failure(s)`); process.exit(1); }
console.log("harness-updates-browser: all passed");
