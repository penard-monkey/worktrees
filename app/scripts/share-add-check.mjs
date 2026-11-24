// A shared lane takes another guest (found in the v0.52.0 three-VM run,
// 2026-11-14): *Sharing…* listed the guests with no way to add one, and a
// request for a shared place said "Sharing… on its lane adds a guest" while
// offering nothing. Holds, in the real sources:
//
//  1. *Sharing…* (`ManageDialog`) offers *Add someone…*, which opens the SAME
//     share dialog (`ShareDialog`, its Who → Pair → Share steps) on that share.
//  2. That dialog's last step lets them in with `share_allow`, never a second
//     `share_create`, and keeps v3's warning; the Pair step still shares
//     nothing (no `share_allow` there either). The Who step offers only the
//     guests not on the share already.
//  3. A request for a shared place offers *Let <name> in…* (the add dialog,
//     the asker ticked), in the corner and in the lane's bar, and no sentence
//     promising an add that is not there.
//  4. The mock's `share_allow` names a guest the share did not, as the host's
//     `ShareStore::allow` does.
//
//   node app/scripts/share-add-check.mjs     # exits non-zero on failure
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { transformWithEsbuild } from "vite";

const here = (rel) => fileURLToPath(new URL(rel, import.meta.url));
const read = (rel) => fs.readFileSync(here(rel), "utf8");
let bad = 0;
const ok = (cond, m) => { if (cond) console.log(`ok   ${m}`); else { console.log(`not ok  ${m}`); bad++; } };
const blank = (src) => src.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " ")).replace(/(^|[^:"'`])\/\/[^\n]*/g, (m, p) => p + " ".repeat(m.length - p.length));

function fnBody(src, name) {
  const at = src.search(new RegExp(`\\n(?:export )?function ${name}\\b`));
  if (at < 0) return "";
  const rest = src.slice(at + 1);
  const next = rest.slice(1).search(/\n(?:export )?(?:async )?function |\n\/\*\* |\nexport (?:const|type|let) /);
  return next < 0 ? rest : rest.slice(0, next + 1);
}

const raw = read("../src/LaneShare.tsx");
const lane = blank(raw);
const manage = fnBody(lane, "ManageDialog");
const dialog = fnBody(lane, "ShareDialog");
const share = fnBody(lane, "ShareStep");
const pair = fnBody(lane, "PairStep");
const bar = fnBody(lane, "LaneShareBar");
const nudge = fnBody(lane, "RequestNudge");
const alerts = fnBody(lane, "ShareAlerts");
ok(manage.length > 200 && dialog.length > 200 && share.length > 200 && pair.length > 200 && bar.length > 200, "LaneShare.tsx: the dialogs and the bar are found");

// ── 1. Add someone… ──
ok(/data-testid="share-add"[^>]*>\s*Add someone…/.test(manage), "Sharing… offers Add someone…");
ok(/<ShareDialog\b[^>]*\baddTo=\{/.test(bar), "the lane's bar opens the share dialog on the share (addTo)");

// ── 2. the last step lets them in ──
ok(/invoke\("share_allow"/.test(share) || /\ballow\(addTo\.id/.test(share), "the share step calls share_allow when it adds to a share");
ok(/for \(const p of picked\) await allow\(addTo\.id, p\);\s*if \(request\) await invoke\("share_request_dismiss", \{ request \}\)/.test(share),
  "…and then closes the request that asked (the host's share_allow leaves it open)");
ok([...lane.matchAll(/"share_create"/g)].length === 1, "…and LaneShare.tsx still has one share_create");
ok(!/share_allow|\ballow\([a-z]/.test(pair), "the Pair step never lets anyone in");
ok(/shareWarning\(names\)/.test(share) && !/addTo[^\n]*&&[^\n]*share-warning/.test(share), "the share step keeps v3's warning in both modes");
ok(/addTo/.test(dialog) && /addTo\?\.guests/.test(dialog), "the Who step leaves out the guests on the share already");

// ── 3. a request for a shared place ──
const fn = raw.match(/export function requestWords[\s\S]*?\n\}/)?.[0] ?? "";
const L = await import("data:text/javascript;base64," + Buffer.from((await transformWithEsbuild(fn, "x.ts", { loader: "ts", format: "esm" })).code).toString("base64"));
const shared = L.requestWords({ project: "/r" }, true, true);
ok(shared.letIn === true && shared.share === false, `requestWords: a shared place offers Let in, not Share (${JSON.stringify(shared)})`);
ok(!/adds a guest/.test(shared.why ?? ""), "…and promises no add that is not there");
const fresh = L.requestWords({ project: "/r" }, true, false);
ok(fresh.share === true && !fresh.letIn, "requestWords: an unshared live place still offers Share…");
ok(L.requestWords({ project: null }, true, true).letIn === false, "requestWords: a place this host does not show offers nothing");
ok(/data-testid="share-request-let-in"[\s\S]{0,300}?\}>\s*Let \{r\.name \?\? "them"\} in…/.test(alerts), "the corner's request offers Let <name> in…");
ok(/addTo=\{/.test(alerts), "…which opens the add dialog");
ok(/Let \{r\.name \?\? "them"\} in…/.test(nudge), "the lane bar's request offers Let <name> in… too");

// ── 4. the mock follows the host ──
const mock = read("../src/mock/install.ts");
const allowCase = mock.slice(mock.indexOf('case "share_allow": {'), mock.indexOf('case "share_remove_guest"'));
ok(allowCase.length > 50 && !/is not on share/.test(allowCase) && /guests\.push\(/.test(allowCase), "mock share_allow names a guest the share did not, as the host");
ok(/Some\(g\) if g\.state == Admission::Allowed => Ok\(false\)[\s\S]{0,400}None => \{\s*s\.guests\.push\(ShareGuest \{ principal: principal\.clone\(\), state: Admission::Allowed/.test(read("../../crates/worktrees-cli/src/serve/shares.rs")),
  "…which is the host's rule (shares.rs ShareStore::allow)");

if (bad) { console.log(`\n${bad} failed`); process.exit(1); }
console.log("\nshare-add-check: all ok");
