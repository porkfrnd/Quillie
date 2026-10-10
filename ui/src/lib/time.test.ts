/**
 * Tests for the TypeScript time mirror.
 *
 * These exist so the UI's idea of time can never silently drift from the Rust
 * core's. The numbers below are the same ones asserted in
 * `crates/quillie-core/src/time.rs`; if one side changes, both suites must be
 * updated together or one of them fails.
 */

import { describe, it, expect } from 'vitest';
import {
  TICKS_PER_SECOND,
  FPS_24,
  FPS_25,
  FPS_30,
  FPS_60,
  NTSC_24,
  NTSC_30,
  NTSC_60,
  ticksPerFrame,
  frameToTicks,
  ticksToFrame,
  formatTimecode,
  type FrameRate,
} from './time';

describe('ticks per frame', () => {
  it('matches the Rust core for every supported rate', () => {
    // These are the values in crates/quillie-core/src/time.rs.
    expect(ticksPerFrame(NTSC_24)).toBe(5005);
    expect(ticksPerFrame(FPS_24)).toBe(5000);
    expect(ticksPerFrame(FPS_25)).toBe(4800);
    expect(ticksPerFrame(NTSC_30)).toBe(4004);
    expect(ticksPerFrame(FPS_30)).toBe(4000);
    expect(ticksPerFrame(FPS_60)).toBe(2000);
    expect(ticksPerFrame(NTSC_60)).toBe(2002);
  });

  it('divides the timebase with no remainder', () => {
    const rates: FrameRate[] = [NTSC_24, FPS_24, FPS_25, NTSC_30, FPS_30, FPS_60, NTSC_60];
    for (const r of rates) {
      const tpf = ticksPerFrame(r);
      expect(tpf * r.num).toBe(TICKS_PER_SECOND * r.den);
    }
  });
});

describe('frame and tick conversion', () => {
  it('round-trips exactly', () => {
    for (const r of [NTSC_24, NTSC_30, NTSC_60, FPS_24, FPS_30, FPS_60]) {
      for (const frame of [0, 1, 2, 7, 100, 1001, 10_000]) {
        expect(ticksToFrame(r, frameToTicks(r, frame))).toBe(frame);
      }
    }
  });

  it('accumulates no error over a long timeline', () => {
    const r = NTSC_30;
    const tpf = ticksPerFrame(r);
    let stepped = 0;
    for (let i = 0; i < 100_000; i++) stepped += tpf;
    expect(stepped).toBe(frameToTicks(r, 100_000));
  });
});

describe('timecode formatting', () => {
  it('renders hours, minutes, seconds and frames', () => {
    // One hour at 30fps exactly: 108000 frames.
    expect(formatTimecode(frameToTicks(FPS_30, 108_000), FPS_30)).toBe('01:00:00:00');
  });

  it('pads single digits', () => {
    // 65 seconds and 5 frames at 30fps.
    expect(formatTimecode(frameToTicks(FPS_30, 65 * 30 + 5), FPS_30)).toBe('00:01:05:05');
  });
});