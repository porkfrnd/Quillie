/**
 * Turning a project into drawable rectangles.
 *
 * # Why this is not in the component
 *
 * "Which clips are visible, how tall is each track, what colour is it" is the
 * part of a timeline that is easy to get subtly wrong and hard to look at in a
 * screenshot. Keeping it pure means it can be asserted directly: a clip that
 * is one pixel narrower than it should be, or a track drawn at zero height
 * when the window is short, is a bug you can write a test for instead of
 * squinting at a canvas.
 *
 * No PixiJS here. The renderer consumes `LayoutRow` and draws rectangles; this
 * file only decides what the rectangles are.
 */

import { visibleRange, type Viewport } from './viewport';

/** Matches the CSS `--track-height` used by the DOM ruler above the canvas. */
export const TRACK_HEIGHT_PX = 56;

/** Clips shorter than this on screen are drawn as a marker, not a rectangle. */
const MIN_USABLE_WIDTH_PX = 2;

/** Vertical padding inside a track row. */
const TRACK_PADDING_PX = 4;

/**
 * Clip fill colours, as 0xRRGGBB for PixiJS.
 *
 * Keyed by `SourceClip['kind']` rather than by a lowercased string. The
 * lowercase spelling looked tidy and was silently wrong: `TrackKind` is
 * `Video`/`Audio`/... in Rust, so `TRACK_COLORS[kind]` returned `undefined` for
 * every clip and they were all drawn with no fill at all. Typing the record
 * against the union makes a missing or mis-cased key a compile error instead
 * of an invisible rectangle.
 */
export const TRACK_FILL: Record<SourceClip["kind"], number> = {
  Video: 0x2f6feb,
  Overlay: 0x8b5cf6,
  Audio: 0x10b981,
  Caption: 0xf59e0b,
};

/**
 * A clip with no resolvable media.
 *
 * Deliberately unlike any track colour: the export path refuses to render
 * these, so the timeline should not look like an ordinary clip that happens to
 * be short.
 */
export const UNRESOLVED_FILL = 0xef4444;

/** Exported for tests and for the legend; the values come from `TRACK_FILL`. */
export const TRACK_COLORS = { ...TRACK_FILL, unresolved: UNRESOLVED_FILL } as const;

export interface SourceClip {
  readonly id: string;
  readonly trackId: string;
  readonly kind: 'Video' | 'Overlay' | 'Audio' | 'Caption';
  readonly startTick: number;
  readonly endTick: number;
  /**
   * What the user calls this clip.
   *
   * Optional because a clip built from a bare id has nothing to show, and an
   * empty label is better than a placeholder: the header already falls back to
   * the track id, and a name-less clip is honest about being unsourced.
   */
  readonly name?: string;
  /** False when the clip has no media the host can resolve. */
  readonly resolved: boolean;
  readonly selected: boolean;
}

export interface LayoutClip {
  readonly id: string;
  readonly trackIndex: number;
  /** Pixel rectangle in canvas space. */
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  readonly color: number;
  readonly resolved: boolean;
  readonly selected: boolean;
  /** True when the clip is too narrow to show a label or edges honestly. */
  readonly tiny: boolean;
}

export interface LayoutRow {
  readonly trackId: string;
  readonly kind: SourceClip['kind'];
  readonly index: number;
  readonly clips: readonly LayoutClip[];
}

export interface Layout {
  readonly rows: readonly LayoutRow[];
  /** Total canvas height needed, so the container can size itself. */
  readonly heightPx: number;
  /**
   * Clips considered and clips drawn.
   *
   * These measure *work done*, not project size: tracks scrolled below the
   * canvas are never visited, so `totalClips` is not the clip count of the
   * project. Anything that needs the real count must walk the model.
   */
  readonly totalClips: number;
  readonly drawnClips: number;
}

/**
 * Build the layout for the visible part of the viewport.
 *
 * Clips outside the visible range are not considered at all, which is what
 * keeps a long project cheap: the cost is proportional to what is on screen,
 * not to the length of the timeline.
 */
