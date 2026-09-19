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
import type { McpStatus } from "./McpPanel";
import type { CatId } from "./SettingsSheet";

export type OfferId = "mcp-server";

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
export type OfferCtx = { mcp: McpStatus | null };

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
//  the dot is on every screen rather than one.
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
