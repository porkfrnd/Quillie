/**
 * A fixed project for browser-mode checks, behind `?mock=1`.
 *
 * # Why this is gated
 *
 * The fix prompt's ground rules forbid fake data in the shipped path: seed
 * clips in the real TimelineView showed a user an edit that did not exist and
 * could not be undone. That was a real defect and it is gone.
 *
 * A test harness still needs something to measure, so this lives here and is
 * only reachable when `?mock=1` is present in the URL. `isMockMode()` is the
 * single gate, it reads a query parameter rather than an environment
 * variable, and `buildMockProject` is never called from anything else.
 *
 * In a packaged build the URL is whatever protocol the host serves; there is
 * no query string, so `isMockMode` is false and the real path runs.
 */

import type { Tick } from './types';

/** 1/120000 s. Must match TICKS_PER_SECOND in lib/time.ts. */
const SECOND = 120_000;

export function isMockMode(search?: string): boolean {
  const q = search ?? (typeof location === 'undefined' ? '' : location.search);
  return new URLSearchParams(q).get('mock') === '1';
}

export function mockVariant(search?: string): 'full' | 'empty' {
  const q = search ?? (typeof location === 'undefined' ? '' : location.search);
  return new URLSearchParams(q).get('state') === 'empty' ? 'empty' : 'full';
}

export interface MockTrack {
  readonly id: string;
  readonly name: string;
  readonly kind: 'Video' | 'Audio';
}

export interface MockClip {
  readonly id: string;
  readonly trackId: string;
  readonly name: string;
  readonly kind: 'Video' | 'Audio';
  readonly startTick: Tick;
  readonly endTick: Tick;
}

/** Two tracks, three clips, with names long enough to need truncating. */
export function buildMockProject(): { tracks: MockTrack[]; clips: MockClip[] } {
  return {
    tracks: [
      { id: 'V1', name: 'Video 1', kind: 'Video' },
      { id: 'A1', name: 'Audio 1', kind: 'Audio' },
    ],
    clips: [
      {
        id: 'm1',
        trackId: 'V1',
        name: 'interview_take_04.mov',
        kind: 'Video',
        startTick: 0,
        endTick: 4 * SECOND,
      },
      {
        id: 'm2',
        trackId: 'V1',
        name: 'b-roll establishing shot.mp4',
        kind: 'Video',
        startTick: 5 * SECOND,
        endTick: 9 * SECOND,
      },
      {
        id: 'm3',
        trackId: 'A1',
        name: 'field-recording.wav',
        kind: 'Audio',
        startTick: 0,
        endTick: 9 * SECOND,
      },
    ],
  };
}

/** The same project with nothing on it, for the empty state. */
export function buildEmptyProject(): { tracks: MockTrack[]; clips: MockClip[] } {
  return { tracks: [], clips: [] };
}

/**
 * Test that the mock gate cannot be opened accidentally.
 *
 * A mock project reaching a real session is the exact defect this module was
 * written to avoid, so the gate itself is asserted rather than trusted.
 */
export function mockGateIsClosedWithoutTheFlag(search: string): boolean {
  return isMockMode(search) === false;
}