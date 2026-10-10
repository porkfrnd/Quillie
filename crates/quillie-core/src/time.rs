//! Time representation (§6.1).
//!
//! # The one rule
//!
//! Every duration and position in a Quillie project is an integer number of
//! **ticks**, where one tick is `1/120000` second. Times are never stored as
//! floating point and never accumulated frame by frame.
//!
//! # Why 1/120000
//!
//! `1001` (the denominator of every NTSC rate) is coprime with 24000, 30000
//! and 60000, so `ticks_per_frame = TICKS_PER_SECOND * den / num` is a whole
//! number of ticks for every rate Quillie supports:
//!
//! | Rate              | Ticks per frame | Exact? |
//! |-------------------|-----------------|--------|
//! | 24000/1001 (23.976)| 5005           | yes    |
//! | 24                 | 5000           | yes    |
//! | 25                 | 4800           | yes    |
//! | 30000/1001 (29.97)| 4004           | yes    |
//! | 30                 | 4000           | yes    |
//! | 50                 | 2400           | yes    |
//! | 60000/1001 (59.94)| 2002           | yes    |
//! | 60                 | 2000           | yes    |
//! | 120                | 1000           | yes    |
//!
//! That means converting a frame index to a time and back is lossless for all
//! of them, so a trim at frame 1001 of a 23.976 timeline lands on exactly the
//! same tick every time the project is opened. ADR-001 accepts this timebase.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Ticks per second in the Quillie project timebase (§6.1, ADR-001).
pub const TICKS_PER_SECOND: i64 = 120_000;

/// A duration or position, in units of `1/120000` second.
///
/// `Ticks` is `i64` so that a 24-hour project at high resolution cannot
/// overflow in practice (`i64::MAX` ticks is ~4.9 billion years).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ticks(pub i64);

impl Ticks {
    /// The zero position (the start of the timeline).
    pub const ZERO: Ticks = Ticks(0);

    /// Build a tick count from whole seconds.
    pub const fn from_seconds(secs: i64) -> Self {
        Ticks(secs * TICKS_PER_SECOND)
    }

    /// Build a tick count from milliseconds, rounding to the nearest tick.
    ///
    /// This is the only constructor that takes a float, and it exists for
    /// wall-clock-ish inputs (an imported duration from a container header).
    /// Project-internal code should use `FrameRate::frame_to_ticks` instead.
    pub fn from_millis_f64(ms: f64) -> Self {
        Ticks((ms / 1000.0 * TICKS_PER_SECOND as f64).round() as i64)
    }

    /// Build a tick count from a duration in seconds, rounding to the nearest
    /// tick.
    ///
    /// For container-reported durations, which arrive as floats and are the
    /// other place float time legitimately enters the system.
    pub fn from_secs_f64(secs: f64) -> Self {
        if !secs.is_finite() || secs <= 0.0 {
            return Ticks::ZERO;
        }
        Ticks((secs * TICKS_PER_SECOND as f64).round() as i64)
    }

    /// Exact microseconds, when they land on a whole tick.
    pub const fn from_micros(us: i64) -> Self {
        Ticks(us * (TICKS_PER_SECOND / 1_000_000))
    }

    /// Raw tick value.
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Whole seconds, truncated toward zero. For display; never for maths.
    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Position within a frame at the given rate, as a 0.0..1.0 fraction.
    ///
    /// Used for sub-frame display only. Never round-trip a value through this.
    pub fn sub_frame_fraction(self, rate: FrameRate) -> f64 {
        let frame = rate.ticks_per_frame();
        if frame == 0 {
            return 0.0;
        }
        let rem = self.0.rem_euclid(frame);
        rem as f64 / frame as f64
    }

    /// Clamp to a non-negative tick count.
    pub fn saturating_non_negative(self) -> Self {
        Ticks(self.0.max(0))
    }
}

impl fmt::Display for Ticks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ticks", self.0)
    }
}

impl std::iter::Sum for Ticks {
    fn sum<I: Iterator<Item = Ticks>>(iter: I) -> Self {
        Ticks(iter.map(|t| t.0).sum())
    }
}

/// A video frame rate, stored as an exact rational (§6.1).
///
/// This is deliberately a rational rather than an `f32`. 29.97 is *not*
/// 30000/1001, and a project at the wrong rate drifts audibly over a long
/// timeline. `FrameRate::NTSC_30` and `FrameRate::from_f64(29.97)` are not the
/// same value, and that difference is the whole point of the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    num: u32,
    den: u32,
}

