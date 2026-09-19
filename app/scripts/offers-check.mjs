// Guards the offer registry — what may nudge, and what a dismissal means.
//
//   node app/scripts/offers-check.mjs        # exits non-zero on failure
//
// WHY THIS EXISTS. The v0.25.0 Home card was correct in every particular and
// reached nobody: `state === "absent"` was right, the dismissal flag was right,
// and it still never rendered, because the SURFACE added two preconditions of
// its own (Home only, and only with a project). Nothing could catch that — the
// bats suite does not render, the unit tests do not know about screens, and the
// mock harness answers instantly and correctly.
//
// So this pins the two decisions that the surfaces are not allowed to re-derive:
//
//   1. WHICH STATES OFFER. Exactly `absent`, mirroring `State::nudgeable` in
//      mcpsetup.rs. The three broken states (stale / read-only / foreign) must
//      never appear as offers: they are problems, they live in Settings, and a
//      user who silenced "install this" has not agreed to be quiet about "the
//      thing you installed is broken". `cli-missing` is likewise not an offer —
//      Settings → Updates owns that sentence.
//
//   2. WHAT A DISMISSAL SILENCES. One id, one fingerprint. The boolean this
//      replaced was safe only because its single suggestion could never change;
//      the moment a second offer exists, a boolean silences a question nobody
//      asked. Same rule `init_dismissed` already keeps.
//
// Same slice-the-real-source shape as race-check.mjs / ctxmenu-check.mjs: it
// evaluates `offers.ts` itself, so it tests the edit and not a paraphrase.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
// vite's esbuild re-export — see race-check.mjs: bare "esbuild" does not resolve
import { transformWithEsbuild } from "vite";

const read = (rel) => fs.readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

let bad = 0;
const fail = (m) => { console.error(`FAIL ${m}`); bad++; };
const ok = (m) => console.log(`ok   ${m}`);

