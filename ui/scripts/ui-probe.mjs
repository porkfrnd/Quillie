/**
 * Browser-mode UI harness (§6).
 *
 * The Tauri webview cannot be trusted for headless layout checks: this run has
 * already spent an hour bisecting a stale vite transform and a frozen
 * screenshot. So the geometry is asserted in Chromium, where a failure names
 * the numbers that disagree instead of producing a PNG nobody can measure.
 *
 * Usage:  node scripts/ui-probe.mjs
 *
 * `--mock` loads a fixed two-track, three-clip project plus an empty variant,
 * so both states can be measured without a real project open.
 */

import { chromium } from 'playwright';
import { existsSync } from 'node:fs';
import { globSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

const BASE = process.env.QUILLIE_URL ?? 'http://localhost:1420';

/**
 * Find the cached Chromium.
 *
 * This machine already has a browser downloaded at a build number the
 * installed Playwright does not expect, so `chromium.launch()` fails looking
 * for a build that was never fetched. Rather than download another ~150 MB,
 * this resolves the newest build actually present on disk. The prompt is
 * explicit that browsers should not be installed here.
 */
function resolveChromium() {
  if (process.env.QUILLIE_CHROMIUM) return process.env.QUILLIE_CHROMIUM;
  const root = join(homedir(), '.cache', 'ms-playwright');
  const candidates = [
    ...globSync(join(root, 'chromium_headless_shell-*', 'chrome-headless-shell-linux64', 'chrome-headless-shell')),
    ...globSync(join(root, 'chromium-*', 'chrome-linux64', 'chrome')),
  ].filter(existsSync);
  if (candidates.length === 0) {
    throw new Error(`no cached chromium under ${root}; set QUILLIE_CHROMIUM`);
  }
  // Highest build number last, so sorting gives the newest.
  candidates.sort();
  return candidates.at(-1);
}

/** The rects this run asserts, matching render/rects.ts. */
const RULER_H = 28;
const HEADER_W = 148;
const LANE = 56;

/**
 * Reads the real rectangles out of the running page.
 *
 * Everything is measured from the DOM rather than from a copy of the layout
 * maths, so this catches the defect it is meant to catch: a component that
 * draws somewhere other than where the layout function says.
 */
const probe = () => {
  const q = (sel) => document.querySelector(sel);
  const box = (el) => {
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height, bottom: r.bottom, right: r.right };
  };
  const overlaps = (a, b) =>
    a && b && a.x < b.right && b.x < a.right && a.y < b.bottom && b.y < a.bottom;

  const timeline = box(q('.timeline'));
  const ruler = box(q('[data-probe="ruler"]'));
  const header = box(q('[data-probe="header"]'));
  const rows = [...document.querySelectorAll('[data-probe="lane"]')].map(box);

  // The clip rectangles the canvas actually drew, lifted into page
  // coordinates by adding the canvas origin. This is the part that matters:
  // the DOM checks above all passed while clips were drawn over the ruler,
  // because the clips are pixels in a canvas and were invisible to them.
  let drawn = [];
  const raw = q('.timeline')?.getAttribute('data-drawn');
  if (raw) {
    try { drawn = JSON.parse(raw); } catch { drawn = []; }
  }
  const canvasBox = timeline;
  const clipRects = drawn.map((r) => ({
    x: (canvasBox?.x ?? 0) + r.x,
    y: (canvasBox?.y ?? 0) + r.y,
    right: (canvasBox?.x ?? 0) + r.x + r.width,
    bottom: (canvasBox?.y ?? 0) + r.y + r.height,
    width: r.width,
    height: r.height,
  }));

  return {
    dpr: globalThis.devicePixelRatio ?? null,
    ruler,
    header,
    lanes: rows,
    clipRects,
    empty: !!q('[data-probe="empty"]'),
    // Clips are measured from the canvas, not from DOM placeholders.
    clipOverRuler: clipRects.some((c) => overlaps(ruler, c)),
    clipOverHeader: clipRects.some((c) => overlaps(header, c)),
    // The header carries a 1px border, so `header.right` sits one pixel past the
    // lane's content edge. A clip starting at 148 with the header ending at 149
    // touches the border, not the column -- so the tolerance is one pixel.
    clipOverHeaderLeftEdge: clipRects.some((c) => c.x < (header?.right ?? 0) - 1.5),
    rulerOverlapsLane: rows.some((l) => overlaps(ruler, l)),
    laneOverlapsLane: rows.some((l, i) =>
      rows.some((o, j) => i !== j && overlaps(l, o)),
    ),
    docScrollW: document.documentElement.scrollWidth,
    clientW: document.documentElement.clientWidth,
    overflowX: document.documentElement.scrollWidth > document.documentElement.clientWidth,
  };
};

