/**
 * Timeline geometry.
 *
 * Pure functions: pixels to ticks, ticks to pixels, visible range. No PixiJS
 * types here, so this is testable without a GPU and without a canvas — which
 * matters because the floor machine may have neither.
 */

import { ticksPerFrame, type FrameRate } from '../lib/time';

export interface Viewport {
  /** Leftmost visible tick. */
  readonly scrollTicks: number;
  /** Width of the viewport in pixels. */
  readonly widthPx: number;
  readonly pxPerTick: number;
}

export const MIN_PX_PER_TICK = 0.0002;
export const MAX_PX_PER_TICK = 2.0;

export function ticksToX(viewport: Viewport, ticks: number): number {
  return (ticks - viewport.scrollTicks) * viewport.pxPerTick;
}

export function xToTicks(viewport: Viewport, x: number): number {
  return viewport.scrollTicks + x / viewport.pxPerTick;
}

/**
 * Snap a tick to the nearest frame boundary at the given rate.
 *
 * Ties (a tick exactly halfway between two frames) round **up**, matching
 * `Math.round` and the Rust `Rounding::Nearest`. The choice is stated rather
 * than left implicit because at a tie the two answers differ by a whole frame
 * and a user dragging a clip to the midpoint will land on the higher one
 * consistently.
 */
export function snapToFrame(ticks: number, rate: FrameRate): number {
  const tpf = ticksPerFrame(rate);
  if (tpf <= 0) return 0;
  return Math.round(ticks / tpf) * tpf;
}

/**
 * Snap to a frame *and* to whichever of `candidates` is within `tolerance`
 * ticks. Used for the visible snapping indicator (§17).
 *
 * Returns the snap target and whether a snap actually happened, because the UI
 * must show which snap engaged rather than only that the clip moved.
 */
export interface SnapResult {
  readonly ticks: number;
  readonly snapped: boolean;
  readonly kind: 'frame' | 'candidate' | null;
}

export function snap(
  ticks: number,
  rate: FrameRate,
  candidates: readonly number[],
  tolerance: number,
): SnapResult {
  const frame = snapToFrame(ticks, rate);
  let best: number | null = null;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const c of candidates) {
    const d = Math.abs(c - ticks);
    if (d < bestDistance) {
      bestDistance = d;
      best = c;
    }
  }
  if (best !== null && bestDistance <= tolerance) {
    return { ticks: best, snapped: true, kind: 'candidate' };
  }
  return { ticks: frame, snapped: true, kind: 'frame' };
}

/** The tick range currently visible, for virtualisation (§14.4). */
export function visibleRange(viewport: Viewport): { start: number; end: number } {
  return {
    start: viewport.scrollTicks,
    end: xToTicks(viewport, viewport.widthPx),
  };
}

/**
 * Clips intersecting the visible range.
 *
 * Only these get drawn. A 2000-clip project shows a few dozen; the rest cost
 * nothing because they are never considered.
 */
export function visibleClips<T>(
  clips: readonly T[],
  bounds: (c: T) => { start: number; end: number },
  range: { start: number; end: number },
  out: T[] = [],
): T[] {
  out.length = 0;
  for (const c of clips) {
    const b = bounds(c);
    if (b.start < range.end && range.start < b.end) {
      out.push(c);
    }
  }
  return out;
}

/** Zoom about a fixed pixel anchor, so the point under the cursor stays put. */
export function zoomAt(
  viewport: Viewport,
  factor: number,
  anchorPx: number,
): Viewport {
  const anchorTicks = xToTicks(viewport, anchorPx);
  const next = Math.min(
    MAX_PX_PER_TICK,
    Math.max(MIN_PX_PER_TICK, viewport.pxPerTick * factor),
  );
  return {
    ...viewport,
    pxPerTick: next,
    scrollTicks: anchorTicks - anchorPx / next,
  };
}