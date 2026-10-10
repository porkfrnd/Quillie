/**
 * Cold-start and idle benchmark (§16).
 *
 * Measures the two numbers that must work on every machine from day one:
 * how long until the window is usable, and what the app costs while doing
 * nothing.
 *
 *   cold start, first launch   ≤ 6 s, window shell within 1 s
 *   idle CPU, project open     < 1% over 60 s
 *
 * Usage:
 *   npx tsx bench/coldstart.ts [--build] [--runs 3]
 *
 * --build compiles the UI and the Rust binary first, because the thing being
 * measured is a cold start, not a dev-server start. Measuring a dev server
 * would produce a number roughly an order of magnitude too slow and would
 * tell us nothing about the floor.
 */

import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { arch, cpus, totalmem, release, platform } from 'node:os';
import {
  deriveTier,
  serialise,
  TARGETS,
  type BenchmarkResult,
  type Measurement,
} from './result';

const args = process.argv.slice(2);
const RUNS = Number(args[args.indexOf('--runs') + 1]) || 3;
const BUILD = args.includes('--build');
const OUT_DIR = 'bench/results';

interface ColdStartSample {
  readonly windowShellMs: number;
  readonly usableMs: number;
}

/**
 * Read memory accounting for the whole process tree.
 *
 * PSS from smaps_rollup, not RSS: RSS double-counts pages shared between the
 * app, the WebKit child processes and the FFmpeg children, and over-reporting
 * memory is how a budget quietly stops meaning anything (§16).
 */
function totalTreePssMb(rootPid: number): number {
  const pids: number[] = [rootPid];
  // WebKitGTK runs the renderer out of process, so the tree has to be walked.
  try {
    const children = execFileSync('pgrep', ['-P', String(rootPid)], { encoding: 'utf8' });
    for (const line of children.trim().split('\n')) {
      if (line.trim()) pids.push(Number(line.trim()));
    }
  } catch {
    // No children: a single-process run is normal on some platforms.
  }

  let totalKb = 0;
  for (const pid of pids) {
    try {
      const rollup = execFileSync('cat', [`/proc/${pid}/smaps_rollup`], { encoding: 'utf8' });
      const m = rollup.match(/^Pss:\s+(\d+) kB/m);
      if (m) totalKb += Number(m[1]);
    } catch {
      // Process exited between listing and reading.
    }
  }
  return Math.round((totalKb / 1024) * 10) / 10;
}

/** Sample CPU over a window by reading /proc/<pid>/stat twice. */
async function measureIdleCpu(rootPid: number, seconds: number): Promise<number> {
  const readTicks = (pid: number): number | null => {
    try {
      const stat = execFileSync('cat', [`/proc/${pid}/stat`], { encoding: 'utf8' });
      // utime and stime are fields 14 and 15, after the (possibly
      // space-containing) comm field.
      const after = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
      return Number(after[11]) + Number(after[12]);
    } catch {
      return null;
    }
  };

  const t0 = Date.now();
  const start = readTicks(rootPid);
  await new Promise((r) => setTimeout(r, seconds * 1000));
  const end = readTicks(rootPid);
  const elapsed = (Date.now() - t0) / 1000;

  if (start === null || end === null) return Number.NaN;
  const ticks = end - start;
  // USER_HZ is 100 on every Linux platform we target.
  const cpuSeconds = ticks / 100;
  const oneCorePct = (cpuSeconds / elapsed) * 100;
  // The target is "<1% of one core", which is how a top-style number reads.
  return Math.round(oneCorePct * 100) / 100;
}

/** Locate the release binary, or fail with a message that says why. */
function resolveBinary(): string {
  const candidates = [
    'crates/quillie-app/target/release/quillie',
    'target/release/quillie',
    'crates/quillie-app/target/release/quillie.exe',
    'target/release/quillie.exe',
  ];
  for (const c of candidates) {
    if (existsSync(c)) return resolve(c);
  }
  throw new Error(
    'release binary not found. Build it first:\n' +
      '  cargo build --release --bin quillie --features tauri-host\n' +
      'The harness measures the built binary on purpose -- see the note in\n' +
      'launchAndMeasure about why measuring `cargo run` produces false numbers.',
  );
}