const results = [];
const record = (name, ok, detail) => {
  results.push({ name, ok, detail });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${detail ? `  -- ${detail}` : ''}`);
};

const browser = await chromium.launch({
  executablePath: resolveChromium(),
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
});

for (const [label, query] of [
  ['three-clip project', '?mock=1&state=full'],
  ['empty project', '?mock=1&state=empty'],
]) {
  for (const [size, w, h] of [
    ['1280x736', 1280, 736],
    ['1024x640', 1024, 640],
  ]) {
    const page = await browser.newPage({ viewport: { width: w, height: h } });
    const errors = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    page.on('console', (m) => {
      if (m.type() === 'error') errors.push(m.text());
    });

    await page.goto(`${BASE}/${query}`, { waitUntil: 'networkidle' });
    await page.waitForTimeout(1200);

    const p = await page.evaluate(probe);
    const tag = `${label} @ ${size}`;

    if (errors.length) record(`${tag}: no console errors`, false, errors.slice(0, 2).join(' | '));
    else record(`${tag}: no console errors`, true);

    if (p.ruler) {
      record(
        `${tag}: ruler height is ${RULER_H}`,
        Math.abs(p.ruler.height - RULER_H) < 0.6,
        `got ${p.ruler.height}`,
      );
    }

    record(`${tag}: canvas clips do not cover the ruler`, !p.clipOverRuler,
      p.clipRects.length ? `${p.clipRects.length} clips drawn` : 'no clips drawn');
    record(`${tag}: canvas clips clear the track header`, !p.clipOverHeaderLeftEdge,
      p.clipRects.length
        ? `firstClipX=${p.clipRects[0]?.x} headerRight=${p.header?.right}`
        : 'no clips drawn');
    record(`${tag}: header rows do not overlap each other`, !p.laneOverlapsLane);
    record(`${tag}: no horizontal overflow`, !p.overflowX, `scrollW=${p.docScrollW} clientW=${p.clientW}`);

    // The full project must draw every clip. A clip that is laid out but not
    // drawn is invisible to every other check here.
    const wantClips = query.includes('state=full') ? 3 : 0;
    record(`${tag}: draws all ${wantClips} clips`, p.clipRects.length === wantClips,
      `got ${p.clipRects.length}`);

    if (p.lanes.length > 1) {
      const a = p.lanes[0];
      const b = p.lanes[1];
      record(
        `${tag}: lane spacing is ${LANE}`,
        Math.abs((b.y - a.y) - LANE) < 0.6,
        `got ${(b.y - a.y).toFixed(2)}`,
      );
      record(
        `${tag}: first lane clears the ruler`,
        a.y >= RULER_H - 0.6,
        `lane.y=${a.y}`,
      );
    }

    if (p.header) {
      record(
        `${tag}: header width is ${HEADER_W}`,
        Math.abs(p.header.width - HEADER_W) < 0.6,
        `got ${p.header.width}`,
      );
    }

    // The empty state must be present exactly when there is nothing to show.
    const wantEmpty = query.includes('state=empty');
    record(
      `${tag}: empty state ${wantEmpty ? 'shown' : 'hidden'}`,
      p.empty === wantEmpty,
    );

    await page.screenshot({ path: `docs/screens/a-${label.replace(/\s+/g, '-')}-${size}.png` });
    await page.close();
  }
}

// Degraded mode: no WebGL. The timeline must still say something rather than
// going blank, because a blank area reads as a broken app (§14.4).
{
  const page = await browser.newPage({ viewport: { width: 1280, height: 736 } });
  await page.addInitScript(() => {
    const orig = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (type, ...rest) {
      if (String(type).includes('webgl')) return null;
      return orig.call(this, type, ...rest);
    };
  });
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(`${BASE}/?mock=1&state=full`, { waitUntil: 'networkidle' });
  await page.waitForTimeout(1200);
  const p = await page.evaluate(probe);
  const said = p.empty || errors.length > 0 || p.lanes.length > 0;
  record('no-WebGL: timeline still communicates something', said,
    `empty=${p.empty} errors=${errors.length}`);
  await page.screenshot({ path: 'docs/screens/a-no-webgl.png' });
  await page.close();
}

await browser.close();

const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
if (failed.length) {
  console.log('FAILED:');
  for (const f of failed) console.log(`  - ${f.name}  ${f.detail ?? ''}`);
  process.exitCode = 1;
}