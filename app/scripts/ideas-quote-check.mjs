// ideas-quote-check — the Ideas window's quote, and its links.
//
// Runs the REAL sources (vite's SSR loader): `ideaquote.ts` (a selection's
// lines and the quote a message carries), `chat.ts`'s `withQuote`, `sendDraft`
// and `draftProblem` over a stubbed door, `linkguard.ts` and `markdown.tsx`.
//
// What it holds:
//   - lines are 1-based in the file's text, LF and CRLF, and a selection that
//     ends just after a line break does not reach into the next line;
//   - the quote names the idea and its lines, and quotes every line; tabs are
//     the only thing changed; nothing selected, or only white space, is none;
//   - a quote goes before the draft through idea_send, and what the host's rule
//     refuses (an invisible character, too many bytes) is refused here, as is a
//     draft the rule refuses alone; sendDraft says whether it was delivered;
//   - a rendered link's click, with the opener refusing, cancels its default
//     and reports; a link whose handler does nothing is cancelled by the guard.
//
// IDEASQUOTE_ROOT points the check at another app/ (to show it red).
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const ROOT = process.env.IDEASQUOTE_ROOT ?? fileURLToPath(new URL("..", import.meta.url));
let failed = 0;
const check = (c, m) => { if (!c) failed++; console.log((c ? "ok   " : "FAIL ") + m); };

