// Drive the mock chat on :1453 in both browsers. Set PLAYWRIGHT_MODULE to a
// Playwright installation whose WebKit and Chromium are already installed.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const orientation = fileURLToPath(new URL("../tests/fixtures/chat-heic/orientation-6.heic", import.meta.url));
const large = fileURLToPath(new URL("../tests/fixtures/chat-heic/large-3000x1600.heic", import.meta.url));
const check = (yes, message) => { if (!yes) throw Error(message); console.log(`ok — ${message}`); };

async function open(engine) {
  const browser = await engine.launch({ headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  page.setDefaultTimeout(15_000);
  await page.goto("http://localhost:1453/?agentprompt=off");
  await page.locator('[data-testid="places-rail"]').click();
  await page.locator('li.row[data-slug="messaging"]').click();
  await page.locator('[data-testid="chat"]').waitFor();
  await page.evaluate(() => {
    window.__heicUploads = [];
    const api = window.__TAURI_INTERNALS__;
    const original = api.invoke;
    api.invoke = (command, args) => {
      if (command === "agent_attach") window.__heicUploads.push({ name: args.name, data: args.data });
      return original(command, args);
    };
  });
  return { browser, page };
}

async function attach(page, file, name) {
  await page.locator('[data-testid="chat-file-input"]').setInputFiles({
    name, mimeType: "application/octet-stream", buffer: readFileSync(file),
  });
}

async function attachEvent(page, event, name) {
  const data = readFileSync(orientation).toString("base64");
  await page.evaluate(({ data, event, name }) => {
    const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
    const transfer = new DataTransfer();
    transfer.items.add(new File([bytes], name, { type: "application/octet-stream" }));
    const target = document.querySelector(event === "paste" ? '[data-testid="chat-field"]' : '[data-testid="chat-composer"]')
      ?? document.querySelector(".chat-composer");
    target.dispatchEvent(event === "paste"
      ? new ClipboardEvent("paste", { clipboardData: transfer, bubbles: true, cancelable: true })
      : new DragEvent("drop", { dataTransfer: transfer, bubbles: true, cancelable: true }));
  }, { data, event, name });
}

async function jpegDetails(page, index) {
  return page.evaluate(async (i) => {
    const captured = window.__heicUploads[i];
    const bytes = Uint8Array.from(atob(captured.data), (c) => c.charCodeAt(0));
    const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/jpeg" }));
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const context = canvas.getContext("2d");
    context.drawImage(bitmap, 0, 0);
    const sample = (x, y) => Array.from(context.getImageData(x, y, 1, 1).data).slice(0, 3);
    const detail = {
      name: captured.name, width: bitmap.width, height: bitmap.height,
      jpeg: bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff,
      exif: new TextDecoder("latin1").decode(bytes).includes("Exif\0\0"),
      topLeft: sample(10, 10), topRight: sample(bitmap.width - 10, 10),
    };
    bitmap.close();
    return detail;
  }, index);
}

const wk = await open(webkit);
try {
  await attach(wk.page, orientation, "IMG_0001.HEIC");
  await wk.page.locator('[data-testid="chat-chip"]').getByText("IMG_0001.jpg").waitFor();
  check((await wk.page.locator('[data-testid="chat-chip"]').innerText()).includes("converted"), "WebKit chip says converted");
  const title = await wk.page.locator(".chat-chip-name").getAttribute("title");
  check(title.includes("location, removed"), "chip tooltip explains metadata removal");
  await wk.page.locator('[data-testid="chat-send"]').click();
  await wk.page.waitForFunction(() => window.__heicUploads.length === 1);
  const rotated = await jpegDetails(wk.page, 0);
  check(rotated.name === "IMG_0001.jpg" && rotated.jpeg, "WebKit uploads JPEG with rewritten name");
  check(rotated.width === 80 && rotated.height === 120, "WebKit applies EXIF orientation 6");
  check(rotated.topLeft[2] > rotated.topLeft[0] && rotated.topRight[0] > rotated.topRight[2], "rotated quadrant colors face the right way");
  check(!rotated.exif, "canvas JPEG has no EXIF metadata");

  await attach(wk.page, large, "large.heic");
  await wk.page.locator('[data-testid="chat-chip"]').getByText("large.jpg").waitFor();
  await wk.page.locator('[data-testid="chat-send"]').click();
  await wk.page.waitForFunction(() => window.__heicUploads.length === 2);
  const shrunk = await jpegDetails(wk.page, 1);
  check(shrunk.name === "large.jpg" && shrunk.jpeg, "large HEIC uploads as JPEG");
  check(shrunk.width === 2048 && shrunk.height === 1092, "long edge is at most 2048 px");
  check(!shrunk.exif, "large JPEG has no EXIF metadata");
  await wk.page.locator('[data-testid="chat-chip"]').waitFor({ state: "detached" });
  await attachEvent(wk.page, "paste", "paste.heic");
  await wk.page.locator('[data-testid="chat-chip"]').getByText("paste.jpg").waitFor();
  check((await wk.page.locator('[data-testid="chat-chip"]').innerText()).includes("converted"), "pasted HEIC follows conversion path");
  await wk.page.locator('[data-testid="chat-chip-remove"]').click();
  await attachEvent(wk.page, "drop", "drop.heic");
  await wk.page.locator('[data-testid="chat-chip"]').getByText("drop.jpg").waitFor();
  check((await wk.page.locator('[data-testid="chat-chip"]').innerText()).includes("converted"), "dropped HEIC follows conversion path");
} finally { await wk.browser.close(); }

const chrome = await open(chromium);
try {
  await attach(chrome.page, orientation, "IMG_0001.HEIC");
  await chrome.page.getByText("This Mac's WebView could not read the HEIC photo", { exact: false }).waitFor();
  check(await chrome.page.locator('[data-testid="chat-chip"]').count() === 0, "Chromium removes undecodable HEIC chip");
  check(await chrome.page.evaluate(() => window.__heicUploads.length) === 0, "Chromium uploads nothing");
} finally { await chrome.browser.close(); }