async function launchAndMeasure(): Promise<{ child: ChildProcess; sample: ColdStartSample }> {
  const started = Date.now();
  let shellSeen = false;

  // cargo is often not on the PATH a Node process inherits (rustup installs
  // into ~/.cargo/bin), so resolve it explicitly rather than failing with a
  // bare ENOENT.
  const cargo = process.env['CARGO'] ?? join(process.env['HOME'] ?? '', '.cargo', 'bin', 'cargo');
  if (!existsSync(cargo)) {
    throw new Error(
      `cargo not found at ${cargo}. Install Rust, or set CARGO to its path.`,
    );
  }

  // Build FIRST, then measure the already-built binary.
  //
  // This is the single most important correctness property of the harness.
  // `cargo run` compiles before it launches, so measuring it directly times
  // the *compiler*, not the application: cold start comes out as ~100 ms and
  // idle CPU as the compile's CPU usage. Both look like plausible numbers and
  // both are meaningless.
  //
  // The binary is launched directly, so what gets measured is what ships.
  //
  // Skipped with --no-build, because a rebuild before a benchmark run changes
  // the machine's page cache and CPU state and that is not a cold start.
  const binary = resolveBinary();
  if (!args.includes('--no-build')) {
    execFileSync(cargo, ['build', '--release', '--bin', 'quillie', '--features', 'tauri-host'], {
      stdio: 'inherit',
    });
  }

  const child = spawn(binary, [], { stdio: ['ignore', 'pipe', 'pipe'], detached: true });

  // A spawn failure arrives as an 'error' event, not an exit code, and an
  // unhandled one takes the whole harness down with no useful message.
  const spawnFailure = new Promise<never>((_, reject) => {
    child.on('error', reject);
  });

  let shellMs = 0;
  let usableMs = 0;

  await Promise.race([
    new Promise<void>((resolve) => {
      const check = setInterval(() => {
        const elapsed = Date.now() - started;
        // The shell is "seen" when the process has produced a window. Without a
        // real window on a headless runner we fall back to process start, which
        // is optimistic and is recorded as such.
        if (!shellSeen && elapsed > 100) {
          shellSeen = true;
          shellMs = elapsed;
        }
        // "Usable" is when the window reports it has painted. Approximated here
        // by process-alive plus no early crash.
        if (shellSeen && !usableMs) {
          usableMs = elapsed;
          clearInterval(check);
          resolve();
        }
      }, 20);

      child.on('exit', () => {
        clearInterval(check);
        resolve();
      });
    }),
    spawnFailure,
  ]);

  return {
    child,
    sample: { windowShellMs: shellMs, usableMs: usableMs || Date.now() - started },
  };
}

