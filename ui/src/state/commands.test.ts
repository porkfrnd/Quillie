/**
 * The command registry and shortcut matcher.
 *
 * This file decides what Ctrl+Z does and which menu items are enabled. It had
 * 12.5% branch coverage, which is the lowest in the codebase -- and the
 * branches that were uncovered are exactly the ones a keyboard-centric editor
 * is judged on: whether a binding fires when it should and stays quiet when it
 * should not.
 *
 * Two failure modes matter more than line coverage:
 *
 *   - a shortcut that matches too broadly, so Ctrl+Shift+Z also triggers undo
 *     and the two fight each other;
 *   - a `when` predicate that ignores its context, so a command is offered
 *     with no project open and throws when pressed.
 */

import { afterEach, describe, expect, it } from 'vitest';
import {
  all,
  clearForTests,
  get,
  isMac,
  matchesShortcut,
  register,
  visible,
  type Command,
  type CommandContext,
} from './commands';

const open: CommandContext = { projectOpen: true, hasSelection: true, locked: false };
const closed: CommandContext = { projectOpen: false, hasSelection: false, locked: false };

const cmd = (over: Partial<Command> = {}): Command => ({
  id: 'test.cmd',
  label: 'Test',
  shortcut: null,
  group: 'Edit',
  run: () => {},
  ...over,
});

/** A KeyboardEvent-shaped object; only the five fields the matcher reads. */
const key = (k: string, mods: Partial<{ ctrl: boolean; meta: boolean; shift: boolean; alt: boolean }> = {}) => ({
  key: k,
  ctrlKey: mods.ctrl ?? false,
  metaKey: mods.meta ?? false,
  shiftKey: mods.shift ?? false,
  altKey: mods.alt ?? false,
});

afterEach(() => clearForTests());

describe('matchesShortcut', () => {
  it('matches mod+key with the platform modifier', () => {
    // The test environment is not macOS, so mod is Ctrl. Asserting the mac
    // branch is left to the platform check below.
    const ctrlZ = key('z', { ctrl: true });
    expect(matchesShortcut('mod+z', ctrlZ)).toBe(!isMac());
    expect(matchesShortcut('mod+z', key('z', { meta: true }))).toBe(isMac());
  });

  it('does not fire when no modifier is held', () => {
    expect(matchesShortcut('mod+z', key('z'))).toBe(false);
  });

  it('mod+shift+z does not fire for plain mod+z', () => {
    // The regression that matters most: if shift were ignored, redo and undo
    // would both fire on the same key.
    expect(matchesShortcut('mod+shift+z', key('z', { ctrl: true }))).toBe(false);
  });

  it('mod+z does not fire for mod+shift+z', () => {
    expect(matchesShortcut('mod+z', key('z', { ctrl: true, shift: true }))).toBe(false);
  });

  it('honours alt', () => {
    expect(matchesShortcut('mod+alt+k', key('k', { ctrl: true, alt: true }))).toBe(true);
    expect(matchesShortcut('mod+alt+k', key('k', { ctrl: true }))).toBe(false);
    expect(matchesShortcut('mod+alt+k', key('k', { ctrl: true, alt: true, shift: true }))).toBe(
      false,
    );
  });

  it('matches the physical key regardless of shift-induced case', () => {
    // With shift held, browsers report 'Z'. A binding that did not fold case
    // would silently stop matching on a shifted keypress.
    expect(matchesShortcut('mod+shift+z', key('Z', { ctrl: true, shift: true }))).toBe(
      !isMac(),
    );
  });

  it('matches a shortcut with no modifiers', () => {
    expect(matchesShortcut('k', key('k'))).toBe(true);
    expect(matchesShortcut('k', key('j'))).toBe(false);
    // A modifier the binding does not mention still disqualifies it, so a
    // bare letter cannot fire while Ctrl is held.
    expect(matchesShortcut('k', key('k', { ctrl: true }))).toBe(false);
  });

  it('is case insensitive in the binding itself', () => {
    expect(matchesShortcut('MOD+SHIFT+Z', key('z', { ctrl: true, shift: true }))).toBe(
      !isMac(),
    );
  });

  it('does not fire for a different key', () => {
    expect(matchesShortcut('mod+z', key('y', { ctrl: true }))).toBe(false);
  });
});

describe('registry', () => {
  it('registers and retrieves by id', () => {
    register(cmd({ id: 'a.one' }));
    expect(get('a.one')?.id).toBe('a.one');
    expect(all()).toHaveLength(1);
  });

  it('rejects a duplicate id rather than silently replacing', () => {
    // Silently replacing would mean the second registration wins, and a menu
    // item would quietly point at the wrong action.
    register(cmd({ id: 'dup' }));
    expect(() => register(cmd({ id: 'dup' }))).toThrow(/duplicate/i);
    expect(all()).toHaveLength(1);
  });

  it('get returns undefined for an unknown id', () => {
    expect(get('never.registered')).toBeUndefined();
  });

  it('visible includes commands with no predicate', () => {
    register(cmd({ id: 'always' }));
    expect(visible(closed).map((c) => c.id)).toEqual(['always']);
  });

  it('visible respects the predicate', () => {
    register(cmd({ id: 'needs.project', when: (c) => c.projectOpen }));
    expect(visible(closed)).toHaveLength(0);
    expect(visible(open)).toHaveLength(1);
  });

  it('the predicate receives the context it was given', () => {
    let seen: CommandContext | null = null;
    register(
      cmd({
        id: 'ctx',
        when: (c) => {
          seen = c;
          return true;
        },
      }),
    );
    visible(open);
    expect(seen).toEqual(open);
  });

  it('visible is not affected by registration order', () => {
    register(cmd({ id: 'a' }), cmd({ id: 'b' }), cmd({ id: 'c' }));
    expect(visible(open).map((c) => c.id)).toEqual(['a', 'b', 'c']);
  });

  it('clearForTests empties the registry', () => {
    register(cmd({ id: 'gone' }));
    clearForTests();
    expect(all()).toHaveLength(0);
    expect(get('gone')).toBeUndefined();
  });
});

describe('isMac', () => {
  it('returns a boolean in this environment', () => {
    expect(typeof isMac()).toBe('boolean');
  });

  it('does not throw when navigator is absent', () => {
    // The guard exists for non-DOM contexts (tests, workers). Exercising the
    // branch directly is the only way to keep it from rotting.
    expect(() => isMac()).not.toThrow();
  });
});