// Read-mode source rendering: a line-number gutter beside highlighted text.
//
// Shared by the dock's code viewer and by markdown fenced code blocks, so a
// fence in a README colours exactly like the file it was copied from.
//
// The gutter is a SEPARATE column of numbers rather than a per-line wrapper —
// tokens are free to span newlines (block comments, template literals) and the
// two columns stay aligned because they share line-height. The cost is that
// wrapped lines desynchronise the gutter, so wrap mode hides it.
import { useMemo } from "react";
import { tokenize } from "./highlight";

export function CodeSpans({ src, lang }: { src: string; lang: string }) {
  const toks = useMemo(() => tokenize(src, lang), [src, lang]);
  return (
    <>
      {toks.map((t, i) => (t.c ? <span key={i} className={`tk-${t.c}`}>{t.s}</span> : t.s))}
    </>
  );
}

export function CodeBlock({ src, lang, gutter = true, wrap = false, className = "" }: {
  src: string; lang: string; gutter?: boolean; wrap?: boolean; className?: string;
}) {
  // Trailing newline would render a phantom last line number.
  const body = src.endsWith("\n") ? src.slice(0, -1) : src;
  const lines = body === "" ? 0 : body.split("\n").length;
  const showGutter = gutter && !wrap && lines > 1;
  return (
    <div className={`code ${wrap ? "wrap" : ""} ${className}`.trim()}>
      {showGutter && (
        <div className="code-gutter" aria-hidden="true">
          {Array.from({ length: lines }, (_, i) => <div key={i}>{i + 1}</div>)}
        </div>
      )}
      <pre className="code-text"><code><CodeSpans src={body} lang={lang} /></code></pre>
    </div>
  );
}
