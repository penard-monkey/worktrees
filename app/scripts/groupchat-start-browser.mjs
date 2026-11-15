// Headless drive of group chat's start design A (docs/proposals/group-chat-
// start.md §3.1) in the mock harness, WebKit AND Chromium, from a first run:
// the beta on, nobody paired, remote access off. It counts the owner's acts
// (clicks plus fields filled) from *Share this session…* to the share, and
// the calls the app made, so §4's contract is held at runtime too: nothing
// before *Share with Bret* calls `share_create`, and `remote_enable` comes
// only from *Turn on and show the link*. Then: closing the dialog cancels
// the window, a same-named guest is offered "Pair another device", the
// pairing toast names the guest and the device (F8), and the guest's *Join
// with a link* refuses with the beta off before any call (S5).
//
//   app/: VITE_MOCK=1 ./node_modules/.bin/vite --port 1437 --force
//   PLAYWRIGHT_MODULE=<a local playwright> node app/scripts/groupchat-start-browser.mjs [port] [shots dir]
const pw = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = Number(process.argv[2] ?? 1437);
const shots = process.argv[3] ?? null;
let failed = 0;
const fail = (m) => { failed++; console.log(`not ok — ${m}`); };
const ok = (m) => console.log(`ok — ${m}`);
const check = (c, m) => (c ? ok(m) : fail(m));

const spy = () => {
  const t = window.__TAURI_INTERNALS__;
  if (t.__spied) return;
  const orig = t.invoke.bind(t);
  window.__calls = [];
  t.invoke = (cmd, args, opts) => { window.__calls.push(cmd); return orig(cmd, args, opts); };
  t.__spied = true;
};
const calls = (page) => page.evaluate(() => window.__calls.slice());

