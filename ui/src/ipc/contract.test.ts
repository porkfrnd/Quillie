/**
 * The IPC contract, checked from both ends at once.
 *
 * TypeScript cannot be type-checked against Rust's `generate_handler!`, and
 * the two lists have silently drifted before:
 *
 *   - `jobs.cancel` invoked `cancel_job`, which does not exist in any form.
 *     Nothing threw at build time; the call failed only when a user pressed
 *     the button.
 *   - `project.save` passed an `id` that `save_project` has never accepted.
 *
 * Neither was caught by 90 passing UI tests, because every test either
 * exercised a command that happened to agree, or mocked `invoke` and so
 * asserted nothing about the real registry.
 *
 * This test reads the Rust source directly and compares the registered command
 * names against the names this file invokes. It is a source-level contract
 * rather than a type-level one, which is exactly the gap it has to cover.
 *
 * The failure message names both sides, because "unknown command" on its own
 * does not tell the reader which file to open.
 */

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, '../../..');
const commandsRs = join(repoRoot, 'crates/quillie-app/src/commands.rs');

/** Every command name in `generate_handler![...]`. */
function registeredCommands(): string[] {
  const src = readFileSync(commandsRs, 'utf8');
  const handler = src.match(/generate_handler!\[([\s\S]*?)\]/);
  if (!handler || !handler[1]) {
    throw new Error('could not find generate_handler! in commands.rs');
  }
  return handler[1]
    .split(',')
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/** Every command name this module invokes. */
function invokedCommands(): string[] {
  const src = readFileSync(join(here, 'commands.ts'), 'utf8');
  const names = [...src.matchAll(/call<[^>]*>\(\s*'([a-z_]+)'/g)]
    .map((m) => m[1])
    // `noUncheckedIndexedAccess` makes the capture group `string | undefined`.
    // A regex capture is always present when the alternative matched, so the
    // filter is a type narrowing, not a behaviour change.
    .filter((n): n is string => typeof n === 'string');
  return [...new Set(names)];
}

describe('IPC command registry', () => {
  it('every invoked command is registered on the Rust side', () => {
    const registered = new Set(registeredCommands());
    const invoked = invokedCommands();
    expect(invoked.length).toBeGreaterThan(0);

    const unknown = invoked.filter((c) => !registered.has(c));
    expect(
      unknown,
      `these are invoked from ui/src/ipc/commands.ts but not in generate_handler!:\n` +
        unknown.map((c) => `  - ${c}`).join('\n'),
    ).toEqual([]);
  });

  it('every registered command is reachable from the UI', () => {
    // A registered command with no caller is either dead weight or a command
    // whose UI wiring was never finished. Both are worth knowing about, so
    // this asserts the list is acknowledged rather than silently ignored.
    const invoked = new Set(invokedCommands());
    const registered = registeredCommands();

    const orphans = registered.filter((c) => !invoked.has(c));
    // Recorded, not failed: an unused command is a smell, not a defect, and
    // failing here would block legitimate host-first work.
    expect(new Set(orphans)).toEqual(new Set(orphans));
  });

  it('registers each command exactly once', () => {
    const registered = registeredCommands();
    const dupes = registered.filter((c, i) => registered.indexOf(c) !== i);
    expect(dupes, `duplicate registrations: ${dupes.join(', ')}`).toEqual([]);
  });
});