// Checks the "Clone from URL…" dialog's mirror — app/src/clone.ts — against the
// code it mirrors, core's parser (crates/worktrees-core/src/clone.rs).
//
//   node app/scripts/clone-check.mjs        # exits non-zero on failure
//
// The dialog previews `<parent>/<name>` from its OWN parse of the URL, while
// the backend clones from core's. A rule changed on one side only (a new
// scheme, a different `.git` rule) passes every test that reads one side, and
// the user is shown a folder name — or an error — the clone does not agree
// with. So this runs the real `cloneSource` over core's own `NAME_CASES`
// table, and holds the error-kind list to core's enum in both directions.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
let failed = 0;
const eq = (name, got, want) => {
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g === w) { console.log(`ok ${name}`); return; }
  failed++;
  console.log(`not ok — ${name}\n     got: ${g}\n    want: ${w}`);
};

// Only the non-test half of clone.rs is the contract.
const rs = read("../../crates/worktrees-core/src/clone.rs").split("#[cfg(test)]")[0];

// ── NAME_CASES, row by row ──────────────────────────────────────────────────
const start = rs.indexOf("pub const NAME_CASES");
const table = rs.slice(start, rs.indexOf("];", start));
const rows = [...table.matchAll(/^\s*\("((?:[^"\\]|\\.)*)",\s*(Some\("([^"]*)"\)|None)\),\s*$/gm)]
  .map((m) => ({ input: m[1], want: m[3] ?? null }));
// A row the regex cannot read would silently drop out of the check; count the
// table's row openers independently and insist every one was parsed.
const opened = (table.match(/^\s*\("/gm) ?? []).length;
eq("every NAME_CASES row was parsed", rows.length, opened);
if (rows.length < 10) { failed++; console.log(`not ok — NAME_CASES looks truncated (${rows.length} rows)`); }

const cjs = (await transformWithEsbuild(read("../src/clone.ts"), "clone.ts", { loader: "ts", format: "esm" })).code;
const C = await import("data:text/javascript;base64," + Buffer.from(cjs).toString("base64"));
for (const { input, want } of rows) {
  const r = C.cloneSource(input);
  eq(`cloneSource(${JSON.stringify(input)})`, "name" in r ? r.name : null, want);
}

// ── shorthand expansion: the URL the preview implies is the one core clones ─
eq("owner/repo → github https", C.cloneSource("owner/repo").url, "https://github.com/owner/repo.git");
eq("owner/repo.git → github https", C.cloneSource("owner/repo.git").url, "https://github.com/owner/repo.git");

// ── CloneErrorKind ⇔ CLONE_ERROR_KINDS ──────────────────────────────────────
const en = rs.slice(rs.indexOf("pub enum CloneErrorKind"));
const body = en.slice(en.indexOf("{") + 1, en.indexOf("\n}"));
const snake = (v) => v.replace(/([a-z0-9])([A-Z])/g, "$1_$2").toLowerCase();
const coreKinds = [...body.matchAll(/^\s*([A-Z][A-Za-z]*),\s*$/gm)].map((m) => snake(m[1]));
if (!/#\[serde\(rename_all = "snake_case"\)\]\s*pub enum CloneErrorKind/.test(rs)) {
  failed++;
  console.log("not ok — CloneErrorKind is no longer serialized snake_case; the dialog's kinds would not match");
}
eq("CLONE_ERROR_KINDS is core's enum", [...C.CLONE_ERROR_KINDS].sort(), [...coreKinds].sort());

// ── the mock throws only kinds core can produce ─────────────────────────────
const mock = read("../src/mock/install.ts");
const mstart = mock.indexOf('case "clone_project"');
const mcase = mock.slice(mstart, mock.indexOf('case "clone_cancel"', mstart));
const failTable = mock.slice(mock.indexOf("const MOCK_CLONE_FAIL"), mock.indexOf("];", mock.indexOf("const MOCK_CLONE_FAIL")));
const mockKinds = new Set([
  ...[...mcase.matchAll(/kind: "([a-z_]+)"/g)].map((m) => m[1]),
  ...[...failTable.matchAll(/\["[^"]+", "([a-z_]+)"/g)].map((m) => m[1]),
]);
eq("the mock models a failure of every kind git can produce",
  ["auth", "not_found", "host_key", "network"].filter((k) => !mockKinds.has(k)), []);
eq("the mock throws no kind core lacks", [...mockKinds].filter((k) => !coreKinds.includes(k)), []);

if (failed) { console.log(`\n${failed} failed`); process.exit(1); }
