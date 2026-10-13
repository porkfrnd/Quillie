/**
 * Prints the measured rectangles so a disagreement can be read rather than
 * guessed at. Run after a failure in ui-probe.mjs.
 */
import { chromium } from 'playwright';
import { existsSync, globSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

const root = join(homedir(), '.cache', 'ms-playwright');
const candidates = [
  ...globSync(join(root, 'chromium_headless_shell-*', 'chrome-headless-shell-linux64', 'chrome-headless-shell')),
  ...globSync(join(root, 'chromium-*', 'chrome-linux64', 'chrome')),
].filter(existsSync).sort();

const browser = await chromium.launch({
  executablePath: candidates.at(-1),
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
});
const page = await browser.newPage({ viewport: { width: 1280, height: 736 } });
await page.goto('http://localhost:1420/?mock=1&state=full', { waitUntil: 'networkidle' });
await page.waitForTimeout(1500);

const geom = await page.evaluate(() => {
  const box = (el) => {
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: +r.x.toFixed(1), y: +r.y.toFixed(1), w: +r.width.toFixed(1), h: +r.height.toFixed(1), right: +r.right.toFixed(1), bottom: +r.bottom.toFixed(1) };
  };
  return {
    timeline: box(document.querySelector('.timeline')),
    ruler: box(document.querySelector('[data-probe="ruler"]')),
    header: box(document.querySelector('[data-probe="header"]')),
    rows: [...document.querySelectorAll('[data-probe="lane"]')].map(box),
    canvas: box(document.querySelector('canvas')),
  };
});
console.log(JSON.stringify(geom, null, 1));
await browser.close();