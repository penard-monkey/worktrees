// The place header's `.identity` row holds TitleEditor and HeaderBranch as
// siblings. Two siblings with the SAME `key` make React warn and mishandle the
// reconcile: after a rename commits, the `<input class="title-input">` is never
// unmounted and a stale editor sits beside the name. Pure source check — slices
// the real header out of App.tsx and fails on a repeated key among the
// identity row's direct children.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const src = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "src", "App.tsx"), "utf8");
const start = src.indexOf('<div className="identity">');
if (start < 0) { console.error("FAIL: <div className=\"identity\"> not found"); process.exit(1); }
const end = src.indexOf("</header>", start);
const row = src.slice(start, end);
// Only the components that own a reconcile slot in this row.
const keys = [];
for (const name of ["TitleEditor", "HeaderBranch"]) {
  const m = row.match(new RegExp(`<${name}\\b[^>]*?\\bkey=\\{([^}]*)\\}`, "s"));
  if (!m) { console.error(`FAIL: <${name} key=…> not found in the identity row`); process.exit(1); }
  keys.push([name, m[1].replace(/\s+/g, " ").trim()]);
}
const seen = new Map();
let bad = false;
for (const [name, k] of keys) {
  if (seen.has(k)) { console.error(`FAIL: ${name} and ${seen.get(k)} share key {${k}}`); bad = true; }
  seen.set(k, name);
}
if (bad) process.exit(1);
console.log("ok", keys.map(([n, k]) => `${n}={${k}}`).join("  "));
