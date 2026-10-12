/**
 * Time helpers mirroring `quillie-core/src/time.rs`.
 *
 * This is a deliberate mirror, not a reimplementation with its own rules. The
 * authoritative definition is in Rust; these constants and conversions exist so
 * the UI can lay out a timeline without an IPC round trip per frame. They are
 * checked against the Rust values by the golden motion vectors (§10) and by
 * `time.test.ts`.
 */

export const TICKS_PER_SECOND = 120_000;

/** A frame rate as an exact rational. Never a float. */
export interface FrameRate {
  readonly num: number;
  readonly den: number;
}

export const FPS_24: FrameRate = { num: 24, den: 1 };
export const FPS_25: FrameRate = { num: 25, den: 1 };
export const FPS_30: FrameRate = { num: 30, den: 1 };
export const FPS_60: FrameRate = { num: 60, den: 1 };
export const NTSC_24: FrameRate = { num: 24000, den: 1001 };
export const NTSC_30: FrameRate = { num: 30000, den: 1001 };
export const NTSC_60: FrameRate = { num: 60000, den: 1001 };

export type Rounding = 'floor' | 'nearest';

/**
 * Ticks per frame: TICKS_PER_SECOND * den / num.
 *
 * The multiply happens before the divide, exactly as in Rust. Doing the divide
 * first truncates and yields 5010 instead of 5005 for NTSC-23.976, which is
 * a bug that survives review because the result is plausible.
 */
export function ticksPerFrame(rate: FrameRate): number {
  return Math.floor((TICKS_PER_SECOND * rate.den) / rate.num);
}

/** Frame index to ticks. Exact on the supported timebase. */
export function frameToTicks(rate: FrameRate, frame: number): number {
  return ticksPerFrame(rate) * frame;
}

/** Time to frame index. */
export function ticksToFrame(rate: FrameRate, ticks: number, rounding: Rounding = 'floor'): number {
  const tpf = ticksPerFrame(rate);
  if (tpf <= 0) return 0;
  const raw = ticks / tpf;
  return rounding === 'floor' ? Math.floor(raw) : Math.round(raw);
}

/** Ticks to seconds, for display only. Never feed this back into a time. */
export function ticksToSeconds(ticks: number): number {
  return ticks / TICKS_PER_SECOND;
}

export function secondsToTicks(seconds: number): number {
  return Math.round(seconds * TICKS_PER_SECOND);
}

/** `HH:MM:SS:FF` or `MM:SS.mmm`, whichever the duration calls for. */
export function formatTimecode(ticks: number, rate: FrameRate): string {
  const tpf = ticksPerFrame(rate);
  // A frame rate the model should have rejected can still reach the display
  // from a malformed file. Without this the timecode renders "NaN:NaN:NaN:NaN",
  // which is worse than showing nothing: it looks like the app is broken
  // rather than like the project is.
  if (!Number.isFinite(tpf) || tpf <= 0) return '00:00:00:00';

  const fps = Math.round(rate.num / rate.den);
  if (!Number.isFinite(fps) || fps <= 0) return '00:00:00:00';

  const totalFrames = Math.max(0, Math.floor(ticks / tpf));
  const frames = totalFrames % fps;
  const totalSeconds = Math.floor(totalFrames / fps);
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600);

  const pad = (n: number): string => String(n).padStart(2, '0');
  return `${pad(hours)}:${pad(minutes)}:${pad(seconds)}:${pad(frames)}`;
}

/** A short duration for tooltips: `1.4s`. */
export function formatDuration(ticks: number): string {
  const s = ticksToSeconds(ticks);
  if (s < 10) return `${s.toFixed(1)}s`;
  if (s < 60) return `${Math.round(s)}s`;
  const m = Math.floor(s / 60);
  return `${m}m ${Math.round(s - m * 60)}s`;
}