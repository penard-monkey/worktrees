// Headless mock-harness drive for Settings → Devices and Remote access.
// PAIR_PAUSED_SHOTS saves the paused Devices, Remote access and live-sheet
// screenshots. Run against `VITE_MOCK=1 vite`, never a running app.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
import fs from "node:fs";

const BASE = process.env.PAIR_PAUSED_URL ?? "http://127.0.0.1:1477";
const SHOTS = process.env.PAIR_PAUSED_SHOTS;
if (SHOTS) fs.mkdirSync(SHOTS, { recursive: true });

async function open(page, query, cat) {
  await page.goto(`${BASE}/?${query}`);
  await page.locator("[data-track=settings]").click();
  await page.locator(".settings-cat", { hasText: cat }).click();
}

async function hit(page, selector) {
  const el = page.locator(selector);
  await el.scrollIntoViewIfNeeded();
  return el.evaluate((node) => {
    const r = node.getBoundingClientRect();
    return node.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  });
}

function assert(ok, what) {
  if (!ok) throw new Error(what);
  console.log(`ok — ${what}`);
}

for (const [name, engine] of Object.entries({ chromium, webkit })) {
  const browser = await engine.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2 });
  try {
    await open(page, "access=paused&devices=0", "Devices");
    const why = page.locator('[data-testid="pair-why"]');
    await why.waitFor();
    assert(await page.locator('[data-track="pair-device"]').isDisabled(), `${name}: pairing is blocked while paused`);
    assert((await why.innerText()).includes("100.96.164.69 is no longer on utun6"), `${name}: Devices names lost address and interface`);
    assert(await hit(page, '[data-testid="pair-open-remote"]'), `${name}: Open Remote access is hit-test reachable`);
    if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/paused-devices-${name}.png` });

    await page.locator('[data-testid="pair-open-remote"]').click();
    await page.locator('.ra-status[data-state="paused"]').waitFor();
    const remote = page.locator('[data-focus="remote-access"]');
    assert((await remote.innerText()).includes("100.96.164.69 is no longer on utun6"), `${name}: Remote access shows pause reason`);
    const choose = remote.getByRole("button", { name: "Use another address…" });
    assert(await hit(page, '[data-focus="remote-access"] button:has-text("Use another address…")'), `${name}: address recovery is hit-test reachable`);
    assert(await hit(page, '[data-focus="remote-access"] button:has-text("Turn off")'), `${name}: Turn off is hit-test reachable`);
    if (SHOTS) await page.locator(".settings-modal").screenshot({ path: `${SHOTS}/paused-remote-${name}.png` });

    await choose.click();
    await page.locator('[data-testid="address-picker"]').waitFor();
    assert(await hit(page, '[data-testid="address-picker"] button:has-text("Turn on")'), `${name}: address picker can turn on another address`);
    await page.locator('[data-testid="address-picker"] button:has-text("Turn on")').click();
    await page.locator('.ra-status[data-state="listening"]').waitFor();
    assert((await remote.innerText()).includes("192.168.1.5"), `${name}: choosing the LAN address resumes listening`);

    await page.goto(`${BASE}/?access=paused&devices=0`);
    await page.locator("[data-track=settings]").click();
    await page.locator(".settings-cat", { hasText: "Remote access" }).click();
    const offAgain = page.locator('[data-focus="remote-access"]').getByRole("button", { name: "Turn off", exact: true });
    await offAgain.click();
    await page.getByRole("dialog", { name: "Turn off remote access?" }).getByRole("button", { name: "Turn off" }).click();
    await page.locator('.ra-status[data-state="off"]').waitFor();
    assert(true, `${name}: Turn off clears the pause`);

    await open(page, "access=on&devices=0", "Devices");
    await page.locator('[data-track="pair-device"]').click();
    await page.getByText("Every project, including ones added later").click();
    await page.locator('[data-track="pair-show-code"]').click();
    await page.locator('.ra-pair[data-phase="open"]').waitFor();
    await page.evaluate(() => window.__mock.remotePause(undefined, undefined, "192.168.1.5 is no longer on en0"));
    const sheetPause = page.locator('[data-testid="pair-paused"]');
    await sheetPause.waitFor();
    assert((await sheetPause.innerText()).includes("192.168.1.5 is no longer on en0"), `${name}: open pairing sheet names the new pause`);
    assert(await hit(page, '[data-testid="pair-paused"] button'), `${name}: sheet recovery action is hit-test reachable`);
    if (SHOTS) await page.locator(".ra-pair").screenshot({ path: `${SHOTS}/paused-pair-sheet-${name}.png` });
    await sheetPause.getByRole("button", { name: "Open Remote access" }).click();
    await page.locator('.ra-status[data-state="paused"]').waitFor();
    assert(await page.locator(".ra-pair").count() === 0, `${name}: sheet action opens Remote access`);
  } finally {
    await browser.close();
  }
}
