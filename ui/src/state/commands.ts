/**
 * Command registry (§17).
 *
 * Every action in Quillie is one entry here. Menus, context menus, the Ctrl+K
 * palette and tooltips all read from this list, which is the only way to keep
 * them from drifting apart — a shortcut that works in the palette but is not
 * listed in the menu is a bug nobody catches until a user reports it.
 *
 * `when` gates visibility and enablement from UI state. It must be cheap and
 * side-effect free: it runs on every keystroke to rebuild the palette.
 */

export type CommandId = string;

export interface CommandContext {
  /** Is a project open? */
  readonly projectOpen: boolean;
  /** Is there a selection to act on? */
  readonly hasSelection: boolean;
  /** Is the timeline locked at the current track? */
  readonly locked: boolean;
}

export interface Command {
  readonly id: CommandId;
  readonly label: string;
  /** Keyboard shortcut in the `mod+shift+k` form. Null for pointer-only. */
  readonly shortcut: string | null;
  /** Grouping for the palette and menus. */
  readonly group: 'File' | 'Edit' | 'Clip' | 'View' | 'Playback';
  /** Predicate for visibility and enablement. */
  readonly when?: (ctx: CommandContext) => boolean;
  readonly run: () => void | Promise<void>;
}

const registry = new Map<CommandId, Command>();

export function register(...cmds: readonly Command[]): void {
  for (const c of cmds) {
    if (registry.has(c.id)) {
      throw new Error(`duplicate command id: ${c.id}`);
    }
    registry.set(c.id, c);
  }
}

export function all(): readonly Command[] {
  return [...registry.values()];
}

export function visible(ctx: CommandContext): readonly Command[] {
  return all().filter((c) => c.when?.(ctx) ?? true);
}

export function get(id: CommandId): Command | undefined {
  return registry.get(id);
}

export function clearForTests(): void {
  registry.clear();
}

/**
 * Parse `mod+shift+k` into a KeyboardEvent match.
 *
 * `mod` is Cmd on macOS and Ctrl elsewhere. Mapping it to the physical key
 * rather than a literal character means the binding works regardless of
 * layout, which matters for a keyboard-centric editor.
 */
export function matchesShortcut(
  shortcut: string,
  e: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'>,
): boolean {
  const parts = shortcut.toLowerCase().split('+');
  const key = parts[parts.length - 1];
  const wantMod = parts.includes('mod');
  const wantShift = parts.includes('shift');
  const wantAlt = parts.includes('alt');

  const mod = isMac() ? e.metaKey : e.ctrlKey;
  if (wantMod !== mod) return false;
  if (wantShift !== e.shiftKey) return false;
  if (wantAlt !== e.altKey) return false;
  return e.key.toLowerCase() === key;
}

export function isMac(): boolean {
  if (typeof navigator === 'undefined') return false;
  return /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
}