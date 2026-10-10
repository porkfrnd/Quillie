/**
 * Guards against source comments leaking into the rendered page.
 *
 * # The bug
 *
 * A `/** ... *\/` block at the top of a `.svelte` file, *above* the `<script>`
 * tag, is not a file header. Svelte parses it as template content and renders
 * it into the page as visible text.
 *
 * It looks completely harmless. It type-checks, it lints, it builds, the app
 * still works — and every user sees a paragraph of source code across the top
 * of the window. It shipped that way once before it was caught by looking at a
 * screenshot of the running app, which is exactly the kind of defect no
 * automated check was watching for.
 *
 * This test makes that impossible to miss again.
 */

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, it, expect } from 'vitest';

function svelteFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) out.push(...svelteFiles(p));
    else if (p.endsWith('.svelte')) out.push(p);
  }
  return out;
}

describe('svelte source hygiene', () => {
  const files = svelteFiles('src');

  it('finds the component tree (guards against a broken glob)', () => {
    expect(files.length).toBeGreaterThan(3);
  });

  it('no component starts with a JS or line comment before its script tag', () => {
    const offenders: string[] = [];
    for (const file of files) {
      const first = readFileSync(file, 'utf8').split('\n')[0] ?? '';
      if (/^\s*(\/\*\*|\/\*|\/\/)/.test(first)) {
        offenders.push(`${file}: ${first.trim()}`);
      }
    }
    expect(
      offenders,
      `These render as visible text in the UI:\n${offenders.join('\n')}`,
    ).toEqual([]);
  });

  it('every component has a script tag', () => {
    for (const file of files) {
      expect(readFileSync(file, 'utf8'), `${file} has no <script> block`).toContain(
        '<script',
      );
    }
  });
});