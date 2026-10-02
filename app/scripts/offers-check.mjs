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
const { pendingOffers, dismissPatch, offersTitle } = await import(
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

// ── 1b. the Codex offers: same rule, plus "Codex is actually here" ─────────
const codex = (state, bin = "/usr/local/bin/codex") => ({ state, codex_bin: bin, worktrees_bin: "/w",
  entry: null, command: null, config_path: "/x/.codex/config.toml" });
const CODEX = ["installed", "read-only", "stale", "foreign", "absent", "cli-missing"];
const ids = (ctx) => pendingOffers({ mcp: null, ...ctx }, {}).map((o) => o.id);
const codexOffering = CODEX.filter((st) => ids({ codexMcp: codex(st) }).includes("codex-mcp"));
if (codexOffering.join(",") !== "absent") {
  fail(`Codex states that offer: [${codexOffering}] — expected exactly [absent]. Broken states are the `
     + "panel's problem to state, never a dismissible suggestion.");
} else ok("codex-mcp: only `absent` offers");
if (ids({ codexMcp: codex("absent", null) }).includes("codex-mcp")) {
  fail("codex-mcp offered with no Codex CLI — a server for an agent the user never installed");
} else ok("codex-mcp needs the Codex CLI present");
if (ids({ codexMcp: null }).length !== 0) fail("an unknown Codex status must offer nothing");
else ok("an unknown Codex status offers nothing");

const skills = (...st) => st.map((status, i) => ({ name: `s${i}`, status }));
if (!ids({ userSkills: skills("missing", "linked") }).includes("codex-skills")) fail("a missing user skill did not offer codex-skills");
else ok("codex-skills offers when a user skill is missing");
if (ids({ userSkills: skills("linked", "conflict") }).includes("codex-skills")) {
  fail("codex-skills offered with nothing MISSING — a conflict is not something linking can fix");
} else ok("linked/conflict skills alone offer nothing");
if (ids({ userSkills: null }).length !== 0 || ids({ userSkills: [] }).length !== 0) fail("no skills data must offer nothing");
else ok("no skills data offers nothing");

// ── 1c. the pi offer: same rule; without pi the state is `pi-missing` ──────
const pi = (state, bin = "/Users/me/.local/bin/pi") => ({ state, pi_bin: bin, worktrees_bin: "/w",
  entry: null, command: null, config_path: "/x/.pi/agent/mcp.json" });
const PI = ["installed", "read-only", "disabled", "stale", "foreign", "unreadable", "absent", "cli-missing", "pi-missing"];
const piOffering = PI.filter((st) => ids({ piMcp: pi(st) }).includes("pi-mcp"));
if (piOffering.join(",") !== "absent") {
  fail(`pi states that offer: [${piOffering}] — expected exactly [absent]. disabled/unreadable are problems for Settings → pi.`);
} else ok("pi-mcp: only `absent` offers");
if (ids({ piMcp: pi("absent", null) }).includes("pi-mcp")) fail("pi-mcp offered with no pi installed");
else ok("pi-mcp needs pi present");
if (ids({ piMcp: null }).length !== 0) fail("an unknown pi status must offer nothing");

// agent-guidance (agent-guidance §4.5): not an `absent` install but "this is
// now happening — review it". True only when delivery is on and some agent is
// installed to get it; asks once per guidance VERSION, never per wording fix.
const guidance = (enabled, installed, version = 1, skill_edit = null, skill_hash = "h1") => ({
  version, settings: { enabled, guard: false }, guard_available: true, settings_path: "", dir: null, error: null,
  harnesses: [{ id: "claude", label: "Claude", installed, state: enabled ? "on" : "off", flags: [] }],
  skill: "", skill_default: "", skill_hash, skill_edit, edit_path: "", rules: "",
});
const edited = (stale, invalid = null, base = { version: 2, hash: "h0", text: "old" }) => ({ text: "mine", base, stale, invalid });
if (!ids({ guidance: guidance(true, true) }).includes("agent-guidance")) fail("agent-guidance: not offered with delivery on and Claude installed");
else ok("agent-guidance: offered when delivery is on and an agent is installed");
if (ids({ guidance: guidance(false, true) }).length !== 0) fail("agent-guidance: offered while the user has delivery OFF");
else ok("agent-guidance: silent with delivery off");
if (ids({ guidance: guidance(true, false) }).length !== 0) fail("agent-guidance: offered with no agent installed");
else ok("agent-guidance: silent with no agent installed");
if (ids({ guidance: null }).length !== 0) fail("agent-guidance: an unknown status must offer nothing");
{
  const [g1] = pendingOffers({ mcp: null, guidance: guidance(true, true, 1) }, {});
  const quiet = pendingOffers({ mcp: null, guidance: guidance(true, true, 1) }, dismissPatch(g1, {}));
  const again = pendingOffers({ mcp: null, guidance: guidance(true, true, 2) }, dismissPatch(g1, {}));
  if (quiet.length !== 0 || again.map((o) => o.id).join() !== "agent-guidance") fail("agent-guidance: a dismissal must hold for its version and lapse on the next");
  else ok("agent-guidance: dismissed per guidance version");
}

// agent-guidance-changed (agent-guidance §11): the user EDITED the skill and the
// shipped default moved under it. Never for a user who did not edit (they get
// the new default silently), never for an edit that is current, never for an
// UNUSABLE edit (a problem, shown in Settings, not silenceable), and never with
// delivery off. Fingerprint = the new default's hash: dismissing silences THIS
// update, and the next change to the default asks again.
{
  const changed = (g, d = {}) => pendingOffers({ mcp: null, guidance: g }, d).filter((o) => o.id === "agent-guidance-changed");
  const dismissedOld = { "agent-guidance": "v1" };
  if (changed(guidance(true, true, 1, null), dismissedOld).length) fail("agent-guidance-changed: offered to a user who never edited the skill");
  else ok("agent-guidance-changed: silent with no edit (the new default just applies)");
  if (changed(guidance(true, true, 1, edited(false)), dismissedOld).length) fail("agent-guidance-changed: offered for an edit based on the current default");
  else ok("agent-guidance-changed: silent while the edit is current");
  if (changed(guidance(true, true, 1, edited(true)), dismissedOld).length !== 1) fail("agent-guidance-changed: not offered for an edit whose default moved");
  else ok("agent-guidance-changed: offered when the default moved under an edit");
  if (changed(guidance(true, true, 1, edited(true, null, null)), dismissedOld).length !== 1) fail("agent-guidance-changed: an edit with no base record must count as changed");
  else ok("agent-guidance-changed: a lost base record still asks");
  if (changed(guidance(true, true, 1, edited(true, "it must start with a `---` frontmatter line")), dismissedOld).length) fail("agent-guidance-changed: an UNUSABLE edit is a problem for Settings, not an offer");
  else ok("agent-guidance-changed: silent for an unusable edit");
  if (changed(guidance(false, true, 1, edited(true)), dismissedOld).length) fail("agent-guidance-changed: offered with delivery off");
  else ok("agent-guidance-changed: silent with delivery off");
  const [c1] = changed(guidance(true, true, 1, edited(true), "h1"));
  const quiet = changed(guidance(true, true, 1, edited(true), "h1"), dismissPatch(c1, {}));
  const again = changed(guidance(true, true, 1, edited(true), "h2"), dismissPatch(c1, {}));
  if (c1?.fingerprint !== "h1" || quiet.length !== 0 || again.length !== 1) fail("agent-guidance-changed: a dismissal must hold for this default and lapse when it changes again");
  else ok("agent-guidance-changed: dismissed per shipped default (its hash)");
}

// cross-project (cross-project P1b, decided: reach OFF by default and OFFERED).
// Only while off, and only with two or more registered projects — with one
// there is nothing else to reach. Turning it on to either level retires it.
const xp = (level, n = 2) => ({ level, config_path: "", projects: Array.from({ length: n }, (_, i) => ({ root: `/r${i}`, name: `p${i}`, private: false })) });
if (!ids({ crossProject: xp("off") }).includes("cross-project")) fail("cross-project: not offered while off with two projects");
else ok("cross-project: offered while off with two registered projects");
if (ids({ crossProject: xp("off", 1) }).length !== 0) fail("cross-project: offered with a single project — nothing to reach");
else ok("cross-project: silent with one project");
if (ids({ crossProject: xp("read") }).length !== 0 || ids({ crossProject: xp("full") }).length !== 0) fail("cross-project: still offered after reach was turned on");
else ok("cross-project: retired once reach is on (read or full)");
if (ids({ crossProject: null }).length !== 0) fail("cross-project: an unknown status must offer nothing");
{
  const [c] = pendingOffers({ mcp: null, crossProject: xp("off") }, {});
  const quiet = pendingOffers({ mcp: null, crossProject: xp("off", 5) }, dismissPatch(c, {}));
  if (quiet.length !== 0) fail("cross-project: a dismissal must hold while reach stays off (more projects is not a new question)");
  else ok("cross-project: a dismissal holds while reach stays off");
}

// ── 2. an offer's action is a DESTINATION, not a deed ──────────────────────
// Every offer id, not just the first: a new offer whose deep link lands
// nowhere is the same bug as the old one.
const every = pendingOffers(
  { mcp: status("absent"), codexMcp: codex("absent"), piMcp: pi("absent"), userSkills: skills("missing"), guidance: guidance(true, true, 1, edited(true)), crossProject: xp("off") }, {});
const sheetSrc = read("../src/SettingsSheet.tsx");
// The render of ONE category: from its `{cat === "x" && <>` to the next
// category's. A data-focus found anywhere in src/ proves nothing about where
// the deep link lands — it could sit in a section another category renders,
// or in a component nothing mounts.
const catSlice = (cat) => {
  const at = sheetSrc.indexOf(`{cat === "${cat}" && <>`);
  if (at < 0) return "";
  const next = sheetSrc.indexOf("{cat === \"", at + 1);
  return sheetSrc.slice(at, next < 0 ? undefined : next);
};
for (const o of every) {
  if (typeof o.to?.cat !== "string" || typeof o.to?.focus !== "string") fail(`${o.id}: offer.to must name a category AND a section`);
  else if (!new RegExp(`id:\\s*"${o.to.cat}"`).test(sheetSrc)) fail(`${o.id}: SettingsSheet has no category "${o.to.cat}"`);
  else if (!catSlice(o.to.cat).includes(`data-focus="${o.to.focus}"`)
    // mcp-server's section predates the call-site convention; section 2 below
    // pins it in McpPanel, which only the claude category mounts.
    && !(o.id === "mcp-server" && read("../src/McpPanel.tsx").includes(`data-focus="${o.to.focus}"`))
    // The pi category mounts PiPanel whole (`{cat === "pi" && <PiPanel`), and
    // PiPanel renders the section: both halves, or the link lands nowhere.
    // …and PiMcpSection must PUT the prop on its section: a literal in
    // PiPanel alone passes even if the component drops it on the floor.
    && !(o.id === "pi-mcp" && sheetSrc.includes(`{cat === "pi" && <PiPanel`)
      && read("../src/PiPanel.tsx").includes(`data-focus="${o.to.focus}"`)
      && /<section[^>]*data-focus=\{focusId\}/.test(read("../src/PiMcpPanel.tsx")))
    // The guidance category mounts GuidanceSection, whose "default changed"
    // band carries this target — rendered for exactly the state that offers it.
    && !(o.id === "agent-guidance-changed" && catSlice("guidance").includes("<GuidanceSection")
      && /<div className="guidance-changed" data-focus="agent-guidance-changed"/.test(read("../src/GuidancePanel.tsx"))
      && /\{usable && edit\.stale && <SkillChanged/.test(read("../src/GuidancePanel.tsx")))) {
    fail(`${o.id}: the "${o.to.cat}" category's render carries no data-focus="${o.to.focus}" — the deep link opens the category and highlights nothing`);
  } else ok(`${o.id} → ${o.to.cat}/${o.to.focus}, rendered by that category`);
  if (Object.values(o).some((v) => typeof v === "function")) fail(`${o.id}: an offer carries no functions`);
  // dismissal, per offer
  if (pendingOffers({ mcp: status("absent"), codexMcp: codex("absent"), piMcp: pi("absent"), userSkills: skills("missing"), guidance: guidance(true, true, 1, edited(true)), crossProject: xp("off") },
    dismissPatch(o, {})).some((x) => x.id === o.id)) fail(`${o.id}: dismissing it did not silence it`);
  else ok(`${o.id}: dismissal silences its own fingerprint (${JSON.stringify(o.fingerprint)})`);
}
if (every.map((o) => o.id).join(",") !== "mcp-server,codex-mcp,pi-mcp,codex-skills,agent-guidance,agent-guidance-changed,cross-project") {
  fail(`expected all seven offers, got [${every.map((o) => o.id)}]`);
}
// The skills fingerprint is the SET of unlinked skills: a new one is a new question.
{
  const [a] = pendingOffers({ mcp: null, userSkills: [{ name: "a", status: "missing" }] }, {});
  const later = pendingOffers({ mcp: null, userSkills: [{ name: "a", status: "missing" }, { name: "b", status: "missing" }] },
    dismissPatch(a, {}));
  if (later.length !== 1) fail("codex-skills: a NEW unlinked skill stayed silenced by an old dismissal");
  else ok("codex-skills: a new unlinked skill re-offers");
  // …and a SHRINK is not a question: declining {a, b} and then linking b must
  // stay quiet, though the fingerprint ("a") differs from the one dismissed.
  const [ab] = pendingOffers({ mcp: null, userSkills: [{ name: "a", status: "missing" }, { name: "b", status: "missing" }] }, {});
  const shrunk = pendingOffers({ mcp: null, userSkills: [{ name: "a", status: "missing" }, { name: "b", status: "linked" }] },
    dismissPatch(ab, {}));
  if (shrunk.length !== 0) fail("codex-skills: linking one of the declined skills re-asked about the rest");
  else ok("codex-skills: a shrinking unlinked set stays silenced");
}

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
// and on nothing else — not even on which way the notes were opened.
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
} else if (feed[1].trim() !== "offers") {
  fail(`WhatsNewModal is fed \`offers={${feed[1].trim()}}\` — every pending offer, in every view. `
     + "The manual view once got `[]`, and since taking an offer closes the notes, one \"Set up…\" "
     + "left the rest unreachable until the next release. A screen or project gate here is the "
     + "v0.25.0 bug moved one line up.");
} else ok("the modal is fed every pending offer, manual view included");
// …and the modal must not re-add that condition INSIDE itself: `manual` may
// retitle the header and nothing else.
{
  const at = app.indexOf("function WhatsNewModal(");
  const body = at < 0 ? "" : app.slice(at, app.indexOf("\n}\n", at));
  // Past the signature (`}) {` closes the props type), so the prop's own
  // declaration does not count as a use.
  const uses = body.slice(body.indexOf("}) {") + 4)
    .split("\n").filter((l) => /\bmanual\b/.test(l) && !/^\s*(\/\/|\*)/.test(l));
  if (!body) fail("App.tsx has no WhatsNewModal");
  else if (uses.length !== 1 || !/manual \? "Release notes"/.test(uses[0])) {
    fail(`WhatsNewModal reads \`manual\` beyond its header title: ${uses.map((l) => l.trim()).join(" | ")}`);
  } else ok("inside the modal, `manual` only retitles the header");
}

// ── 4b. the way BACK: a rail button while anything is pending ──────────────
// Taking an offer closes the notes, so something outside them must reopen the
// band — on every screen, with or without a project, exactly while offers exist.
{
  const at = app.indexOf('<nav className="rail rail-right"');
  const rail = at < 0 ? "" : app.slice(at, app.indexOf("</nav>", at));
  if (!rail) fail("App.tsx has no dock rail (`rail rail-right`) to host the offers button");
  else {
    const gate = rail.match(/\{([^{}]*)&& \(\s*<>\s*<div className="rail-spacer" \/>\s*<button className="rail-icon rail-offers"/);
    if (!gate) fail("the dock rail has no `rail-offers` button behind a spacer — nothing reopens the band once an offer is taken");
    else if (gate[1].trim() !== "offers.length > 0") {
      fail(`the offers button is gated on \`${gate[1].trim()}\` — offers alone, or it is the v0.25.0 bug on a new surface`);
    } else ok("the dock rail's offers button is gated on offers alone");
    const btn = rail.slice(rail.indexOf("rail-offers"));
    if (!/onClick=\{\(\) => showReleaseNotes\(\)\}/.test(btn)) fail("the offers button does not open the release notes (the band)");
    else ok("the offers button opens the notes, whose band lists every offer");
    if (!/\{offers\.length\}/.test(btn)) fail("the offers button shows no count");
    else ok("the offers button carries the count");
  }
  // One mark per fact: the gear's dot is the UPDATE's again.
  const ra = app.match(/const railAlert = ([^;]*);/);
  if (!ra) fail("App.tsx has no `railAlert`");
  else if (/offer/i.test(ra[1])) fail(`railAlert = ${ra[1]} — offers have their own button; a second mark on the gear competes with it`);
  else ok("the gear's dot no longer doubles as the offers mark");
  if (/upd-offer/.test(app) || /upd-offer/.test(read("../src/App.css"))) fail("the purple gear dot (`upd-offer`) is still wired");
  else ok("`upd-offer` is gone");
}
if (offersTitle(1) !== "1 thing to set up — open" || offersTitle(3) !== "3 things to set up — open") {
  fail(`offersTitle: ${JSON.stringify([offersTitle(1), offersTitle(3)])}`);
} else ok("offersTitle agrees in number");

// …and the CONTEXT: an offer whose input is never fed can never render. The two
// Codex inputs must be the machine-level probes, not some project's status.
const ctxCall = app.match(/pendingOffers\(\{([^}]*)\}/);
if (!ctxCall || !/\bcodexMcp\b/.test(ctxCall[1]) || !/\bpiMcp\b/.test(ctxCall[1]) || !/\buserSkills\b/.test(ctxCall[1]) || !/\bguidance\b/.test(ctxCall[1]) || !/\bcrossProject\b/.test(ctxCall[1])) {
  fail(`App.tsx feeds pendingOffers({${ctxCall?.[1] ?? "?"}}) — codexMcp, piMcp, userSkills, guidance and crossProject must all reach it`);
} else ok("pendingOffers is fed mcp + codexMcp + piMcp + userSkills + guidance + crossProject");
if (!/invoke<CrossProjectStatus>\("cross_project_status"\)\.then\(setCrossProject\)/.test(app)) {
  fail("App.tsx no longer probes cross_project_status straight into the offer input");
} else ok("the cross-project offer input comes from a machine-level probe");
{
  const xpSrc = read("../src/CrossProjectPanel.tsx");
  if (!/offerPending/.test(xpSrc) || !/onSilenceOffer/.test(xpSrc) || !/<section[^>]*data-focus=\{focusId\}/.test(xpSrc)) {
    fail("CrossProjectSection must carry offerPending/onSilenceOffer and put data-focus on its section");
  } else ok("Settings → Agent guidance → Other projects can end its suggestion on demand, and is a deep-link target");
  if (!/crossProjectOfferPending=\{!!crossProjectOffer\}/.test(app)) fail("App.tsx does not tell Settings whether the cross-project offer is pending");
  else ok("Settings is told whether the cross-project offer is pending");
}
if (!/invoke<GuidanceStatus>\("agent_guidance_status"\)\.then\(setGuidance\)/.test(app)) {
  fail("App.tsx no longer probes agent_guidance_status straight into the offer input");
} else ok("the agent-guidance offer input comes from a machine-level probe");
{
  const gp = read("../src/GuidancePanel.tsx");
  if (!/offerPending/.test(gp) || !/onSilenceOffer/.test(gp) || !/<section[^>]*data-focus=\{focusId\}/.test(gp)) {
    fail("GuidanceSection must carry offerPending/onSilenceOffer and put data-focus on its section");
  } else ok("Settings → Agent guidance can end its suggestion on demand, and is a deep-link target");
  if (!/changeOfferPending/.test(gp) || !/onSilenceOffer=\{onSilenceChangeOffer\}/.test(gp)) fail("the \"default changed\" band must be able to end its own suggestion");
  else ok("the \"default changed\" band can end its suggestion on demand");
  if (!/guidanceChangeOfferPending=\{!!guidanceChangeOffer\}/.test(app)) fail("App.tsx does not tell Settings whether the agent-guidance-changed offer is pending");
  else ok("Settings is told whether the agent-guidance-changed offer is pending");
}
if (!/invoke<PiMcpStatus>\("pi_mcp_status"\)\.then\(setPiMcp\)/.test(app)) {
  fail("App.tsx no longer probes pi_mcp_status straight into the offer input");
} else ok("the pi offer input comes from a machine-level probe");
if (!/invoke<CodexMcpStatus>\("codex_mcp_status"\)\.then\(setCodexMcp\)/.test(app)
  || !/invoke<UserSkill\[\]>\("agent_user_skills"\)\.then\(setUserSkills\)/.test(app)) {
  fail("App.tsx no longer probes codex_mcp_status / agent_user_skills straight into the offer inputs");
} else ok("the Codex offer inputs come from machine-level probes");

// Taking an offer lands you on its panel with the band closed behind you, so
// the panel carries the same dismissal where you are standing.
const panel = read("../src/McpPanel.tsx");
if (!/offerPending/.test(panel) || !/onSilenceOffer/.test(panel)) {
  fail("McpPanel has no offerPending/onSilenceOffer — Settings → Claude is the only "
     + "on-demand surface, and without it a fresh install gets a permanent dot it cannot clear");
} else ok("Settings → Claude can end the suggestion on demand");
// Same for both Codex offers: each destination section carries its own off switch.
const codexPanel = read("../src/CodexMcpPanel.tsx");
const agentSrc = read("../src/AgentSetup.tsx");
if (!/offerPending/.test(codexPanel) || !/onSilenceOffer/.test(codexPanel)) fail("CodexMcpSection has no offerPending/onSilenceOffer");
else ok("Settings → Codex (MCP) can end its suggestion on demand");
if (!/function UserSkillsSection[\s\S]*offerPending[\s\S]*onSilenceOffer/.test(agentSrc)) fail("UserSkillsSection has no offerPending/onSilenceOffer");
else ok("Settings → Codex (skills) can end its suggestion on demand");
const piPanel = read("../src/PiMcpPanel.tsx");
if (!/offerPending/.test(piPanel) || !/onSilenceOffer/.test(piPanel)) fail("PiMcpSection has no offerPending/onSilenceOffer");
else ok("Settings → pi can end its suggestion on demand");
if (!/piMcpOfferPending=\{!!piMcpOffer\}/.test(app)) fail("App.tsx does not tell Settings whether the pi offer is pending");
else ok("Settings is told whether the pi offer is pending");
if (!/codexMcpOfferPending=\{!!codexMcpOffer\}/.test(app) || !/skillsOfferPending=\{!!skillsOffer\}/.test(app)) {
  fail("App.tsx does not tell Settings which Codex offers are pending");
} else ok("Settings is told which Codex offers are pending");

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
