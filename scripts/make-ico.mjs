#!/usr/bin/env node
/**
 * Build a multi-size Windows .ico from the app's SVG mark.
 *
 * Tauri's icon set wants a .ico for the NSIS installer. ImageMagick on Debian
 * frequently has no ICO encode delegate installed, and pulling in another tool
 * to produce one file is not worth it -- the Vista-era ICO format is just a
 * header, a directory of PNG payloads, and the PNGs themselves.
 *
 * Usage: node scripts/make-ico.mjs <icon.svg> <out.ico>
 */

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';

const [, , svgPath, outPath] = process.argv;
if (!svgPath || !outPath) {
  console.error('usage: node scripts/make-ico.mjs <icon.svg> <out.ico>');
  process.exit(1);
}

const SIZES = [16, 24, 32, 48, 64, 128, 256];
const work = join(tmpdir(), `quillie-ico-${process.pid}`);
execFileSync('mkdir', ['-p', work]);

/** Render one PNG at the given square size. */
function render(size) {
  const p = join(work, `${size}.png`);
  execFileSync('rsvg-convert', ['-w', String(size), '-h', String(size), svgPath, '-o', p]);
  return readFileSync(p);
}

/**
 * ICO directory entry, 16 bytes.
 *
 * width/height are stored as a single byte; 256 is encoded as 0. Each entry
 * carries its own offset, so the payload order can differ from the entry order.
 */
function entry(size, byteLength, offset) {
  const b = Buffer.alloc(16);
  b.writeUInt8(size >= 256 ? 0 : size, 0); // width
  b.writeUInt8(size >= 256 ? 0 : size, 1); // height
  b.writeUInt8(0, 2); // palette size (0 = truecolour)
  b.writeUInt8(0, 3); // reserved
  b.writeUInt16LE(1, 4); // colour planes
  b.writeUInt16LE(32, 6); // bits per pixel
  b.writeUInt32LE(byteLength, 8);
  b.writeUInt32LE(offset, 12);
  return b;
}

const payloads = SIZES.map(render);

// Header: reserved (0), type (1 = icon), image count.
const header = Buffer.alloc(6);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(payloads.length, 4);

const dirSize = 16 * payloads.length;
let offset = 6 + dirSize;
const entries = payloads.map((p, i) => {
  const e = entry(SIZES[i], p.length, offset);
  offset += p.length;
  return e;
});

writeFileSync(outPath, Buffer.concat([header, ...entries, ...payloads]));
rmSync(work, { recursive: true, force: true });

console.log(`${outPath}: ${payloads.length} sizes (${SIZES.join(', ')})`);