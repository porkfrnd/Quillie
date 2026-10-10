/**
 * Benchmark result schema and helpers (§16).
 *
 * Every performance claim in this project has to be backed by one of these
 * files. That is a hard rule, not a convention: a number in a commit message
 * or a report with no result file behind it is a guess, and guesses about
 * performance are how a floor machine gets silently abandoned.
 *
 * A result file records the hardware it came from, because a number without
 * its context is not a measurement.
 */

export interface HardwareSummary {
  readonly os: string;
  readonly arch: string;
  readonly cpuModel: string;
  readonly physicalCores: number;
  readonly logicalCores: number;
  readonly totalRamMb: number;
  readonly availableRamMb: number;
  readonly gpuRenderer: string | null;
  readonly diskType: string;
}

export type Tier = 'floor' | 'mid' | 'high';

/**
 * Derive the tier from RAM and cores (§13).
 *
 * Deliberately simple and deliberately conservative: the floor is the weakest
 * machine a user plausibly owns, so anything at or below the floor numbers is
 * treated as floor even if it is probably a little better.
 */
export function deriveTier(totalRamMb: number, physicalCores: number): Tier {
  if (totalRamMb <= 4096 && physicalCores <= 2) return 'floor';
  if (totalRamMb <= 8192) return 'mid';
  return 'high';
}

export interface Measurement {
  readonly name: string;
  readonly value: number;
  readonly unit: string;
  /** What "good" looks like, or null when no target is set yet. */
  readonly target?: number;
  readonly pass?: boolean;
  /** Notes on how this was measured, or caveats that limit its meaning. */
  readonly note?: string;
}

export interface BenchmarkResult {
  /** ISO 8601. */
  readonly timestamp: string;
  readonly commit: string;
  readonly ffmpegVersion: string;
  readonly webviewVersion: string;
  readonly hardware: HardwareSummary;
  readonly tier: Tier;
  readonly scenario: string;
  readonly measurements: readonly Measurement[];
  /**
   * True when these numbers came from a machine that is not the real floor.
   * The performance ratchet (§15.1) uses the latest accepted floor result as
   * its baseline, so mixing tiers silently would corrupt that baseline.
   */
  readonly isFloorMachine: boolean;
}

/** §16 targets. Null means "report it, no target until measured". */
export const TARGETS: Record<string, number | null> = {
  'cold_start_first_launch_s': 6,
  'window_shell_s': 1,
  'idle_cpu_percent': 1,
  'ui_frame_p95_ms': 33,
  'scrub_p95_1080p_ms': 150,
  'scrub_p95_4k_proxy_ms': 150,
  'dropped_frame_percent': 5,
  'audio_dropouts': 0,
  'peak_memory_mb': 1200,
  'cost_model_error_ratio': 0.3,
};

export function filename(result: BenchmarkResult): string {
  const d = result.timestamp.slice(0, 10);
  const os = result.hardware.os.replace(/[^a-z0-9]+/gi, '-').toLowerCase();
  return `${d}-${os}-${result.tier}.json`;
}

export function serialise(result: BenchmarkResult): string {
  return `${JSON.stringify(result, null, 2)}\n`;
}

/** Compare two results on one metric, for the ratchet check (§15.1). */
export function regression(
  baseline: BenchmarkResult,
  candidate: BenchmarkResult,
  metric: string,
  toleranceRatio = 0.05,
): { regressed: boolean; baseline: number; candidate: number } | null {
  const b = baseline.measurements.find((m) => m.name === metric);
  const c = candidate.measurements.find((m) => m.name === metric);
  if (!b || !c) return null;
  const regressed = c.value > b.value * (1 + toleranceRatio);
  return { regressed, baseline: b.value, candidate: c.value };
}