for (const engine of ["webkit", "chromium"]) {
  const browser = await pw[engine].launch();
  const tag = engine;
  const shot = async (page, name) => { if (shots && engine === "webkit") await page.screenshot({ path: `${shots}/${name}.png` }); };

  // ── the owner ──
  {
    const page = await browser.newPage({ viewport: { width: 1365, height: 904 } });
    await page.goto(`http://localhost:${port}/?beta=group_chat&guests=0`);
    await page.waitForSelector("[data-track=settings]");
    await page.waitForTimeout(1500);
    await page.evaluate(spy);
    const live = await page.evaluate(async () => {
      const ws = await window.__TAURI_INTERNALS__.invoke("list_workspace", {});
      const out = [];
      for (const pv of ws.projects) for (const p of pv.snapshot?.places ?? []) {
        const a = p.agent_sessions ?? {};
        if (a.claude?.up || a.codex?.up) out.push({ repo: pv.root, slug: p.slug });
      }
      return out;
    });
    if (live.length < 2) { fail(`${tag}: the mock has two live places (${live.length})`); await browser.close(); continue; }
    let acts = 0;
    const press = async (sel) => { acts++; await page.click(sel); await page.waitForTimeout(250); };
    const fill = async (sel, v) => { acts++; await page.fill(sel, v); await page.waitForTimeout(150); };
    const ctxOn = async (slug) => {
      await page.evaluate((slug) => {
        const el = document.querySelector(`li.row[data-slug="${CSS.escape(slug)}"]`);
        const r = el.getBoundingClientRect();
        el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 30, clientY: r.top + 5 }));
      }, slug);
      await page.waitForTimeout(300);
    };

    await ctxOn(live[0].slug);
    check(await page.$("[data-testid=ctx-share]"), `${tag}: the place's menu offers Share this session…`);
    await shot(page, "h-owner-1-ctx-menu");
    await press("[data-testid=ctx-share]");
    await page.waitForSelector("[data-testid=share-dialog]");
    check(await page.isChecked("[data-testid=share-someone-new]"), `${tag}: nobody paired: Someone new is chosen`);
    await fill("[data-testid=share-new-name]", "Bret");
    await shot(page, "h-owner-2-who");
    await press("[data-testid=share-invite]");
    await page.waitForSelector("[data-testid=share-reach]");
    const addr = await page.$eval("[data-testid=share-reach-address]", (e) => e.value);
    check(addr.startsWith("100.101.102.103"), `${tag}: the reach block preselects the overlay address (${addr})`);
    check(!(await calls(page)).includes("remote_enable") && !(await calls(page)).includes("pair_begin"), `${tag}: nothing is turned on or opened before the press`);
    await shot(page, "h-owner-3-reach");
    await press("[data-testid=share-reach-on]");
    await page.waitForSelector("[data-testid=pair-send-link]", { timeout: 8000 });
    check(/Send Bret this link/.test(await page.textContent("[data-testid=pair-send-link]")), `${tag}: the window says "Send Bret this link" (F9)`);
    await shot(page, "h-owner-4-window");
    await press("[data-testid=pair-copy]");
    await page.evaluate(() => window.__mock.pairAttempt("Bret's MacBook", "macos", "100.101.4.7", "482913"));
    await page.waitForSelector("[data-testid=share-pair-allow]", { timeout: 5000 });
    check(/shares nothing yet/.test(await page.textContent("[data-testid=pair-request]")), `${tag}: the Allow screen says it shares nothing yet`);
    await shot(page, "h-owner-5-code");
    await press("[data-testid=share-pair-allow]");
    await page.waitForSelector("[data-testid=share-paired]", { timeout: 5000 });
    const c1 = await calls(page);
    check(!c1.includes("share_create"), `${tag}: Allow shared nothing: no share_create yet (§4)`);
    check(c1.filter((x) => x === "remote_enable").length === 1, `${tag}: remote access was turned on once`);
    check(/Share with Bret/.test(await page.textContent("[data-testid=share-confirm]")) && !!(await page.$("[data-testid=share-warning]")),
      `${tag}: the Share step shows v3's warning and "Share with Bret"`);
    await shot(page, "h-owner-6-share");
    await press("[data-testid=share-confirm]");
    await page.waitForSelector("[data-testid=share-dialog]", { state: "detached", timeout: 5000 }).catch(() => {});
    check((await calls(page)).filter((x) => x === "share_create").length === 1, `${tag}: Share with Bret made the one share`);
    await page.waitForTimeout(800);
    const toast = await page.evaluate(() => [...document.querySelectorAll(".ra-alert-txt")].map((e) => e.textContent));
    check(toast.some((t) => /Bret's Bret's MacBook was paired as a guest\./.test(t)) && !toast.some((t) => /\bd_[a-z0-9]+ was paired/.test(t)),
      `${tag}: the pairing toast names the guest and the device (F8): ${JSON.stringify(toast)}`);
    await shot(page, "h-owner-7-shared");
    console.log(`#  ${tag}: owner acts in the dialog = ${acts} (menu, name, Invite, Turn on and show the link, Copy, Allow, Share with Bret)`);
    check(acts === 7, `${tag}: seven acts from the menu to the share (the proposal's 8 in the dialog include Allowing the join)`);

    // The first lane is shared: its menu no longer offers a first share.
    await ctxOn(live[0].slug);
    check(!(await page.$("[data-testid=ctx-share]")), `${tag}: a shared place's menu has no Share this session…`);
    await page.keyboard.press("Escape");

    // Same name, another case: refused before asking, with the way that works.
    await ctxOn(live[1].slug);
    await page.click("[data-testid=ctx-share]");
    await page.waitForSelector("[data-testid=share-dialog]");
    const meta = await page.evaluate(() => document.querySelector(".ls-guest-meta")?.textContent ?? "");
    check(/1 device · paired /.test(meta), `${tag}: a paired guest shows their devices and when (${meta})`);
    await page.click("[data-testid=share-someone-new]");
    await page.fill("[data-testid=share-new-name]", "bret");
    await page.waitForTimeout(200);
    check(!!(await page.$("[data-testid=share-name-taken]")) && (await page.isDisabled("[data-testid=share-invite]")) && !!(await page.$("[data-testid=share-pair-another]")),
      `${tag}: "bret" beside Bret is refused, with Pair another device for Bret`);
    await shot(page, "h-owner-8-same-name");
    // Pair another device for Bret, then close the dialog: the window is cancelled.
    await page.click("[data-testid=share-pair-another]");
    await page.waitForSelector("[data-testid=pair-send-link]", { timeout: 5000 });
    check(!(await calls(page)).slice(-6).includes("remote_enable"), `${tag}: remote access already listening: nothing is turned on again`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(400);
    const st = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("pair_status", {}));
    check((await calls(page)).includes("pair_cancel") && st.state === "closed", `${tag}: closing the dialog cancelled the window (${JSON.stringify(st)})`);
    await page.close();
  }

  // ── the guest ──
  for (const beta of [true, false]) {
    const page = await browser.newPage({ viewport: { width: 1365, height: 904 } });
    await page.goto(`http://localhost:${port}/?${beta ? "beta=group_chat&" : ""}guests=0`);
    await page.waitForSelector("[data-track=settings]");
    await page.waitForTimeout(1200);
    await page.evaluate(spy);
    const home = await page.$("[data-testid=join-home]");
    check(beta ? !!home : !home, `${tag}: Home's Join with a link… is there only with the beta on (beta ${beta})`);
    if (beta) {
      await page.click("[data-testid=join-home]");
      await page.waitForSelector("[data-testid=join-dialog]");
      await shot(page, "h-guest-1-join");
      await page.keyboard.press("Escape");
    }
    // Settings → Remote → Join with a link, drawn either way.
    await page.click("[data-track=settings]");
    await page.waitForTimeout(400);
    await page.evaluate(() => [...document.querySelectorAll("button")].find((b) => /^Remote$/.test(b.textContent?.trim() ?? ""))?.click());
    await page.waitForSelector("[data-testid=join-host]", { timeout: 4000 }).catch(() => {});
    check(!!(await page.$("[data-testid=join-host]")), `${tag}: Settings → Remote has Join with a link (beta ${beta})`);
    if (!beta) {
      await page.fill("[data-testid=join-host] [data-testid=join-link]", "worktrees-pair:v1?addr=100.101.1.2:49350&pin=x");
      await page.click("[data-testid=join-host] [data-testid=join-go]");
      await page.waitForTimeout(300);
      const said = await page.textContent("[data-testid=join-beta-off]").catch(() => "");
      check(/Turn on Group chat in Settings → General → Beta features first/.test(said ?? "") && !(await calls(page)).includes("remote_pair"),
        `${tag}: with the beta off the join is refused in words, before any call (S5)`);
      await shot(page, "h-guest-2-beta-off");
    }
    await page.close();
  }
  await browser.close();
}
if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
console.log("\ngroupchat-start-browser: all ok");
