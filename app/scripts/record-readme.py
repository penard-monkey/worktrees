#!/usr/bin/env python3
"""Record the README desktop media from the mock harness.

Drives the REAL App.tsx running under VITE_MOCK=1 in Chromium and records two
webm clips plus two retina stills into --out. A synthetic cursor is injected so
the clips show pointer motion. Encoding webm -> gif is the sibling
record-readme.sh wrapper's job (ffmpeg); this script only produces raw frames.

  flow.webm      Home -> open the Places tree -> new worktree (branch, agent,
                 create) -> the new place lands live with its session.
  dock.webm      Resume a place from Home -> widen the dock -> Files (README
                 preview) -> Docs (the document index) -> a document -> Plan.
  shot_home.png     Home, 2x
  shot_session.png  a place open: Places tree + terminal + dock, 2x

Every selector goes through need()/need_loc(), which fail with the selector and
a "the UI changed" hint instead of hanging on Playwright's 30s actionability
timeout. Two traps this script exists to avoid:
  * `.row` is ATTACHED while the Places nav is hidden (`.nav.hidden` is
    display:none), so a *visibility* wait on it blocks forever. The nav starts
    collapsed — toggle it first, then wait for visible.
  * the right rail's dock buttons are DISABLED until a place is selected (their
    title becomes "… — select a place first"), so a click before that hangs.
Also: the viewer's Preview/Source/Diff mode is global and persists across
files, so a beat that leaves it on Diff changes what every later beat shows.

Deps: `pip install playwright` + `playwright install chromium`.
Usage: assumes the mock harness is already serving on --port (the .sh wrapper
starts it). Run the wrapper for the full one-command regen.
"""
import argparse
import json
import time

from playwright.sync_api import TimeoutError as PWTimeout
from playwright.sync_api import sync_playwright

W, H = 1440, 900          # nav 300 + dock + main + two rails all fit (min ~968)
DOCK_WIDE = 260           # px to drag the dock resizer left (360 -> ~620)
DOCK_STILL = 170          # the still keeps the tree, so it widens the dock less
                          # (360 -> ~530: enough for the viewer's whole tab
                          # strip, which 110 clipped at "A+")

# The init script runs at document_start, before <body> exists — so defer the
# node insert to DOMContentLoaded, else it is appended to <html> and discarded
# when the parser builds the real document tree.
CURSOR_JS = r"""
(() => {
  function install() {
    if (document.getElementById('__fake_cursor__')) return;
    const c = document.createElement('div');
    c.id = '__fake_cursor__';
    c.style.cssText = [
      'position:fixed','left:0','top:0','width:22px','height:22px',
      'pointer-events:none','z-index:2147483647','margin:-2px 0 0 -2px',
      'will-change:transform','filter:drop-shadow(0 1px 2px rgba(0,0,0,.5))'
    ].join(';');
    c.innerHTML =
      '<svg width="22" height="22" viewBox="0 0 24 24" fill="none">' +
      '<path d="M5 3l14 8-6 1.5L10 19 5 3z" fill="#fff" stroke="#111" ' +
      'stroke-width="1.2" stroke-linejoin="round"/></svg>';
    document.body.appendChild(c);
    move(window.innerWidth / 2, window.innerHeight / 2);
  }
  function move(x, y) {
    const c = document.getElementById('__fake_cursor__');
    if (c) c.style.transform = `translate(${x}px,${y}px)`;
  }
  if (document.readyState !== 'loading') install();
  else document.addEventListener('DOMContentLoaded', install, { once: true });
  window.addEventListener('mousemove', e => move(e.clientX, e.clientY), true);
  window.addEventListener('mousedown', () => {
    const c = document.getElementById('__fake_cursor__'); if (c) c.style.opacity = '0.6';
  }, true);
  window.addEventListener('mouseup', () => {
    const c = document.getElementById('__fake_cursor__'); if (c) c.style.opacity = '1';
  }, true);
})();
"""


class Missing(SystemExit):
    pass


def _boom(what, selector, state):
    raise Missing(
        f"record-readme: {what} never became {state}.\n"
        f"  selector: {selector}\n"
        f"  The app's markup changed — fix this script's selectors rather than\n"
        f"  letting it hang on Playwright's actionability timeout."
    )


def need(page, selector, what, state="visible", timeout=8000):
    """Wait for a raw selector, failing loudly with the selector that broke."""
    try:
        page.wait_for_selector(selector, state=state, timeout=timeout)
    except PWTimeout:
        _boom(what, selector, state)
    return page.locator(selector)


