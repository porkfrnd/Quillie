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
  save: (id: number) => call<void>('save_project', { id }),
};

export const jobs = {
  cancel: (jobId: string) => call<void>('cancel_job', { jobId }),
};