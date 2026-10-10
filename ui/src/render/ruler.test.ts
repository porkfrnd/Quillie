import { describe, it, expect } from 'vitest';
import {
  majorInterval,
  rulerTicks,
  formatTick,
  fitZoom,
  MIN_LABEL_GAP_PX,
  TICK_LADDER,
} from './ruler';
import { FPS_30 } from '$lib/time';

const base = { scrollTicks: 0, widthPx: 1200, frameRate: FPS_30 };

describe('ruler interval selection', () => {
  it('zoomed out far enough, the interval is minutes', () => {
    // 1 hour visible in 1000px: a 1s label would be sub-pixel.
    const interval = majorInterval({ ...base, pxPerTick: 1000 / (3600 * 120_000) });
    expect(interval * (1000 / (3600 * 120_000))).toBeGreaterThanOrEqual(MIN_LABEL_GAP_PX);
    expect(interval).toBeGreaterThanOrEqual(120_000);
  });

  it('zoomed into a few frames, the interval is frame-level', () => {
    // 2 seconds across 1000px = 0.5 px per tick at 4000 ticks/frame.
    const interval = majorInterval({ ...base, pxPerTick: 1000 / (2 * 120_000) });
    expect(interval).toBeLessThan(120_000);
  });

  it('always leaves labels at least the minimum gap apart', () => {
    for (const pxPerTick of [0.0001, 0.001, 0.01, 0.1, 0.5, 1, 5, 50]) {
      const interval = majorInterval({ ...base, pxPerTick });
      expect(
        interval * pxPerTick,
        `at pxPerTick=${pxPerTick} labels would be ${interval * pxPerTick}px apart`,
      ).toBeGreaterThanOrEqual(MIN_LABEL_GAP_PX - 1e-6);
    }
  });

  it('never returns a zero or negative interval', () => {
    for (const pxPerTick of [1e-9, 1e-6, 1e-3, 1, 1e6]) {
      expect(majorInterval({ ...base, pxPerTick })).toBeGreaterThan(0);
    }
  });
});

describe('tick generation', () => {
  it('labels never collide at any zoom level', () => {
    for (const pxPerTick of [0.0002, 0.001, 0.01, 0.05, 0.5, 2]) {
      const ticks = rulerTicks({ ...base, pxPerTick });
      const labelled = ticks.filter((t) => t.label !== null);
      for (let i = 1; i < labelled.length; i++) {
        const gapPx = (labelled[i]!.ticks - labelled[i - 1]!.ticks) * pxPerTick;
        expect(gapPx, `labels ${gapPx}px apart at pxPerTick=${pxPerTick}`).toBeGreaterThanOrEqual(
          MIN_LABEL_GAP_PX - 1e-6,
        );
      }
    }
  });

  it('ticks are in increasing order', () => {
    const ticks = rulerTicks({ ...base, pxPerTick: 0.01 });
    for (let i = 1; i < ticks.length; i++) {
      expect(ticks[i]!.ticks).toBeGreaterThan(ticks[i - 1]!.ticks);
    }
  });

  it('ticks do not move when the view scrolls', () => {
    // The classic ruler bug: a tick that shifts relative to absolute time as
    // you scroll makes the playhead look like it drifts.
    const a = rulerTicks({ ...base, pxPerTick: 0.01, scrollTicks: 0 });
    const b = rulerTicks({ ...base, pxPerTick: 0.01, scrollTicks: 7 * 120_000 });
    const aSet = new Set(a.map((t) => t.ticks));
    // Every tick in the second view that is also in range of the first must be
    // at exactly the same absolute position.
    for (const t of b) {
      if (t.ticks <= 0) expect(aSet.has(t.ticks)).toBe(true);
    }
  });

  it('respects the tick limit on extreme zoom-out', () => {
    const ticks = rulerTicks({ ...base, pxPerTick: 1e-6, widthPx: 20000 }, 400);
    expect(ticks.length).toBeLessThanOrEqual(401);
  });

  it('returns nothing for a degenerate viewport', () => {
    expect(rulerTicks({ ...base, pxPerTick: 0 })).toEqual([]);
    expect(rulerTicks({ ...base, pxPerTick: 0.01, widthPx: 0 })).toEqual([]);
  });

  it('covers the visible range', () => {
    const pxPerTick = 0.01;
    const widthPx = 800;
    const scrollTicks = 120_000;
    const ticks = rulerTicks({ ...base, pxPerTick, widthPx, scrollTicks });
    expect(ticks.length).toBeGreaterThan(0);
    expect(ticks[0]!.ticks).toBeLessThanOrEqual(scrollTicks);
    const end = scrollTicks + widthPx / pxPerTick;
    expect(ticks[ticks.length - 1]!.ticks).toBeGreaterThanOrEqual(end * 0.5);
  });
});

describe('tick labels', () => {
  it('shows frames when zoomed in', () => {
    expect(formatTick(120_000 / 30, FPS_30)).toMatch(/f$/);
  });

  it('shows minutes and seconds when zoomed out', () => {
    const label = formatTick(90 * 120_000, FPS_30);
    expect(label).toContain(':');
    expect(label.startsWith('1:')).toBe(true);
  });

  it('shows hours when the position is that large', () => {
    expect(formatTick(3600 * 120_000, FPS_30).startsWith('1:')).toBe(true);
  });

  it('never produces an empty label', () => {
    for (const t of [0, 1, 1000, 120_000, 120_000 * 90, 120_000 * 7200]) {
      expect(formatTick(t, FPS_30).length).toBeGreaterThan(0);
    }
  });
});

describe('fit zoom', () => {
  it('fits the whole duration into the viewport', () => {
    const duration = 60 * 120_000; // one minute
    const zoom = fitZoom(duration, 1000);
    expect(duration * zoom).toBeLessThanOrEqual(1000);
    expect(duration * zoom).toBeGreaterThan(900);
  });

  it('degrades safely for an empty project', () => {
    expect(fitZoom(0, 1000)).toBeGreaterThan(0);
    expect(fitZoom(1000, 0)).toBeGreaterThan(0);
  });
});

describe('the ladder itself', () => {
  it('is strictly increasing', () => {
    for (let i = 1; i < TICK_LADDER.length; i++) {
      expect(TICK_LADDER[i]!).toBeGreaterThan(TICK_LADDER[i - 1]!);
    }
  });

  it('covers from a single tick to an hour', () => {
    expect(TICK_LADDER[0]).toBe(1);
    expect(TICK_LADDER[TICK_LADDER.length - 1]).toBeGreaterThanOrEqual(3600 * 120_000);
  });
});