def need_loc(loc, what, state="visible", timeout=8000):
    """Same, for a locator built with has_text / get_by_title / get_by_text."""
    try:
        loc.wait_for(state=state, timeout=timeout)
    except PWTimeout:
        _boom(what, repr(loc), state)
    return loc


def centre(loc):
    box = loc.bounding_box()
    if not box:
        _boom("an element with no box", repr(loc), "laid out")
    return box["x"] + box["width"] / 2, box["y"] + box["height"] / 2


def glide(page, loc, steps=22):
    """Move the real mouse (and so the synthetic cursor) onto an element."""
    x, y = centre(loc)
    page.mouse.move(x, y, steps=steps)
    return x, y


def tap(page, loc, what, steps=22, before=0.18, after=0.0):
    """Glide to an element and click it — the gesture the gif is teaching."""
    need_loc(loc, what)
    glide(page, loc, steps=steps)
    time.sleep(before)
    loc.click()
    if after:
        time.sleep(after)


def new_clip(browser, out_dir):
    ctx = browser.new_context(
        viewport={"width": W, "height": H},
        device_scale_factor=1,
        record_video_dir=out_dir,
        record_video_size={"width": W, "height": H},
    )
    ctx.add_init_script(CURSOR_JS)
    return ctx, ctx.new_page()


def widen_dock(page, px):
    """Drag the dock's resizer left. The resizer only exists once the dock is
    open, so every caller opens a dock tab first."""
    rz = need(page, ".dock-resizer", "the dock resizer")
    box = rz.bounding_box()
    y = box["y"] + box["height"] * 0.55
    page.mouse.move(box["x"] + box["width"] / 2, y)
    page.mouse.down()
    page.mouse.move(box["x"] - px, y, steps=18)
    page.mouse.up()


# ---------------------------------------------------------------- the clips


def record_flow(p, url, out, trims):
    """Home -> the tree -> new worktree -> the place lands live."""
    browser = p.chromium.launch(args=["--force-color-profile=srgb"])
    t0 = time.time()
    ctx, page = new_clip(browser, out)

    page.goto(url, wait_until="networkidle")
    need(page, ".home-hero", "the Home hero")
    need(page, ".resume-row", "the resume list")
    trims["flow"] = time.time() - t0
    time.sleep(1.7)                                   # beat 1: Home

    # beat 2: the Places tree. It starts COLLAPSED — `.nav.hidden` is
    # display:none, so `.row` is attached but never visible until this click.
    rail = need(page, '[data-testid="places-rail"]', "the Places rail toggle")
    tap(page, rail, "the Places rail toggle", steps=18)
    need(page, ".nav:not(.hidden)", "the Places nav")
    need(page, ".row", "a place row")                  # now a visibility wait
    time.sleep(1.8)                                    # tiers, agent dots

    # beat 3: hover a project header — that is what reveals its controls
    header = need_loc(page.locator(".project-h", has_text="worktrees").first,
                      "the worktrees project header")
    glide(page, header)
    header.hover()
    time.sleep(0.4)
    tap(page, header.get_by_title("new worktree"), "the new-worktree button",
        steps=12, after=0.40)

    # beat 4: fill the dialog
    need(page, '[data-testid="new-place-dialog"]', "the new-worktree dialog")
    branch = need(page, '[data-testid="nw-branch"]', "the branch field")
    glide(page, branch, steps=16)
    branch.click()
    branch.type("feat/checkout", delay=80)             # visible keystrokes
    time.sleep(0.9)                                    # verdict + path preview
    branch.press("Escape")                             # popover only; dialog stays
    need(page, '[data-testid="new-place-dialog"]', "the dialog after Escape")
    time.sleep(0.4)

    # beat 5: the agent is a CHOICE — the model row re-renders for Codex
    tap(page, need(page, '[data-testid="nw-provider-codex"]', "the Codex chip"),
        "the Codex chip", steps=14, after=1.0)

    # beat 6: create, and watch the place land live with its session
    tap(page, need(page, '[data-testid="nw-create"]', "the Create button"),
        "the Create button", steps=14)
    need(page, ".term-host", "the new place's terminal", timeout=12000)
    time.sleep(1.1)
    row = need_loc(page.locator(".row", has_text="feat-checkout").first,
                   "the new place's row", state="attached")
    row.scroll_into_view_if_needed()
    time.sleep(0.3)
    glide(page, row, steps=18)
    time.sleep(1.5)

    ctx.close()                                        # flushes the video
    page.video.save_as(f"{out}/flow.webm")
    page.video.delete()                                # drop the random-named copy
    browser.close()


