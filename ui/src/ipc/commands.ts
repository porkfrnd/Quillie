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

export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (e) {
    throw toCommandError(e);
  }
}

function toCommandError(e: unknown): CommandError {
  if (typeof e === 'object' && e !== null && 'code' in e && 'message' in e) {
    return e as CommandError;
  }
  return {
    code: 'unknown',
    message: typeof e === 'string' ? e : 'Something went wrong.',
    recoverable: false,
  };
}

export const project = {
  open: (path: string) => call<number>('open_project', { path }),
  save: (id: number) => call<void>('save_project', { id }),
};

export const jobs = {
  cancel: (jobId: string) => call<void>('cancel_job', { jobId }),
};