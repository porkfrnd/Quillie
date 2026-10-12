/**
 * Typed wrappers over Tauri commands (§5 rule 6).
 *
 * Commands and events carry IDs, numbers, short strings and progress. Frames,
 * waveforms and thumbnails never come through here — they travel through the
 * custom URI protocol as encoded images or compact binary, requested by ID.
 * Sending decoded RGBA through JSON is the mistake this boundary exists to
 * prevent.
 */

import { invoke } from '@tauri-apps/api/core';

/** The error shape the host returns (rule 8). */
export interface CommandError {
  readonly code: string;
  readonly message: string;
  /** Diagnostics. Never shown verbatim to the user as the headline. */
  readonly detail?: string;
  readonly recoverable: boolean;
}

/**
 * A failure that came back from the host.
 *
 * The host returns `{ code, message, detail?, recoverable }` (rule 8), so
 * errors cross the IPC boundary as data rather than as a thrown value of
 * unknown shape. Wrapping them in a real Error keeps stack traces intact and
 * keeps `only-throw-error` satisfied at the throw site.
 */
export class QuillieError extends Error {
  readonly code: string;
  readonly detail: string | undefined;
  readonly recoverable: boolean;

  constructor(e: CommandError) {
    super(e.message);
    this.name = 'QuillieError';
    this.code = e.code;
    this.detail = e.detail;
    this.recoverable = e.recoverable;
  }
}

function isCommandError(e: unknown): e is CommandError {
  return (
    typeof e === 'object' &&
    e !== null &&
    'code' in e &&
    'message' in e &&
    typeof (e as { code: unknown }).code === 'string' &&
    typeof (e as { message: unknown }).message === 'string'
  );
}

export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  let result: unknown;
  try {
    result = await invoke(command, args);
  } catch (e) {
    throw new QuillieError(
      isCommandError(e)
        ? e
        : { code: 'unknown', message: 'Something went wrong.', recoverable: false },
    );
  }
  return result as T;
}

export const project = {
  open: (path: string) => call<number>('open_project', { path }),
  /**
   * Takes no argument.
   *
   * This used to pass an `id`, which the host has never accepted: the Rust
   * command is `save_project(state)` and saves whatever project is open. A
   * phantom argument forces callers to invent a value that means nothing and
   * hides the fact that there is exactly one open project.
   */
  save: () => call<void>('save_project'),
};

/** One file that came in from disk. Mirrors the host's `ImportedSummary`. */
export interface ImportedMediaSummary {
  readonly media_id: string;
  readonly file_name: string;
  readonly duration_seconds: number;
  readonly width: number | null;
  readonly height: number | null;
  readonly codec: string | null;
  readonly vfr: boolean;
  readonly rotation: number;
  readonly proxy_reason: string | null;
}

/**
 * The result of importing a selection.
 *
 * `failed` is populated alongside `imported` rather than thrown, because
 * dropping nine good files because the tenth was corrupt is a worse outcome
 * than importing nine and telling the user about the tenth.
 */
export interface ImportOutcome {
  readonly imported: readonly ImportedMediaSummary[];
  readonly failed?: readonly string[];
}

export interface ExportSummary {
  readonly duration_seconds: number;
  readonly planned_seconds: number;
  readonly width: number | null;
  readonly height: number | null;
  readonly codec: string | null;
}

/**
 * Whether undo and redo are available.
 *
 * The host decides this, because only it knows whether the top of the stack is
 * a no-op inverse. Guessing in the UI is how a button ends up enabled and
 * doing nothing.
 */
export interface UndoState {
  readonly can_undo: boolean;
  readonly can_redo: boolean;
}

/**
 * An edit, as the host's `EditOp` serialises it.
 *
 * The Rust enum is `#[serde(tag = "op", rename_all = "snake_case")]`, so the
 * discriminant is an `op` field and every variant name is snake_case. This type
 * mirrors that exactly rather than inventing its own shape -- a mismatch here
 * would surface as an op the host silently fails to parse, which is the worst
 * kind of boundary bug.
 *
 * `Ticks` is a newtype over i64 and serialises as a bare integer, so `to`/`at`
 * are plain numbers measured in ticks.
 */
export type EditOp =
  | {
      readonly op: 'insert_clip';
      sequence: string;
      track: string;
      index: number;
      clip: unknown;
    }
  | { readonly op: 'delete_clip'; sequence: string; track: string; clip: string }
  | { readonly op: 'move_clip'; sequence: string; track: string; clip: string; to: number }
  | {
      readonly op: 'trim_clip';
      sequence: string;
      track: string;
      clip: string;
      edge: 'in' | 'out';
      to: number;
    }
  | { readonly op: 'split_clip'; sequence: string; track: string; clip: string; at: number };

export const media = {
  import: (paths: readonly string[]) => call<ImportOutcome>('import_media', { paths: [...paths] }),
};

export const render = {
  // Argument keys are camelCase: Tauri converts them to the snake_case
  // parameter names on the Rust side. A snake_case key here reaches the host
  // as `None` and the export silently runs at the wrong bitrate.
  export: (path: string, width: number, height: number, videoBitrateBps: number) =>
    call<ExportSummary>('export_video', {
      path,
      width,
      height,
      videoBitrateBps,
    }),
};

export const editing = {
  apply: (op: EditOp) => call<UndoState>('apply_edit', { op }),
  undo: () => call<UndoState>('undo'),
  redo: () => call<UndoState>('redo'),
};