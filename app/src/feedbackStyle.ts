/** Render the Orfis dialog the way jobmepls does — the reference integration.
 *
 *  Two separate corrections, and only the second is cosmetic polish.
 *
 *  1. THE FORM HAS NO ROW SPACING UPSTREAM. `gap: 1rem` lives on `.panel`, so
 *     it separates the panel's children — and the whole form is ONE of them.
 *     The type picker, both fields and the buttons therefore sit flush against
 *     each other. jobmepls hit this too and added four `margin-block-start`
 *     rules (`src/renderer/src/orfis/styles.ts`, "worth upstreaming"); they are
 *     reproduced verbatim below. This is the difference you can actually see.
 *
 *  2. `rem` RESOLVES AGAINST THE WRONG BASE. The SDK sizes in `rem`, which is
 *     root-relative, and this app pins `<html>` to `--ui-rem` (15px) rather
 *     than the 16px the dialog was drawn against — so everything also rendered
 *     ~6% tight. The sheet below is the vendored stylesheet's own layout rules
 *     with every `<n>rem` rewritten to `<n * 16>px`: same rules, same order,
 *     upstream's numbers, nothing chosen by eye.
 *
 *  Literal px, not `calc(n * var(--base))`, which is what this tried first and
 *  why two rounds of "still wrong" happened: a `calc()` whose custom property
 *  does not resolve is invalid at computed-value time, so the declaration is
 *  dropped and the property takes its INITIAL value — `gap` became `normal`,
 *  i.e. 0. Padding and borders survived, every gap vanished, and the result
 *  looked like a layout nobody had styled rather than a variable nobody had
 *  defined. Literal values cannot fail that way.
 *
 *  Regenerate after a re-vendor: take the vendored sheet's layout rules from
 *  `app/src/vendor/orfis/orfis.es.js`, rewrite rem -> px at 16, re-append the
 *  four form rules. `feedback-check.mjs` pins that the vendored sheet still
 *  uses `rem` and still carries these selectors, so a re-pin that changes
 *  either fails loudly rather than silently reverting to the flush layout.
 *
 *  Colours are deliberately absent: those come from `[data-orfis]` in
 *  tokens.css and must keep following the app's theme. */
const SHEET = `
.launcher {
  position: fixed; inset-block-end: 20px; inset-inline-end: 20px; z-index: 2147483000;
  padding: 11.2px 18.4px; border: 0; border-radius: 999px; cursor: pointer;
  background: var(--orfis-accent); color: var(--orfis-accent-ink);
  font: 600 14.4px/1 inherit; box-shadow: 0 6px 20px rgb(0 0 0 / .18);
}
.launcher:hover { filter: brightness(1.06); }

.backdrop {
  position: fixed; inset: 0; z-index: 2147483001; display: grid; place-items: center;
  padding: 16px; background: rgb(12 14 18 / .45); backdrop-filter: blur(2px);
}
.panel {
  width: min(480px, 100%); max-height: min(672px, 90vh); overflow-y: auto;
  display: flex; flex-direction: column; gap: 16px;
  padding: 24px; border-radius: 14px; border: 1px solid var(--orfis-line);
  background: var(--orfis-ground); box-shadow: 0 24px 60px rgb(0 0 0 / .3);
}
h2 { margin: 0; font-size: 18.4px; font-weight: 650; }
p  { margin: 0; color: var(--orfis-muted); font-size: 14px; line-height: 1.5; }

.types { display: flex; gap: 8px; border: 0; padding: 0; margin: 0; }
.types legend { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
.type {
  flex: 1; display: flex; align-items: center; justify-content: center; gap: 8px;
  padding: 9.6px; border: 1px solid var(--orfis-line); border-radius: 10px; cursor: pointer;
  font-size: 14.4px; background: var(--orfis-ground);
}
.type:has(input:checked) { border-color: var(--orfis-accent); background: var(--orfis-sunk); font-weight: 600; }
.type:has(input:focus-visible) { outline: 2px solid var(--orfis-accent); outline-offset: 2px; }
.type input { accent-color: var(--orfis-accent); margin: 0; }

label.field { display: flex; flex-direction: column; gap: 5.6px; font-size: 13.12px; color: var(--orfis-muted); }
textarea, input[type="email"] {
  width: 100%; padding: 10.4px 12px; border-radius: 10px; font: inherit; font-size: 14.4px;
  border: 1px solid var(--orfis-line); background: var(--orfis-ground); color: var(--orfis-ink);
}
textarea { min-height: 120px; resize: vertical; }
textarea:focus-visible, input:focus-visible { outline: 2px solid var(--orfis-accent); outline-offset: 1px; }

.disclosure { border: 1px solid var(--orfis-line); border-radius: 10px; background: var(--orfis-sunk); padding: 12px; }
.disclosure > p { font-size: 12.8px; }
.disclosure-actions { display: flex; align-items: center; gap: 16px; margin-block-start: 8px; flex-wrap: wrap; }
.linky { border: 0; background: none; padding: 0; cursor: pointer; font: inherit; font-size: 12.8px;
         color: var(--orfis-accent); text-decoration: underline; }
.optout { display: flex; align-items: center; gap: 6.4px; font-size: 12.8px; color: var(--orfis-muted); }
pre {
  margin: 9.6px 0 0; padding: 9.6px; max-height: 176px; overflow: auto; border-radius: 8px;
  background: var(--orfis-ground); border: 1px solid var(--orfis-line);
  font: 11.52px/1.45 ui-monospace, SFMono-Regular, Menlo, monospace; white-space: pre-wrap; word-break: break-word;
}

.actions { display: flex; justify-content: flex-end; gap: 9.6px; align-items: center; }
button.primary, button.ghost {
  padding: 9.6px 18.4px; border-radius: 999px; font: 600 14px/1 inherit; cursor: pointer;
}
button.primary { border: 0; background: var(--orfis-accent); color: var(--orfis-accent-ink); }
button.ghost { border: 1px solid var(--orfis-line); background: transparent; color: var(--orfis-ink); }
button:disabled { opacity: .55; cursor: not-allowed; }
button:focus-visible { outline: 2px solid var(--orfis-accent); outline-offset: 2px; }

.error { color: var(--orfis-danger); font-size: 13.12px; }
.counter { margin-inline-end: auto; font-size: 12px; color: var(--orfis-muted); font-variant-numeric: tabular-nums; }
.honeypot { position: absolute; left: -9999px; width: 1px; height: 1px; opacity: 0; }
.done { display: flex; flex-direction: column; gap: 8px; }

@media (prefers-reduced-motion: no-preference) {
  .panel { animation: rise .18s ease-out; }
  @keyframes rise { from { transform: translateY(8px); opacity: 0; } to { transform: none; opacity: 1; } }
}

/* The four rules jobmepls adds, resolved at the same 16px base.
   Upstream puts gap:1rem on .panel, which only separates the PANEL's children
   -- and the whole form is one of them. The type picker, both fields and the
   buttons therefore get no spacing at all. jobmepls calls these "worth
   upstreaming"; until they are, we carry the same four.
   (No backticks in here: this block lives inside a template literal.) */
form h2 + p { margin-block-start: 8px; }
label.field { margin-block-start: 12px; }
.disclosure { margin-block-start: 12px; }
form .actions { margin-block-start: 24px; }
`;

export function applyFeedbackSpacing(host: HTMLElement): boolean {
  const root = host.shadowRoot;
  if (!root) return false;
  const style = document.createElement("style");
  style.textContent = SHEET;
  // Appended after the SDK's own sheet, so equal specificity wins on order.
  root.append(style);
  return true;
}
