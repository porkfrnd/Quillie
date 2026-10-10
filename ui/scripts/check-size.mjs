/**
 * Enforce the bundle budgets from §14.6 in CI.
 *
 * Budgets are measured, not aspirations: these run on every build and fail it,
 * which is the only way a budget is a budget. The numbers are checked in from
 * the accepted baseline and move deliberately, with a measurement behind the
 * change, rather than drifting upward one dependency at a time.
 *
 * Usage: node scripts/check-size.mjs [distDir]
 */

import { gzipSync } from 'node:zlib';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

/** §14.6. Initial JS shell, gzip. */
const MAX_SHELL_JS_GZIP = 250 * 1024;
/** §14.6. Total CSS across the app, gzip. */
const MAX_TOTAL_CSS_GZIP = 40 * 1024;
/**
 * The app binary plus UI, excluding sidecars.
 *
 * Not measurable from the JS build alone, so this checks the UI half here and
 * the Rust half in CI against the built artefact.
 */
const MAX_UI_BYTES = 2 * 1024 * 1024;

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) out.push(...walk(p));
    else out.push(p);
  }
  return out;
}

function gzipSize(path) {
  return gzipSync(readFileSync(path), { level: 9 }).length;
}

function kib(n) {
  return `${(n / 1024).toFixed(1)} KiB`;
}

const distDir = resolve(process.argv[2] ?? 'dist');
const files = walk(distDir).filter((f) => /\.(js|css|html)$/.test(f));

const js = files.filter((f) => f.endsWith('.js'));
const css = files.filter((f) => f.endsWith('.css'));

// The entry is whatever index.html actually loads. Everything else is lazy
// and is not counted against the startup budget.
const html = readFileSync(join(distDir, 'index.html'), 'utf8');
const entryRefs = [...html.matchAll(/(?:src|href)="\/assets\/([^"]+)"/g)].map((m) => m[1]);
const entryJs = js.filter((f) => entryRefs.some((r) => f.endsWith(r)));
const shellJs = entryJs.length > 0 ? entryJs : js.filter((f) => f.includes('index-'));

const shellGzip = shellJs.reduce((sum, f) => sum + gzipSize(f), 0);
const cssGzip = css.reduce((sum, f) => sum + gzipSize(f), 0);
const uiBytes = files.reduce((sum, f) => sum + readFileSync(f).length, 0);

const results = [
  { name: 'initial JS shell (gzip)', value: shellGzip, limit: MAX_SHELL_JS_GZIP },
  { name: 'total CSS (gzip)', value: cssGzip, limit: MAX_TOTAL_CSS_GZIP },
  { name: 'UI total (uncompressed)', value: uiBytes, limit: MAX_UI_BYTES },
];

let failed = false;
console.log('bundle budget check\n');
for (const r of results) {
  const pct = ((r.value / r.limit) * 100).toFixed(1);
  const ok = r.value <= r.limit;
  if (!ok) failed = true;
  console.log(
    `  ${ok ? 'PASS' : 'FAIL'}  ${r.name.padEnd(28)} ${kib(r.value).padStart(11)} / ${kib(r.limit)} (${pct}%)`,
  );
}

// Lazy chunks are reported but not budgeted: PixiJS at 143 KiB gzip is
// expected and only loads when a timeline is opened. What matters is that it
// is *not* in the shell, which the entry check above already proves.
const lazy = js.filter((f) => !shellJs.includes(f));
if (lazy.length > 0) {
  console.log('\n  lazy chunks (not counted against startup):');
  for (const f of lazy) {
    console.log(`    ${kib(gzipSize(f)).padStart(11)}  ${f.split('/').pop()}`);
  }
}

if (failed) {
  console.error('\nBundle budget exceeded. Raise the budget with a measurement, do not just raise it.');
  process.exit(1);
}
console.log('\nAll budgets met.');