export function layoutTimeline(
  clips: readonly SourceClip[],
  trackOrder: readonly string[],
  viewport: Viewport,
  options: { readonly canvasHeightPx?: number } = {},
): Layout {
  // A non-positive height means "not known yet", which is the normal state on
  // the first draw: the canvas has not been laid out, so its height is 0.
  //
  // Treating that as "nothing fits" is what makes a timeline render blank on
  // open and stay blank, because the element does not resize afterwards and
  // so nothing schedules the redraw that would fix it. A caller that has a
  // real height still gets the skipping; a caller that does not gets a
  // correct frame.
  const raw = options.canvasHeightPx ?? Number.POSITIVE_INFINITY;
  const canvasHeight = raw > 0 ? raw : Number.POSITIVE_INFINITY;
  const range = visibleRange(viewport);

  const rows: LayoutRow[] = [];
  let drawn = 0;
  let total = 0;

  for (let i = 0; i < trackOrder.length; i++) {
    const trackId = trackOrder[i];
    // `noUncheckedIndexedAccess` is on, so the element is `string | undefined`
    // even though the bound guarantees it exists. Skipping the impossible
    // case is clearer than an assertion that cannot be checked here.
    if (trackId === undefined) continue;
    const y = i * TRACK_HEIGHT_PX + TRACK_PADDING_PX;
    // A track scrolled below the canvas still occupies its slot, but drawing
    // its clips is pointless. Skipping early keeps the loop flat.
    if (y > canvasHeight) break;

    const rowClips: LayoutClip[] = [];
    for (const c of clips) {
      if (c.trackId !== trackId) continue;
      total++;

      // Intersect with the visible range so a partially scrolled clip is
      // drawn at the correct offset rather than clipped away entirely.
      if (c.endTick <= range.start || c.startTick >= range.end) continue;

      const x0 = (Math.max(c.startTick, range.start) - viewport.scrollTicks) * viewport.pxPerTick;
      const x1 = (Math.min(c.endTick, range.end) - viewport.scrollTicks) * viewport.pxPerTick;
      const width = Math.max(0, x1 - x0);

      const color = c.resolved ? TRACK_FILL[c.kind] : UNRESOLVED_FILL;

      rowClips.push({
        id: c.id,
        trackIndex: i,
        x: x0,
        y,
        width,
        height: TRACK_HEIGHT_PX - TRACK_PADDING_PX * 2,
        color,
        resolved: c.resolved,
        selected: c.selected,
        tiny: width < MIN_USABLE_WIDTH_PX,
      });
      drawn++;
    }

    // Rows are emitted even with no visible clips: the lane has to be there
    // for the user to drop something into.
    rows.push({
      trackId,
      kind: clips.find((c) => c.trackId === trackId)?.kind ?? 'Video',
      index: i,
      clips: rowClips,
    });
  }

  return {
    rows,
    heightPx: trackOrder.length * TRACK_HEIGHT_PX,
    totalClips: total,
    drawnClips: drawn,
  };
}

/**
 * Which clip is under a point, topmost first.
 *
 * Later tracks draw over earlier ones, so the hit test walks backwards. This
 * is the function that decides what a click selects, so it must agree with
 * what was actually drawn rather than with what was intended.
 */
export function hitTest(
  layout: Layout,
  x: number,
  y: number,
): { clipId: string; trackIndex: number } | null {
  for (let r = layout.rows.length - 1; r >= 0; r--) {
    const row = layout.rows[r];
    if (!row) continue;
    const top = r * TRACK_HEIGHT_PX + TRACK_PADDING_PX;
    const bottom = top + TRACK_HEIGHT_PX - TRACK_PADDING_PX * 2;
    if (y < top || y > bottom) continue;
    for (const c of row.clips) {
      if (x >= c.x && x <= c.x + c.width) {
        return { clipId: c.id, trackIndex: c.trackIndex };
      }
    }
  }
  return null;
}