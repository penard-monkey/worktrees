// Offers — "there is a thing you have not set up, and here is where it lives".
//
// One registry behind every after-update suggestion, replacing the Home card
// that shipped in v0.25.0 and reached nobody: it rendered only on Home AND only
// with a project, two preconditions that multiply, while the surface that DOES
// reach everyone (the release notes) carried the same message as prose with
// nothing to press.
//
// # The rule that shapes the type: an action is a DEEP LINK, never the act
//
// `action` is a destination, not a function. "Set up" was never one click — the
// MCP panel's button carries a checkbox that decides whether Claude may close
// and REMOVE worktrees, plus the caveat that says what removing one discards.
// An inline button in a modal either drops that choice (installing the mutating
// server on the user's behalf, which is a permission granted by nobody) or
// rebuilds the panel somewhere it does not belong. So an offer's whole job is to
// carry you to the one surface that already owns the decision.
//
// It also keeps a list of offers a LIST: N pending offers are N rows with N
// links, never N embedded panels needing an ordering rule between them.
//
// # Dismissal stores a FINGERPRINT, never a boolean
//
// Generalised from `init_dismissed` (whose value is the suggestion's content
// hash, so a repo that later gains a credential file correctly re-suggests) and
// NOT from `mcp_nudge_dismissed`, which got away with a boolean only because its
// one suggestion can never change. A dismissal silences the suggestion you were
// shown; a different suggestion under the same id is a new question.
//
// # Machine-wide only
//
// Every offer here is about the MACHINE — the same answer whichever project you
// are in. Per-project things to do (doctor drift, a repo's agent setup) are
// NOT offers: they live in `projectTodos.ts`, behind the project's header badge
// and its "Repair / upgrade…" entry, because an offer about one repo would
// otherwise stand in a list with no repo named.
//
// # Where they show: ONE list, two ways in
//
// The band in the release notes is the only place offers are listed, and it is
// the same band whichever way the notes were opened — the automatic "What's
// new" after an update, or Settings → Updates → Release notes. The way back to
// it is the rail button at the bottom of the dock rail, which exists exactly
// while `pendingOffers` is non-empty and opens those notes. Taking an offer
// closes the notes to show its destination, so every entry point has to reopen
// the band — the manual view used to pass no offers, and one "Set up…" made
// the rest unreachable until the next release. A new offer needs no surface of
// its own: return it from `pendingOffers` and both ways in list it.
import type { McpStatus } from "./McpPanel";
import type { CodexMcpStatus } from "./CodexMcpPanel";
import type { PiMcpStatus } from "./PiMcpPanel";
import type { UserSkill } from "./AgentSetup";
import type { GuidanceStatus } from "./GuidancePanel";
import type { CrossProjectStatus } from "./CrossProjectPanel";
import type { CatId } from "./SettingsSheet";

export type OfferId = "mcp-server" | "codex-mcp" | "codex-skills" | "pi-mcp" | "agent-guidance" | "cross-project";

export type Offer = {
  id: OfferId;
  /** Headline, in the user's terms — what they get, not what we install. */
  title: string;
  /** One line. The release-notes row is not the place for the caveats; the
   *  destination states those, where they can be acted on. */
  body: string;
  /** Always the ellipsis form: this opens something, it does not finish it. */
  cta: string;
  /** Where the decision actually lives. */
  to: { cat: CatId; focus: string };
  /** What "this suggestion" currently IS. Dismissal records this, so an offer
   *  whose substance changes asks again while a repeat of the same one stays
   *  quiet. */
  fingerprint: string;
};

/** Everything an offer is allowed to consult. Deliberately narrow: an offer may
 *  ask about the MACHINE, never about which screen you are on. */
export type OfferCtx = {
  mcp: McpStatus | null;
  /** App's startup `codex_mcp_status`. Optional so a caller that knows only
   *  about Claude still type-checks — absent reads as "unknown", which offers
   *  nothing. */
  codexMcp?: CodexMcpStatus | null;
  /** App's startup `pi_mcp_status`. Optional, as above. */
  piMcp?: PiMcpStatus | null;
  /** `agent_user_skills` — a machine-level command, deliberately NOT taken from
   *  some project's `agent_setup_status`: that would make the offer need a
   *  project, which is the v0.25.0 precondition bug again. */
  userSkills?: UserSkill[] | null;
  /** `agent_guidance_status` — machine-level too (agent-guidance §4.5). */
  guidance?: GuidanceStatus | null;
  /** `cross_project_status` — the user's level and the registered projects. */
  crossProject?: CrossProjectStatus | null;
};

/** The pending offers, in the order they should be listed.
 *
 *  `absent` and nothing else, which is `State::nudgeable` on the Rust side. The
 *  three states that are also "not working" — `stale`, `read-only`, `foreign` —
 *  are deliberately NOT offers: they are problems, they belong in Settings where
 *  they cannot be silenced, and a user who dismissed "install this" has not
 *  agreed to be quiet about "the thing you installed is broken".
 *
 *  `cli-missing` is likewise not here: its remedy is the installer, which
 *  Settings → Updates already owns and already badges. */
