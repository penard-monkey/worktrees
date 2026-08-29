#!/usr/bin/env python3
"""Record the AI-profiles walkthrough (webm) from the mock harness.

Drives the REAL App.tsx under VITE_MOCK=1 and records the flow the feature page
leads with: open Settings, pick AI profiles, create a profile, give it rules,
enable a skill, then see the badge on a live session. A synthetic cursor is
injected so the gif shows pointer motion.

Encoding webm -> gif is done by record-profiles.sh (ffmpeg); this only produces
the raw webm.

Deps: `pip install playwright` + `playwright install chromium`.
"""
import argparse
from playwright.sync_api import sync_playwright

W, H = 1280, 800

# Same cursor trick record-readme.py uses: the init script runs before <body>
# exists, so the node insert is deferred to DOMContentLoaded.
CURSOR_JS = r"""
(() => {
  function install() {
    const c = document.createElement('div');
    c.id = '__fake_cursor__';
    c.style.cssText = 'position:fixed;left:0;top:0;z-index:2147483647;pointer-events:none;' +
      'transition:transform .18s cubic-bezier(.4,0,.2,1);will-change:transform';
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
})();
"""


def glide(page, locator, pause=0.35):
    """Move the synthetic cursor onto an element, so the gif reads as pointing."""
    box = locator.bounding_box()
    if box:
        page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2, steps=18)
    page.wait_for_timeout(int(pause * 1000))


def tap(page, locator, pause=0.45):
    """Glide, then dispatch on the element.

    The sheets render as `.scrim > aside`, and the scrim resolves as the topmost
    node over the whole sheet — a coordinate click lands on it and CLOSES the
    sheet. Gliding keeps the cursor honest for the recording; the dispatch is
    what actually activates the control.
    """
    glide(page, locator, 0.25)
    locator.dispatch_event("click")
    page.wait_for_timeout(int(pause * 1000))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=1441)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    url = f"http://localhost:{args.port}/"

    with sync_playwright() as p:
        browser = p.chromium.launch(args=["--force-color-profile=srgb"])
        ctx = browser.new_context(
            viewport={"width": W, "height": H},
            record_video_dir=args.out,
            record_video_size={"width": W, "height": H},
        )
        ctx.add_init_script(CURSOR_JS)
        page = ctx.new_page()
        page.goto(url)
        page.wait_for_selector(".row", timeout=15000)
        page.wait_for_timeout(1200)

        # Open Settings → AI profiles.
        gear = page.locator('button[title^="settings"]').first
        glide(page, gear, 0.5)
        gear.click()
        page.wait_for_selector(".settings-sheet", timeout=8000)
        page.wait_for_timeout(700)
        tap(page, page.locator(".settings-cat", has_text="AI profiles").first, 0.9)

        # Select the profile — the editor opens.
        tap(page, page.locator(".settings-body .settings-cat", has_text="Work").first, 0.9)

        # Type a rule, so the point of the feature is on screen.
        ta = page.locator("textarea").first
        glide(page, ta, 0.3)
        ta.type("\nAlways run the gates before you commit.", delay=42)
        page.wait_for_timeout(900)

        # Enable the second skill — the checkbox saves immediately.
        boxes = page.locator('.settings-body input[type="checkbox"]')
        if boxes.count() > 1:
            tap(page, boxes.nth(1), 0.9)

        # Scroll down through skills → MCP so the rest of the surface is seen.
        page.locator(".settings-body").first.evaluate(
            "el => el.scrollTo({top: el.scrollHeight * 0.55, behavior:'smooth'})")
        page.wait_for_timeout(1400)
        page.locator(".settings-body").first.evaluate(
            "el => el.scrollTo({top: el.scrollHeight, behavior:'smooth'})")
        page.wait_for_timeout(1500)

        # Close, and land on the live session so the badge is the last beat.
        tap(page, page.locator('.settings-h button[title^="close"]').first, 0.6)
        page.wait_for_timeout(600)
        row = page.locator(".row", has_text="messaging").first
        glide(page, row, 0.4)
        row.dblclick()  # a single click only selects — opening is the double
        page.wait_for_timeout(2000)

        ctx.close()   # flushes the video
        browser.close()


if __name__ == "__main__":
    main()
