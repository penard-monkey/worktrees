// Browser gate for the Ideas conversation pane: the terminal under the chat,
// links that never take the window, and the selection quoted into a message.
// Chromium AND WebKit, against the mock.
//
//   VITE_MOCK=1 vite --port 1477 --strictPort --force           (in app/)
//   PLAYWRIGHT_MODULE=<…>/node_modules/playwright/index.mjs \
//     IDEAS_MOCK_URL=http://localhost:1477 node app/scripts/ideas-pane-browser.mjs
// IDEAS_SCREENSHOTS=<dir> saves the window per engine.
//
// What it holds:
//   - a click (and ⌘-, ⇧-, middle click, Enter, and a click whose handler
//     fails to cancel it) on a link in a reply, with the opener refusing,
//     navigates nothing, and a plain click says so in the window;
//   - Terminal shows a pane attached by idea_term_open (never term_open) that
//     fits its host, detaches on Hide, re-attaches on show, and goes when the
//     conversation is closed; no notice about tmux; "Open in terminal app" only
//     with a terminal command set;
//   - a selection shows as a quote, whole, before anything is sent; it goes
//     with the message through idea_send, once; removing it sends none and
//     leaves Quote selection; the file is never written for it; the Read view
//     and an unselected editor offer none; text replaced under a selection is
//     offered, not attached.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const BASE = process.env.IDEAS_MOCK_URL ?? "http://localhost:1477";
const SHOTS = process.env.IDEAS_SCREENSHOTS;
const JELLY = "01JC8Z4K3M9QX2T7VB00000901";
const LINKS = "see [the notes](https://example.com/a), https://example.com/bare and <https://example.com/auto>";

let failed = 0;
const ok = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };
const until = (page, fn, arg, ms = 6000) => page.waitForFunction(fn, arg, { timeout: ms }).then(() => true, () => false);
const reachable = (page, sel) => page.evaluate((sel) => {
  const el = document.querySelector(sel);
  if (!el) return false;
  const r = el.getBoundingClientRect();
  const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
  return !!hit && (hit === el || el.contains(hit));
}, sel);

async function open(engine, { reply, term } = {}) {
  const browser = await engine.launch();
  const page = await browser.newPage({ viewport: { width: 1400, height: 860 } });
  const s = { browser, page, calls: [], navs: [], errors: [] };
  // xterm 5.5's viewport reads its renderer in a timer that outlives a
  // dispose: StrictMode's throwaway mount throws it, in the main window too.
  const XTERM_DISPOSED = /reading 'dimensions'|_renderer\.value\.dimensions/;
  page.on("pageerror", (e) => { if (!XTERM_DISPOSED.test(String(e))) s.errors.push(String(e)); });
  await page.exposeFunction("__noteCall", (c, a) => s.calls.push([c, a]));
  await page.route("**://example.com/**", (r) => { s.navs.push(r.request().url()); r.fulfill({ body: "gone" }); });
  page.context().on("page", (p) => s.navs.push(`popup ${p.url()}`));
  await page.addInitScript(({ reply, term }) => {
    if (term) sessionStorage.setItem("wt-mock-ui-state", JSON.stringify({ settings_rev: 5, beta_ideas: true, terminal_cmd: "open -a Terminal" }));
    const id = setInterval(() => {
      const t = window.__TAURI_INTERNALS__;
      if (!t?.invoke || t.__wrapped) return;
      t.__wrapped = true;
      const inner = t.invoke.bind(t);
      t.invoke = async (cmd, args, opts) => {
        window.__noteCall(cmd, JSON.parse(JSON.stringify(args ?? {}, (k, v) => (k === "onBytes" ? "ch" : v))));
        if (cmd === "plugin:opener|open_url") throw "Command plugin:opener|open_url not allowed by ACL";
        const r = await inner(cmd, args, opts);
        if (reply && cmd === "idea_turns") r.turns = r.turns.map((x) => ({ ...x, reply }));
        return r;
      };
    }, 2);
  }, { reply, term });
  await page.goto(`${BASE}/companion/index.html?ideas=seed&ideareply=150&ideastick=150`);
  await page.waitForSelector("[data-testid=ideas-rows] li");
  await page.click(`[data-idea='${JELLY}'] button.ix-row-main`);
  await until(page, () => document.querySelector(".cm-content"));
  await page.click("[data-testid=ideas-chat-toggle]");
  await page.click("[data-testid=idea-conv-first]");
  await until(page, () => document.querySelector(".chat-bubble"));
  s.count = (cmd) => s.calls.filter(([c]) => c === cmd).length;
  return s;
}

