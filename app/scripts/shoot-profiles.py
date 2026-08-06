#!/usr/bin/env python3
"""Screenshot the AI-profiles UI from the mock harness.

Drives the REAL App.tsx under VITE_MOCK=1 in Chromium and captures the states
the feature page needs. Deterministic: the same fixtures every run, so the
images can be regenerated and diffed.

Deps: `pip install playwright` + `playwright install chromium`.
Usage: assumes the harness is already serving on --port (shoot-profiles.sh
starts it).
"""
import argparse
import time
from playwright.sync_api import sync_playwright

W, H = 1440, 900


def tap(locator):
    """Click a control inside a sheet, by DISPATCHING on the element.

    The sheets render as `.scrim > aside`, and the scrim resolves as the topmost
    node at every point over the sheet. A normal click is refused; `force=True`
    is worse than useless here because it still dispatches BY COORDINATE, so the
    scrim receives it and its onClick closes the sheet. Dispatching straight at
    the element skips coordinates entirely, and React still sees it (its
    listener is delegated at the root container).
    """
    locator.dispatch_event("click")


def shot(page, out, name, clip=None):
    page.wait_for_timeout(350)  # let transitions settle
    path = f"{out}/{name}.png"
    page.screenshot(path=path, clip=clip)
    print(f"  ✓ {name}.png")
    return path


def open_settings_ai(page):
    """Rail → settings → the AI profiles category."""
    page.locator('button[title^="settings"]').first.click()
    page.wait_for_selector(".settings-sheet", timeout=8000)
    tap(page.locator(".settings-cat", has_text="AI profiles").first)
    page.wait_for_timeout(400)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=1426)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    url = f"http://localhost:{args.port}/"

    with sync_playwright() as p:
        browser = p.chromium.launch(args=["--force-color-profile=srgb"])
        page = browser.new_page(viewport={"width": W, "height": H}, device_scale_factor=2)
        page.goto(url)
        page.wait_for_selector(".row", timeout=15000)
        page.wait_for_timeout(600)

        # 1. The app with a profiled session live — the topbar badge in place.
        page.locator(".row", has_text="messaging").first.click()
        page.wait_for_selector(".term-host", timeout=8000)
        page.wait_for_timeout(500)
        shot(page, args.out, "01-badge-stale")

        # Tight crop of just the topbar, so the badge is legible on the page.
        bar = page.locator(".topbar").first.bounding_box()
        if bar:
            shot(page, args.out, "02-badge-crop",
                 clip={"x": bar["x"], "y": bar["y"], "width": bar["width"], "height": bar["height"]})

        # 2. The profiles editor: list, rules, model.
        open_settings_ai(page)
        shot(page, args.out, "03-editor")

        # 3. Pick the profile so the editor body is populated, then shoot the
        #    rules textarea with real text typed into it.
        tap(page.locator(".settings-body .settings-cat", has_text="Work").first)
        page.wait_for_timeout(300)
        # The rules text comes from the fixture, not typed here: a coordinate
        # click lands on the scrim and closes the sheet, and fixture content is
        # deterministic across runs.
        page.wait_for_timeout(300)
        shot(page, args.out, "04-rules")

        # 4. Scroll to the skills block — the capability tag is the point.
        page.locator(".settings-body").first.evaluate("el => el.scrollTop = el.scrollHeight * 0.45")
        page.wait_for_timeout(350)
        shot(page, args.out, "05-skills")

        # 5. The git review gate: paste a URL, hit Review, show what it found
        #    BEFORE anything installs.
        url_input = page.locator('input[placeholder^="https://github.com"]').first
        url_input.fill("https://github.com/acme/claude-skills")
        # Scoped to the sheet: an unscoped `button:has-text("Review")` matches the
        # left nav's "Not configured" banner, and the shot came back empty because
        # the click landed there.
        tap(page.locator('.settings-body button:has-text("Review")').first)
        page.wait_for_selector('.settings-body pre.update-log', timeout=8000)
        page.wait_for_timeout(400)
        # Scroll the preview into view — it renders under the URL row.
        page.locator('.settings-body pre.update-log').first.evaluate(
            "el => el.scrollIntoView({block: 'center'})")
        page.wait_for_timeout(350)
        shot(page, args.out, "06-skill-review")

        # 6. MCP section.
        page.locator(".settings-body").first.evaluate("el => el.scrollTop = el.scrollHeight")
        page.wait_for_timeout(350)
        shot(page, args.out, "07-mcp")

        # Close the sheet and WAIT for its scrim to detach — clicking through a
        # scrim that is still fading is what made this step flaky.
        # 7. Per-project picker, with the cold-conversation warning.
        #
        # From a FRESH page rather than closing the settings sheet: unwinding one
        # sheet to open another means fighting the scrim's hit-testing for no
        # benefit, and a clean load is deterministic.
        page.close()
        page = browser.new_page(viewport={"width": W, "height": H}, device_scale_factor=2)
        page.goto(url)
        page.wait_for_selector(".row", timeout=15000)
        page.wait_for_timeout(700)
        page.locator(".project-h", has_text="casa-del-valle").first.click(button="right")
        page.wait_for_selector(".pop-item", timeout=8000)
        page.wait_for_timeout(250)
        tap(page.locator(".pop-item", has_text="Project").first)
        page.wait_for_selector(".project-sheet", timeout=8000)
        page.wait_for_timeout(800)
        shot(page, args.out, "08-project-picker")

        browser.close()


if __name__ == "__main__":
    main()