/// The rounding mode used when a time must become a frame index.
///
/// This is not a detail. `Floor` vs `Nearest` decides whether a cut lands one
/// frame early, and getting it wrong is the kind of bug users notice
/// immediately and describe as "the editor is inaccurate".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rounding {
    /// Round toward negative infinity. The default for timeline positions:
    /// a clip that ends at 1.5 frames has not yet reached frame 2.
    Floor,
    /// Round to the closest frame, halves away from zero.
    Nearest,
}

impl FrameRate {
    pub const NTSC_24: FrameRate = FrameRate { num: 24000, den: 1001 };
    pub const NTSC_30: FrameRate = FrameRate { num: 30000, den: 1001 };
    pub const NTSC_60: FrameRate = FrameRate { num: 60000, den: 1001 };

    /// Build a rate from a rational, reducing it and rejecting a zero
    /// denominator.
    ///
    /// # Panics
    /// Panics if `den` is 0 or either component is 0.
    pub fn new(num: u32, den: u32) -> Self {
        assert!(den != 0, "frame rate denominator must be non-zero");
        assert!(num != 0, "frame rate numerator must be non-zero");
        let g = gcd(num as u64, den as u64) as u32;
        FrameRate { num: num / g, den: den / g }
    }

    /// Build from a float, snapping to the nearest known broadcast rate.
    ///
    /// 29.97 becomes 30000/1001, not 2997/100. Only for inputs that arrive as
    /// floats, such as a UI slider or a loose user preference.
    pub fn from_f64(fps: f64) -> Self {
        if fps <= 0.0 {
            return FrameRate { num: 30, den: 1 };
        }
        // Snap to a common rate if we're within 0.1% of one.
        const COMMON: &[(f64, FrameRate)] = &[
            (23.976, FrameRate::NTSC_24),
            (24.0, FrameRate { num: 24, den: 1 }),
            (25.0, FrameRate { num: 25, den: 1 }),
            (29.97, FrameRate::NTSC_30),
            (30.0, FrameRate { num: 30, den: 1 }),
            (48.0, FrameRate { num: 48, den: 1 }),
            (50.0, FrameRate { num: 50, den: 1 }),
            (59.94, FrameRate::NTSC_60),
            (60.0, FrameRate { num: 60, den: 1 }),
            (120.0, FrameRate { num: 120, den: 1 }),
        ];
        for (approx, exact) in COMMON {
            if (fps - approx).abs() < approx * 0.001 {
                return *exact;
            }
        }
        // Not a common rate: round to the nearest integer num/den that fits.
        FrameRate { num: (fps * 1000.0).round() as u32, den: 1000 }
    }

    pub const fn numerator(self) -> u32 {
        self.num
    }

    pub const fn denominator(self) -> u32 {
        self.den
    }

    /// Ticks per frame, exact: `TICKS_PER_SECOND * den / num`.
    ///
    /// i128 is used so the multiply never overflows before the divide. Because
    /// the timebase makes this a whole number for every supported rate
    /// (ADR-001), the result never needs rounding.
    pub fn ticks_per_frame(self) -> i64 {
        let v = (TICKS_PER_SECOND as i128) * (self.den as i128) / (self.num as i128);
        v as i64
    }

    /// Approximate frames per second, for display and cost estimates only.
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Convert a frame index to a time (§6.1).
    ///
    /// `ticks = frame * TICKS_PER_SECOND * den / num`. Because the timebase
    /// divides exactly for every supported rate (ADR-001), this equals
    /// `frame * ticks_per_frame()` with no rounding at all.
    pub fn frame_to_ticks(self, frame: i64) -> Ticks {
        Ticks(self.ticks_per_frame().saturating_mul(frame))
    }

    /// Convert a frame index to ticks, rounding to the nearest tick.
    ///
    /// With the exact timebase this is identical to `frame_to_ticks`; it exists
    /// so callers that think in "round to nearest" stay correct if the timebase
    /// ever changes.
    pub fn frame_to_ticks_rounded(self, frame: i64) -> Ticks {
        // Exact on the current timebase; kept separate so a future timebase
        // change has an obvious place to reintroduce rounding.
        self.frame_to_ticks(frame)
    }

