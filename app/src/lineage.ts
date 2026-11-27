// Lanes started by a lane (docs/proposals/lane-children.md): the shapes the
// engine sends. Types only — the RULE is core's (`worktrees_core::lineage`),
// resolved into `Place.lineage` on every snapshot row, and the nav builds its
// tree from `lineage.parent` alone. Nothing here may re-derive it.

/** One declared link (`store::Link`): `started_by`, `handed_to` or `under`.
 *  `place` is a slug of the same project, or `(main)`. */
export type Link = {
  place: string;
  at: number;
  /** `started_by` only: the starter's registry project, when it is another. */
  project?: string;
  /** `handed_to` only: the removed place this one hung from. */
  from?: string;
  /** `under` only: who moved it — `(main)` (its agent) or a principal. */
  by?: string;
  /** `started_by` only: the starter is a workspace chat, at this bus address
   *  (`workspace:<ws>/<chat>`); `place` is then the chat's id. */
  chat?: string;
};

/** What a caller that may not see other projects gets in place of a starter
 *  in another project (`lineage.starter` and `declared.started_by`): when, and
 *  that it was elsewhere — no project, no slug (serve's `scope`, an MCP
 *  server with cross-project reach off; `lineage::cut_foreign_starter`). */
export type Elsewhere = { at: number; elsewhere: true };

/** `lineage.starter`: the recorded starter, as a header names it, or
 *  `Elsewhere` when it is in a project the caller may not see. */
export type Starter = Elsewhere | {
  place: string;
  at: number;
  project?: string;
  /** A workspace chat's bus address, when a chat started it. */
  chat?: string;
  /** The place it names is gone (removed, or its slug reused). */
  removed?: boolean;
};

/** `Place.lineage` (`lineage::Lineage`). Every key is optional on the wire. */
export type Lineage = {
  starter?: Starter;
  /** `under`, when it holds. */
  under?: Link;
  /** `handed_to`, when it holds. */
  handed_to?: Link;
  /** The drawn parent's slug. Absent: top level. */
  parent?: string;
  /** Next place up for reports and for a removal's handoff. Absent: (main). */
  up?: string;
  /** Where a `report` with no `to` goes now. Absent only for (main). */
  reports_to?: string;
  /** Places whose starter is this one. */
  started?: string[];
  /** Places that move up when this one is removed. */
  lanes?: string[];
};
