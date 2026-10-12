/**
 * Edit and File commands, bound to the host (§17).
 *
 * These are the actions a keyboard-first editor is judged on. They go through
 * the command registry rather than being wired to `keydown` handlers directly,
 * so the menu, the palette and the keyboard all enable and disable from one
 * predicate. A shortcut that works in the palette but is greyed out in the
 * menu is the kind of drift this file exists to prevent.
 *
 * # Why undo state comes from the host
 *
 * `canUndo` is the host's answer, not a guess made here. Only the host knows
 * whether the top of the undo stack is a no-op inverse, and a button that
 * enables itself for a step that does nothing is worse than one that stays
 * disabled: the user presses undo and nothing happens, which reads as a
 * dropped keystroke. Every command therefore refreshes this state from the
 * result of the call it made.
 */

import { editing, QuillieError, type UndoState } from '../ipc/commands';
import { register, type CommandContext } from './commands';

/**
 * Undo/redo affordances.
 *
 * A module-level value rather than component state, because the command
 * registry holds `run` closures with no component to re-render. Subscribers
 * repaint when it changes. It starts pessimistic: before the first call, undo
 * is disabled, so a freshly opened window does not show an enabled button
 * that turns out to have nothing behind it.
 */
let undoState: UndoState = { can_undo: false, can_redo: false };

const subscribers = new Set<(s: UndoState) => void>();

export function undoAffordance(): UndoState {
  return undoState;
}

export function onUndoStateChange(fn: (s: UndoState) => void): () => void {
  subscribers.add(fn);
  return () => subscribers.delete(fn);
}

function publish(next: UndoState): void {
  undoState = next;
  for (const fn of subscribers) fn(next);
}

/**
 * Adopt a state the host reported.
 *
 * Split out from `publish` so tests can seed the store without a Tauri host,
 * and so there is exactly one place that mutates it.
 */
export function seedUndoState(next: UndoState): void {
  publish(next);
}

export function resetForTests(): void {
  undoState = { can_undo: false, can_redo: false };
  subscribers.clear();
}

/**
 * Run an IPC call and fold its undo state back in.
 *
 * A failed call must leave the affordances alone: the host did not change the
 * stack, so the buttons still reflect reality. Swallowing the state update on
 * failure is what keeps a rejected edit from disabling undo entirely.
 */
async function applyCommand<T extends UndoState | void>(fn: () => Promise<T>): Promise<void> {
  try {
    const result = await fn();
    if (result && 'can_undo' in result) publish(result);
  } catch (e) {
    if (e instanceof QuillieError) throw e;
    throw new QuillieError({
      code: 'unknown',
      message: 'Something went wrong.',
      recoverable: false,
    });
  }
}

const withProject = (ctx: CommandContext): boolean => ctx.projectOpen;

export function installEditingCommands(): void {
  register(
    {
      id: 'edit.undo',
      label: 'Undo',
      shortcut: 'mod+z',
      group: 'Edit',
      when: (ctx) => withProject(ctx) && undoState.can_undo,
      run: () => applyCommand(() => editing.undo()),
    },
    {
      id: 'edit.redo',
      label: 'Redo',
      // Shift is the redo modifier everywhere the user has a keyboard, and
      // Y is the muscle memory of every editor made before undo existed.
      shortcut: 'mod+shift+z',
      group: 'Edit',
      when: (ctx) => withProject(ctx) && undoState.can_redo,
      run: () => applyCommand(() => editing.redo()),
    },
  );
}