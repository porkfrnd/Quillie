/**
 * The single source of truth for where everything goes in the timeline.
 *
 * # Why one function
 *
 * Every rectangle in the timeline comes from here: the ruler, the track header
 * column, each track lane, the content area, and the playhead. The Pixi
 * layers, the DOM overlays and hit-testing all read these numbers.
 *
 * The reason is a defect this file exists to make impossible. The ruler was a
 * DOM element 24px tall while the Pixi canvas drew clips starting at y=4 --
 * two pieces of code each with their own idea of where the ruler ended, and
 * nothing checking that they agreed. The result was clips drawn over the
 * ruler's own labels. Neither the ruler nor the clip code was wrong on its
 * own; the layout had no owner.
 *
 * So the invariants are stated once and asserted once. If a rect must not
 * overlap another, that is a property of these numbers, not a convention
 * someone has to remember while drawing.
 *
 * No PixiJS and no DOM here. The renderer draws these rects; the tests check
 * the arithmetic, which matters because the floor machine may have no GPU to
 * render on.
 */

/** Height of the time ruler at the top of the timeline. */
export const RULER_HEIGHT_PX = 28;

/** Width of the fixed track-header column down the left edge. */
export const HEADER_WIDTH_PX = 148;

/** Height of one track lane, including the gap below it. */
export const TRACK_LANE_PX = 56;

/** Padding inside a lane: half above, half below, forming the 8px gap. */
export const TRACK_PADDING_PX = 4;

/** Height of the actual clip rectangle inside its lane. */
export const CLIP_HEIGHT_PX = TRACK_LANE_PX - TRACK_PADDING_PX * 2;

/** How far a dragged clip may go above the top lane before it stops. */
export const MIN_CLIP_WIDTH_PX = 2;

export type Rect = Readonly<{
  x: number;
  y: number;
  width: number;
  height: number;
}>;

/** A rect with its far edges resolved, for cheap collision tests. */
export type BoundedRect = Rect &
  Readonly<{ right: number; bottom: number }>;

export interface Lane extends Rect {
  readonly trackId: string;
  readonly index: number;
}

/** Everything the timeline draws, in one object. */
export interface TimelineRects {
  readonly viewportWidth: number;
  readonly viewportHeight: number;
  /** The ruler strip. Content below it must never start above this. */
  readonly ruler: Rect;
  /** The fixed header column. */
  readonly header: Rect;
  /** The scrollable clip area. */
  readonly content: Rect;
  /** Lanes, top to bottom. Lanes past the viewport are omitted. */
  readonly lanes: readonly Lane[];
  /** Vertical offset that scrolls lanes into view. */
  readonly scrollY: number;
  /** Total height of all lanes, whether or not they fit. */
  readonly totalHeight: number;
  /**
   * Where the playhead line is drawn, or `null` when it is scrolled off to
   * the left. Drawing it anyway would put a line at a negative x, which reads
   * as a clip edge rather than as the playhead.
   */
  readonly playheadX: number | null;
}

export interface RectOptions {
  readonly viewportWidth: number;
  readonly viewportHeight: number;
  readonly trackIds: readonly string[];
  readonly playheadTick: number;
  /** Ticks at the left edge of the viewport. */
  readonly scrollTicks: number;
  readonly pxPerTick: number;
  /** Lane offset, for vertical scrolling when tracks exceed the height. */
  readonly scrollY?: number;
}

/**
 * Build every rect.
 *
 * The invariant this function exists to guarantee: `ruler.bottom` is
 * `RULER_HEIGHT_PX` and the first lane starts at or below it, always, for any
 * number of tracks and any viewport size. `assertNoOverlap` states it.
 */
