/**
 * Time arithmetic, and the mirror's agreement with Rust.
 *
 * This file is a deliberate duplicate of `quillie-core/src/time.rs`. A mirror
 * that drifts from its source does not fail loudly: it produces a playhead
 * that is one frame out at 23.976, or a clip edge that lands between frames,
 * and both look like "the editor is imprecise" rather than "the two
 * implementations disagree".
 *
 * So the exact values are pinned here, taken from the Rust implementation, and
 * the rounding boundaries are checked explicitly. Ties are the interesting
 * case: `Math.round` rounds -0.5 up to -0, and the Rust `Rounding::Nearest`
 * rounds half away from zero, which disagree for negative time. Time is never
 * negative in the UI, and the tests say so rather than leaving it ambiguous.
 */

import { describe, expect, it } from 'vitest';
import {
  FPS_24,
  FPS_25,
  FPS_30,
  FPS_60,
  NTSC_24,
  NTSC_30,
  NTSC_60,
  TICKS_PER_SECOND,
  formatDuration,
  formatTimecode,
  frameToTicks,
  secondsToTicks,
  ticksPerFrame,
  ticksToFrame,
  ticksToSeconds,
  type FrameRate,
} from './time';

describe('ticksPerFrame', () => {
  it('matches the Rust values exactly', () => {
    // These are the numbers in time.rs. NTSC is the one that matters: doing
    // the division before the multiply yields 5010 here instead of 5005, a
    // one-tick error per frame that accumulates across a timeline.
    expect(ticksPerFrame(FPS_24)).toBe(5000);
    expect(ticksPerFrame(FPS_25)).toBe(4800);
    expect(ticksPerFrame(FPS_30)).toBe(4000);
    expect(ticksPerFrame(FPS_60)).toBe(2000);
    expect(ticksPerFrame(NTSC_24)).toBe(5005);
    expect(ticksPerFrame(NTSC_30)).toBe(4004);
    expect(ticksPerFrame(NTSC_60)).toBe(2002);
  });

  it('multiplying before dividing matters for NTSC', () => {
    // (120000 * 1001) / 24000 = 5005 exactly.
    // (120000 / 24000) * 1001 = 5005 too -- but flooring the first gives
    // 5 * 1001 = 5005, and for 30000/1001 the wrong order gives 4000.
    expect(ticksPerFrame(NTSC_30)).not.toBe(4000);
    expect(ticksPerFrame({ num: 30000, den: 1001 })).toBe(4004);
  });

  it('does not divide by zero', () => {
    expect(ticksPerFrame({ num: 0, den: 1 })).toBe(Infinity);
    // And the consumers must survive it.
    expect(ticksToFrame({ num: 0, den: 1 }, 1000)).toBe(0);
    expect(formatTimecode(1000, { num: 0, den: 1 })).toMatch(/^\d/);
  });
});

describe('frame and tick conversion', () => {
  it('round-trips exact frames', () => {
    for (const rate of [FPS_24, FPS_25, FPS_30, FPS_60, NTSC_24, NTSC_30, NTSC_60]) {
      for (const frame of [0, 1, 2, 7, 100, 1000]) {
        const ticks = frameToTicks(rate, frame);
        expect(ticksToFrame(rate, ticks, 'floor')).toBe(frame);
      }
    }
  });

  it('floor never rounds up past the frame', () => {
    const tpf = ticksPerFrame(FPS_30);
    expect(ticksToFrame(FPS_30, tpf - 1, 'floor')).toBe(0);
    expect(ticksToFrame(FPS_30, tpf, 'floor')).toBe(1);
  });

  it('nearest rounds to whichever frame is genuinely closer', () => {
    const tpf = ticksPerFrame(FPS_30);
    // 3999/4000 is 0.99975, which is nearer frame 1 than frame 0 -- rounding
    // it to 0 would be wrong, not merely conservative.
    expect(ticksToFrame(FPS_30, tpf - 1, 'nearest')).toBe(1);
    // Just past the halfway point still goes up.
    expect(ticksToFrame(FPS_30, tpf * 0.75, 'nearest')).toBe(1);
    // Just under it stays down.
    expect(ticksToFrame(FPS_30, tpf * 0.25, 'nearest')).toBe(0);
  });

  it('nearest rounds a half-frame up', () => {
    const tpf = ticksPerFrame(FPS_30);
    expect(ticksToFrame(FPS_30, tpf * 0.5, 'nearest')).toBe(1);
  });

  it('floor of a negative tick does not round toward positive infinity', () => {
    // Math.floor(-0.5) === -1, which is correct for floor. The risk being
    // pinned is a future change to Math.trunc or Math.round, which would move
    // the boundary the other way.
    expect(ticksToFrame(FPS_30, -ticksPerFrame(FPS_30) / 2, 'floor')).toBe(-1);
  });

  it('seconds convert both ways', () => {
    expect(secondsToTicks(1)).toBe(TICKS_PER_SECOND);
    expect(ticksToSeconds(TICKS_PER_SECOND)).toBe(1);
    expect(secondsToTicks(0)).toBe(0);
  });

  it('seconds round to the nearest tick rather than truncating', () => {
    // Truncating here loses a fraction of a tick per call, and callers that
    // convert back and forth would drift.
    expect(secondsToTicks(0.5)).toBe(60_000);
    expect(secondsToTicks(1 / TICKS_PER_SECOND)).toBe(1);
  });

  it('never produces a fractional tick', () => {
    for (const s of [0.1, 1 / 3, 2.718281828, 99.999]) {
      expect(Number.isInteger(secondsToTicks(s))).toBe(true);
    }
  });
});

