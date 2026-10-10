/**
 * The three views over one project model (§1).
 *
 * Switching views never discards or reinterprets data: all three read and
 * write the same project through the same ops. This file exists only to name
 * them and keep the shell from hard-coding the list.
 */

export type ViewId = 'quick' | 'timeline' | 'motion';

export interface ViewDescriptor {
  readonly id: ViewId;
  readonly label: string;
  /** Shown while the chunk loads. Never a spinner over the whole window. */
  readonly loadingLabel: string;
}

export const VIEWS: readonly ViewDescriptor[] = [
  { id: 'quick', label: 'Quick Edit', loadingLabel: 'Loading Quick Edit…' },
  { id: 'timeline', label: 'Timeline', loadingLabel: 'Loading Timeline…' },
  { id: 'motion', label: 'Motion Studio', loadingLabel: 'Loading Motion Studio…' },
] as const;