// A small hand-rolled syntax highlighter for the dock's READ mode.
//
// Why not a library: the app ships no UI dependencies (see CLAUDE.md), and a
// real highlighter (Prism/Shiki/CodeMirror) is 100–800 KB for what is a
// quick-peek viewer. This is a single-pass scanner over a per-language table:
// comments, strings, numbers, words. It is deliberately approximate — it never
// parses, so it can mis-colour an exotic construct, but it cannot crash or
// reorder text, and the fallback for an unknown grammar is plain text.
//
// Output is a flat token list; the viewer wraps each in a <span class="tk-…">.
// Tokens may contain newlines (block comments, template strings) — the line
// gutter is a separate column, so alignment holds.

export type TokClass =
  | "" // plain
  | "com" // comment
  | "str" // string / char literal
  | "num" // numeric + boolean/null literals
  | "kw" // keyword
  | "typ" // type-ish (Capitalized ident, primitive type name)
  | "fn" // identifier immediately followed by "("
  | "attr" // object key / attribute / section header
  | "punc"; // brackets and operators

export type Tok = { s: string; c: TokClass };

type Grammar = {
  line: string[]; // line-comment prefixes
  block: [string, string][]; // block-comment delimiters
  quotes: string[]; // string delimiters (same open/close, backslash escapes)
  kw: Set<string>;
  typ: Set<string>;
  lit: Set<string>; // true/false/null/nil/None …
  /** `key:` / `key =` before a value reads as an attribute (JSON/YAML/TOML). */
  keys?: boolean;
  /** `#` comments only after whitespace — `url#frag` / `$#` are not comments. */
  hash?: boolean;
  /** Keyword matching is case-insensitive (SQL). Everywhere else `Set` and
   *  `set` are different identifiers and folding them mislabels types. */
  ci?: boolean;
  /** A quote only opens a string right after `=` — XML/HTML attribute values.
   *  Without this, an apostrophe in body text ("don't") starts a string. */
  attrQuotes?: boolean;
  /** `-` is an identifier character (CSS custom properties, `--ui-rem`). */
  dashWords?: boolean;
  /** Rust `r#"…"#` raw strings, where `#` count sets the terminator. */
  rawHash?: boolean;
};

const set = (s: string) => new Set(s.split(/\s+/).filter(Boolean));

