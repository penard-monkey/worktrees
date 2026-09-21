// The Docs tree — the ONE implementation of "entries in backend order, nested
// into the directories the backend named".
//
// It lives here rather than in `DocsPane.tsx` because two surfaces render it:
// the app's Docs tab, and the browser viewer's persistent navigation
// (`app/viewer/DocsNav.tsx`). A second implementation of this rule is exactly
// the drift this repo keeps paying for — it is why `dnd-check.mjs` and this
// module's own `docs-check.mjs` exist at all.
//
// `app/scripts/docs-check.mjs` slices `tree()` and `DocNode` OUT OF THIS FILE
// and evaluates the real source, so moving either one silently disarms the
// check unless that script's path moves with it.

/** `worktrees_core::docs::DocEntry`. `mtime_ms` is milliseconds, and it is
 *  mtime rather than git status because the documents the mark exists for —
 *  the brief, `task_plan.md`, everything else under `.planning/` — are
 *  gitignored and can never carry one (the field's docstring in `docs.rs` has
 *  the long version). Those are also the rows that move most often, which is
 *  why the mark is worth having at all. */
export type DocEntry = { path: string; rel: string; title: string; group: string; mtime_ms: number };
/** A row in the tree: a document, or a directory holding more rows. */
export type DocNode =
  | { kind: "doc"; entry: DocEntry }
  | { kind: "dir"; path: string; name: string; kids: DocNode[]; count: number };

/** Entries in backend order, nested into the directories the backend named.
 *
 *  **Insertion order is the whole algorithm.** Every node is appended where it
 *  is FIRST seen and never sorted, at any level, because the order is the
 *  backend's decision and some of it is the repo's: `docs::index_with` puts the
 *  root files in a fixed reading order, and `[docs] paths = ["b", "a"]` is
 *  listed b-then-a because the repo said so (`declared_paths_keep_one_
 *  contiguous_run_per_group`). Alphabetising here would quietly overrule that
 *  and nothing would fail — the list would just be subtly not what the project
 *  asked for, which is the silent-mirror failure this component was built
 *  without (see the file note).
 *
 *  The nesting comes from `group`, never from `rel`'s directory part. They
 *  differ on the one row where it matters: the brief lives at
 *  `.planning/brief.md` and is grouped with the ROOT files on purpose, because
 *  a group of one under a gitignored directory name reads as an accident
 *  (`DocEntry::group`). Deriving the parent from `rel` would file it under
 *  `.planning/` — and since the walk now lists the REST of that directory
 *  (`docs.rs` step 4: the plan sets under `.planning/<slug>/`), the brief would
 *  not land in an invented node, it would be swallowed by a real one. The bug
 *  got quieter when the working memory was added, not louder. */
export function tree(entries: DocEntry[]): DocNode[] {
  const roots: DocNode[] = [];
  const dirs = new Map<string, Extract<DocNode, { kind: "dir" }>>();
  for (const e of entries) {
    let into = roots;
    if (e.group) {
      let path = "";
      for (const seg of e.group.split("/")) {
        path = path ? `${path}/${seg}` : seg;
        let dir = dirs.get(path);
        if (!dir) {
          dir = { kind: "dir", path, name: seg, kids: [], count: 0 };
          dirs.set(path, dir);
          into.push(dir);
        }
        into = dir.kids;
      }
    }
    into.push({ kind: "doc", entry: e });
  }
  // The count on a directory row is its WHOLE subtree, which is the number a
  // collapsed row has to state — "archive (31)" while showing nothing is the
  // only thing telling you what is behind it.
  const count = (n: DocNode): number =>
    n.kind === "doc" ? 1 : (n.count = n.kids.reduce((a, k) => a + count(k), 0));
  roots.forEach(count);
  return roots;
}
