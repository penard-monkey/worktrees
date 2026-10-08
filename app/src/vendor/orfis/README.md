# Orfis web SDK pin

- Upstream: Orfis, `packages/sdk-web`, package version 0.1.0 (private).
- Source revision: `3f8c14b` (PR #15, "License the web SDK under MIT").
- Artifact: `dist/orfis.es.js`, unchanged bytes, 34,930 bytes.
- SHA-256: `ccda0aabe1367b9b27c7e85ab620a2923f2049df1165e7e3a900fa5589ddcf17`.
- Reproduced from a clean git archive on macOS arm64, Corepack pnpm 9.15.9
  (upstream `packageManager`), upstream frozen lockfile. **Orfis built the same
  revision independently and reported the same hash and the same byte count**,
  which is what makes this a reproducibility claim rather than a transcription.
- Upstream's own suite passes at 80/80, measured at `3222a74`, and that result
  carries to this revision because `packages/sdk-web/src` is BYTE-IDENTICAL
  between the two (`git diff 3222a74 3f8c14b -- packages/sdk-web/src` is
  empty). `3f8c14b` adds only a LICENSE file, a `license` field and a
  post-minify `postBanner`; the 66-byte growth is that banner and nothing else.
- Upstream exports: `init`, `OrfisWidget`, `collectDeviceContext`.
  `orfis.es.d.ts` describes only the host-consumed subset, and types the three
  privacy opt-outs as literal `false` so flipping one cannot be a one-character
  edit.
- The ESM bundle calls its own `autoInit()` at import time. It reads
  `document.currentScript`, which is `null` for a module import, so it returns
  before doing anything — but it DOES run, and that is why the adapter imports
  the module lazily rather than at module scope.
- **License: MIT**, from this revision onward. `LICENSE` beside this file is a
  verbatim copy of `packages/sdk-web/LICENSE` at `3f8c14b`, and the artifact
  carries the terms in its own first line, so a copy of the bundle alone still
  names them. `package.json` keeps `"private": true` — that governs registry
  PUBLISHING, not redistribution rights, which the license grants. The earlier
  pin had no license at all; that was the blocker, and it is settled.

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
blocked an earlier pin are fixed, the license is settled, and both keys are
minted. What remains is Worktrees-side: the packaged acceptance run, which
needs a real build pointed at a running endpoint. See
[the integration status](../../../../docs/orfis-feedback.md).
