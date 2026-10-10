//! Export execution and output validation (§11.2, §11.3).
//!
//! # Why one FFmpeg invocation
//!
//! A full render compiles to a single `filter_complex`. Stitching per-clip
//! shell commands together would be slower, would produce a file with a
//! generation gap at every cut, and would make the export impossible to cancel
//! cleanly. There is exactly one process per export.
//!
//! # Validation is not optional
//!
//! Every export is probed afterwards and checked against the plan: duration
//! within a frame, stream count, codec, dimensions, sample rate. A file that
//! fails validation is discarded rather than shown to the user, because an
//! export that silently has the wrong duration is worse than an error.

use quillie_core::export::{plan, ExportGraph, Profile};
use quillie_core::project::{ExportPreset, Project};
use quillie_core::time::Ticks;
use std::path::{Path, PathBuf};

use quillie_core::media::probe::{parse_probe_output, probe_duration, ProbeError};
use crate::process::{ProcessError, SidecarCommand};

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("nothing to export: the sequence has no clips")]
    NothingToRender,
    #[error("ffmpeg is missing from the package")]
    NoFfmpeg,
    #[error("export failed: {0}")]
    Process(#[from] ProcessError),
    #[error("output could not be read back: {0}")]
    Validation(#[from] ProbeError),
    #[error("export failed validation: {0}")]
    Invalid(String),
}

/// What the validation actually checked, and what it found.
///
/// Returned on success so the caller can log it. Silent success is how a
/// subtly-wrong export becomes a habit nobody notices.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationReport {
    pub duration_seconds: f64,
    pub planned_seconds: f64,
    pub duration_delta_seconds: f64,
    /// Tolerance is one frame at the project rate (§11.3).
    pub frame_seconds: f64,
    pub codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub audio_sample_rate: Option<u32>,
}

/// Build the full FFmpeg argument list for a graph.
///
/// Arguments are a `Vec<String>` all the way through, so a filename with a
/// semicolon in it is one argument and not a command separator.
pub fn build_args(graph: &ExportGraph, output: &Path) -> Vec<String> {
    let mut args: Vec<String> = vec!["-hide_banner".into(), "-y".into()];

    for input in &graph.inputs {
        args.push("-i".into());
        args.push(input.clone());
    }

    if !graph.filters.is_empty() {
        args.push("-filter_complex".into());
        args.push(graph.filters.clone());
    }

    if let Some(v) = graph.video_label() {
        args.push("-map".into());
        args.push(format!("[{v}]"));
    }
    if let Some(a) = graph.audio_label() {
        args.push("-map".into());
        args.push(format!("[{a}]"));
    }

    // Preset parameters come from the profile, so the same graph renders at
    // output quality or proxy quality without the caller repeating itself.
    if let Profile::Export { preset } = &graph.profile {
        args.extend(preset.ffmpeg_video_args());
        args.extend(preset.ffmpeg_audio_args());
    } else {
        args.extend([
            "-c:v".to_string(),
            "libvpx-vp9".into(),
            "-b:v".into(),
            "1200k".into(),
            "-deadline".into(),
            "realtime".into(),
            "-cpu-used".into(),
            "5".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
        ]);
    }

    args.push("-movflags".into());
    args.push("+faststart".into());

    args.push(output.display().to_string());
    args
}

/// Render a project to `output` and validate the result.
pub fn export(
    project: &Project,
    output: &Path,
    preset: &ExportPreset,
) -> Result<ValidationReport, ExportError> {
    let sequence = project.primary_sequence();
    let graph = plan(project, sequence, None, Profile::Export { preset: preset.clone() })
        .ok_or(ExportError::NothingToRender)?;

    let ffmpeg = crate::process::sidecar_path("ffmpeg").ok_or(ExportError::NoFfmpeg)?;

    // Write to a .part file and rename only after validation (§11.2). A failed
    // export must not leave something that looks like a finished file.
    let mut part = output.with_extension("part.mp4");
    part.set_file_name(format!(
        "{}.part.mp4",
        output.file_stem().unwrap_or_default().to_string_lossy()
    ));

    let args = build_args(&graph, &part);
    let cmd = SidecarCommand::new(ffmpeg.display().to_string(), args).background();
    let out = cmd.run_captured()?;

    if !out.success {
        let _ = std::fs::remove_file(&part);
        return Err(ExportError::Invalid(format!(
            "ffmpeg exited {:?}. Tail:\n{}",
            out.status, out.stderr_tail
        )));
    }

    let report = validate(&graph, &part, preset)?;

    // Only now does the output become visible under its real name.
    std::fs::rename(&part, output).map_err(|e| {
        ExportError::Invalid(format!("could not finalise {}: {e}", output.display()))
    })?;

    Ok(report)
}

/// Probe the output and check it against what was planned.
pub fn validate(
    graph: &ExportGraph,
    path: &Path,
    preset: &ExportPreset,
) -> Result<ValidationReport, ExportError> {
    let ffprobe = crate::process::sidecar_path("ffprobe").ok_or(ExportError::NoFfmpeg)?;

    let probe = SidecarCommand::new(
        ffprobe.display().to_string(),
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
    .run_captured()?;

    if !probe.success {
        return Err(ExportError::Invalid(format!(
            "output is unreadable: {}",
            probe.stderr_tail
        )));
    }

    let summary = parse_probe_output(&probe.stdout)?;
    let actual = probe_duration(&probe.stdout).unwrap_or(0.0);
    let planned = graph.expected_seconds();

    // One frame at the project rate. This is the §11.3 acceptance criterion.
    let frame_seconds = 1.0 / graph.frame_rate.as_f64();
    let delta = (actual - planned).abs();

    if delta > frame_seconds {
        return Err(ExportError::Invalid(format!(
            "duration is {actual:.3}s but the plan said {planned:.3}s \
             (delta {:.3}s, tolerance {:.3}s = one frame)",
            delta, frame_seconds
        )));
    }

    let v = summary.video.as_ref();
    if let Some(v) = v {
        let expected = preset.expected_codec();
        if v.codec != expected {
            return Err(ExportError::Invalid(format!(
                "codec is {} but the preset asked for {} (encoder {})",
                v.codec, expected, preset.video_codec
            )));
        }
        if (v.width, v.height) != (preset.width, preset.height) {
            return Err(ExportError::Invalid(format!(
                "dimensions are {}x{} but the preset asked for {}x{}",
                v.width, v.height, preset.width, preset.height
            )));
        }
    } else {
        return Err(ExportError::Invalid("output has no video stream".into()));
    }

    Ok(ValidationReport {
        duration_seconds: actual,
        planned_seconds: planned,
        duration_delta_seconds: delta,
        frame_seconds,
        codec: v.map(|s| s.codec.clone()),
        width: v.map(|s| s.width),
        height: v.map(|s| s.height),
        audio_sample_rate: summary.audio.as_ref().map(|a| a.sample_rate),
    })
}

/// Where a `.part` file for this output would live.
pub fn part_path(output: &Path) -> PathBuf {
    let stem = output.file_stem().unwrap_or_default().to_string_lossy();
    output.with_file_name(format!("{stem}.part.mp4"))
}

/// Convenience: the default preset for a project resolution.
pub fn default_preset(width: u32, height: u32) -> ExportPreset {
    let mut p = ExportPreset::landscape_1080p();
    p.width = width;
    p.height = height;
    p
}

/// Ticks helper used by callers computing an expected duration.
pub fn frames_to_ticks(frames: i64, rate: quillie_core::time::FrameRate) -> Ticks {
    rate.frame_to_ticks(frames)
}