const G: Record<string, Grammar> = {
  ts: {
    line: ["//"], block: [["/*", "*/"]], quotes: ['"', "'", "`"],
    kw: set(`abstract as async await break case catch class const continue debugger declare default delete do
      else enum export extends finally for from function get if implements import in instanceof interface keyof let
      new of package private protected public readonly return satisfies set static super switch this throw try
      type typeof var void while with yield`),
    typ: set(`any bigint boolean never number object string symbol undefined unknown Array Promise Record Map Set
      Partial Readonly ReactNode`),
    lit: set("true false null undefined NaN Infinity"),
  },
  rust: {
    line: ["//"], block: [["/*", "*/"]], quotes: ['"'],
    kw: set(`as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod
      move mut pub ref return self Self static struct super trait type unsafe use where while macro_rules`),
    typ: set(`bool char f32 f64 i8 i16 i32 i64 i128 isize str u8 u16 u32 u64 u128 usize String Vec Option Result
      Box HashMap HashSet PathBuf Path Arc Rc RefCell`),
    lit: set("true false None Some Ok Err"), rawHash: true,
  },
  python: {
    line: ["#"], block: [['"""', '"""'], ["'''", "'''"]], quotes: ['"', "'"],
    kw: set(`and as assert async await break class continue def del elif else except finally for from global if
      import in is lambda nonlocal not or pass raise return try while with yield match case`),
    typ: set("bool bytes dict float int list set str tuple object type Any Optional List Dict"),
    lit: set("True False None self cls"),
  },
  go: {
    line: ["//"], block: [["/*", "*/"]], quotes: ['"', "`", "'"],
    kw: set(`break case chan const continue default defer else fallthrough for func go goto if import interface
      map package range return select struct switch type var`),
    typ: set("bool byte complex64 complex128 error float32 float64 int int8 int16 int32 int64 rune string uint uintptr"),
    lit: set("true false nil iota"),
  },
  ruby: {
    line: ["#"], block: [], quotes: ['"', "'"],
    kw: set(`alias and begin break case class def defined? do else elsif end ensure for if in module next not or
      redo rescue retry return self super then undef unless until when while yield require require_relative attr_accessor`),
    typ: set("Array Hash String Symbol Integer Float Struct Module Class"),
    lit: set("true false nil"),
  },
  clike: {
    line: ["//"], block: [["/*", "*/"]], quotes: ['"', "'"],
    kw: set(`abstract assert break case catch class const continue default do else enum extends final finally for
      fun goto if implements import in instanceof interface internal is let namespace new open operator override
      package private protected public return sealed static struct super switch this throw throws try typedef union
      using val var virtual void volatile where while #include #define #ifdef #endif`),
    typ: set("bool boolean byte char double float int long short signed size_t string unsigned var String List Map"),
    lit: set("true false null nil nullptr NULL None self this"),
  },
  json: {
    line: ["//"], block: [["/*", "*/"]], quotes: ['"'],
    kw: set(""), typ: set(""), lit: set("true false null"), keys: true,
  },
  yaml: {
    line: ["#"], block: [], quotes: ['"', "'"],
    kw: set(""), typ: set(""), lit: set("true false null yes no on off ~"), keys: true, hash: true,
  },
  toml: {
    line: ["#"], block: [], quotes: ['"', "'"],
    kw: set(""), typ: set(""), lit: set("true false"), keys: true, hash: true,
  },
  // INI/conf share TOML's shape but really do take `;` comments.
  ini: {
    line: ["#", ";"], block: [], quotes: ['"', "'"],
    kw: set(""), typ: set(""), lit: set("true false"), keys: true,
  },
  shell: {
    line: ["#"], block: [], quotes: ['"', "'"],
    kw: set(`case do done elif else esac fi for function if in local readonly return select then time until while
      export source alias set unset shift trap exec eval declare typeset let echo printf cd test`),
    typ: set(""), lit: set("true false"), hash: true,
  },
  // Plain CSS has NO line comments — `url(https://…)` would grey out the rest
  // of the line. Only the scss/less superset below gets `//`.
  css: {
    line: [], block: [["/*", "*/"]], quotes: ['"', "'"],
    kw: set(`@media @import @supports @keyframes @font-face @container @layer and not only from to important`),
    typ: set(""), lit: set(""), keys: true, dashWords: true,
  },
  scss: {
    line: ["//"], block: [["/*", "*/"]], quotes: ['"', "'"],
    kw: set(`@media @import @use @forward @mixin @include @extend @supports @keyframes @font-face @container @layer
      @if @else @each @for @function and not only from to important`),
    typ: set(""), lit: set(""), keys: true, dashWords: true,
  },
  xml: {
    line: [], block: [["<!--", "-->"]], quotes: ['"', "'"],
    kw: set(""), typ: set(""), lit: set(""), attrQuotes: true,
  },
  sql: {
    line: ["--"], block: [["/*", "*/"]], quotes: ["'", '"'],
    kw: set(`select from where group by order having limit offset insert into values update set delete create
      table index view drop alter add column primary key foreign references join left right inner outer on as
      and or not null distinct union all case when then else end returning with`),
    typ: set("int integer text varchar boolean timestamp date real blob serial uuid jsonb"),
    lit: set("true false null"), ci: true,
  },
  // NOTE: no `markdown` entry on purpose. Prose has no keywords, and the
  // capitalized-word heuristic below turned every sentence into type colour —
  // markdown source falls through to the plain-text path instead.
  diff: { line: [], block: [], quotes: [], kw: set(""), typ: set(""), lit: set("") },
};