// `offers.ts` imports only TYPES, so stripping the import lines leaves a module
// that stands on its own. Inlining the real source, never a stub — a stub would
// carry a second answer to the rule this file exists to guard.
const src = read("../src/offers.ts").replace(/^import type .*$|^import \{ type .*$/gm, "");
const js = (await transformWithEsbuild(src, "offers-check.ts", { loader: "ts", format: "esm" })).code;
const { pendingOffers, dismissPatch } = await import(
  `data:text/javascript;base64,${Buffer.from(js).toString("base64")}`
);

const status = (state) => ({ state, ai_cmd: "claude", claude_bin: "/c", worktrees_bin: "/w",
  user: null, found_in: [], command: "claude mcp add …", config_path: "/x/.claude.json" });

// ── 1. exactly one state offers ────────────────────────────────────────────
const ALL = ["not-applicable", "installed", "read-only", "stale", "foreign",
             "elsewhere", "absent", "cli-missing"];
const offering = ALL.filter((s) => pendingOffers({ mcp: status(s) }, {}).length > 0);
if (offering.join(",") !== "absent") {
  fail(`states that produce an offer: [${offering}] — expected exactly [absent]. `
     + `A broken server must not be dismissible, and cli-missing belongs to Updates.`);
} else ok("only `absent` offers — stale/read-only/foreign/cli-missing stay silent");

if (pendingOffers({ mcp: null }, {}).length !== 0) fail("a null status (probe failed) must offer nothing");
else ok("an unknown status offers nothing");

// ── 2. an offer's action is a DESTINATION, not a deed ──────────────────────
const [offer] = pendingOffers({ mcp: status("absent") }, {});
if (!offer) {
  fail("no offer for `absent` — nothing else below can run");
} else {
  if (typeof offer.to?.cat !== "string" || typeof offer.to?.focus !== "string") {
    fail("offer.to must name a Settings category AND a section: an offer links, it never acts. "
       + "`Set up` carries a permissions checkbox, and an inline button would decide it for the user.");
  } else ok(`offer.to = ${offer.to.cat}/${offer.to.focus}`);

  if (Object.values(offer).some((v) => typeof v === "function")) {
    fail("an offer carries no functions — a `run()` here is how the choice gets hidden again");
  } else ok("the offer is data, with no action to invoke");

  // the destination must actually exist on the other side
  const sheet = read("../src/SettingsSheet.tsx");
  if (!new RegExp(`id:\\s*"${offer.to.cat}"`).test(sheet)) {
    fail(`SettingsSheet has no category "${offer.to.cat}" — the deep link lands nowhere`);
  } else ok(`SettingsSheet has the "${offer.to.cat}" category`);
  const panels = read("../src/McpPanel.tsx");
  if (!panels.includes(`data-focus="${offer.to.focus}"`)) {
    fail(`nothing carries data-focus="${offer.to.focus}" — the sheet opens but highlights nothing`);
  } else ok(`data-focus="${offer.to.focus}" is on the section`);
}

// ── 3. dismissal is per-id AND per-fingerprint ─────────────────────────────
if (offer) {
  const after = dismissPatch(offer, {});
  if (pendingOffers({ mcp: status("absent") }, after).length !== 0) {
    fail("dismissing the offer did not silence it");
  } else ok("a dismissed offer stays silent for the same fingerprint");

  if (typeof after[offer.id] !== "string" || after[offer.id] === "true") {
    fail(`offers_dismissed["${offer.id}"] = ${JSON.stringify(after[offer.id])} — must be the `
       + "FINGERPRINT, never a boolean-shaped value");
  } else ok(`dismissal stores the fingerprint (${JSON.stringify(after[offer.id])})`);

  // a DIFFERENT fingerprint under the same id is a new question
  const stale = { [offer.id]: "something-else" };
  if (pendingOffers({ mcp: status("absent") }, stale).length !== 1) {
    fail("a dismissal of a DIFFERENT fingerprint silenced this one — that is the boolean bug back");
  } else ok("a different fingerprint re-offers");

  // and it must not disturb its neighbours
  const keep = dismissPatch(offer, { "other-offer": "abc" });
  if (keep["other-offer"] !== "abc") fail("dismissPatch dropped another offer's entry");
  else ok("dismissing one offer leaves the others alone");
}

// ── 4. the surface adds no preconditions (the actual v0.25.0 bug) ──────────
// The release-notes list is the reach; it may be gated on the offers existing
// and on the notes not being the manual view, and on nothing else.
const app = read("../src/App.tsx");
const row = app.match(/\{offers\.length > 0 && \(/);
if (!row) {
  fail("App.tsx no longer renders the offer list from `offers.length > 0` — "
     + "if a screen or project condition crept back in, that is the original bug");
} else ok("the offer list is gated on offers alone");
// …and so is the FEED. Pinning only the render leaves the same bug one line
// away: `offers={[]}` or `offers={sel ? offers : []}` on the modal passes a
// check that looks at the render alone (verified — it did).
const feed = app.match(/offers=\{([^}]*)\}/);
if (!feed) {
  fail("App.tsx no longer passes an `offers=` prop to WhatsNewModal");
} else if (feed[1].trim() !== "whatsNew.manual ? [] : offers") {
  fail(`WhatsNewModal is fed \`offers={${feed[1].trim()}}\` — the only condition allowed on `
     + "the feed is the manual-notes view. A screen or project gate here is the v0.25.0 bug "
     + "moved one line up.");
} else ok("the modal is fed every pending offer (bar the manual view)");

// The band is once-per-version and absent on a fresh install, so the panel must
// carry a dismissal of its own or the gear dot has no off switch.
const panel = read("../src/McpPanel.tsx");
if (!/offerPending/.test(panel) || !/onSilenceOffer/.test(panel)) {
  fail("McpPanel has no offerPending/onSilenceOffer — Settings → Claude is the only "
     + "on-demand surface, and without it a fresh install gets a permanent dot it cannot clear");
} else ok("Settings → Claude can end the suggestion on demand");

if (/canNudge|McpNudge|mcp_nudge_dismissed/.test(app)) {
  fail("App.tsx still references the retired Home card (canNudge/McpNudge/mcp_nudge_dismissed)");
} else ok("the Home card and its boolean are gone");

// ── 5. the dismissal a user already made must survive the rename ───────────
// Dropping this migration re-asks a question they answered — on the very
// release that claims to have fixed how we ask. Static, because `settings.ts`
// cannot be imported here (it pulls in @tauri-apps/api), which is the same
// constraint zoom-check.mjs works under.
const set = read("../src/settings.ts");
const mig = set.match(/if \(from < 3\) \{[\s\S]*?\n  \}/);
if (!mig) {
  fail("settings.ts: no `if (from < 3)` migration — a user who silenced the old Home card "
     + "will be asked again");
} else {
  const m = mig[0];
  if (!m.includes("mcp_nudge_dismissed")) fail("the from<3 migration never reads the old boolean");
  else if (!/offers_dismissed\["mcp-server"\]\s*=/.test(m)) fail("the from<3 migration never writes offers_dismissed[\"mcp-server\"]");
  else ok("an old `mcp_nudge_dismissed: true` carries over to offers_dismissed");
}
if (!/SETTINGS_REV = 3/.test(set)) fail("SETTINGS_REV must be 3 or the migration never runs");
else ok("SETTINGS_REV bumped to 3");

console.log(bad ? `\n${bad} failure(s)` : "\nall good");
process.exit(bad ? 1 : 0);
