# Build-time measurements

Measured on the development machine (mid tier: i5-6200U, 2 physical cores /
4 logical, 7.6 GB RAM, SSD). Every number below came from a timed command;
where something was not measured, it says so rather than estimating.

**These are mid-tier numbers, not floor numbers.** A floor machine
(4 GB, 2 cores, mechanical disk, no AVX2) will be slower. Treat the ordering as
the result and the absolute times as an upper bound.

## Why this exists

The release profile was `lto = "fat"` with `codegen-units = 1`. Rebuilding
after a one-line edit to the host crate took 262 seconds, which is long enough
to discourage verifying changes by building them. This document records what
was measured so the profile can be chosen from data rather than from the
general advice that fat LTO is best for release builds.

## The headline: Tauri is not the slow part

`cargo build --timings` for a partial release rebuild (254.2s total, max
concurrency 5 on 4 logical cores):

| crate | time |
|---|---|
| gtk | 116.1s |
| tauri-utils | 64.3s |
| quillie-app | 53.1s |
| tauri | 25.6s |
| regex-automata | 22.8s |
| webkit2gtk | 18.3s |
| tokio | 17.8s |

The GTK/webkit stack is the build-time bottleneck, and it is a fixed cost of
the platform rather than anything about how this project is structured.
`quillie-core` does not appear in the top ten: the pure logic is not what takes
long to compile. Tauri's reputation for small, fast *runtime* is accurate; it
is not a claim about compile time, and the two should not be conflated.

## Profile comparison

All builds: `cargo build --release --bin quillie --features tauri-host`, with
`panic = "abort"`, `strip = true` and
`[profile.release.package.quillie-core] opt-level = 2` unchanged.

| profile | wall clock | binary size | what was rebuilt |
|---|---|---|---|
| `cargo check --workspace` | 54.6s | n/a | incremental |
| A: `lto="fat"`, `cgu=1` | 262.5s | 7,392,464 B (7.05 MiB) | incremental (host + core + link) |
| B: `lto="thin"`, `cgu=1` | 469.8s | 7,801,288 B (7.44 MiB) | full, 518 units |
| C: `lto="off"`, `cgu=16` | 254.4s | 8,456,336 B (8.06 MiB) | 210 of 518 units |

Commands, verbatim:

```
touch crates/quillie-app/src/lib.rs && time cargo check --workspace --features tauri-host
touch crates/quillie-app/src/lib.rs && time cargo build --release --bin quillie --features tauri-host
touch crates/quillie-app/src/lib.rs && time cargo build --release --bin quillie --features tauri-host \
  --config 'profile.release.lto="thin"' --config 'profile.release.codegen-units=1'
touch crates/quillie-app/src/lib.rs && time cargo build --release --bin quillie --features tauri-host \
  --config 'profile.release.lto="off"' --config 'profile.release.codegen-units=16' --timings
```

### CPU utilisation

A's 262.5s wall against 205.3s user means roughly 0.78 cores: the fat LTO link
is effectively single-threaded and the machine sat idle. B used 918.1s user
across 469.8s wall, about 1.95 cores. This is the mechanism behind the
recommendation: fat does not just take longer, it takes longer while leaving
the other three quarters of the CPU doing nothing.

## Reading the comparison honestly

**The A-vs-B and B-vs-C comparisons are not like-for-like**, and the numbers
should not be read as a clean ranking:

- B was a genuine from-scratch rebuild of all 518 units. A was incremental.
- C rebuilt only 210 of 518. Its apparent speed therefore reflects doing less
  work as well as cheaper work, so "C is faster than B" overstates the
  difference between the two settings.
- Switching between LTO settings in cargo 1.99 did **not** dirty every unit,
  which is not what the fingerprint change alone would predict. The hypothesis
  is that cargo excludes `lto` and `codegen-units` from the fingerprint of
  non-root units. This is unconfirmed.

What can be said with confidence: fat LTO costs a single-threaded 262s link on
an incremental rebuild, and buys 409 KB over thin (7.05 vs 7.44 MiB).

## Recommendation

**For day-to-day work, do not build release at all.** `cargo check` gives
type-level feedback in 54.6s, and the dev profile gives a runnable binary with
no LTO. Building `--release` to *check* that something compiles is the slowest
way to ask a question.

**For shipping, `lto = "thin"` is the defensible default** — which is why the
profile should be changed from `"fat"`:

- It keeps cross-crate optimisation, which matters for a UI that crosses the
  Tauri boundary constantly and is exactly where an unoptimised call shows.
- It avoids the single-threaded link that makes fat painful.
- The measured cost of keeping fat is 409 KB of binary size.

**`lto = "off"` is plausible and was not adopted here.** It is the fastest of
the three measured, but the comparison is confounded as described above, and no
runtime benchmark was run to show that the 655 KB is worth nothing. If someone
wants to make that call, the next step is a runtime benchmark of thin vs off,
not another build-time measurement.

## Not measured

Stated plainly, because a build-time document that implies more coverage than
it has is worse than a short one:

- **Runtime performance of fat vs thin vs off.** No benchmark was run. Every
  statement about runtime above is reasoning, not measurement.
- **The dev profile's wall clock.** The run was cut short by a time limit. The
  recommendation to prefer it rests on the fact that dev builds skip LTO, not
  on a number here.
- **Peak memory per build.** `/usr/bin/time -v` is not installed on this
  machine, and cargo 1.99 rejects `--timings=json`, which is where the per-unit
  memory would have come from. The HTML report has no memory column.
- **Idle CPU after the app starts** for each profile. That is a runtime
  property and belongs in `bench/results/`, not here.

The machine carried roughly load 4 (a browser and the editor itself) during
all of these. Absolute times would be lower on an idle machine; the ordering is
unaffected.