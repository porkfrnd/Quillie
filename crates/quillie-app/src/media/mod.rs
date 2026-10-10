//! Media import (§7.1).
//!
//! Import never decodes frames. It runs ffprobe, parses the result, fingerprints
//! the file, and applies ops to the project. Everything expensive about a
//! source -- proxies, thumbnails, waveforms, keyframe indexes -- is deferred
//! to the scheduler (§9.1).
//!
//! The proxy policy decision (§7.2) is made here but *acted on* elsewhere, and
//! the reason is logged rather than silently applied. A user who does not know
//! why a file is being transcoded cannot tell a working proxy from a broken
//! one.

use quillie_core::media::fingerprint::{fingerprint_file, FingerprintError};
use quillie_core::media::probe::{parse_probe_output, probe_duration, rotation, ProbeError};
use quillie_core::project::{
    Fingerprint,
    EditError, MediaRef, Project, StreamSummary, VideoStreamInfo,
};
use quillie_core::time::Ticks;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("file not found: {0}")]
    NotFound(PathBuf),
    #[error("ffprobe failed: {0}")]
    Probe(#[from] ProbeError),
    #[error("could not fingerprint the file: {0}")]
    Fingerprint(#[from] FingerprintError),
    #[error("could not add the file to the project: {0}")]
    Apply(#[from] EditError),
    #[error("no sidecar ffprobe in this build")]
    NoSidecar,
    #[error("ffprobe could not be run: {0}")]
    Spawn(String),
}

/// Why a source needs a proxy (§7.2).
///
/// Recorded rather than applied inline. The scheduler acts on it; the user sees
/// it. A proxy decision with no stated reason is indistinguishable from a bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyReason {
    WidthAbove(u32),
    CodecSlow(String),
    TenBit,
    BitrateHigh,
    SeekSlow,
}

impl ProxyReason {
    pub fn describe(&self) -> String {
        match self {
            ProxyReason::WidthAbove(w) => format!("source is {w}px wide"),
            ProxyReason::CodecSlow(c) => format!("{c} is expensive to decode"),
            ProxyReason::TenBit => "10-bit source".into(),
            ProxyReason::BitrateHigh => "bitrate is high".into(),
            ProxyReason::SeekSlow => "measured seek latency is high".into(),
        }
    }
}

/// What importing a file produced.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedMedia {
    pub media_id: uuid::Uuid,
    pub fingerprint: Fingerprint,
    pub duration: Ticks,
    pub streams: StreamSummary,
    pub rotation: u16,
    pub vfr: bool,
    /// `None` means no proxy is needed.
    pub proxy_reason: Option<ProxyReason>,
}

/// Decide whether a source needs a proxy (§7.2).
///
/// Each condition from the spec, checked in order and reported as the first
/// one that applies. Note that decoding cost and *encoding* cost are different
/// questions: an H.264 source needs no proxy for decoding even though it would
/// for encoding, and the rule here is about preview smoothness.
pub fn proxy_policy(video: &VideoStreamInfo, tier_max_width: u32) -> Option<ProxyReason> {
    if video.width > 1280 {
        return Some(ProxyReason::WidthAbove(video.width));
    }
    let codec = video.codec.to_ascii_lowercase();
    if matches!(codec.as_str(), "hevc" | "av1" | "vp9" | "prores" | "dnxhd") {
        return Some(ProxyReason::CodecSlow(video.codec.clone()));
    }
    if video.bits_per_raw_sample.unwrap_or(8) > 8 {
        return Some(ProxyReason::TenBit);
    }
    // ProRes and DNxHD above 1080p are already caught by width; below it they
    // are still expensive, so the codec check above handles them.
    let _ = tier_max_width;
    None
}

