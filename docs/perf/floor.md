# The floor machine

The floor is the weakest machine a Quillie user plausibly owns. Every basic
operation must work there. Higher tiers unlock higher preview resolution, more
parallel jobs and heavier effects — but they never unlock *basic editing*.

## Definition

| Property | Floor |
|---|---|
| RAM | 4 GB |
| Physical CPU cores | 2 |
| SIMD | baseline x86-64, **no AVX2** |
| Graphics | integrated; WebGL 1 required, WebGL 2 optional |
| Disk | 5400 RPM mechanical, poor random access |
| OS | Linux (MX Linux / Debian, default WebKitGTK) or Windows 10/11 (WebView2) |

Tiers above it: **mid** = 8 GB RAM; **high** = 16 GB or more.

## Consequences that are binding on the design

- No feature may assume a GPU, AVX2, SSD speeds, or more than 4 GB RAM.
- Weak hardware gets **slower previews and slower background work**. It never
  gets a worse editor.
- Every effect that ships must have a lower preview level of detail rather than
  being switched off by tier. The Feature Gate makes this rule 5.

## The machine used for development so far

Recorded honestly, because passing a measurement off as floor performance when
it was taken on a faster machine defeats the purpose of having a floor at all.

| Property | Dev machine | Floor? |
|---|---|---|
| OS | MX Linux 6.12 (Debian 13 based) | comparable platform, yes |
| RAM | **7.8 GB** | **no** — this is mid tier |
| Logical / physical cores | 4 / 4 | **no** — 2x floor cores |
| CPU | x86-64, AVX2 present | **no** — floor has no AVX2 |
| Graphics | Intel HD Graphics 520 (Mesa) | comparable |
| Disk | SSD, 799 GB free | **no** — floor is 5400 RPM |
| WebView | WebKitGTK 2.54 | yes |

**This machine is "mid", not "floor".** Consequences:

1. Every number measured here is labelled `tier=mid`.
2. Idle-cost, memory-budget and disk-guard results from this machine are
   **upper bounds**: an SSD and twice the cores will always look better than a
   5400 RPM drive with two cores.
3. Wall-clock acceptance criteria in the phase list that specify the floor
   machine are marked **not measured** until a real floor machine is available.
   §2 of the build document requires this gap to be recorded rather than
   substituted.
4. The most floor-sensitive targets, in the order they will degrade first:
   - scrub latency p95 (§16: ≤ 150 ms) — disk-bound, will be worse on 5400 RPM
   - preview cache and disk-guard behaviour (§8.5, §7.2)
   - memory ceiling (§16: ≤ 1.2 GB peak) — this machine has 4 GB, the floor
     has 4 GB total
   - CPU-heavy job concurrency (§9.1) — floor allows `max(1, cores - 1)` = 1

## Memory accounting method

When peak memory is measured, the footprint summed is the app process, all
WebView child processes, and all FFmpeg children. On Linux this uses **PSS
from `/proc/<pid>/smaps_rollup`**, so shared pages are not double-counted.
WebKitGTK runs the renderer in separate processes and those count toward the
total.

On Windows the counter used is stated alongside the number, because working set
counts shared pages more than once.

## Disk guard defaults

- Proxy generation refuses below **2 GB** free space (configurable).
- Disk-heavy jobs run **one at a time** on the floor. Whether two is better on
  higher tiers is a Phase 2 measurement, not an assumption.

## What must be re-measured on a real floor machine

Everything in §16 of the build document, before any of it is presented as a
commitment. Until then every figure here is a mid-tier data point and the
phase reports say so.

To close the gap, either provision a 4 GB / 2-core / 5400 RPM machine, or
constrain an existing one with `cpulimit`, a cgroup memory cap of 4 GB, and a
throttled block device. The memory cap is the easiest part to do faithfully;
emulating a 5400 RPM spindle is not, and disk-bound numbers should be treated
as optimistic.