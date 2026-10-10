import { describe, it, expect } from 'vitest';
import {
  ticksToX,
  xToTicks,
  snapToFrame,
  snap,
  visibleRange,
  visibleClips,
  zoomAt,
  MIN_PX_PER_TICK,
  MAX_PX_PER_TICK,
  type Viewport,
} from './viewport';
import { FPS_30, NTSC_30 } from '../lib/time';

const vp: Viewport = { scrollTicks: 0, widthPx: 1000, pxPerTick: 0.001 };

describe('coordinate conversion', () => {
  it('round-trips pixels to ticks', () => {
    for (const x of [0, 1, 250, 999.5]) {
      const ticks = xToTicks(vp, x);
      expect(ticksToX(vp, ticks)).toBeCloseTo(x, 6);
    }
  });

  it('puts tick zero at the viewport origin', () => {
    expect(ticksToX(vp, 0)).toBe(0);
  });
});

describe('snapping', () => {
  it('snaps to the nearest frame at 30fps', () => {
    expect(snapToFrame(0, FPS_30)).toBe(0);
    expect(snapToFrame(1999, FPS_30)).toBe(0);
    // Exactly halfway between frame 0 and frame 1: ties round up.
    expect(snapToFrame(2000, FPS_30)).toBe(4000);
    expect(snapToFrame(2001, FPS_30)).toBe(4000);
    // 1.5 frames exactly: ties round up again.
    expect(snapToFrame(6000, FPS_30)).toBe(8000);
    expect(snapToFrame(5999, FPS_30)).toBe(4000);
  });

  it('uses the NTSC tick count, not the decimal one', () => {
    // NTSC-30 is 4004 ticks per frame, not 4000. 4005 is just past one frame,
    // so it stays on frame 1 -- snapping to 8008 would require crossing the
    // halfway point of frame 2.
    expect(snapToFrame(4004, NTSC_30)).toBe(4004);
    expect(snapToFrame(4005, NTSC_30)).toBe(4004);
    expect(snapToFrame(6006, NTSC_30)).toBe(8008);
  });

  it('prefers a candidate within tolerance over the frame grid', () => {
    const r = snap(3990, FPS_30, [3990], 100);
    expect(r.snapped).toBe(true);
    expect(r.kind).toBe('candidate');
    expect(r.ticks).toBe(3990);
  });

  it('falls back to the frame grid when no candidate is close', () => {
    const r = snap(3990, FPS_30, [999_999], 100);
    expect(r.kind).toBe('frame');
    expect(r.ticks).toBe(4000);
  });
});

describe('virtualisation', () => {
  it('returns only clips intersecting the visible range', () => {
    const clips = [
      { start: 0, end: 100 },
      { start: 500, end: 600 },
      { start: 5_000_000, end: 5_000_100 },
    ];
    const bounds = (c: { start: number; end: number }) => c;
    const range = visibleRange(vp);
    const out = visibleClips(clips, bounds, range);
    expect(out).toHaveLength(2);
    expect(out.map((c) => c.start)).toEqual([0, 500]);
  });

  it('reuses the output array rather than allocating per frame', () => {
    const clips = [{ start: 0, end: 100 }];
    const out: { start: number; end: number }[] = [];
    const first = visibleClips(clips, (c) => c, visibleRange(vp), out);
    const second = visibleClips(clips, (c) => c, visibleRange(vp), out);
    // Same backing store: allocation in a per-frame path is what this avoids.
    expect(first).toBe(second);
  });
});

describe('zoom', () => {
  it('keeps the point under the cursor fixed', () => {
    const anchor = 400;
    const before = xToTicks(vp, anchor);
    const zoomed = zoomAt(vp, 2, anchor);
    expect(xToTicks(zoomed, anchor)).toBeCloseTo(before, 3);
  });

  it('clamps to the supported range in both directions', () => {
    expect(zoomAt(vp, 1e9, 500).pxPerTick).toBe(MAX_PX_PER_TICK);
    expect(zoomAt(vp, 1e-9, 500).pxPerTick).toBe(MIN_PX_PER_TICK);
  });
});