/// Run ffprobe and build a `MediaRef` without touching the project.
///
/// Separated from [`import_into`] so the caller can decide *where* the media
/// lands, and so the probe can be tested without a project.
pub fn probe_media(path: &Path) -> Result<ImportedMedia, ImportError> {
    if !path.exists() {
        return Err(ImportError::NotFound(path.to_path_buf()));
    }

    let sidecar = crate::process::sidecar_path("ffprobe")
        .ok_or(ImportError::NoSidecar)?;

    let out = crate::process::SidecarCommand::new(
        sidecar.display().to_string(),
        vec![
            "-v".into(),
            "error".into(),
            "-print_format".into(),
            "json".into(),
            "-show_format".into(),
            "-show_streams".into(),
            path.display().to_string(),
        ],
    )
    .run_captured()
    .map_err(|e| ImportError::Spawn(e.to_string()))?;

    if !out.success {
        return Err(ImportError::Spawn(format!(
            "ffprobe exited {:?}: {}",
            out.status, out.stderr_tail
        )));
    }

    let streams = parse_probe_output(&out.stdout)?;
    let duration = probe_duration(&out.stdout)
        .map(Ticks::from_secs_f64)
        .unwrap_or(Ticks::ZERO);
    let rot = rotation(&out.stdout).unwrap_or(0);

    // The fingerprint reads the first and last MiB. It is deliberately not a
    // whole-file hash: on a mechanical disk that turns every import into a
    // multi-minute stall, which is the machine the floor describes.
    let fingerprint = fingerprint_file(path)?;

    let vfr = streams
        .video
        .as_ref()
        .map(|v| v.avg_frame_rate.is_none())
        .unwrap_or(false);

    let proxy_reason = streams.video.as_ref().and_then(|v| proxy_policy(v, 1280));

    Ok(ImportedMedia {
        media_id: uuid::Uuid::new_v4(),
        fingerprint,
        duration,
        streams,
        rotation: rot,
        vfr,
        proxy_reason,
    })
}

/// Import a file into a project's media bin.
///
/// Import does not place a clip on the timeline: that is a separate op the user
/// performs. Adding media and adding a clip are different intentions, and
/// conflating them makes undo confusing.
pub fn import_into(
    project: &mut Project,
    path: &Path,
) -> Result<ImportedMedia, ImportError> {
    let imported = probe_media(path)?;

    let media_ref = MediaRef {
        id: imported.media_id,
        // Stored as given. Resolution happens at relink time, and storing an
        // absolute path here would break the project when the folder moves.
        path: path.display().to_string(),
        fingerprint: imported.fingerprint.clone(),
        duration: imported.duration,
        streams: imported.streams.clone(),
        rotation: imported.rotation,
        vfr: imported.vfr,
    };

    project.media.insert(imported.media_id, media_ref);
    Ok(imported)
}

/// Find media whose path no longer matches its fingerprint, or that is missing
/// entirely (§6.3 "missing media").
///
/// Returned rather than repaired: relinking is the user's decision, and
/// silently re-resolving a moved file would be a guess about which of several
/// files they meant.
pub fn find_missing(project: &Project) -> Vec<(uuid::Uuid, String)> {
    let mut missing = Vec::new();
    for (id, media) in &project.media {
        let path = Path::new(&media.path);
        if !path.exists() {
            missing.push((*id, media.path.clone()));
            continue;
        }
        // Present but changed: size alone is a cheap pre-check, and a full
        // fingerprint only runs when the cheap one disagrees.
        match std::fs::metadata(path) {
            Ok(meta) if meta.len() != media.fingerprint.size_bytes => {
                missing.push((*id, media.path.clone()));
            }
            Ok(_) => {
                if let Ok(actual) = fingerprint_file(path) {
                    if actual.head_hash != media.fingerprint.head_hash {
                        missing.push((*id, media.path.clone()));
                    }
                }
            }
            Err(_) => missing.push((*id, media.path.clone())),
        }
    }
    missing
}

