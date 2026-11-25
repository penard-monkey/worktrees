// Headless drive of the mock harness for a markdown link to a local file in
// the chat (fix/chat-local-links), in Chromium. Run the harness on its own
// port (`VITE_MOCK=1 vite --port 1471`) and set PLAYWRIGHT_MODULE:
//
//   CHAT_LINKS_URL=http://localhost:1471 PLAYWRIGHT_MODULE=… node app/scripts/chat-links-browser.mjs
//
// `?turns=links`: a reply with an absolute link to an .html file, a relative
// one with a #fragment and one with %20. A click must open the Files tab on
// THAT file (the mock's viewer prints `// <path>` for a file it has no
// fixture for, so the viewer's text names the path it was asked for); `file:`
// and `javascript:` are not drawn as links at all (markdown.tsx's safeHref).
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const { chromium } = pw.default ?? pw;
const base = process.env.CHAT_LINKS_URL ?? "http://localhost:1471";
const PLACE = "/Users/demo/workspace/casadelvalle/casa-del-valle-monorepo/.worktrees/messaging";

function check(ok, message) {
  if (!ok) throw new Error(message);
  console.log(`ok — ${message}`);
}

const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  page.setDefaultTimeout(12_000);
  const popups = [];
  page.on("popup", (p) => popups.push(p.url()));
  await page.goto(`${base}/?turns=links`);
  await page.locator('[data-testid="places-rail"]').click();
  await page.locator('li.row[data-slug="messaging"]').click();
  await page.locator('[data-testid="chat"]').waitFor();

  const links = page.locator('[data-testid="chat"] .chat-bubble-agent a');
  const hrefs = await links.evaluateAll((a) => a.map((x) => x.getAttribute("href")));
  check(hrefs.length === 4, `the reply draws 4 links (file: and javascript: are plain text): ${hrefs.length}`);

  const viewer = () => page.evaluate(() => document.querySelector(".dock")?.textContent ?? "");
  const cases = [
    ["Open the prototype", `${PLACE}/docs/design/takes/astra/index.html`],
    ["Concept review", `${PLACE}/docs/design/takes/astra/REVIEW.md`],
    ["Notes", `${PLACE}/docs/my notes/DESIGN.md`],
  ];
  for (const [text, want] of cases) {
    await page.locator('[data-testid="chat"] .chat-bubble-agent a', { hasText: text }).click();
    await page.waitForFunction((w) => (document.querySelector(".dock")?.textContent ?? "").includes(`// ${w}`), want);
    check((await viewer()).includes(`// ${want}`), `"${text}" opens the Files tab on ${want.replace(PLACE, "<place>")}`);
    const filesTab = await page.evaluate(() => !!document.querySelector(".dock") && /FILES/i.test(document.querySelector(".dock").textContent));
    check(filesTab, `the dock is on the Files tab after "${text}"`);
  }

  // A bare #anchor is a no-op: the viewer keeps the last file, nothing opens.
  const before = await viewer();
  await page.locator('[data-testid="chat"] .chat-bubble-agent a', { hasText: "Up" }).click();
  await page.waitForTimeout(300);
  check((await viewer()) === before, "a bare #anchor changes nothing");
  check(popups.length === 0, "no link opened a browser window");
} finally {
  await browser.close();
}