const isWordStart = (ch: string) => /[A-Za-z_$@#]/.test(ch);
// `-` is only an identifier char where the language says so (CSS custom
// properties). Elsewhere it is subtraction and belongs to punctuation.
const isWord = (ch: string, dash: boolean) => (dash ? /[A-Za-z0-9_$?!-]/ : /[A-Za-z0-9_$?!]/).test(ch);
const isDigit = (ch: string) => ch >= "0" && ch <= "9";

/** Diff/patch files highlight per LINE, not per token — colour by the marker. */
function diffTokens(src: string): Tok[] {
  const out: Tok[] = [];
  for (const line of src.split(/(?<=\n)/)) {
    const c: TokClass = /^(\+\+\+|---|diff |index |@@)/.test(line)
      ? "attr"
      : line.startsWith("+") ? "str" : line.startsWith("-") ? "kw" : "";
    out.push({ s: line, c });
  }
  return out;
}

/**
 * Tokenize `src` under grammar `lang`. Unknown lang → one plain token, which is
 * exactly what the viewer renders for plain text.
 */
export function tokenize(src: string, lang: string): Tok[] {
  if (lang === "diff") return diffTokens(src);
  const g = G[lang];
  if (!g) return [{ s: src, c: "" }];

  const out: Tok[] = [];
  const push = (s: string, c: TokClass) => {
    if (!s) return;
    const last = out[out.length - 1];
    if (last && last.c === c) last.s += s; // merge runs so the DOM stays small
    else out.push({ s, c });
  };

  const n = src.length;
  let i = 0;
  let atLineStart = true;

  while (i < n) {
    const ch = src[i];

    // block comments (also python's triple-quoted docstrings)
    let matched = false;
    for (const [o, c] of g.block) {
      if (src.startsWith(o, i)) {
        const end = src.indexOf(c, i + o.length);
        const stop = end < 0 ? n : end + c.length;
        push(src.slice(i, stop), "com");
        i = stop;
        matched = true;
        break;
      }
    }
    if (matched) { atLineStart = false; continue; }

    // line comments. `#` in shell only counts at a word boundary so `#!/bin/sh`
    // and `$#` behave; elsewhere the prefix table decides.
    for (const p of g.line) {
      if (src.startsWith(p, i) && !(g.hash && p === "#" && !atLineStart && !/\s/.test(src[i - 1] ?? " "))) {
        const end = src.indexOf("\n", i);
        const stop = end < 0 ? n : end;
        push(src.slice(i, stop), "com");
        i = stop;
        matched = true;
        break;
      }
    }
    if (matched) { atLineStart = false; continue; }

    // Rust raw strings: r"…", r#"…"#, r##"…"## — the `#` count sets the
    // terminator, so an inner `"` doesn't end the literal.
    if (g.rawHash && ch === "r" && /^r#*"/.test(src.slice(i, i + 8)) && !isWord(src[i - 1] ?? " ", false)) {
      const hashes = /^r(#*)"/.exec(src.slice(i))![1].length;
      const term = `"${"#".repeat(hashes)}`;
      const from = i + 1 + hashes + 1;
      const end = src.indexOf(term, from);
      const stop = end < 0 ? n : end + term.length;
      push(src.slice(i, stop), "str");
      i = stop;
      atLineStart = false;
      continue;
    }

    // Strings. A quote that never closes ON THIS LINE is NOT a string — it is
    // an apostrophe in prose ("don't"), a Rust lifetime (&'a str), or a regex
    // body. Colouring to end-of-line there is the single worst-looking failure
    // this scanner can make, so the delimiter degrades to punctuation instead.
    // Backticks are exempt: template literals legitimately span lines.
    if (g.quotes.includes(ch) && (!g.attrQuotes || /=\s*$/.test(src.slice(Math.max(0, i - 4), i)))) {
      let j = i + 1;
      let closed = false;
      while (j < n) {
        if (src[j] === "\\") { j += 2; continue; }
        if (src[j] === ch) { j += 1; closed = true; break; }
        if (src[j] === "\n" && ch !== "`") break;
        j += 1;
      }
      if (ch === "`") closed = true; // may run to EOF by design
      if (closed) {
        // JSON/YAML/TOML/CSS: a quoted token followed by `:`/`=` is a key
        const isKey = g.keys && /^\s*[:=]/.test(src.slice(j, j + 8));
        push(src.slice(i, j), isKey ? "attr" : "str");
        i = j;
        atLineStart = false;
        continue;
      }
      push(ch, "punc");
      i += 1;
      atLineStart = false;
      continue;
    }

    // numbers (hex/binary/float/underscored/suffixed all fold into one run)
    if (isDigit(ch) || (ch === "." && isDigit(src[i + 1] ?? ""))) {
      let j = i;
      while (j < n && /[0-9a-fA-FxXbBoO._+\-]/.test(src[j])) {
        if ((src[j] === "+" || src[j] === "-") && !/[eE]/.test(src[j - 1] ?? "")) break;
        j += 1;
      }
      push(src.slice(i, j), "num");
      i = j;
      atLineStart = false;
      continue;
    }

    // words
    if (isWordStart(ch)) {
      let j = i + 1;
      while (j < n && isWord(src[j], !!g.dashWords)) j += 1;
      const w = src.slice(i, j);
      const after = src.slice(j, j + 8);
      let c: TokClass = "";
      if (g.kw.has(w) || (g.ci && g.kw.has(w.toLowerCase()))) c = "kw";
      else if (g.lit.has(w)) c = "num";
      else if (g.typ.has(w)) c = "typ";
      else if (/^\(/.test(after)) c = "fn";
      else if (g.keys && /^\s*[:=]/.test(after)) c = "attr";
      else if (/^[A-Z]/.test(w) && /[a-z]/.test(w)) c = "typ";
      push(w, c);
      i = j;
      atLineStart = false;
      continue;
    }

    // whitespace / punctuation
    if (ch === "\n") { push(ch, ""); i += 1; atLineStart = true; continue; }
    if (/\s/.test(ch)) { push(ch, ""); i += 1; continue; }
    push(ch, /[{}[\]()<>;,.:=+\-*/%!&|^~?@]/.test(ch) ? "punc" : "");
    i += 1;
    atLineStart = false;
  }
  return out;
}

/** Lines in `src` — the gutter renders 1..countLines. */
export const countLines = (src: string) => (src ? src.split("\n").length : 0);

/**
 * The same tokens, split so that none of them crosses a line: `out[n]` is line
 * `n`'s tokens and contains no "\n".
 *
 * The scanner still runs over the WHOLE source, which is the entire point. A
 * side-by-side diff needs one DOM row per line, and a token that spanned rows
 * cannot exist there — but tokenizing each line on its own would lose every
 * piece of state that opened above it (a block comment, a template literal, a
 * docstring), so those lines would come back mis-coloured. Tokenize once, cut
 * afterwards.
 *
 * `CodeBlock` deliberately does NOT use this: its gutter is a separate column,
 * so it can leave the multi-line tokens whole and keep the DOM smaller.
 */
export function tokenizeLines(src: string, lang: string): Tok[][] {
  const out: Tok[][] = [[]];
  for (const t of tokenize(src, lang)) {
    if (!t.s.includes("\n")) {
      if (t.s) out[out.length - 1].push(t);
      continue;
    }
    const parts = t.s.split("\n");
    for (let i = 0; i < parts.length; i++) {
      if (i > 0) out.push([]);
      // An empty piece is dropped rather than pushed: it carries no text, and a
      // zero-length <span> per blank line is pure DOM for nothing.
      if (parts[i]) out[out.length - 1].push({ s: parts[i], c: t.c });
    }
  }
  return out;
}
