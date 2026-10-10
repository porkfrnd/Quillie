# ADR-001: Project time base

- **Status:** Accepted
- **Date:** 2026-10-10
- **Deciders:** Quillie contributors
- **References:** build prompt §6.1, §22

## Context

The build document specifies a project timebase of **1/120000 s**, stored as
`Ticks(i64)`, and requires that this represent "every common frame rate
exactly, including 24000/1001, 30000/1001, and 60000/1001 (ticks per frame:
5005, 4004, 2002)".

§22 pre-authorises this decision: "Accept 1/120000 unless the tests in Phase 1
show a problem." Phase 1 is where `time.rs` and its tests land, so this ADR
records the verification.

## Decision

**Accept 1/120000 s.** No change.

## Verification

Ticks per frame is `TICKS_PER_SECOND * den / num`. Checking that this is a
whole number for every supported rate:

| Rate | `120000 * den / num` | Whole? |
|---|---|---|
| 24000/1001 (23.976) | 5005 | yes |
| 24 | 5000 | yes |
| 25 | 4800 | yes |
| 30000/1001 (29.97) | 4004 | yes |
| 30 | 4000 | yes |
| 50 | 2400 | yes |
| 60000/1001 (59.94) | 2002 | yes |
| 60 | 2000 | yes |
| 120 | 1000 | yes |

All nine divide exactly. The figures in the document are correct.

This works because `120000 = 2^7 * 3 * 5^3`, which is divisible by 24, 25, 30,
50, 60 and 120, and because `gcd(1001, 24000) = gcd(1001, 30000) =
gcd(1001, 60000) = 1`, so the NTSC numerators need the full `120000` to divide
without remainder.

Consequences verified by `crates/quillie-core/src/time.rs`:

- `frame -> ticks -> frame` is lossless for every rate above.
- Repeated frame stepping over 200 000 frames accumulates **zero** error.
- A trim at frame 1001 on a 23.976 timeline lands on the same tick forever.

### A note on an intermediate false finding

An earlier draft of this work concluded the timebase was wrong, on the basis
that `24000 * 120000 / 1001` is not an integer. That expression is not the
conversion. Ticks per frame is `TICKS_PER_SECOND / fps`, i.e.
`TICKS_PER_SECOND * den / num`, not `TICKS_PER_SECOND * num / den`. With the
correct ratio every rate divides exactly. The document was right; the first
implementation had `num` and `den` inverted.

The error is recorded here rather than deleted because it is exactly the kind
of mistake this time base exists to prevent, and because the fix belongs in the
tests: `ticks_per_frame_are_exact_for_every_required_rate` now asserts the
divisibility identity directly, so an inverted ratio fails the suite instead of
producing plausible-looking wrong numbers.

## Consequences

- Every time in the project model is an integer. No floating-point time is ever
  accumulated across frames or clips.
- Frame rates are stored as rationals `(num, den)`, never as `f32`. 29.97 and
  30000/1001 are different values and the type keeps them apart.
- Rounding direction is an explicit parameter (`Floor` vs `Nearest`), because
  it decides whether a cut lands one frame early.
- `Ticks` is `i64`: the maximum representable duration is roughly 4.9 billion
  years, so overflow is not a practical concern.
- VFR sources are mapped by timestamp and flagged, never silently converted to
  CFR. Any conversion is a visible proxy step.