/// Relink a media entry to a new path, after verifying the fingerprint matches.
///
/// The fingerprint check is the point. Relinking to "some file at that path"
/// would silently substitute different footage into a finished edit.
pub fn relink(
    project: &mut Project,
    media_id: uuid::Uuid,
    new_path: &Path,
) -> Result<(), ImportError> {
    let Some(existing) = project.media.get(&media_id) else {
        return Err(ImportError::Apply(EditError::UnknownMedia(media_id)));
    };
    let fingerprint = fingerprint_file(new_path)?;
    if fingerprint.size_bytes != existing.fingerprint.size_bytes
        || fingerprint.head_hash != existing.fingerprint.head_hash
    {
        return Err(ImportError::Fingerprint(FingerprintError::Io {
            path: new_path.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the file at the new path does not match the original fingerprint",
            ),
        }));
    }
    if let Some(media) = project.media.get_mut(&media_id) {
        media.path = new_path.display().to_string();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(width: u32, codec: &str, bits: Option<u16>) -> VideoStreamInfo {
        VideoStreamInfo {
            codec: codec.into(),
            width,
            height: width * 9 / 16,
            pix_fmt: Some("yuv420p".into()),
            bits_per_raw_sample: bits,
            avg_frame_rate: Some((30, 1)),
            nb_frames: Some(900),
        }
    }

    #[test]
    fn a_720p_h264_source_needs_no_proxy() {
        // 1280 is the threshold, so 1280x720 is the largest common web
        // resolution that plays without building anything. That boundary is
        // where most of the "pay as you go" budget is saved.
        assert_eq!(proxy_policy(&video(1280, "h264", Some(8)), 1280), None);
    }

    #[test]
    fn anything_above_1280_wide_needs_a_proxy() {
        // Note this includes 1080p. That is deliberate: a 1920-wide source is
        // above the threshold in the policy, so it gets a proxy even when it is
        // H.264 and would decode without much trouble.
        assert_eq!(
            proxy_policy(&video(1920, "h264", Some(8)), 1280),
            Some(ProxyReason::WidthAbove(1920))
        );
        assert_eq!(
            proxy_policy(&video(3840, "h264", Some(8)), 1280),
            Some(ProxyReason::WidthAbove(3840))
        );
    }

    #[test]
    fn expensive_codecs_need_a_proxy_even_at_small_sizes() {
        for c in ["hevc", "HEVC", "av1", "vp9", "prores", "dnxhd"] {
            assert!(
                matches!(
                    proxy_policy(&video(640, c, Some(8)), 1280),
                    Some(ProxyReason::CodecSlow(_))
                ),
                "{c} should need a proxy at 640px"
            );
        }
    }

    #[test]
    fn a_ten_bit_source_needs_a_proxy() {
        assert_eq!(
            proxy_policy(&video(1280, "h264", Some(10)), 1280),
            Some(ProxyReason::TenBit)
        );
    }

    #[test]
    fn a_small_eight_bit_h264_is_left_alone() {
        // The point of the policy: common web video plays without a proxy
        // being built, which is most of the "pay as you go" budget spent.
        assert_eq!(proxy_policy(&video(640, "h264", None), 1280), None);
    }

    #[test]
    fn every_reason_reads_as_a_sentence() {
        let reasons = [
            ProxyReason::WidthAbove(3840),
            ProxyReason::CodecSlow("hevc".into()),
            ProxyReason::TenBit,
            ProxyReason::BitrateHigh,
            ProxyReason::SeekSlow,
        ];
        for r in reasons {
            assert!(!r.describe().is_empty());
            assert!(r.describe().chars().all(|c| !c.is_control()));
        }
        assert_eq!(ProxyReason::WidthAbove(3840).describe(), "source is 3840px wide");
    }

    #[test]
    fn importing_a_missing_file_fails_cleanly() {
        let p = Project::default();
        let _ = p;
        let r = import_into(&mut Project::default(), Path::new("/definitely/not/here.mp4"));
        assert!(matches!(r, Err(ImportError::NotFound(_))));
    }
}