async function links(engine, tag) {
  const s = await open(engine, { reply: LINKS });
  const { page } = s;
  const here = page.url();
  ok(await until(page, () => document.querySelectorAll(".chat-bubble a.md-link").length >= 3), `${tag} the reply's three links are drawn`);
  const a = page.locator(".chat-bubble a.md-link").first();
  for (const o of [{}, { modifiers: ["Meta"] }, { modifiers: ["Shift"] }, { button: "middle" }]) await a.click(o);
  await a.focus();
  await page.keyboard.press("Enter");
  // A handler that forgets: React's own is stopped before it can cancel, as a
  // link drawn with none would be.
  const forgot = await page.evaluate(() => {
    const el = document.querySelector(".chat-bubble a.md-link");
    el.parentElement.addEventListener("click", (e) => e.stopPropagation(), { once: true });
    const bare = Object.assign(document.createElement("a"), { href: "https://example.com/bare-anchor", textContent: "x" });
    document.querySelector(".chat-bubble").append(bare);
    const ev = () => new MouseEvent("click", { bubbles: true, cancelable: true, composed: true });
    const one = ev(), two = ev();
    el.dispatchEvent(one);
    bare.dispatchEvent(two);
    bare.remove();
    return [one.defaultPrevented, two.defaultPrevented];
  });
  ok(forgot.every(Boolean), `${tag} a link whose handler does not cancel the click is cancelled anyway (${forgot})`);
  await page.waitForTimeout(500);
  ok(page.url() === here && s.navs.length === 0, `${tag} no gesture on a link navigates the window (${s.navs.join(", ") || "none"})`);
  ok((await page.textContent("[data-testid=ideas-notice]")).includes("open https://example.com/a: Command plugin:opener|open_url not allowed by ACL"), `${tag} a refused open is said in the window`);
  ok(s.count("plugin:opener|open_url") === 4, `${tag} each click asks the opener (${s.count("plugin:opener|open_url")} of 4: a click, ⌘, ⇧, Enter)`);
  await s.browser.close();
}

