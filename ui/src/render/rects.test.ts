/**
 * The layout invariants.
 *
 * The defect this file guards against is real and was observed in the running
 * app: the ruler is a DOM strip 24px tall, the Pixi canvas drew clips starting
 * at y=4, and the clips covered the ruler's own labels. Neither piece of code
 * was individually wrong -- the layout simply had no owner, so nothing
 * checked that the two agreed.
 *
 * The assertions below are the ones that would have caught it, stated as
 * geometry rather than as a convention someone has to remember.
 */

import { describe, expect, it } from 'vitest';
import {
  assertNoOverlap,
  clampScrollY,
  clipRectInLane,
  CLIP_HEIGHT_PX,
  HEADER_WIDTH_PX,
  RULER_HEIGHT_PX,
  timelineRects,
  TRACK_LANE_PX,
} from './rects';

const SECOND = 120_000;
const VIEWS = [
  { w: 1280, h: 736 },
  { w: 1024, h: 640 },
  { w: 1920, h: 1080 },
  { w: 1100, h: 700 },
] as const;

const tracks = (n: number): string[] => Array.from({ length: n }, (_, i) => `T${i}`);

function rects(opts: Partial<Parameters<typeof timelineRects>[0]> = {}) {
  return timelineRects({
    viewportWidth: 1280,
    viewportHeight: 736,
    trackIds: tracks(2),
    playheadTick: 0,
    scrollTicks: 0,
    pxPerTick: 1000 / (10 * SECOND),
    ...opts,
  });
}

describe('the ruler is never covered', () => {
  it('the first lane starts at or below the ruler, for every track count', () => {
    for (const n of [0, 1, 2, 8]) {
      for (const v of VIEWS) {
        const r = rects({ trackIds: tracks(n), viewportWidth: v.w, viewportHeight: v.h });
        const first = r.lanes[0];
        if (!first) continue;
        expect(
          first.y,
          `${n} tracks at ${v.w}x${v.h}: lane top ${first.y} must be >= ruler bottom ${r.ruler.height}`,
        ).toBeGreaterThanOrEqual(r.ruler.y + r.ruler.height);
        assertNoOverlap(r.ruler, first, `ruler vs first lane (${n} tracks, ${v.w}x${v.h})`);
      }
    }
  });

  it('no lane ever overlaps the ruler, even when scrolled', () => {
    for (let scrollY = 0; scrollY < 600; scrollY += 37) {
      const r = rects({ trackIds: tracks(8), scrollY });
      for (const lane of r.lanes) {
        assertNoOverlap(r.ruler, lane, `ruler vs lane ${lane.index} at scrollY=${scrollY}`);
      }
    }
  });

  it('the ruler is exactly RULER_HEIGHT_PX and spans the width', () => {
    for (const v of VIEWS) {
      const r = rects({ viewportWidth: v.w, viewportHeight: v.h });
      expect(r.ruler.height).toBe(RULER_HEIGHT_PX);
      expect(r.ruler.y).toBe(0);
      expect(r.ruler.width).toBe(v.w);
    }
  });
});

describe('the header column', () => {
  it('sits below the ruler and does not overlap the content', () => {
    const r = rects();
    assertNoOverlap(r.ruler, r.header, 'ruler vs header');
    assertNoOverlap(r.header, r.content, 'header vs content');
    expect(r.header.y).toBeGreaterThanOrEqual(r.ruler.height);
  });

  it('content starts exactly where the header ends', () => {
    const r = rects();
    expect(r.content.x).toBe(r.header.x + r.header.width);
    expect(r.header.width).toBe(HEADER_WIDTH_PX);
  });

  it('is clear of the ruler even at zero tracks', () => {
    const r = rects({ trackIds: [] });
    expect(r.lanes).toHaveLength(0);
    assertNoOverlap(r.ruler, r.header, 'ruler vs header with no tracks');
  });
});

describe('lanes', () => {
  it('are stacked with no vertical overlap', () => {
    for (const n of [1, 2, 8]) {
      const r = rects({ trackIds: tracks(n) });
      for (let i = 1; i < r.lanes.length; i++) {
        const prev = r.lanes[i - 1];
        const cur = r.lanes[i];
        if (!prev || !cur) continue;
        expect(cur.y, `lane ${i} must start at or below lane ${i - 1}`).toBeGreaterThanOrEqual(
          prev.y + prev.height,
        );
      }
    }
  });

  it('are all the same height and evenly spaced', () => {
    const r = rects({ trackIds: tracks(4) });
    for (const lane of r.lanes) expect(lane.height).toBe(TRACK_LANE_PX);
    if (r.lanes.length > 1) {
      const a = r.lanes[0];
      const b = r.lanes[1];
      if (a && b) expect(b.y - a.y).toBe(TRACK_LANE_PX);
    }
  });

  it('start at or below the ruler even with many tracks', () => {
    const r = rects({ trackIds: tracks(20) });
    for (const lane of r.lanes) {
      expect(lane.y).toBeGreaterThanOrEqual(RULER_HEIGHT_PX);
    }
  });

  it('report total height for the whole project, not just what fits', () => {
    const r = rects({ trackIds: tracks(20), viewportHeight: 400 });
    expect(r.totalHeight).toBe(20 * TRACK_LANE_PX);
    expect(r.lanes.length).toBeLessThan(20);
  });
});