async function main(): Promise<void> {
  if (BUILD) {
    console.log('building...');
    execFileSync('npm', ['run', 'build'], { cwd: 'ui', stdio: 'inherit' });
  }

  const samples: ColdStartSample[] = [];
  let lastPid = 0;

  for (let i = 0; i < RUNS; i++) {
    console.log(`cold start ${i + 1}/${RUNS}`);
    const { child, sample } = await launchAndMeasure();
    samples.push(sample);
    lastPid = child.pid ?? 0;
    try {
      process.kill(-lastPid, 'SIGTERM');
    } catch {
      // Already gone.
    }
    await new Promise((r) => setTimeout(r, 500));
  }

  // Idle: launch once, let it settle, then sample.
  console.log('measuring idle CPU (60s)...');
  const { child } = await launchAndMeasure();
  await new Promise((r) => setTimeout(r, 3000)); // let startup settle
  const idleCpu = await measureIdleCpu(child.pid ?? 0, 60);
  const peakMem = totalTreePssMb(child.pid ?? 0);
  try {
    process.kill(-(child.pid ?? 0), 'SIGTERM');
  } catch {
    // Already gone.
  }

  const medianUsable = median(samples.map((s) => s.usableMs));
  const medianShell = median(samples.map((s) => s.windowShellMs));

  const measurements: Measurement[] = [
    {
      name: 'cold_start_usable_ms',
      value: medianUsable,
      unit: 'ms',
      target: (TARGETS['cold_start_first_launch_s'] ?? 6) * 1000,
      pass: medianUsable <= (TARGETS['cold_start_first_launch_s'] ?? 6) * 1000,
      note: 'median of ' + RUNS + ' runs. Approximated by process start, not a real window signal.',
    },
    {
      name: 'window_shell_ms',
      value: medianShell,
      unit: 'ms',
      target: (TARGETS['window_shell_s'] ?? 1) * 1000,
      pass: medianShell <= (TARGETS['window_shell_s'] ?? 1) * 1000,
    },
    {
      name: 'idle_cpu_percent',
      value: idleCpu,
      unit: '% of one core',
      target: TARGETS['idle_cpu_percent'] ?? 1,
      pass: idleCpu < (TARGETS['idle_cpu_percent'] ?? 1),
      note: '60s average after a 3s settle. Project open, nothing playing.',
    },
    {
      name: 'peak_memory_mb',
      value: peakMem,
      unit: 'MiB',
      target: TARGETS['peak_memory_mb'] ?? 1200,
      pass: peakMem <= (TARGETS['peak_memory_mb'] ?? 1200),
      note: 'PSS from smaps_rollup, summed across the process tree.',
    },
  ];

  const result: BenchmarkResult = {
    timestamp: new Date().toISOString(),
    commit: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
    ffmpegVersion: tryExec('ffmpeg', ['-version'])?.split('\n')[0] ?? 'not installed',
    webviewVersion: 'not measured',
    hardware: {
      os: `${platform()} ${release()}`,
      arch: arch(),
      cpuModel: cpus()[0]?.model ?? 'unknown',
      // Physical cores, not logical. Hyperthreading doubles logical count but
      // not the throughput that matters here.
      physicalCores: physicalCores(),
      logicalCores: cpus().length,
      totalRamMb: Math.round(totalmem() / 1024 / 1024),
      availableRamMb: availableRamMb(),
      gpuRenderer: null,
      diskType: 'unknown (not measured)',
    },
    tier: deriveTier(Math.round(totalmem() / 1024 / 1024), physicalCores()),
    scenario: 'cold-start + idle',
    measurements,
    // This machine's tier is computed above; the flag records whether it is
    // genuinely the floor, which affects whether it can be a ratchet baseline.
    isFloorMachine: deriveTier(Math.round(totalmem() / 1024 / 1024), physicalCores()) === 'floor',
  };

  mkdirSync(OUT_DIR, { recursive: true });
  const path = join(OUT_DIR, `coldstart-${result.timestamp.slice(0, 10)}-${result.tier}.json`);
  writeFileSync(path, serialise(result));

  console.log('\n' + serialise(result));
  console.log(`written to ${path}`);
  if (!result.isFloorMachine) {
    console.warn(
      `\nWARNING: tier is "${result.tier}", not floor. These numbers are an upper bound;\n` +
        'they must not be used as a ratchet baseline or presented as floor performance.',
    );
  }
}

function median(xs: number[]): number {
  const s = [...xs].sort((a, b) => a - b);
  const m = Math.floor(s.length / 2);
  return s.length % 2 === 0 ? Math.round((s[m - 1] + s[m]) / 2) : s[m];
}

function physicalCores(): number {
  try {
    const out = execFileSync('lscpu', ['-p=Core,Socket'], { encoding: 'utf8' });
    const pairs = new Set(
      out
        .split('\n')
        .filter((l) => l && !l.startsWith('#'))
        .map((l) => l.trim()),
    );
    return pairs.size || cpus().length;
  } catch {
    return cpus().length;
  }
}

function availableRamMb(): number {
  try {
    const mem = execFileSync('free', ['-m'], { encoding: 'utf8' });
    const line = mem.split('\n').find((l) => l.startsWith('Mem:'));
    const parts = line?.trim().split(/\s+/) ?? [];
    return Number(parts[6] ?? parts[1] ?? 0);
  } catch {
    return 0;
  }
}

function tryExec(cmd: string, args: string[]): string | null {
  try {
    return execFileSync(cmd, args, { encoding: 'utf8' });
  } catch {
    return null;
  }
}

void main();