def record_dock(p, url, out, trims):
    """Resume a place from Home, then walk all four dock tabs."""
    browser = p.chromium.launch(args=["--force-color-profile=srgb"])
    t0 = time.time()
    ctx, page = new_clip(browser, out)

    page.goto(url, wait_until="networkidle")
    need(page, ".resume-row", "the resume list")
    trims["dock"] = time.time() - t0
    time.sleep(0.6)

    # Open from Home's resume list, so the Places nav stays collapsed and the
    # dock gets the width — this clip is about the workspace, not the tree.
    resume = need_loc(page.locator(".resume-row", has_text="billing-refactor").first,
                      "the billing-refactor resume row")
    tap(page, resume.locator(".enter-btn"), "its Enter button", steps=20)
    need(page, ".term-host", "the place's terminal", timeout=12000)
    time.sleep(0.8)

    # The right rail is disabled until a place is selected; it is now.
    files_btn = need_loc(page.get_by_title("Files (⌘J)"), "the Files dock button")
    tap(page, files_btn, "the Files dock button", steps=20, after=0.4)
    need(page, ".dock", "the dock")
    widen_dock(page, DOCK_WIDE)
    time.sleep(0.5)

    # Files: a tracked file opens in the viewer (Preview / Source / Diff)
    tap(page, need_loc(page.locator(".tree-name", has_text="README.md").first,
                       "README.md in the file tree"),
        "README.md", steps=18, after=0.2)
    need(page, ".md", "the rendered README")
    time.sleep(1.3)

    # Docs: the per-place document index — grouped, with paths
    tap(page, need_loc(page.get_by_title("Docs (⌘J)"), "the Docs dock button"),
        "the Docs dock button", steps=20, after=0.3)
    need(page, ".docs-title", "the document index")
    time.sleep(1.5)
    tap(page, need_loc(page.locator(".docs-title",
                                    has_text="Task plan — staging on AWS").first,
                       "a document in the index"),
        "a document", steps=18, after=0.2)
    need(page, ".md", "the rendered document")
    time.sleep(0.9)

    # Plan: the place's plan, read out of the files the session already writes
    tap(page, need_loc(page.get_by_title("Plan (⌘J)"), "the Plan dock button"),
        "the Plan dock button", steps=20)
    need(page, ".md", "the rendered plan")
    time.sleep(1.6)

    ctx.close()
    page.video.save_as(f"{out}/dock.webm")
    page.video.delete()
    browser.close()


# --------------------------------------------------------------- the stills


def capture_stills(p, url, out):
    """Retina 2x stills: Home, and a place with tree + terminal + dock."""
    browser = p.chromium.launch(args=["--force-color-profile=srgb"])
    ctx = browser.new_context(viewport={"width": W, "height": H},
                              device_scale_factor=2)
    page = ctx.new_page()

    page.goto(url, wait_until="networkidle")
    need(page, ".home-hero", "the Home hero")
    need(page, ".resume-row", "the resume list")
    time.sleep(1.0)
    page.screenshot(path=f"{out}/shot_home.png")

    rail = need(page, '[data-testid="places-rail"]', "the Places rail toggle")
    rail.click()
    need(page, ".nav:not(.hidden)", "the Places nav")
    need(page, ".row", "a place row")                  # visible now, not before
    need_loc(page.locator(".row", has_text="billing-refactor").first,
             "the billing-refactor row").dblclick()    # a single click only selects
    need(page, ".term-host", "the place's terminal", timeout=12000)
    time.sleep(1.0)
    need_loc(page.get_by_title("Files (⌘J)"), "the Files dock button").click()
    need(page, ".dock", "the dock")
    widen_dock(page, DOCK_STILL)
    need_loc(page.locator(".tree-name", has_text="README.md").first,
             "README.md in the file tree").click()
    need(page, ".md", "the rendered README")
    time.sleep(1.2)
    page.mouse.move(W / 2, H + 40)                     # park the cursor off-frame
    page.screenshot(path=f"{out}/shot_session.png")
    browser.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", default="1425")
    ap.add_argument("--out", required=True, help="output dir for webm + stills")
    args = ap.parse_args()
    url = f"http://localhost:{args.port}/"
    trims = {}
    with sync_playwright() as p:
        record_flow(p, url, args.out, trims)
        record_dock(p, url, args.out, trims)
        capture_stills(p, url, args.out)
    # Seconds of blank/white frames at the head of each clip (context creation
    # until the app has painted) — the wrapper trims them with ffmpeg -ss.
    with open(f"{args.out}/trims.json", "w") as fh:
        json.dump(trims, fh)
    print(f"recorded flow.webm + dock.webm + stills into {args.out}")


if __name__ == "__main__":
    main()
