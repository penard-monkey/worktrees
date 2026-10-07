# Orfis web SDK pin

- Upstream: Orfis, `packages/sdk-web`, package version 0.1.0 (private).
- Source revision: `3222a74` (PR #11, "Web SDK: send and promise only what it does").
- Artifact: `dist/orfis.es.js`, unchanged bytes, 34,864 bytes.
- SHA-256: `64a692dabe21409a8f9eb6d961fb890c62ffaaf547ccadd0cb77869649ff8623`.
- Reproduced from a clean git archive on macOS arm64, Corepack pnpm 9.15.9
  (upstream `packageManager`), upstream frozen lockfile. Upstream's own suite
  passes at this revision: `pnpm --filter=@orfis/sdk-web test` → 80/80.
- Upstream exports: `init`, `OrfisWidget`, `collectDeviceContext`.
  `orfis.es.d.ts` describes only the host-consumed subset, and types the three
  privacy opt-outs as literal `false` so flipping one cannot be a one-character
  edit.
- The ESM bundle calls its own `autoInit()` at import time. It reads
  `document.currentScript`, which is `null` for a module import, so it returns
  before doing anything — but it DOES run, and that is why the adapter imports
  the module lazily rather than at module scope.
- No license file is present at this private revision; redistribution rights
  must be confirmed by the owner before distributing an enabled integration.

From the Worktrees root with its pinned Node active:

```sh
node app/scripts/vendor-orfis.mjs /path/to/an/orfis-checkout
```

The script exports the exact commit into a temporary directory, installs with
its lockfile, builds, checks the reviewed hash and copies only the ESM artifact.
It does not modify the source checkout. The checkout path is only a regeneration
input; builds/runtime need no Orfis checkout, registry package or remote script.
The upstream source-map trailer is preserved for byte-for-byte verification;
the map is deliberately not distributed. No runtime code depends on it.

**This pin is bundled and wired, but still gated.** The three SDK defects that
blocked the previous pin are fixed here; what remains is Worktrees-side
acceptance on a packaged build, and a product key Orfis cannot mint until we
report the Origin a packaged build sends. See
[the integration status](../../../../docs/orfis-feedback.md).