describe('zero and degenerate viewports', () => {
  it('does not produce negative widths at zero size', () => {
    const r = rects({ viewportWidth: 0, viewportHeight: 0 });
    for (const rect of [r.ruler, r.header, r.content, ...r.lanes]) {
      expect(rect.width).toBeGreaterThanOrEqual(0);
      expect(rect.height).toBeGreaterThanOrEqual(0);
    }
  });

  it('does not throw on a negative viewport', () => {
    expect(() =>
      rects({ viewportWidth: -10, viewportHeight: -10 }),
    ).not.toThrow();
  });

  it('survives a single track narrower than the header', () => {
    const r = rects({ viewportWidth: 100, viewportHeight: 400 });
    expect(r.content.width).toBeGreaterThanOrEqual(0);
  });
});

describe('the playhead', () => {
  it('sits at the left edge of the content when at zero', () => {
    const r = rects({ playheadTick: 0 });
    expect(r.playheadX).toBe(HEADER_WIDTH_PX);
  });

  it('is null when scrolled off to the left', () => {
    // Drawing it anyway would render a line at negative x, which reads as a
    // clip edge rather than as the playhead.
    const r = rects({ playheadTick: 0, scrollTicks: 10 * SECOND });
    expect(r.playheadX).toBeNull();
  });

  it('is null when scrolled off to the right', () => {
    const r = rects({ playheadTick: 1000 * SECOND, scrollTicks: 0 });
    expect(r.playheadX).toBeNull();
  });

  it('moves right as the playhead advances', () => {
    const a = rects({ playheadTick: SECOND });
    const b = rects({ playheadTick: 2 * SECOND });
    expect(b.playheadX).toBeGreaterThan(a.playheadX ?? 0);
  });

  it('never sits inside the header column', () => {
    for (const t of [-10, 0, 1, 5, 100]) {
      const r = rects({ playheadTick: t * SECOND });
      if (r.playheadX !== null) {
        expect(r.playheadX).toBeGreaterThanOrEqual(HEADER_WIDTH_PX);
      }
    }
  });
});

describe('clip rectangles', () => {
  it('sit inside their lane, inset from its edges', () => {
    const r = rects({ trackIds: tracks(2) });
    const lane = r.lanes[0];
    expect(lane).toBeDefined();
    if (!lane) return;
    const clip = clipRectInLane(lane, 0, SECOND, 0, 1000 / (10 * SECOND));
    expect(clip.y).toBeGreaterThanOrEqual(lane.y);
    expect(clip.y + clip.height).toBeLessThanOrEqual(lane.y + lane.height);
    expect(clip.height).toBe(CLIP_HEIGHT_PX);
  });

  it('never draw left of the header column', () => {
    const r = rects({ trackIds: tracks(1) });
    const lane = r.lanes[0];
    if (!lane) return;
    // A clip starting well before the viewport still cannot be drawn over the
    // header: the renderer clips at lane.x.
    const clip = clipRectInLane(lane, -100 * SECOND, SECOND, 0, 1000 / (10 * SECOND));
    expect(clip.x).toBeGreaterThanOrEqual(lane.x);
  });

  it('never produce a negative width from an inverted range', () => {
    const r = rects({ trackIds: tracks(1) });
    const lane = r.lanes[0];
    if (!lane) return;
    const clip = clipRectInLane(lane, SECOND, 0, 0, 1000 / (10 * SECOND));
    expect(clip.width).toBeGreaterThanOrEqual(0);
  });
});

describe('vertical scrolling', () => {
  it('cannot push the first lane under the ruler', () => {
    // Negative scroll would move lanes up into the ruler.
    const scrolled = clampScrollY(-500, 20 * TRACK_LANE_PX, 400);
    expect(scrolled).toBe(0);
    const r = rects({ trackIds: tracks(20), scrollY: scrolled, viewportHeight: 400 });
    for (const lane of r.lanes) expect(lane.y).toBeGreaterThanOrEqual(RULER_HEIGHT_PX);
  });

  it('cannot scroll past the end of the last lane', () => {
    const max = clampScrollY(99999, 20 * TRACK_LANE_PX, 400);
    expect(max).toBe(20 * TRACK_LANE_PX - (400 - RULER_HEIGHT_PX));
  });

  it('is zero when everything already fits', () => {
    expect(clampScrollY(100, 2 * TRACK_LANE_PX, 800)).toBe(0);
  });
});

describe('assertNoOverlap', () => {
  it('passes for disjoint rects and throws for overlapping ones', () => {
    const a = { x: 0, y: 0, width: 10, height: 10 };
    const b = { x: 10, y: 0, width: 10, height: 10 };
    expect(() => assertNoOverlap(a, b, 'edge')).not.toThrow();
    expect(() => assertNoOverlap(a, { x: 5, y: 0, width: 10, height: 10 }, 'overlap')).toThrow(
      /overlap/,
    );
  });

  it('treats shared edges as disjoint', () => {
    // Ruler bottom == lane top is the normal case, not an overlap.
    const ruler = { x: 0, y: 0, width: 100, height: 28 };
    const lane = { x: 0, y: 28, width: 100, height: 56 };
    expect(() => assertNoOverlap(ruler, lane, 'adjacent')).not.toThrow();
  });
});