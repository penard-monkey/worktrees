// Markdown → React nodes.
//
// `marked` is used for its LEXER ONLY (`marked.lexer`) — it hands back a token
// tree and every DOM node below is one we construct. Nothing here touches
// `dangerouslySetInnerHTML`, so a hostile README cannot inject markup or script
// and no sanitizer is needed. That is the whole reason for taking the token
// stream instead of `marked.parse()`'s HTML string.
//
// Raw HTML in the source is rendered as literal text (visible, inert) rather
// than dropped — a doc with an <img> tag shows the tag, which is honest.
import { marked, type Token, type Tokens } from "marked";
import { memo, useMemo, type ReactNode } from "react";
import { CodeBlock } from "./CodeView";

export type MarkdownProps = {
  src: string;
  /** Relative image sources land here — the viewer loads them off disk. */
  renderImage?: (src: string, alt: string, title: string | null) => ReactNode;
  /** Link activation. `href` is verbatim from the doc (may be relative). */
  onLink?: (href: string) => void;
};

// marked's lexer leaves the five XML entities encoded in `text` tokens (it
// escapes at render time, which we skip). Decode them so prose reads right.
const ENT: Record<string, string> = {
  "&amp;": "&", "&lt;": "<", "&gt;": ">", "&quot;": '"', "&#39;": "'", "&apos;": "'", "&nbsp;": " ",
};
const decode = (s: string) => s.replace(/&(?:amp|lt|gt|quot|#39|apos|nbsp);/g, (m) => ENT[m] ?? m);

/**
 * What may appear in a rendered `href`. The click handler alone is NOT a
 * boundary: a middle-click fires `auxclick` with no `click`, so
 * `preventDefault()` never runs and the WebView follows the attribute. Anything
 * that is not http(s), a relative path, or an in-document anchor therefore gets
 * NO href at all — it renders as inert text carrying its own label.
 */
function safeHref(href: string): string | undefined {
  const h = href.trim();
  if (!h) return undefined;
  if (h.startsWith("#")) return h;
  // A scheme-looking prefix is only allowed if it is http(s).
  if (/^[a-z][a-z0-9+.-]*:/i.test(h)) return /^https?:/i.test(h) ? h : undefined;
  // Protocol-relative ("//host/x") is a URL, not a repo path.
  if (h.startsWith("//")) return undefined;
  return h; // relative path — resolved against the doc's dir by the caller
}

/** GitHub-style heading slug, so `[x](#some-heading)` has something to hit. */
function slugify(text: string): string {
  return text.toLowerCase().trim()
    .replace(/[^\w\s-]/g, "")
    .replace(/\s+/g, "-")
    .replace(/-+/g, "-");
}

// Recursion guard. `**`×4000 in an 8 KB file lexes fine and then blows the
// stack in the renderer — and with no error boundary above us that unmounts
// the whole app window. Past this depth we stop descending and print the raw
// text, which is ugly but bounded.
const MAX_DEPTH = 40;

// ── inline ────────────────────────────────────────────────────────────────
function inline(tokens: Token[] | undefined, ctx: MarkdownProps, keyBase = "i", depth = 0): ReactNode {
  if (!tokens) return null;
  if (depth > MAX_DEPTH) return tokens.map((t) => (t as Tokens.Generic).raw ?? "").join("");
  return tokens.map((t, i) => {
    const k = `${keyBase}-${i}`;
    switch (t.type) {
      case "text": {
        const tt = t as Tokens.Text;
        return tt.tokens ? <span key={k}>{inline(tt.tokens, ctx, k, depth + 1)}</span> : decode(tt.text);
      }
      case "escape":
        return decode((t as Tokens.Escape).text);
      case "strong":
        return <strong key={k}>{inline((t as Tokens.Strong).tokens, ctx, k, depth + 1)}</strong>;
      case "em":
        return <em key={k}>{inline((t as Tokens.Em).tokens, ctx, k, depth + 1)}</em>;
      case "del":
        return <del key={k}>{inline((t as Tokens.Del).tokens, ctx, k, depth + 1)}</del>;
      case "codespan":
        return <code key={k} className="md-code-inline">{decode((t as Tokens.Codespan).text)}</code>;
      case "br":
        return <br key={k} />;
      case "checkbox":
        return null; // drawn from the list item's `task`/`checked`, not inline
      case "link": {
        const lt = t as Tokens.Link;
        const href = safeHref(lt.href);
        if (!href) {
          // Refused scheme (file:, data:, vbscript:, a custom app scheme…).
          // Shown, never linked — and the URL is visible so the reader can
          // judge it themselves.
          return (
            <span key={k} className="md-link-blocked" title={`blocked link: ${lt.href}`}>
              {inline(lt.tokens, ctx, k, depth + 1)}
            </span>
          );
        }
        return (
          <a
            key={k}
            className="md-link"
            href={href}
            title={lt.title ?? lt.href}
            onClick={(e) => { e.preventDefault(); ctx.onLink?.(href); }}
            onAuxClick={(e) => e.preventDefault()}
          >
            {inline(lt.tokens, ctx, k, depth + 1)}
          </a>
        );
      }
      case "image": {
        const it = t as Tokens.Image;
        return <span key={k}>{ctx.renderImage?.(it.href, it.text, it.title ?? null) ?? <em>{it.text}</em>}</span>;
      }
      case "html":
        // literal, inert — see the file header
        return <span key={k} className="md-rawhtml">{(t as Tokens.HTML).raw}</span>;
      default:
        return <span key={k}>{decode((t as Tokens.Generic).raw ?? "")}</span>;
    }
  });
}

// ── blocks ────────────────────────────────────────────────────────────────
function block(tokens: Token[], ctx: MarkdownProps, keyBase = "b", depth = 0): ReactNode[] {
  const out: ReactNode[] = [];
  if (depth > MAX_DEPTH) return [<pre key="deep" className="md-rawhtml-block">{tokens.map((t) => (t as Tokens.Generic).raw ?? "").join("")}</pre>];
  tokens.forEach((t, i) => {
    const k = `${keyBase}-${i}`;
    switch (t.type) {
      case "space":
      case "checkbox":
        break; // checkbox: the list item draws its own <input>
      case "heading": {
        const ht = t as Tokens.Heading;
        const d = Math.min(6, Math.max(1, ht.depth));
        const H = (["h1", "h2", "h3", "h4", "h5", "h6"] as const)[d - 1];
        // id + scroll target so in-document links ([x](#heading)) work
        out.push(<H key={k} id={slugify(ht.text)} className={`md-h md-h${d}`}>{inline(ht.tokens, ctx, k)}</H>);
        break;
      }
      case "paragraph":
        out.push(<p key={k} className="md-p">{inline((t as Tokens.Paragraph).tokens, ctx, k)}</p>);
        break;
      case "text": {
        const tt = t as Tokens.Text;
        out.push(<p key={k} className="md-p">{tt.tokens ? inline(tt.tokens, ctx, k) : decode(tt.text)}</p>);
        break;
      }
      case "code": {
        const ct = t as Tokens.Code;
        out.push(
          <div key={k} className="md-fence">
            {ct.lang ? <div className="md-fence-lang">{ct.lang.split(/\s+/)[0].slice(0, 24)}</div> : null}
            <div className="md-fence-body"><CodeBlock src={ct.text} lang={fenceLang(ct.lang)} gutter={false} /></div>
          </div>,
        );
        break;
      }
      case "blockquote":
        out.push(
          <blockquote key={k} className="md-quote">
            {block((t as Tokens.Blockquote).tokens, ctx, k, depth + 1)}
          </blockquote>,
        );
        break;
      case "list": {
        const lt = t as Tokens.List;
        const items = lt.items.map((it, j) => {
          const flat = lt.loose ? null : itemInline(it.tokens);
          return (
          <li key={`${k}-${j}`} className={it.task ? "md-task" : undefined}>
            {it.task && (
              <input className="md-check" type="checkbox" checked={!!it.checked} readOnly tabIndex={-1} />
            )}
            {/* A TIGHT list item is inline content — wrapping it in <p> would
                push the text onto its own line under the checkbox. Only a
                loose list (blank lines between items) gets paragraphs. */}
            {flat ? inline(flat, ctx, `${k}-${j}`, depth + 1) : block(it.tokens, ctx, `${k}-${j}`, depth + 1)}
          </li>
          );
        });
        out.push(
          lt.ordered
            ? <ol key={k} className="md-list" start={typeof lt.start === "number" ? lt.start : undefined}>{items}</ol>
            : <ul key={k} className="md-list">{items}</ul>,
        );
        break;
      }
      case "table": {
        const tt = t as Tokens.Table;
        out.push(
          <div key={k} className="md-tablewrap">
            <table className="md-table">
              <thead>
                <tr>{tt.header.map((c, j) => (
                  <th key={j} style={c.align ? { textAlign: c.align } : undefined}>{inline(c.tokens, ctx, `${k}-h${j}`)}</th>
                ))}</tr>
              </thead>
              <tbody>
                {tt.rows.map((row, r) => (
                  <tr key={r}>{row.map((c, j) => (
                    <td key={j} style={c.align ? { textAlign: c.align } : undefined}>{inline(c.tokens, ctx, `${k}-${r}-${j}`)}</td>
                  ))}</tr>
                ))}
              </tbody>
            </table>
          </div>,
        );
        break;
      }
      case "hr":
        out.push(<hr key={k} className="md-hr" />);
        break;
      case "html":
        out.push(<pre key={k} className="md-rawhtml-block">{(t as Tokens.HTML).raw.replace(/\n+$/, "")}</pre>);
        break;
      case "def":
        break; // link reference definition — consumed by the lexer, nothing to draw
      default: {
        const g = t as Tokens.Generic;
        if (g.tokens) out.push(<div key={k}>{block(g.tokens, ctx, k, depth + 1)}</div>);
        else if (g.raw?.trim()) out.push(<p key={k} className="md-p">{decode(g.raw)}</p>);
      }
    }
  });
  return out;
}

// Token types that must go through the BLOCK renderer — an item containing one
// of these is not a one-liner, whatever the list's looseness says.
const BLOCK_TYPES = new Set(["list", "code", "blockquote", "table", "heading", "hr", "html", "space", "paragraph"]);

/** A tight list item's tokens are one `text` block wrapping the real inline
 *  tokens — unwrap it so the item renders on one line instead of pushing its
 *  text under the bullet. An item carrying a NESTED list (or a fence, or a
 *  quote) is left alone: inline() would flatten it into literal text. */
function itemInline(tokens: Token[]): Token[] | null {
  // A task item leads with a `checkbox` token; the <input> is drawn from the
  // item's own task/checked flags, so drop it before looking for the text.
  const rest = tokens.filter((t) => t.type !== "checkbox");
  if (rest.length !== 1 || rest[0].type !== "text") return null;
  const t = rest[0] as Tokens.Text;
  if (!t.tokens || t.tokens.some((x) => BLOCK_TYPES.has(x.type))) return null;
  return t.tokens;
}

// Fence info string → a grammar id our highlighter knows. Aliases that differ
// from the extension table live here (```js, ```console, ```jsonc …).
const FENCE_ALIAS: Record<string, string> = {
  js: "ts", javascript: "ts", jsx: "ts", typescript: "ts", tsx: "ts", ts: "ts",
  rs: "rust", rust: "rust",
  py: "python", python: "python",
  golang: "go", go: "go",
  rb: "ruby", ruby: "ruby",
  c: "clike", cpp: "clike", "c++": "clike", java: "clike", kotlin: "clike", swift: "clike", cs: "clike", php: "clike",
  json: "json", jsonc: "json", json5: "json",
  yaml: "yaml", yml: "yaml",
  toml: "toml",
  sh: "shell", bash: "shell", zsh: "shell", shell: "shell", console: "shell", terminal: "shell", fish: "shell",
  css: "css", scss: "scss", less: "scss", sass: "scss",
  ini: "ini", cfg: "ini", conf: "ini", properties: "ini",
  html: "xml", xml: "xml", svg: "xml",
  sql: "sql",
  diff: "diff", patch: "diff",
};
const fenceLang = (info: string | undefined) => (info ? FENCE_ALIAS[info.split(/\s+/)[0].toLowerCase()] ?? "" : "");

/**
 * Both halves are memoized on purpose: re-lexing a 1 MB doc costs ~85ms and
 * rebuilding its element tree ~165ms, and the pane re-renders on every
 * mousemove of a divider drag. Without these a drag over a big document is a
 * visible freeze.
 */
export const Markdown = memo(function Markdown(props: MarkdownProps) {
  // `gfm` gives tables/strikethrough/task lists; `breaks` stays off so hard
  // wraps in a source file don't become <br> soup.
  //
  // The lexer itself recurses per nesting level and can blow the stack on
  // pathological input well inside the 1 MB read cap — 4000 `*` then a letter
  // then 4000 `*` (8 KB) is enough, as is a 2000-deep blockquote. That is a
  // RangeError from inside marked, which our own depth guard cannot prevent,
  // so it is caught here and the document degrades to plain text rather than
  // taking the pane (or, without the boundary above, the window) with it.
  const parsed = useMemo(() => {
    try {
      return { tokens: marked.lexer(props.src, { gfm: true, breaks: false }), failed: false };
    } catch {
      return { tokens: [] as Token[], failed: true };
    }
  }, [props.src]);
  const body = useMemo(() => (parsed.failed ? null : block(parsed.tokens, props)), [parsed, props]);
  if (parsed.failed) {
    return (
      <div className="md">
        <div className="tree-note">too deeply nested to render as markdown — showing the source</div>
        <pre className="md-rawhtml-block">{props.src}</pre>
      </div>
    );
  }
  return <div className="md">{body}</div>;
});