export function timelineRects(options: RectOptions): TimelineRects {
  const {
    viewportWidth,
    viewportHeight,
    trackIds,
    playheadTick,
    scrollTicks,
    pxPerTick,
    scrollY = 0,
  } = options;

  // Negative viewport sizes are nonsense, but a zero-height container is a
  // real transient state during window resize, and clamping here keeps every
  // consumer from having to defend against it.
  const w = Math.max(0, viewportWidth);
  const h = Math.max(0, viewportHeight);

  const contentTop = RULER_HEIGHT_PX;
  const contentHeight = Math.max(0, h - contentTop);

  const lanes: Lane[] = [];
  for (let i = 0; i < trackIds.length; i++) {
    const trackId = trackIds[i];
    if (trackId === undefined) continue;
    // Lanes are positioned in scroll space, then shifted by scrollY, so the
    // first lane can never land under the ruler regardless of scroll
    // position: the scroll is clamped rather than allowed to go negative.
    const laneTop = contentTop + i * TRACK_LANE_PX - Math.max(0, scrollY);
    if (laneTop + TRACK_LANE_PX <= contentTop) continue; // fully scrolled off
    if (laneTop >= h) break; // below the viewport, and so is everything after

    // A lane only partly scrolled off the top is trimmed to the content edge
    // rather than being returned at its true position. Returning it unchanged
    // would put part of the lane above the content area -- underneath the
    // ruler -- which is the exact overlap this module exists to prevent. The
    // caller still knows which track this is; it just gets the part that is
    // actually on screen.
    const clippedTop = Math.max(laneTop, contentTop);
    lanes.push({
      trackId,
      index: i,
      x: HEADER_WIDTH_PX,
      y: clippedTop,
      width: Math.max(0, w - HEADER_WIDTH_PX),
      height: Math.min(laneTop + TRACK_LANE_PX, h) - clippedTop,
    });
  }

  const playhead = (playheadTick - scrollTicks) * pxPerTick + HEADER_WIDTH_PX;
  const playheadVisible = playhead >= HEADER_WIDTH_PX && playhead <= w;

  return {
    viewportWidth: w,
    viewportHeight: h,
    ruler: { x: 0, y: 0, width: w, height: RULER_HEIGHT_PX },
    header: {
      x: 0,
      y: contentTop,
      width: HEADER_WIDTH_PX,
      height: contentHeight,
    },
    content: {
      x: HEADER_WIDTH_PX,
      y: contentTop,
      width: Math.max(0, w - HEADER_WIDTH_PX),
      height: contentHeight,
    },
    lanes,
    scrollY: Math.max(0, scrollY),
    totalHeight: trackIds.length * TRACK_LANE_PX,
    playheadX: playheadVisible ? playhead : null,
  };
}

/**
 * The clip rectangle inside a lane.
 *
 * Kept here rather than in the renderer so the vertical inset is decided in
 * the same place as the lane it sits in.
 */
export function clipRectInLane(
  lane: Lane,
  startTick: number,
  endTick: number,
  scrollTicks: number,
  pxPerTick: number,
): BoundedRect {
  const x0 = (startTick - scrollTicks) * pxPerTick + lane.x;
  const x1 = (endTick - scrollTicks) * pxPerTick + lane.x;
  const left = Math.max(lane.x, x0);
  const width = Math.max(0, x1 - left);
  const y = lane.y + TRACK_PADDING_PX;
  return {
    // Clamped to the lane's left edge. A clip that starts before the viewport
    // would otherwise be drawn across the track header column, which covers
    // the track name.
    x: left,
    y,
    // An inverted or zero-length range must not produce a negative width;
    // a negative rect silently draws nothing and looks like a missing clip.
    width,
    height: CLIP_HEIGHT_PX,
    right: left + width,
    bottom: y + CLIP_HEIGHT_PX,
  };
}

/**
 * Fail loudly if a rect overlaps another that must stay disjoint.
 *
 * Only used by tests. Kept in the shipped module rather than the test file so
 * the two cannot drift: a test that re-implements the geometry it is checking
 * proves nothing.
 */
export function assertNoOverlap(a: Rect, b: Rect, why: string): void {
  const disjoint = a.x + a.width <= b.x || b.x + b.width <= a.x ||
    a.y + a.height <= b.y || b.y + b.height <= a.y;
  if (!disjoint) {
    throw new Error(
      `${why}: ${JSON.stringify(a)} overlaps ${JSON.stringify(b)}`,
    );
  }
}

/**
 * Clamp a vertical scroll offset so the first lane can never be pushed under
 * the ruler, and the last can never be scrolled past the bottom.
 */
export function clampScrollY(
  scrollY: number,
  totalHeight: number,
  viewportHeight: number,
): number {
  const max = Math.max(0, totalHeight - Math.max(0, viewportHeight - RULER_HEIGHT_PX));
  return Math.min(Math.max(0, scrollY), max);
}