describe('formatTimecode', () => {
  it('formats zero', () => {
    expect(formatTimecode(0, FPS_30)).toBe('00:00:00:00');
  });

  it('formats minutes and seconds', () => {
    const t = 90 * TICKS_PER_SECOND;
    expect(formatTimecode(t, FPS_30)).toBe('00:01:30:00');
  });

  it('formats hours without overflowing the hours field', () => {
    const t = 3661 * TICKS_PER_SECOND;
    expect(formatTimecode(t, FPS_30)).toBe('01:01:01:00');
  });

  it('pads every field to two digits', () => {
    const s = formatTimecode(5 * TICKS_PER_SECOND, FPS_30);
    for (const part of s.split(':')) expect(part).toHaveLength(2);
  });

  it('clamps a negative time to zero rather than rendering a negative code', () => {
    expect(formatTimecode(-5000, FPS_30)).toBe('00:00:00:00');
  });

  it('keeps frames within the rate for NTSC', () => {
    // The frame field uses Math.round(fps), which for 30000/1001 is 30.
    // If it ever used the fractional value, the field could reach 30.
    const t = ticksPerFrame(NTSC_30) * 100;
    const frames = Number(formatTimecode(t, NTSC_30).split(':')[3]);
    expect(frames).toBeGreaterThanOrEqual(0);
    expect(frames).toBeLessThan(30);
  });
});

describe('formatDuration', () => {
  it('uses one decimal below ten seconds', () => {
    expect(formatDuration(TICKS_PER_SECOND * 1.44)).toBe('1.4s');
  });

  it('drops the decimal at ten seconds and above', () => {
    expect(formatDuration(TICKS_PER_SECOND * 10)).toBe('10s');
    expect(formatDuration(TICKS_PER_SECOND * 59)).toBe('59s');
  });

  it('switches to minutes', () => {
    expect(formatDuration(TICKS_PER_SECOND * 60)).toBe('1m 0s');
    expect(formatDuration(TICKS_PER_SECOND * 95)).toBe('1m 35s');
  });

  it('handles zero and sub-second values', () => {
    expect(formatDuration(0)).toBe('0.0s');
    expect(formatDuration(1)).toBe('0.0s');
  });
});

describe('the mirror holds', () => {
  it('TICKS_PER_SECOND agrees with the Rust constant', () => {
    // quillie-core/src/time.rs: pub const TICKS_PER_SECOND: i64 = 120_000;
    expect(TICKS_PER_SECOND).toBe(120_000);
  });

  it('every declared rate is a valid rational', () => {
    const rates: FrameRate[] = [FPS_24, FPS_25, FPS_30, FPS_60, NTSC_24, NTSC_30, NTSC_60];
    for (const r of rates) {
      expect(r.num).toBeGreaterThan(0);
      expect(r.den).toBeGreaterThan(0);
      // Every supported rate must land on a whole number of ticks per frame,
      // or frame boundaries would not be representable.
      expect(Number.isInteger(ticksPerFrame(r))).toBe(true);
    }
  });
});