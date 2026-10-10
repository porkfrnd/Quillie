//! Media import: ffprobe parsing, fingerprints, and proxy policy.
//!
//! Import never decodes frames (§7.1). Everything here is either a pure
//! function over ffprobe's output or a policy decision, so it is testable
//! without FFmpeg installed.

pub mod fingerprint;
pub mod probe;

pub use fingerprint::{fingerprint_file, FingerprintError};
pub use probe::{
    is_vfr, parse_probe_output, probe_duration, rotation, ProbeError, ProbeOutput, ProbeStream,
};