const calls = [];
let answer = async () => ({ delivered: true });
globalThis.__invoke = async (cmd, args) => { calls.push([cmd, args]); return cmd === "idea_send" ? answer() : { waiting: false, turns: [], harness: "claude" }; };
const stub = {
  name: "stub-ipc", enforce: "pre",
  resolveId: (id) => (/(^|\/)ipc$/.test(id) ? "\0ipc-stub" : null),
  load: (id) => (id === "\0ipc-stub" ? "export const invoke = (...a) => globalThis.__invoke(...a); export const windowOnHost = () => true; export const remoteHost = () => null; export const hostSendCaps = () => null; export const listen = async () => () => {}; export const engineState = () => ({kind:'up',gen:1}); export const onEngineState = () => () => {}; export const notOnHost = (s) => s; export class Channel {}" : null),
};
// The anchors, as the renderer creates them: the JSX runtime, seen through.
const anchors = [];
globalThis.__react = await import(path.join(ROOT, "node_modules/react/index.js"));
globalThis.__el = (type, props, ...kids) => {
  if (type === "a") anchors.push(props);
  return globalThis.__react.createElement(type, props, ...kids);
};
globalThis.__frag = globalThis.__react.Fragment;
// vite leaves react external, so the runtime cannot be a virtual module: the
// file is compiled with the classic pragma instead, its text unchanged.
const jsx = {
  name: "see-jsx", enforce: "pre",
  transform: (src, id) => (/\/src\/markdown\.tsx$/.test(id) ? `/** @jsxRuntime classic @jsx globalThis.__el @jsxFrag globalThis.__frag */\n${src}` : null),
};
const server = await createServer({
  root: ROOT, configFile: false, logLevel: "silent", appType: "custom", plugins: [stub, jsx],
  server: { middlewareMode: true, hmr: false, watch: null }, optimizeDeps: { noDiscovery: true, include: [] },
});
try {
  const { lineRange, quoteOf } = await server.ssrLoadModule("/companion/ideaquote.ts");
  const at = (text, s) => ({ start: text.indexOf(s), end: text.indexOf(s) + s.length });
  const range = (text, s) => { const a = at(text, s); const r = lineRange(text, a.start, a.end); return `${r.from}-${r.to}`; };
  const lf = "---\nid: x\n---\n\n# T\n\n- one\n- two\nlast";
  check(range(lf, "- one") === "7-7" && range(lf, "- one\n- two") === "7-8" && range(lf, "last") === "9-9" && range(lf, "---") === "1-1", "lines are 1-based in the whole file, frontmatter included");
  check(range(lf, "- one\n") === "7-7" && range(lf, "- one\n- two\n") === "7-8", "a selection ending on a line break does not reach the next line");
  const crlf = lf.replaceAll("\n", "\r\n");
  check(range(crlf, "- one\r\n- two") === "7-8" && range(crlf, "- one\r\n") === "7-7", "a CRLF file counts the same lines");
  const q = (text, s, title = "Jellyfin Sharing", auto = true) => quoteOf(title, text, at(text, s), auto);
  check(q(lf, "- one\n- two").text === "> from *Jellyfin Sharing*, lines 7–8:\n> - one\n> - two", "the quote: its source, then every line");
  check(q(lf, "- one").text.startsWith("> from *Jellyfin Sharing*, line 7:\n> - one"), "one line is “line 7”");
  check(q(crlf, "- one\r\n- two").text === q(lf, "- one\n- two").text, "a CRLF file quotes the same text");
  check(q("a\n\n\tb  \n", "a\n\n\tb  ").text === "> from *Jellyfin Sharing*, lines 1–3:\n> a\n>\n>     b", "an empty line stays a quote line, a tab is four spaces");
  check(q("a \n b", " \n ") === null && quoteOf("T", "abc", { start: 1, end: 1 }, true) === null, "white space alone, or nothing, is no quote");
  check(q(lf, "- one", " a\n b ").text.startsWith("> from *a b*, "), "a title is one line");
  check(q(lf, "- one").key !== q(lf, "- two").key && q(lf, "- one", "T", false).auto === false, "two selections differ, and a replaced text's quote is not automatic");

  const C = await server.ssrLoadModule("/src/chat.ts");
  const rules = C.sendRulesFor("claude", false, null);
  const t = { repo: "", slug: "", path: "idea:I:C", session: "s", idea: { id: "I", conv: "C" } };
  const send = async (draft, quote) => { calls.length = 0; C.setDraft(t.path, draft); const r = await C.sendDraft(t, "Claude", rules, quote); return { r, text: calls.find(([c]) => c === "idea_send")?.[1]?.text }; };
  const quote = q(lf, "- one\n- two").text;
  const sent = await send("why?", quote);
  check(sent.r === true && sent.text === `${quote}\n\nwhy?`, `the quote goes before the draft, a blank line between (${JSON.stringify(sent)})`);
  check((await send("plain", null)).text === "plain" && (await send("plain")).text === "plain", "no quote, the draft alone");
  for (const [draft, with_, why] of [["/clear", quote, "a draft that starts with /"], ["ok", `${quote}\n> a​b`, "an invisible character in the quote"], ["ok", "> " + "x".repeat(17000), "more bytes than a message holds"], ["  ", quote, "a quote with no message"]]) {
    const s = await send(draft, with_);
    check(s.r === false && s.text === undefined, `refused here, nothing sent: ${why}`);
  }
  answer = async () => ({ delivered: false, stopped: false });
  check((await send("again", quote)).r === false, "an unconfirmed send answers false, so the quote stays");
  answer = async () => { throw "no"; };
  check((await send("again", quote)).r === false, "a refused send answers false");

  // ── links ──
  const { Markdown } = await server.ssrLoadModule("/src/markdown.tsx");
  const { guardLinks } = await server.ssrLoadModule("/companion/linkguard.ts");
  const { createElement } = await import(path.join(ROOT, "node_modules/react/index.js"));
  const { renderToStaticMarkup } = await import(path.join(ROOT, "node_modules/react-dom/server.node.js"));
  const opened = [], said = [];
  const onLink = (href) => { opened.push(href); Promise.reject("not allowed by ACL").catch((e) => said.push(`open ${href}: ${e}`)); };
  const html = renderToStaticMarkup(createElement(Markdown, { src: "[a](https://example.com/a) https://example.com/b <https://example.com/c> [t]\n\n[t]: https://example.com/d\n\n[x](file:///etc/passwd)", onLink }));
  check((html.match(/<a /g) ?? []).length === 4 && !/href="file:/.test(html), "an authored, bare, angled and reference link are anchors; a file: one is not");
  check(anchors.length === 4, `the renderer's anchors were seen (${anchors.length})`);
  let cancelled = 0;
  for (const a of anchors) { a.onClick({ preventDefault: () => cancelled++ }); a.onAuxClick({ preventDefault: () => cancelled++ }); }
  await new Promise((r) => setTimeout(r, 0));
  check(cancelled === 8 && opened.length === 4, "every one cancels its click and its middle click, and a click asks to open it");
  check(said.length === 4 && said[0] === "open https://example.com/a: not allowed by ACL", "a refused open is reported, the click already cancelled");
  const on = {};
  guardLinks({ addEventListener: (k, f, capture) => { on[k] = { f, capture }; } });
  const el = (nodeName, href) => ({ nodeName, hasAttribute: (n) => n === "href" && href });
  const fire = (kind, pathOf) => { let p = false; on[kind].f({ composedPath: () => pathOf, preventDefault: () => { p = true; } }); return p; };
  check(on.click?.capture === true && on.auxclick?.capture === true, "the guard hears click and auxclick first, in capture");
  check(fire("click", [el("SPAN"), el("A", true), el("DIV")]) && fire("auxclick", [el("A", true)]), "it cancels a click inside any link, whatever its handler does");
  check(!fire("click", [el("BUTTON"), el("DIV")]) && !fire("click", [el("A", false)]), "and nothing else");
} finally {
  await server.close();
}
console.log(failed ? `\nideas-quote-check: ${failed} failure(s)` : "\nideas-quote-check: all good");
process.exit(failed ? 1 : 0);
