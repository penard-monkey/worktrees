// Guards the one declaration that lets an embedded terminal get NARROWER.
//
//   node app/scripts/termfit-check.mjs        # exits non-zero on failure
//
// `.term-host` is a ROW flex item (its parent `.term-wrap` is `display: flex`
// with no `flex-direction`), so its automatic minimum size is its MIN-CONTENT
// width — and xterm writes an explicit `width: <cols × cell>px` onto
// `.xterm-screen`, which means that floor is the grid the terminal is painting
// right now. Drop `min-width: 0` and the host becomes a ratchet: it grows with
// the pane and can never shrink back. Open or widen the dock and `.main` gets
// narrower while the host keeps its old width, overhanging the dock by
// hundreds of px (354 when this was found). Nothing ever corrects it, because
// the ResizeObserver in TerminalPane.tsx observes THAT box — no shrink, no
// `fit()`, no `term_resize` — so tmux keeps painting columns that now live
// under the dock, clipped mid-glyph at the seam.
//
// No test can see this. It is not a value anything reads back: every unit test,
// every bats case and the whole mock harness pass with the bug in, because the
// terminal is CORRECT at the moment it is sized and only wrong once something
// else takes width away from it. Same family as `dnd-check.mjs` — a static
// assertion standing in for a check the suites structurally cannot make.
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const CSS_PATH = fileURLToPath(new URL("../src/App.css", import.meta.url));
const css = fs.readFileSync(CSS_PATH, "utf8");

/** The declaration block for an exact selector, comments stripped. Anchored on
 *  the selector standing alone before its `{` so `.term-host` does not match
 *  `.dock-body .term-host` (a real, separate rule 38 lines below). */
function block(selector) {
  const esc = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const m = new RegExp(`(?:^|\\n)\\s*${esc}\\s*\\{([^}]*)\\}`).exec(css);
  return m ? m[1].replace(/\/\*[\s\S]*?\*\//g, "") : null;
}

let failed = 0;
const fail = (msg) => { failed++; console.log(`not ok — ${msg}`); };
const ok = (msg) => console.log(`ok — ${msg}`);

const host = block(".term-host");
const wrap = block(".term-wrap");

if (!host) fail(".term-host rule not found in App.css — did the selector get renamed?");
if (!wrap) fail(".term-wrap rule not found in App.css — did the selector get renamed?");

// The premise. If the wrapper stops being a row flex container the automatic
// minimum size no longer applies on the width axis, and the assertions below
// would be guarding a rule that no longer carries anything — worth failing over
// so whoever changed it revisits this file rather than inheriting a dead guard.
if (wrap) {
  if (!/display:\s*flex/.test(wrap)) {
    fail(".term-wrap is no longer `display: flex` — re-derive what floors .term-host's width");
    // `flex-flow` too: the shorthand sets the direction just as well, and a
    // check that only knows the longhand green-lights the very drift it exists
    // to catch.
  } else if (/flex-(?:direction|flow):\s*column/.test(wrap)) {
    fail(".term-wrap became a COLUMN — .term-host's width is now a cross-axis size; re-check this guard");
  } else {
    ok(".term-wrap is still a row flex container (so .term-host's width has a min-content floor)");
  }
}

if (host) {
  if (/min-width:\s*0/.test(host)) ok(".term-host can shrink (min-width: 0)");
  else fail(".term-host has no `min-width: 0` — the terminal will grow into the dock and never shrink back");

  if (/overflow:\s*hidden/.test(host)) ok(".term-host clips (overflow: hidden) — a wide grid can never paint over the dock");
  else fail(".term-host has no `overflow: hidden` — a mid-resize frame can paint the grid over its neighbour");
}

console.log(failed ? `\n${failed} failed` : "\nall good");
process.exit(failed ? 1 : 0);
