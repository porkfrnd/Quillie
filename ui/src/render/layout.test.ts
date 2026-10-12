/**
 * Timeline layout: what actually gets drawn.
 *
 * The properties asserted here are the ones that are invisible in a
 * screenshot. A clip one pixel too narrow, a track drawn at zero height, or a
 * hit test that disagrees with what is on screen all look fine until the user
 * tries to click something.
 */

import { describe, expect, it } from 'vitest';
import {
  hitTest,
  layoutTimeline,
  TRACK_COLORS,
  TRACK_HEIGHT_PX,
  type SourceClip,
} from './layout';
import type { Viewport } from './viewport';

const SECOND = 120_000;

const viewport = (over: Partial<Viewport> = {}): Viewport => ({
  scrollTicks: 0,
  widthPx: 1000,
  pxPerTick: 1000 / (10 * SECOND), // 10 seconds across 1000px
  ...over,
});

function clip(over: Partial<SourceClip> = {}): SourceClip {
  return {
    id: 'c1',
    trackId: 'V1',
    kind: 'Video',
    startTick: 0,
    endTick: SECOND,
    resolved: true,
    selected: false,
    ...over,
  };
}

describe('layoutTimeline', () => {
  it('places a clip at the right x and width', () => {
    const vp = viewport();
    const l = layoutTimeline([clip()], ['V1'], vp);

    const c = l.rows[0]?.clips[0];
    expect(c?.x).toBe(0);
    // One second of a ten-second viewport across 1000px is 100px.
    expect(c?.width).toBeCloseTo(100, 6);
    expect(c?.height).toBeGreaterThan(0);
  });

  it('offsets a clip that starts later', () => {
    const vp = viewport();
    const l = layoutTimeline([clip({ startTick: SECOND * 2 })], ['V1'], vp);
    const c = l.rows[0]?.clips[0];
    expect(c?.x).toBeCloseTo(200, 6);
  });

  it('clips a partially scrolled clip to the visible range', () => {
    // Scrolled to 2s: the clip spans 0..1s and is entirely off the left edge.
    const vp = viewport({ scrollTicks: SECOND * 2 });
    const l = layoutTimeline([clip()], ['V1'], vp);
    expect(l.drawnClips).toBe(0);

    // Half on screen: the visible part is 2.0..2.5s, so 0.5s of a 10s viewport
    // across 1000px = 50px, starting at the left edge.
    const half = layoutTimeline(
      [clip({ startTick: SECOND * 1.5, endTick: SECOND * 2.5 })],
      ['V1'],
      vp,
    );
    const c = half.rows[0]?.clips[0];
    expect(c).toBeDefined();
    expect(c?.x).toBeCloseTo(0, 6);
    expect(c?.width).toBeCloseTo(50, 6);
  });

  it('never emits a negative width even for a degenerate clip', () => {
    // An inverted range must not produce a rectangle drawn backwards.
    const l = layoutTimeline(
      [clip({ startTick: SECOND * 5, endTick: SECOND })],
      ['V1'],
      viewport(),
    );
    const c = l.rows[0]?.clips[0];
    expect(c).toBeDefined();
    expect(c?.width).toBeGreaterThanOrEqual(0);
  });

  it('gives every track a row even when it has no clips', () => {
    const l = layoutTimeline([], ['V1', 'A1'], viewport());
    expect(l.rows).toHaveLength(2);
    expect(l.rows[0]?.clips).toHaveLength(0);
    expect(l.heightPx).toBe(TRACK_HEIGHT_PX * 2);
  });

  it('stacks tracks vertically without overlap', () => {
    const l = layoutTimeline(
      [clip({ id: 'v', trackId: 'V1' }), clip({ id: 'a', trackId: 'A1', kind: 'Audio' })],
      ['V1', 'A1'],
      viewport(),
    );
    const v = l.rows[0]?.clips[0];
    const a = l.rows[1]?.clips[0];
    expect(v?.y).toBeLessThan(a?.y ?? 0);
    expect((a?.y ?? 0) - (v?.y ?? 0)).toBe(TRACK_HEIGHT_PX);
  });

  it('only considers clips on visible tracks', () => {
    // A ten-track project in a 200px window: the lower tracks must not be
    // walked at all, which is the point of breaking early.
    const tracks = Array.from({ length: 10 }, (_, i) => `T${i}`);
    const clips = tracks.map((t) => clip({ id: t, trackId: t }));
    const l = layoutTimeline(clips, tracks, viewport(), { canvasHeightPx: 200 });

    // Four tracks of 56px fit in 200px; the fifth starts below the canvas.
    expect(l.rows).toHaveLength(4);
    expect(l.heightPx).toBe(10 * TRACK_HEIGHT_PX);
    // `totalClips` counts what was *considered*, which is the point: it is the
    // work done, not the size of the project. A caller using it to estimate
    // project size would be misled, so it is named for what it measures.
    expect(l.totalClips).toBe(4);
    expect(l.drawnClips).toBe(4);
  });

  it('colours by track kind and by resolvability', () => {
    const l = layoutTimeline(
      [
        clip({ id: 'v', trackId: 'V1', kind: 'Video' }),
        clip({ id: 'a', trackId: 'A1', kind: 'Audio', startTick: SECOND }),
        clip({ id: 'u', trackId: 'V2', kind: 'Video', resolved: false, startTick: SECOND * 2 }),
      ],
      ['V1', 'A1', 'V2'],
      viewport(),
    );
    // Keys are PascalCase to match Rust's `TrackKind`. The lowercased version
    // of this table returned undefined for every clip.
    expect(l.rows[0]?.clips[0]?.color).toBe(TRACK_COLORS.Video);
    expect(l.rows[1]?.clips[0]?.color).toBe(TRACK_COLORS.Audio);
    // An unresolved clip must not look like a normal one: the export path
    // refuses to render it, so the timeline says so.
    expect(l.rows[2]?.clips[0]?.color).toBe(TRACK_COLORS.unresolved);
    expect(l.rows[2]?.clips[0]?.resolved).toBe(false);
  });

  it('gives every clip a defined colour', () => {
    // Regression guard for the bug above: `TRACK_COLORS[kind]` with
    // lowercased keys yielded undefined, which JSON.stringify drops and
    // PixiJS reads as "no fill". The clip rendered, just not visibly.
    for (const kind of ['Video', 'Overlay', 'Audio', 'Caption'] as const) {
      const l = layoutTimeline([clip({ kind })], ['V1'], viewport());
      const color = l.rows[0]?.clips[0]?.color;
      expect(color, `${kind} has no colour`).toBeDefined();
      expect(typeof color).toBe('number');
      expect(color).not.toBe(0);
    }
  });

  it('flags a clip too narrow to draw honestly', () => {
    // Below the 2px threshold: a rectangle here would suggest a duration the
    // clip does not have, so it gets drawn as a marker instead.
    const l = layoutTimeline(
      [clip({ endTick: SECOND / 120 })],
      ['V1'],
      viewport(),
    );
    expect(l.rows[0]?.clips[0]?.tiny).toBe(true);

    const wide = layoutTimeline([clip()], ['V1'], viewport());
    expect(wide.rows[0]?.clips[0]?.tiny).toBe(false);
  });

  it('keeps a stable order so redraws do not shimmer', () => {
    const clips = [
      clip({ id: 'b', startTick: SECOND * 2 }),
      clip({ id: 'a', startTick: 0 }),
      clip({ id: 'c', startTick: SECOND * 4 }),
    ];
    const first = layoutTimeline(clips, ['V1'], viewport()).rows[0]?.clips.map((c) => c.id);
    const second = layoutTimeline(clips, ['V1'], viewport()).rows[0]?.clips.map((c) => c.id);
    expect(first).toEqual(second);
    expect(first).toEqual(['b', 'a', 'c']);
  });
});

