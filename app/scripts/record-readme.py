#!/usr/bin/env python3
"""Record the README media (desktop-flow gif + stills) from the mock harness.

Drives the REAL App.tsx running under VITE_MOCK=1 in Chromium and records the
core flow (list -> new -> open) to a webm, plus two stills. A synthetic cursor
is injected so the gif shows pointer motion. Encoding webm -> gif is done by the
sibling record-readme.sh wrapper (ffmpeg); this script only produces the raw
webm + PNGs into --out.

Deps: `pip install playwright` + `playwright install chromium`.
Usage: assumes the mock harness is already serving on --port (the .sh wrapper
starts it). Run the wrapper for the full one-command regen.
"""
import argparse
import time
from playwright.sync_api import sync_playwright

W, H = 1280, 800

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


def record_flow(p, url, out):
    """Video: list -> new (create feat/checkout, session spins up) -> open."""
    browser = p.chromium.launch(args=["--force-color-profile=srgb"])
    ctx = browser.new_context(
        viewport={"width": W, "height": H},
        device_scale_factor=1,
        record_video_dir=out,
        record_video_size={"width": W, "height": H},
    )
    ctx.add_init_script(CURSOR_JS)
    page = ctx.new_page()

    def move_to(loc, steps=26, pause=0.0):
        box = loc.bounding_box()
        page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2, steps=steps)
        if pause:
            time.sleep(pause)

    page.goto(url, wait_until="networkidle")
    page.wait_for_selector(".row", timeout=10000)
    time.sleep(1.5)  # beat 1: list — populated workspace

    # beat 2: new — create a place on the worktrees project
    header = page.locator(".project-h", has_text="worktrees").first
    move_to(header, pause=0.35)  # hover reveals the new-worktree control
    time.sleep(0.4)
    plus = header.get_by_title("new worktree")
    move_to(plus, steps=12, pause=0.2)
    plus.click()
    time.sleep(0.5)

    # the + opens a dialog now, not a card in the nav
    page.wait_for_selector('[data-testid="new-place-dialog"]', timeout=5000)
    branch = page.locator('[data-testid="nw-branch"]')
    move_to(branch, pause=0.2)
    branch.click()
    branch.type("feat/checkout", delay=85)
    time.sleep(0.8)  # let the verdict + folder-name mirror land in frame
    branch.press("Escape")  # close the branch popover; the dialog stays up
    time.sleep(0.4)

    create = page.locator('[data-testid="nw-create"]')
    move_to(create, steps=14, pause=0.2)
    create.click()
    page.wait_for_selector(".term-host", timeout=8000)  # new place lands live
    time.sleep(2.2)

    # beat 3: open — attach to another existing place. DOUBLE-click: a single
    # click only selects, and this beat is teaching the gesture that opens.
    move_to(page.locator(".home-item").first, pause=0.2)
    page.locator(".home-item").first.click()
    time.sleep(0.9)
    other = page.locator(".row", has_text="feat-redesign").first
    move_to(other, pause=0.25)
    other.dblclick()
    page.wait_for_selector(".term-host", timeout=8000)
    time.sleep(2.4)

    ctx.close()  # flush the video
    browser.close()


def capture_stills(p, url, out):
    """Retina 2x stills: workspace overview + a place with its terminal."""
    browser = p.chromium.launch(args=["--force-color-profile=srgb"])
    ctx = browser.new_context(viewport={"width": W, "height": H}, device_scale_factor=2)
    page = ctx.new_page()
    page.goto(url, wait_until="networkidle")
    page.wait_for_selector(".row")
    time.sleep(0.8)
    page.screenshot(path=f"{out}/shot_home.png")
    page.locator(".row", has_text="messaging").first.dblclick()  # click selects; double opens
    page.wait_for_selector(".term-host")
    time.sleep(1.0)
    page.screenshot(path=f"{out}/shot_session.png")
    browser.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", default="1425")
    ap.add_argument("--out", required=True, help="output dir for webm + stills")
    args = ap.parse_args()
    url = f"http://localhost:{args.port}/"
    with sync_playwright() as p:
        record_flow(p, url, args.out)
        capture_stills(p, url, args.out)
    print(f"recorded webm + stills into {args.out}")


if __name__ == "__main__":
    main()