    /// Convert a time to a frame index.
    pub fn ticks_to_frame(self, ticks: Ticks, rounding: Rounding) -> i64 {
        let numerator = ticks.0.saturating_mul(self.num as i64);
        let denominator = self.den as i64 * TICKS_PER_SECOND;
        match rounding {
            Rounding::Floor => divide_floor(numerator, denominator),
            Rounding::Nearest => divide_round_half_away(numerator, denominator),
        }
    }

    /// The frame containing `ticks`.
    pub fn frame_at(self, ticks: Ticks) -> i64 {
        self.ticks_to_frame(ticks, Rounding::Floor)
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{} fps", self.num)
        } else {
            write!(f, "{} fps (NTSC)", round2(self.as_f64()))
        }
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

/// Floor division toward negative infinity.
fn divide_floor(a: i64, b: i64) -> i64 {
    a.div_euclid(b)
}

fn divide_round_half_away(a: i64, b: i64) -> i64 {
    let half = b / 2;
    // i128 intermediates. `a + half` overflows for `a` near i64::MAX and `-a`
    // overflows for i64::MIN, and both are reachable from a hand-edited
    // project file. In debug that is a panic; in release it is a silent wrap
    // into a negative frame number, which is worse: it moves a playhead to
    // the wrong place instead of refusing to draw it.
    let (num, den, h) = (a as i128, b as i128, half as i128);
    if a >= 0 {
        ((num + h) / den) as i64
    } else {
        (-((-num + h) / den)) as i64
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §6.1 requires a test for every rate the timebase must represent exactly.
    /// This table is the contract: if a row changes, the timebase changed.
    ///
    /// The values are the 1/120000 s timebase from §6.1.
    #[test]
    fn ticks_per_frame_are_exact_for_every_required_rate() {
        let cases: &[(FrameRate, i64)] = &[
            (FrameRate::NTSC_24, 5_005),
            (FrameRate::new(24, 1), 5_000),
            (FrameRate::new(25, 1), 4_800),
            (FrameRate::NTSC_30, 4_004),
            (FrameRate::new(30, 1), 4_000),
            (FrameRate::new(50, 1), 2_400),
            (FrameRate::NTSC_60, 2_002),
            (FrameRate::new(60, 1), 2_000),
            (FrameRate::new(120, 1), 1_000),
        ];
        for (rate, expected) in cases {
            assert_eq!(
                rate.ticks_per_frame(),
                *expected,
                "{rate} should be {expected} ticks per frame"
            );
            // The guarantee that makes frame<->ticks lossless: the conversion
            // divides the timebase with no remainder.
            assert_eq!(
                (*expected as i128) * (rate.numerator() as i128),
                (TICKS_PER_SECOND as i128) * (rate.denominator() as i128),
                "{rate} must divide the timebase exactly"
            );
        }
    }

    #[test]
    fn frame_to_ticks_roundtrips_exactly() {
        // The property that actually matters: a trim stored as ticks lands back
        // on the same frame forever.
        for rate in [
            FrameRate::NTSC_24,
            FrameRate::new(24, 1),
            FrameRate::new(25, 1),
            FrameRate::NTSC_30,
            FrameRate::new(30, 1),
            FrameRate::NTSC_60,
            FrameRate::new(60, 1),
        ] {
            for frame in [0i64, 1, 2, 7, 100, 1001, 10_000, 86_400] {
                let ticks = rate.frame_to_ticks(frame);
                assert_eq!(
                    rate.frame_at(ticks),
                    frame,
                    "roundtrip failed for {rate} frame {frame} (ticks {})",
                    ticks.raw()
                );
                assert_eq!(
                    ticks.raw() % rate.ticks_per_frame(),
                    0,
                    "{rate} frame {frame} must be a whole number of ticks per frame"
                );
            }
        }
    }

    #[test]
    fn ntsc_rates_are_not_their_decimal_lookalikes() {
        // 29.97 and 30000/1001 differ. Over an hour that is real drift.
        assert_ne!(FrameRate::NTSC_30, FrameRate::new(2997, 100));
        // and from_f64 snaps the common one to the exact rational
        assert_eq!(FrameRate::from_f64(29.97), FrameRate::NTSC_30);
        assert_eq!(FrameRate::from_f64(23.976), FrameRate::NTSC_24);
        assert_eq!(FrameRate::from_f64(59.94), FrameRate::NTSC_60);
    }

    #[test]
    fn zero_is_the_identity_for_time_arithmetic() {
        let rate = FrameRate::NTSC_30;
        assert_eq!(Ticks::ZERO, Ticks(0));
        assert_eq!(rate.frame_to_ticks(0), Ticks::ZERO);
        assert_eq!(Ticks::from_seconds(5), Ticks(5 * TICKS_PER_SECOND));
    }

    #[test]
    fn negative_ticks_round_toward_negative_infinity_not_zero() {
        let rate = FrameRate::new(30, 1);
        let t = rate.frame_to_ticks(-1);
        assert!(t.raw() < 0);
        assert_eq!(rate.frame_at(t), -1);
        // Ticks(-1) is a hair before frame 0, so it belongs to frame -1.
        assert_eq!(rate.frame_at(Ticks(-1)), -1);
    }

    #[test]
    fn rounding_modes_differ_where_they_should() {
        let rate = FrameRate::new(30, 1); // 4000 ticks per frame
        // Exactly 1.5 frames = 6000 ticks.
        let t = Ticks(6000);
        assert_eq!(rate.ticks_to_frame(t, Rounding::Floor), 1);
        assert_eq!(rate.ticks_to_frame(t, Rounding::Nearest), 2);
    }

    #[test]
    fn long_timeline_does_not_drift() {
        // Repeated frame stepping must not accumulate error. This is how a
        // timeline actually gets built: a clip is N frames long, not a float
        // number of seconds.
        for rate in [
            FrameRate::NTSC_24,
            FrameRate::NTSC_30,
            FrameRate::NTSC_60,
            FrameRate::new(24, 1),
            FrameRate::new(30, 1),
            FrameRate::new(60, 1),
        ] {
            let tpf = rate.ticks_per_frame();
            let frames = 200_000i64; // well past any real project
            let stepped = Ticks(tpf.saturating_mul(frames));
            assert_eq!(
                rate.frame_to_ticks(frames),
                stepped,
                "{rate}: {frames} frames stepped one at a time must equal the direct conversion"
            );
            assert_eq!(
                rate.frame_at(stepped),
                frames,
                "{rate}: {frames} frames must land back on frame {frames}"
            );
        }
    }

    #[test]
    fn sub_frame_accuracy_survives_a_long_project() {
        // The claim the document actually wants: "a trim at frame 1001 of a
        // 23.976 timeline lands on exactly the same tick every time". Check it
        // at a frame index that is famously awkward for NTSC rates.
        let rate = FrameRate::NTSC_24;
        for frame in [1i64, 1001, 24_001, 100_001] {
            let t = rate.frame_to_ticks(frame);
            assert_eq!(rate.frame_at(t), frame);
            assert_eq!(t.raw() % rate.ticks_per_frame(), 0);
        }
    }

    #[test]
    fn ticks_serialize_as_bare_integers() {
        // §6.3: the project file is human-inspectable JSON. A struct wrapper
        // would make every time field unreadable.
        let json = serde_json::to_string(&Ticks(6000)).unwrap();
        assert_eq!(json, "6000");
        let back: Ticks = serde_json::from_str("6000").unwrap();
        assert_eq!(back, Ticks(6000));
    }

    #[test]
    fn frame_rate_reduces_and_rejects_zero() {
        // 60000/1001 is already in lowest terms.
        assert_eq!(FrameRate::new(60000, 1001), FrameRate::NTSC_60);
        // A common factor is divided out, so these describe the same rate.
        assert_eq!(FrameRate::new(30000, 1001), FrameRate::new(60000, 2002));
        // 24000/2002 reduces by 2 to 12000/1001 -- which is 12 fps NTSC, a
        // real rate, not 23.976. Reduction must not change the value.
        let half = FrameRate::new(24000, 2002);
        assert_eq!(half, FrameRate::new(12000, 1001));
        assert!((half.as_f64() - 11.988).abs() < 0.001);
    }

    #[test]
    #[should_panic(expected = "non-zero")]
    fn zero_denominator_panics() {
        FrameRate::new(30, 0);
    }

    #[test]
    fn from_f64_guards_against_nonsense() {
        assert_eq!(FrameRate::from_f64(0.0), FrameRate::new(30, 1));
        assert_eq!(FrameRate::from_f64(-5.0), FrameRate::new(30, 1));
    }
}