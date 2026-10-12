/**
 * The undo/redo commands and the affordances they drive.
 *
 * The property under test is the one that produces the worst bug in an editor:
 * a button whose enabled state drifts from whether pressing it does anything.
 * A user who presses undo and sees nothing conclude that their keypress was
 * dropped, and stops trusting the undo button entirely.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke } from '@tauri-apps/api/core';
import { QuillieError } from '../ipc/commands';
import {
  installEditingCommands,
  onUndoStateChange,
  resetForTests,
  seedUndoState,
  undoAffordance,
} from './editing-commands';
import { clearForTests, get, visible, type CommandContext } from './commands';

const mockedInvoke = vi.mocked(invoke);

const openProject: CommandContext = { projectOpen: true, hasSelection: false, locked: false };
const noProject: CommandContext = { projectOpen: false, hasSelection: false, locked: false };

/** Answer the next invoke with a host-shaped undo state. */
function hostSays(next: { can_undo: boolean; can_redo: boolean }): void {
  mockedInvoke.mockResolvedValueOnce(next);
}

beforeEach(() => {
  clearForTests();
  resetForTests();
  mockedInvoke.mockReset();
  installEditingCommands();
});

afterEach(() => {
  clearForTests();
  resetForTests();
});

describe('undo/redo commands', () => {
  it('are hidden until the host says undo is available', () => {
    // Pessimistic by default: an enabled button with nothing behind it is the
    // failure mode, so a fresh window shows neither.
    expect(get('edit.undo')?.when?.(openProject)).toBe(false);
    expect(get('edit.redo')?.when?.(openProject)).toBe(false);

    seedUndoState({ can_undo: true, can_redo: false });
    expect(get('edit.undo')?.when?.(openProject)).toBe(true);
    expect(get('edit.redo')?.when?.(openProject)).toBe(false);
  });

  it('are hidden entirely when no project is open', () => {
    seedUndoState({ can_undo: true, can_redo: true });
    expect(get('edit.undo')?.when?.(noProject)).toBe(false);
    expect(get('edit.redo')?.when?.(noProject)).toBe(false);
  });

  it('sends the command the host registered', async () => {
    seedUndoState({ can_undo: true, can_redo: false });
    hostSays({ can_undo: false, can_redo: true });

    await get('edit.undo')?.run();

    expect(mockedInvoke).toHaveBeenCalledWith('undo', undefined);
  });

  it('adopts the undo state the host returns', async () => {
    seedUndoState({ can_undo: true, can_redo: false });
    hostSays({ can_undo: true, can_redo: true });

    await get('edit.undo')?.run();

    expect(undoAffordance()).toEqual({ can_undo: true, can_redo: true });
  });

  it('leaves the affordances alone when the host refuses', async () => {
    // The failure this guards: a rejected undo leaves the stack untouched, so
    // clearing the state here would disable undo for a step that still exists.
    seedUndoState({ can_undo: true, can_redo: false });
    mockedInvoke.mockRejectedValueOnce(
      new QuillieError({ code: 'no_history', message: 'nothing to undo', recoverable: true }),
    );

    await expect(get('edit.undo')?.run()).rejects.toThrow(QuillieError);

    expect(undoAffordance()).toEqual({ can_undo: true, can_redo: false });
  });

  it('surfaces a host error as a QuillieError, not a bare rejection', async () => {
    seedUndoState({ can_undo: true, can_redo: false });
    mockedInvoke.mockRejectedValueOnce(
      new QuillieError({ code: 'not_loaded', message: 'No project is open.', recoverable: false }),
    );

    await expect(get('edit.redo')?.run()).rejects.toMatchObject({
      code: 'not_loaded',
      recoverable: false,
    });
  });

  it('notifies subscribers when the affordances change', () => {
    const seen: { can_undo: boolean; can_redo: boolean }[] = [];
    const unsubscribe = onUndoStateChange((s) => seen.push(s));

    seedUndoState({ can_undo: true, can_redo: false });
    unsubscribe();

    expect(seen).toEqual([{ can_undo: true, can_redo: false }]);
  });

  it('stops notifying after unsubscribe', () => {
      const seen: unknown[] = [];
      const unsubscribe = onUndoStateChange((s) => seen.push(s));
      unsubscribe();
      seedUndoState({ can_undo: true, can_redo: false });
      expect(seen).toEqual([]);
    });

  it('uses the shortcuts an editor user already expects', () => {
    expect(get('edit.undo')?.shortcut).toBe('mod+z');
    expect(get('edit.redo')?.shortcut).toBe('mod+shift+z');
  });

  it('groups both under Edit so the palette can section them', () => {
    expect(get('edit.undo')?.group).toBe('Edit');
    expect(get('edit.redo')?.group).toBe('Edit');
  });

  it('lists an available command in visible() and hides an unavailable one', () => {
    seedUndoState({ can_undo: true, can_redo: false });
    const ids = visible(openProject).map((c) => c.id);
    expect(ids).toContain('edit.undo');
    expect(ids).not.toContain('edit.redo');

    expect(visible(noProject)).toHaveLength(0);
  });
});