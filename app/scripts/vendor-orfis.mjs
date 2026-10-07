// Regenerate in a disposable directory. Never builds in/modifies the source checkout.
// Usage: node app/scripts/vendor-orfis.mjs /path/to/orfis-checkout
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
const revision = '3222a74';
const expected = '64a692dabe21409a8f9eb6d961fb890c62ffaaf547ccadd0cb77869649ff8623';
const source = process.argv[2];
if (!source) throw Error('Pass a local Orfis checkout containing the pinned revision');
const dir = mkdtempSync(join(tmpdir(), 'worktrees-orfis-'));
try {
  const tar = execFileSync('git', ['-C', source, 'archive', revision], { maxBuffer: 64 * 1024 * 1024 });
  execFileSync('tar', ['-x', '-C', dir], { input: tar });
  execFileSync('corepack', ['pnpm', 'install', '--frozen-lockfile'], { cwd: dir, stdio: 'inherit' });
  execFileSync('corepack', ['pnpm', '--filter=@orfis/sdk-web', 'build'], { cwd: dir, stdio: 'inherit' });
  const bundle = readFileSync(join(dir, 'packages/sdk-web/dist/orfis.es.js'));
  if (createHash('sha256').update(bundle).digest('hex') !== expected) throw Error('Bundle differs from reviewed pin; review before changing the hash');
  writeFileSync(new URL('../src/vendor/orfis/orfis.es.js', import.meta.url), bundle);
  console.log(`Verified ${revision}: ${expected}`);
} finally {
  rmSync(dir, { recursive: true, force: true });
}
