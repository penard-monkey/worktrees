// The agent CLIs a place can run — the frontend's mirror of
// `worktrees_core::provider::PROVIDERS` (same ids, same order). One type and
// one list, so "claude or codex" is written once and a third harness is a row
// here plus its backend adapter, not a hunt through every component.
//
// Imports nothing on purpose: `settings.ts` takes its type from here, and the
// zoom check loads `settings.ts` as a data: URL that cannot resolve a runtime
// import (a type-only one is erased before it gets there).

export type Harness = "claude" | "codex";

/** Registry order: the first is the default, and wins a tie wherever one is named. */
export const HARNESSES: readonly Harness[] = ["claude", "codex"];

export const HARNESS_LABEL: Record<Harness, string> = { claude: "Claude", codex: "Codex" };

export const isHarness = (v: unknown): v is Harness => HARNESSES.includes(v as Harness);