describe('hitTest', () => {
  it('finds the clip under a point', () => {
    const l = layoutTimeline([clip()], ['V1'], viewport());
    const hit = hitTest(l, 50, 10);
    expect(hit?.clipId).toBe('c1');
  });

  it('returns nothing in empty space', () => {
    const l = layoutTimeline([clip()], ['V1'], viewport());
    expect(hitTest(l, 500, 10)).toBeNull();
    expect(hitTest(l, 50, TRACK_HEIGHT_PX + 100)).toBeNull();
  });

  it('picks the topmost track when clips overlap visually', () => {
    // Two clips on different tracks at the same time; the later track draws
    // over the earlier one, so it must win the click.
    const l = layoutTimeline(
      [clip({ id: 'lower', trackId: 'V1' }), clip({ id: 'upper', trackId: 'V2' })],
      ['V1', 'V2'],
      viewport(),
    );
    expect(hitTest(l, 50, TRACK_HEIGHT_PX + 10)?.clipId).toBe('upper');
    expect(hitTest(l, 50, 10)?.clipId).toBe('lower');
  });

  it('agrees with the rectangle it was drawn from', () => {
    // The hit test must never report a hit on a clip that was not drawn.
    const clips = [clip({ id: 'x', startTick: 0, endTick: SECOND })];
    const l = layoutTimeline(clips, ['V1'], viewport());
    const drawn = l.rows[0]?.clips[0];
    expect(drawn).toBeDefined();
    if (!drawn) return;
    // Inside the drawn rectangle.
    expect(hitTest(l, drawn.x + drawn.width / 2, drawn.y + 1)?.clipId).toBe('x');
    // Just outside it.
    expect(hitTest(l, drawn.x + drawn.width + 1, drawn.y + 1)).toBeNull();
  });

  it('finds nothing in an empty project', () => {
    const l = layoutTimeline([], ['V1'], viewport());
    expect(hitTest(l, 10, 10)).toBeNull();
  });
});