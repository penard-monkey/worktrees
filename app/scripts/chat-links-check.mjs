// chat-links-check — a markdown link in a chat reply is never a silent no-op.
// ChatPane's onLink once acted only on http(s) and dropped everything else, so
// a Codex reply ending in `[Open the prototype](/abs/path/index.html)` drew a
// link that did nothing. The rule is FilesPane's and PlanPane's, in one pure
// function, `chatLinkAction` (chatlink.ts): http(s) → the browser; any other
// scheme → refused in words; a bare #anchor → nothing; the rest is a PATH,
// opened in the Files tab through `onOpenPath` (never straight in another app:
// a reply is untrusted, and the Files tab's own Open button is the step
// between). GuestSession has no Files tab, so it refuses a path in words.
//
// Runs the REAL chatLinkAction from chatlink.ts (a module of its own, with no
// imports: guestview-check bars the guest views from chat.ts), then reads ChatPane.tsx and GuestSession.tsx for the wiring.
//
// Run from the repo root, as CI does. CHATLINKS_SRC / _PANE / _GUEST point at
// other copies (to show it red).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (p) => path.join(path.dirname(fileURLToPath(import.meta.url)), p);
const SRC = process.env.CHATLINKS_SRC ?? here("../src/chatlink.ts");
const PANE = process.env.CHATLINKS_PANE ?? here("../src/ChatPane.tsx");
const GUEST = process.env.CHATLINKS_GUEST ?? here("../src/GuestSession.tsx");

let failed = 0;
const fail = (m) => { failed++; console.log(`FAIL ${m}`); };
const ok = (m) => console.log(`ok   ${m}`);
const expect = (what, got, want) => (JSON.stringify(got) === JSON.stringify(want) ? ok(what) : fail(`${what}: want ${JSON.stringify(want)}, got ${JSON.stringify(got)}`));

const src = fs.readFileSync(SRC, "utf8");
if (/^import /m.test(src)) fail("chatlink.ts gained an import — the guest views share it and may not reach chat.ts");
const js = (await transformWithEsbuild(src, "chatlink.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(js).toString("base64"));

if (typeof C.chatLinkAction !== "function") {
  fail("chatlink.ts exports no chatLinkAction");
  console.log(`\nchat-links-check: ${failed} failure(s)`);
  process.exit(1);
}
const R = "/p/.worktrees/x";
const act = (h) => C.chatLinkAction(h, R);
expect("absolute path is used as is", act("/Users/b/o/takes/astra/index.html"), { kind: "path", path: "/Users/b/o/takes/astra/index.html" });
expect("relative path resolves against the place root", act("docs/a/REVIEW.md"), { kind: "path", path: `${R}/docs/a/REVIEW.md` });
expect("./ and .. are folded", act("./docs/../README.md"), { kind: "path", path: `${R}/README.md` });
expect("%20 is decoded", act("/tmp/My%20Doc/a%20b.html"), { kind: "path", path: "/tmp/My Doc/a b.html" });
expect("a #fragment is dropped", act("docs/DESIGN.md#goals"), { kind: "path", path: `${R}/docs/DESIGN.md` });
expect("a bare #anchor does nothing", act("#goals"), { kind: "none" });
expect("https opens", act("https://example.com/a"), { kind: "url", url: "https://example.com/a" });
expect("http opens", act("http://example.com/a"), { kind: "url", url: "http://example.com/a" });
expect("file: is refused", act("file:///etc/passwd"), { kind: "refuse", scheme: "file" });
expect("javascript: is refused", act("javascript:alert(1)"), { kind: "refuse", scheme: "javascript" });
expect("JavaScript: case is refused", act("JavaScript:alert(1)"), { kind: "refuse", scheme: "javascript" });
expect("a stray % stays in the name", act("/tmp/100%.md"), { kind: "path", path: "/tmp/100%.md" });

// ── the wiring a pure function cannot show ──────────────────────────────────
const pane = fs.readFileSync(PANE, "utf8");
const m = /const onLink = useCallback\(([\s\S]*?)\n  \}, \[[^\]]*\]\);/.exec(pane);
if (!m) fail("ChatPane.tsx has no `const onLink = useCallback(…)`");
else {
  const body = m[1];
  if (!/chatLinkAction\(href, root\)/.test(body)) fail("ChatPane's onLink does not ask chatLinkAction(href, root)");
  else ok("ChatPane's onLink asks chatLinkAction with the place's root");
  if (!/props\.onOpenPath\(a\.path, \{\}\)/.test(body)) fail("ChatPane's onLink does not reach props.onOpenPath");
  else ok("a path reaches onOpenPath (the Files tab)");
  if (!/props\.onError\(/.test(body.split("refuse")[1] ?? "")) fail("ChatPane's onLink does not say a refusal through props.onError");
  else ok("a refusal goes through onError");
  if (/openPath|shell_open|open_in|revealItem/i.test(body.replace(/props\.onOpenPath/g, ""))) fail("ChatPane's onLink opens something outside the Files tab");
}
const guest = fs.readFileSync(GUEST, "utf8");
if (!/chatLinkAction\(href, ""\)/.test(guest) || !/data-testid="guest-link-note"/.test(guest)) fail("GuestSession does not route links through chatLinkAction and show a refusal");
else ok("GuestSession refuses what it cannot open, in words");

if (failed) {
  console.log(`\nchat-links-check: ${failed} failure(s)`);
  process.exit(1);
}
console.log("\nchat-links-check: ok");
