/**
 * Ruler ticks.
 *
 * The ruler has to stay readable from a 10-minute overview to a single frame,
 * on a strip that may be 300 px wide. Fixed tick spacing fails both ends: a
 * label every second is unreadable when zoomed out to an hour, and there are no
 * labels at all when zoomed into two seconds.
 *
 * So the spacing is chosen from a ladder of "nice" intervals, picking the
 * smallest whose pixel gap is still wide enough for its label. That is the
 * same approach every timeline uses, and the reason is uninteresting: it is the
 * only one where the result is always legible.
 */

/** Candidate intervals, in ticks, from sub-second to many minutes. */
export const TICK_LADDER: readonly number[] = [
  1, // one tick: 1/120000 s. Present so frame-level zoom still shows marks.
  120_000 / 60, // a frame at 60fps
  120_000 / 30, // a frame at 30fps
  120_000 / 10, // 100ms
  120_000 / 4, // 250ms
  120_000 / 2, // 500ms
  120_000, // 1s
  120_000 * 2,
  120_000 * 5,
  120_000 * 10,
  120_000 * 15,
  120_000 * 30,
  120_000 * 60, // 1min
  120_000 * 120,
  120_000 * 300, // 5min
  120_000 * 600, // 10min
  120_000 * 1800, // 30min
  120_000 * 3600, // 1h
];

/**
 * Minimum pixel gap between labelled ticks.
 *
 * 64px fits "00:01:30:00" at the UI font size without labels colliding. Below
 * that the labels overlap and the ruler becomes noise, which is worse than
 * showing fewer of them.
 */
export const MIN_LABEL_GAP_PX = 64;

/** Minimum pixel gap between *minor* ticks. */
export const MIN_TICK_GAP_PX = 8;

export interface RulerTick {
  readonly ticks: number;
  readonly major: boolean;
  readonly label: string | null;
}

export interface RulerOptions {
  /** pxPerTick, same units as the viewport. */
  readonly pxPerTick: number;
  readonly scrollTicks: number;
  readonly widthPx: number;
  /** Frame rate, for frame-level labels. */
  readonly frameRate: { num: number; den: number };
}

function ticksPerFrame(rate: { num: number; den: number }): number {
  return Math.floor((120_000 * rate.den) / rate.num);
}

/** Choose the smallest ladder interval that keeps labels apart. */
export function majorInterval(options: RulerOptions): number {
  for (const interval of TICK_LADDER) {
    if (interval * options.pxPerTick >= MIN_LABEL_GAP_PX) return interval;
  }
  // Beyond the ladder's largest entry, keep doubling rather than clamping.
  // A ruler that stops marking time entirely at high zoom is unusable.
  let last = TICK_LADDER[TICK_LADDER.length - 1]!;
  while (last * options.pxPerTick < MIN_LABEL_GAP_PX) last *= 2;
  return last;
}

/** Format a tick position for the ruler. */
export function formatTick(ticks: number, frameRate: { num: number; den: number }): string {
  const tpf = ticksPerFrame(frameRate);
  if (tpf === 0) return '0:00';

  const totalSeconds = ticks / 120_000;
  if (totalSeconds < 1) {
    const frames = Math.round(ticks / tpf);
    return `${frames}f`;
  }

  const s = Math.floor(totalSeconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const pad = (n: number) => String(n).padStart(2, '0');

  if (h > 0) return `${h}:${pad(m)}:${pad(sec)}`;
  // Include frames when they are distinguishable at this spacing.
  if (tpf * majorInterval({ pxPerTick: 0, scrollTicks: 0, widthPx: 0, frameRate }) < 120_000 / 2) {
    const frames = Math.round((ticks - s * 120_000) / tpf);
    return `${m}:${pad(sec)}:${pad(frames)}`;
  }
  return `${m}:${pad(sec)}`;
}

/**
 * Generate the ticks for a visible range.
 *
 * Ticks are generated from a multiple of the interval *before* the scroll
 * position, so scrolling never shifts them relative to absolute time. A tick
 * that moves when the user scrolls is the classic ruler bug: it makes the
 * playhead look like it is drifting.
 */
export function rulerTicks(options: RulerOptions, limit = 400): RulerTick[] {
  const { pxPerTick, scrollTicks, widthPx, frameRate } = options;
  if (pxPerTick <= 0 || widthPx <= 0) return [];

  const major = majorInterval(options);
  // Minor ticks are a fifth of the major interval where that fits.
  const minor = major / (major * pxPerTick >= MIN_LABEL_GAP_PX ? 1 : 5);

  const end = scrollTicks + widthPx / pxPerTick;
  const firstIndex = Math.floor(scrollTicks / minor);
  const count = Math.min(limit, Math.ceil((end - scrollTicks) / minor) + 1);

  const out: RulerTick[] = [];
  for (let i = 0; i <= count; i++) {
    const index = firstIndex + i;
    const ticks = index * minor;
    if (ticks < scrollTicks - minor) continue;
    if (ticks > end) break;
    const isMajor = Math.abs(ticks % major) < 1e-6 || Math.abs((ticks % major) - major) < 1e-6;
    out.push({
      ticks,
      major: isMajor,
      label: isMajor ? formatTick(ticks, frameRate) : null,
    });
  }
  return out;
}

/** Choose a default zoom so the whole project fits, with a little margin. */
export function fitZoom(durationTicks: number, widthPx: number): number {
  if (durationTicks <= 0 || widthPx <= 0) return 0.002;
  return (widthPx * 0.95) / durationTicks;
}