async function terminal(engine, tag, name) {
  const s = await open(engine);
  const { page } = s;
  ok((await page.textContent("[data-testid=idea-conv-terminal]")) === "Terminal" && await reachable(page, "[data-testid=idea-conv-terminal]"), `${tag} the button says Terminal and is reachable`);
  await page.click("[data-testid=idea-conv-terminal]");
  ok(await until(page, () => document.querySelector("[data-testid=idea-term] .xterm-rows")?.textContent.includes("mock terminal")), `${tag} the terminal's output shows`);
  const at = s.calls.find(([c]) => c === "idea_term_open")?.[1];
  ok(at?.id === JELLY && /^[0-9A-Z]{26}$/.test(at?.conv ?? "") && at.cols > 20 && at.rows > 4 && !("session" in (at ?? {})), `${tag} it attached by idea_term_open, naming the conversation (${JSON.stringify(at)})`);
  ok(s.count("term_open") === 0, `${tag} never by term_open`);
  ok(!(await page.$("[data-testid=ideas-notice]")), `${tag} no notice about attaching by hand`);
  ok(!(await page.$("[data-testid=idea-conv-terminal-app]")), `${tag} no "Open in terminal app" without a terminal command`);
  const box = () => page.evaluate(() => {
    const r = (q) => document.querySelector(q).getBoundingClientRect();
    const [side, host, screen, chat, field] = [r(".ix-chat"), r(".ix-term .term-host"), r(".ix-term .xterm-screen"), r(".ix-chat-view"), r("[data-testid=chat-field]")];
    const cs = getComputedStyle(document.querySelector(".ix-term .term-host"));
    return { inSide: host.right <= side.right + 0.5 && host.bottom <= side.bottom + 0.5, fits: screen.right <= host.right && screen.bottom <= host.bottom, below: chat.bottom <= host.top, field: field.bottom <= chat.bottom + 0.5, sizing: cs.boxSizing, min: cs.minWidth, h: host.height, chatH: chat.height };
  });
  const b = await box();
  ok(b.inSide && b.fits && b.below && b.field && b.sizing === "content-box" && b.min === "0px", `${tag} the pane sits under the chat, inside the panel, its grid inside its host (${JSON.stringify(b)})`);
  ok(await reachable(page, "[data-testid=chat-field]") && await reachable(page, "[data-testid=idea-term-hide]"), `${tag} the composer and Hide are reachable with the terminal up`);
  // The divider: one settled size, not one per frame.
  await page.waitForTimeout(250);
  const before = s.count("term_resize");
  const bar = await page.locator("[data-testid=idea-term-bar]").boundingBox();
  await page.mouse.move(bar.x + 40, bar.y + 3);
  await page.mouse.down();
  for (let i = 1; i <= 20; i++) await page.mouse.move(bar.x + 40, bar.y - i * 5);
  await page.mouse.up();
  await page.waitForTimeout(400);
  const after = await box();
  ok(after.h > b.h + 60 && after.fits, `${tag} dragging the divider grows the terminal (${Math.round(b.h)} → ${Math.round(after.h)} px)`);
  ok(s.count("term_resize") - before <= 3, `${tag} a drag is a settled resize, not one per frame (${s.count("term_resize") - before})`);
  // Typing goes to the attach.
  await page.click(".ix-term .xterm-screen");
  await page.keyboard.type("ls");
  ok(await until(page, () => true) && s.calls.some(([c, a]) => c === "term_write" && String(a.data).includes("108")), `${tag} keys go to the attach`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/ideas-terminal-${name}.png` });
  const opens = s.count("idea_term_open");
  await page.click("[data-testid=idea-term-hide]");
  ok(await until(page, () => !document.querySelector("[data-testid=idea-term]")) && s.count("term_close") === opens, `${tag} Hide detaches (${s.count("term_close")} closes for ${opens} opens)`);
  await page.click("[data-testid=idea-conv-terminal]");
  ok(await until(page, () => document.querySelector("[data-testid=idea-term] .xterm-rows")?.textContent.includes("mock terminal")), `${tag} Terminal attaches again`);
  await page.click("[data-testid=idea-conv-close]");
  ok(await until(page, () => document.querySelector("[data-testid=idea-conv-ended]") && !document.querySelector("[data-testid=idea-term]")), `${tag} closing the conversation takes its terminal away`);
  await page.waitForTimeout(100);
  ok(s.count("term_close") === s.count("idea_term_open"), `${tag} every attach was closed (${s.count("term_close")}/${s.count("idea_term_open")})`);
  ok(s.errors.length === 0, `${tag} no page errors${s.errors.length ? `: ${s.errors.join(" | ")}` : ""}`);
  await s.browser.close();

  const set = await open(engine, { term: true });
  await set.page.click("[data-testid=idea-conv-terminal]");
  ok(await until(set.page, () => document.querySelector("[data-testid=idea-conv-terminal-app]")), `${tag} with a terminal command set, "Open in terminal app" is offered in the pane`);
  await set.page.click("[data-testid=idea-conv-terminal-app]");
  ok(set.calls.some(([c, a]) => c === "open_terminal" && a.cmd === "open -a Terminal" && a.session.startsWith("worktrees~idea~")), `${tag} and opens the conversation there`);
  await set.browser.close();
}

async function quote(engine, tag, name) {
  const s = await open(engine);
  const { page } = s;
  const select = async (word, down) => {
    await page.locator(".cm-line", { hasText: word }).first().click();
    await page.keyboard.press("Home");
    for (let i = 0; i < down; i++) await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Shift+End");
  };
  const sends = () => page.evaluate(() => window.__mock.ideaSends().map((x) => x.text));
  const writes = () => page.evaluate(() => window.__mock.ideaWrites().length);
  await page.locator(".cm-line", { hasText: "transcode" }).first().click();
  await page.waitForTimeout(250);
  ok(!(await page.$("[data-testid=chat-quote]")) && !(await page.$("[data-testid=chat-quote-offer]")), `${tag} nothing selected, nothing quoted`);
  const w0 = await writes();
  const want = "> from *Jellyfin on the home server*, lines 10–11:\n> - transcode on the **GPU**?\n> - which *box*";
  await select("transcode", 1);
  ok(await until(page, (want) => document.querySelector("[data-testid=chat-quote-text]")?.textContent === want, want), `${tag} the selection shows whole, with its lines: ${JSON.stringify(await page.textContent("[data-testid=chat-quote-text]").catch(() => null))}`);
  ok(await reachable(page, "[data-testid=chat-quote-remove]"), `${tag} its ✕ is reachable`);
  await page.click("[data-testid=chat-field]");
  ok(await page.$("[data-testid=chat-quote]"), `${tag} the quote stays while the person types in the chat`);
  ok(await page.isDisabled("[data-testid=chat-send]"), `${tag} a quote alone is not a message`);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/ideas-quote-${name}.png` });
  await page.fill("[data-testid=chat-field]", "why these two?");
  await page.press("[data-testid=chat-field]", "Enter");
  ok(await until(page, () => window.__mock.ideaSends().length === 1), `${tag} the message is sent`);
  ok((await sends())[0] === `${want}\n\nwhy these two?`, `${tag} the quote goes before it, a blank line between (${JSON.stringify((await sends())[0])})`);
  ok(await until(page, () => !document.querySelector("[data-testid=chat-quote]") && document.querySelector("[data-testid=chat-quote-offer]")), `${tag} it goes once: the same selection is then only offered`);
  await until(page, () => !document.querySelector("[data-testid=chat-field]").value);
  await page.waitForTimeout(400);
  await page.fill("[data-testid=chat-field]", "and then?");
  await page.press("[data-testid=chat-field]", "Enter");
  ok(await until(page, () => window.__mock.ideaSends().length === 2) && (await sends())[1] === "and then?", `${tag} the next message carries no quote`);
  // Small (the window's default, then its floor), the terminal dragged as
  // tall as it goes, a quote on: the field, ✕ and Send still take a click.
  await page.click("[data-testid=chat-quote-offer]");
  await page.click("[data-testid=idea-conv-terminal]");
  await until(page, () => document.querySelector("[data-testid=idea-term] .xterm-rows"));
  await page.focus("[data-testid=idea-term-bar]");
  for (let i = 0; i < 12; i++) await page.keyboard.press("ArrowUp");
  for (const [w, h] of [[1040, 720], [900, 480]]) {
    await page.setViewportSize({ width: w, height: h });
    await page.waitForTimeout(250);
    const r = {};
    for (const id of ["chat-field", "chat-quote-remove", "chat-send", "idea-term-hide"]) r[id] = await reachable(page, `[data-testid=${id}]`);
    ok(Object.values(r).every(Boolean), `${tag} ${w}×${h}, a tall terminal and a quote: all reachable (${JSON.stringify(r)})`);
  }
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/ideas-small-${name}.png` });
  await page.setViewportSize({ width: 1400, height: 860 });
  await page.click("[data-testid=idea-term-hide]");
  await page.click("[data-testid=chat-quote-remove]");
  // The control puts it back; ✕ takes it off.
  await page.waitForTimeout(400);
  await page.click("[data-testid=chat-quote-offer]");
  ok(await until(page, () => document.querySelector("[data-testid=chat-quote]")), `${tag} Quote selection attaches it`);
  await page.click("[data-testid=chat-quote-remove]");
  await page.fill("[data-testid=chat-field]", "plain");
  await page.press("[data-testid=chat-field]", "Enter");
  ok(await until(page, () => window.__mock.ideaSends().length === 3) && (await sends())[2] === "plain", `${tag} a removed quote is not sent`);
  // A command in the draft is refused with a quote as without one.
  await page.waitForTimeout(400);
  await page.click("[data-testid=chat-quote-offer]");
  await page.fill("[data-testid=chat-field]", "/clear");
  ok(await until(page, () => document.querySelector("[data-testid=chat-line].alert")) && await page.isDisabled("[data-testid=chat-send]"), `${tag} a draft that starts with / is refused with a quote too`);
  await page.fill("[data-testid=chat-field]", "");
  await page.click("[data-testid=chat-quote-remove]");
  ok((await writes()) === w0, `${tag} the idea was never written for a quote`);
  // A new selection, then the text replaced under it by someone else.
  await select("which", 0);
  ok(await until(page, () => document.querySelector("[data-testid=chat-quote-text]")?.textContent.includes("line 11:")), `${tag} another selection is quoted afresh, one line as "line 11"`);
  await page.click("[data-testid=ideas-mode-read]");
  ok(await until(page, () => !document.querySelector("[data-testid=chat-quote]") && !document.querySelector("[data-testid=chat-quote-offer]")), `${tag} the Read view quotes nothing`);
  await page.click("[data-testid=ideas-mode-edit]");
  ok(await until(page, () => document.querySelector("[data-testid=chat-quote]")), `${tag} back in Edit it is there again`);
  await page.fill("[data-testid=chat-field]", "please add a line");
  await page.click("[data-testid=chat-quote-remove]");
  await page.press("[data-testid=chat-field]", "Enter");
  await until(page, () => document.querySelector("[data-testid=chat-prompt]"));
  await select("transcode", 0);
  await until(page, () => document.querySelector("[data-testid=chat-quote]"));
  await page.click("[data-testid=chat-choice-1]");
  await page.click("[data-testid=chat-send-answer]");
  ok(await until(page, () => document.querySelector(".cm-content")?.innerText.includes("- please add a line")), `${tag} Claude's edit replaced the text`);
  ok(await until(page, () => !document.querySelector("[data-testid=chat-quote]") && document.querySelector("[data-testid=chat-quote-offer]")), `${tag} the selection left over it is offered, not attached`);
  ok(s.errors.length === 0, `${tag} no page errors${s.errors.length ? `: ${s.errors.join(" | ")}` : ""}`);
  await s.browser.close();
}

for (const [engine, name] of [[chromium, "chromium"], [webkit, "webkit"]]) {
  const tag = `[${name}]`;
  await links(engine, tag);
  await terminal(engine, tag, name);
  await quote(engine, tag, name);
}
console.log(failed ? `\nideas-pane-browser: ${failed} failure(s)` : "\nideas-pane-browser: all good");
process.exit(failed ? 1 : 0);