//  ⚠ `ctx.mcp` is App's startup probe, which runs with NO repo — so it cannot
//  see a server installed at LOCAL or PROJECT scope (`mcpsetup::status` only
//  consults those with a repo in hand). Such a machine is covered and will
//  still be offered the server until Settings → Claude re-probes with the repo
//  and corrects it. Inherited from the Home card, but it matters more now that
//  the dock rail's offers button is on every screen rather than one.
export function pendingOffers(ctx: OfferCtx, dismissed: Record<string, string>): Offer[] {
  const out: Offer[] = [];
  if (ctx.mcp?.state === "absent") {
    out.push({
      id: "mcp-server",
      title: "Let Claude drive your worktrees",
      body: "Claude can create, inspect and close worktrees as tools instead of running git by hand — one setup, every project.",
      cta: "Set up…",
      to: { cat: "claude", focus: "mcp-server" },
      // The state IS the suggestion here: any other state is either done or a
      // problem, and both retire this offer rather than changing it.
      fingerprint: "absent",
    });
  }
  // The Codex twin of `mcp-server`, and the same rule: `absent` only. Codex's
  // broken states (stale / read-only / foreign) are problems for its panel;
  // `cli-missing` (no worktrees CLI) is Updates' sentence. And it also needs
  // the Codex CLI itself — a user without Codex has nothing to connect, and
  // suggesting a server for an agent they never installed is noise.
  if (ctx.codexMcp?.state === "absent" && ctx.codexMcp.codex_bin) {
    out.push({
      id: "codex-mcp",
      title: "Let Codex drive your worktrees",
      body: "Codex can create, inspect and close worktrees as tools, the same server Claude uses — one setup, every project.",
      cta: "Set up…",
      to: { cat: "codex", focus: "codex-mcp" },
      fingerprint: "absent",
    });
  }
  // pi's twin, same rule. `absent` already means pi is installed (without pi
  // the state is `pi-missing`), and `disabled`/`unreadable` are problems for
  // Settings → pi, not suggestions.
  if (ctx.piMcp?.state === "absent" && ctx.piMcp.pi_bin) {
    out.push({
      id: "pi-mcp",
      title: "Let pi drive your worktrees",
      body: "pi lanes can report back, wait for other places and create or close worktrees as tools — the same server Claude and Codex use.",
      cta: "Set up…",
      to: { cat: "pi", focus: "pi-mcp" },
      fingerprint: "absent",
    });
  }
  const missing = (ctx.userSkills ?? []).filter((u) => u.status === "missing").map((u) => u.name).sort();
  // Dismissed = the SET the user was shown. Re-ask only when something NEW is
  // unlinked: linking one of five (or deleting a skill) shrinks the set, which
  // is a different fingerprint but no new question — a plain `!==` re-asked
  // exactly then, nagging about skills the user had already declined.
  const declined = new Set((dismissed["codex-skills"] ?? "").split(",").filter(Boolean));
  if (missing.some((n) => !declined.has(n))) {
    out.push({
      id: "codex-skills",
      title: "Let Codex use your Claude skills",
      body: `${missing.length} skill${missing.length === 1 ? "" : "s"} in ~/.claude/skills ${missing.length === 1 ? "isn't" : "aren't"} visible to Codex. Linking adds symlinks and changes nothing else.`,
      cta: "Review…",
      to: { cat: "codex", focus: "codex-skills" },
      // The SET of unlinked skills: a skill outside the declined set is a new
      // question; the same set, or any subset of it, stays quiet (see above).
      // `conflict` skills are not in it — linking cannot help them.
      fingerprint: missing.join(","),
    });
  }
  // Agent guidance (agent-guidance §4.5, decision Q8). Not an `absent` thing to
  // install: per-launch delivery is ON by default, which changes what agents
  // are told, so the offer is "this is now happening — review it, and choose
  // the guard". It asks once per guidance VERSION (core's `guidance::VERSION`,
  // bumped only for a change worth re-asking), never per wording fix, and only
  // when it is true: delivery on, and at least one agent installed to get it.
  const g = ctx.guidance;
  if (g && g.settings.enabled && g.harnesses.some((h) => h.installed)) {
    out.push({
      id: "agent-guidance",
      title: "Your agents now learn to work in places",
      body: "Claude, Codex and pi get the worktrees skill and rule when Worktrees launches them. Review what they are told, and whether Claude may be stopped from branching in (main).",
      cta: "Review…",
      to: { cat: "guidance", focus: "agent-guidance" },
      fingerprint: `v${g.version}`,
    });
  }
  // Cross-project reach (cross-project P1b, decided 2026-10-01: off by
  // default, OFFERED after the update). Only while it is off, and only on a
  // machine with at least two registered projects — with one there is nothing
  // else to reach, and the offer would be noise. The level is the suggestion:
  // turning reach on (to either level) retires it, and a dismissal holds for
  // as long as it stays off.
  const xp = ctx.crossProject;
  if (xp && xp.level === "off" && xp.projects.length >= 2) {
    out.push({
      id: "cross-project",
      title: "Let agents see your other projects",
      body: "An orchestrator in one repo can check on lanes in your other projects and message their agents, addressed as project:place. Private projects stay out.",
      cta: "Review…",
      to: { cat: "guidance", focus: "cross-project" },
      fingerprint: "off",
    });
  }
  return out.filter((o) => dismissed[o.id] !== o.fingerprint);
}

/** The patch that silences one offer. Never clears another id, and never
 *  silences a different fingerprint of the same id. */
export function dismissPatch(
  o: Offer,
  dismissed: Record<string, string>,
): Record<string, string> {
  return { ...dismissed, [o.id]: o.fingerprint };
}

/** The rail button's tooltip: how many, in the band's own words. */
export function offersTitle(n: number): string {
  return n === 1 ? "1 thing to set up — open" : `${n} things to set up — open`;
}
