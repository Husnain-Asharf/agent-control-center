// Builds accd and acc-hook in release mode and copies them to
// src-tauri/binaries/<name>-<target-triple>, where Tauri's externalBin expects them.
// Target triple: --target <triple>, else TAURI_ENV_TARGET_TRIPLE, else the rustc host.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

function targetTriple() {
  const i = process.argv.indexOf('--target');
  if (i !== -1) {
    const value = process.argv[i + 1];
    if (!value) throw new Error('--target needs a value');
    return value;
  }
  if (process.env.TAURI_ENV_TARGET_TRIPLE) return process.env.TAURI_ENV_TARGET_TRIPLE;
  const host = execFileSync('rustc', ['-vV'], { encoding: 'utf8' })
    .split('\n')
    .find((line) => line.startsWith('host:'));
  if (!host) throw new Error('rustc -vV has no host line');
  return host.slice('host:'.length).trim();
}

const triple = targetTriple();
const explicit = process.argv.includes('--target') || Boolean(process.env.TAURI_ENV_TARGET_TRIPLE);
const bins = ['accd', 'acc-hook'];

const args = ['build', '--release', '-p', 'accd', '-p', 'acc-hook'];
if (explicit) args.push('--target', triple);
console.log(`cargo ${args.join(' ')}`);
execFileSync('cargo', args, { cwd: root, stdio: 'inherit' });

const outDir = join(root, 'target', ...(explicit ? [triple] : []), 'release');
const dest = join(root, 'src-tauri', 'binaries');
mkdirSync(dest, { recursive: true });
for (const bin of bins) {
  const to = join(dest, `${bin}-${triple}`);
  copyFileSync(join(outDir, bin), to);
  console.log(`copied ${bin} -> ${to}`);
}
