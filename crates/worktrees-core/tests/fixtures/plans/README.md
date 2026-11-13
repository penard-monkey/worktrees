# Plan reader fixtures

Each `<name>.md` is a plan document and `<name>.json` is what
`worktrees_core::plans::read` answers for it, serialized. Two checks hold the
pair:

- `plans::tests::the_golden_fixtures_read_as_recorded` (Rust) reads every
  `.md` here and compares the reader's answer with its `.json`, so a change
  to the reader that moves a line number or a word fails there first.
- `app/scripts/plans-check.mjs` reads the same `.json` and checks what the
  windows SAY for it (`plans.ts`'s `problemLine`), and that each problem's
  `text` is the `.md`'s own line at that number.

`jellyfin-typo.md` is §2.2's example with one typo (`projet:`) in `## Work`.
To re-record after a deliberate reader change:
`WT_PLANS_RECORD=1 cargo test -p worktrees-core --lib plans::tests::the_golden`.
