//! ffprobe output parsing (§7.1).
//!
//! # Why parsing lives in core
//!
//! Import never decodes frames, and it must run in tests with no FFmpeg
//! installed (§4). So the *parsing* is a pure function over ffprobe's JSON,
//! lives here, and is tested against captured output. The *invocation* lives in
//! `quillie-app::process`, which is the only layer that knows where the
//! sidecar binary is.
//!
//! # What is deliberately not parsed
//!
//! Nothing is guessed. A stream with no frame rate is `None`, not 30. A file
//! with no audio stream has `audio: None`, not a silent track. Guessing here
//! would produce a project that opens fine and then exports wrong, which is
//! much harder to diagnose than a missing field.

use crate::project::{AudioStreamInfo, StreamSummary, VideoStreamInfo};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ProbeError {
    #[error("ffprobe produced no JSON")]
    Empty,
    #[error("ffprobe JSON could not be parsed: {0}")]
    Json(String),
    #[error("file has no video stream")]
    NoVideo,
}

/// The subset of ffprobe's JSON that matters.
///
/// Unknown fields are ignored rather than rejected: ffprobe adds keys between
/// releases and an import must not fail because of that.
#[derive(Debug, serde::Deserialize)]
pub struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct ProbeFormat {
    // ffprobe reports duration as a string in some containers and a number in
    // others. Neither form is worth failing an import over.
    #[serde(default, deserialize_with = "de_opt_f64")]
    duration: Option<f64>,
}

fn de_opt_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        Num(f64),
        Str(String),
    }
    Ok(match <Option<NumOrStr> as serde::Deserialize>::deserialize(d)? {
        Some(NumOrStr::Num(n)) => Some(n),
        Some(NumOrStr::Str(s)) => s.trim().parse().ok(),
        None => None,
    })
}

#[derive(Debug, serde::Deserialize)]
pub struct ProbeStream {
    #[serde(default)]
    codec_type: String,
    #[serde(default)]
    codec_name: String,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    pix_fmt: Option<String>,
    #[serde(default, deserialize_with = "de_opt_u16")]
    bits_per_raw_sample: Option<u16>,
    #[serde(default)]
    avg_frame_rate: Option<String>,
    #[serde(default)]
    r_frame_rate: Option<String>,
    #[serde(default, deserialize_with = "de_opt_u64")]
    nb_frames: Option<u64>,
    #[serde(default)]
    sample_rate: Option<String>,
    #[serde(default, deserialize_with = "de_opt_u16")]
    channels: Option<u16>,
    #[serde(default)]
    channel_layout: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    tags: std::collections::BTreeMap<String, String>,
    // Disposition carries `rotate`, which is how a phone-shot video records
    // its orientation. Missing this makes portrait footage play landscape.
    #[serde(default)]
    disposition: std::collections::BTreeMap<String, serde_json::Value>,
}

/// ffprobe is inconsistent about numeric fields: some containers report
/// `bits_per_raw_sample` as a number, others as a quoted string. Accept both,
/// because rejecting a valid file at import is much worse than the ambiguity.
fn de_opt_u16<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u16>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        Num(u16),
        Str(String),
    }
    Ok(match <Option<NumOrStr> as serde::Deserialize>::deserialize(d)? {
        Some(NumOrStr::Num(n)) => Some(n),
        Some(NumOrStr::Str(s)) => s.trim().parse().ok(),
        None => None,
    })
}

fn de_opt_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum NumOrStr {
        Num(u64),
        Str(String),
    }
    Ok(match <Option<NumOrStr> as serde::Deserialize>::deserialize(d)? {
        Some(NumOrStr::Num(n)) => Some(n),
        Some(NumOrStr::Str(s)) => s.trim().parse().ok(),
        None => None,
    })
}

/// Parse the `avg_frame_rate` / `r_frame_rate` "num/den" form.
///
/// ffprobe reports these as strings and uses `"0/0"` for "unknown". Returning
/// `None` there is the honest answer; substituting 30 would silently convert a
/// VFR source into a CFR one, which the spec forbids (§6.1).
pub fn parse_rational(s: &str) -> Option<(u32, u32)> {
    let (n, d) = s.split_once('/')?;
    let num: u32 = n.trim().parse().ok()?;
    let den: u32 = d.trim().parse().ok()?;
    if num == 0 || den == 0 {
        return None;
    }
    Some((num, den))
}

/// Detect variable frame rate.
///
/// A file is VFR when its average frame rate disagrees with its nominal
/// `r_frame_rate` beyond a small tolerance, or when the nominal rate is
/// unknown. Either means frame timings cannot be trusted to be uniform, and the
/// media panel must flag it (§6.1).
pub fn is_vfr(stream: &ProbeStream) -> bool {
    match (stream.avg_frame_rate.as_deref(), stream.r_frame_rate.as_deref()) {
        (Some(avg), Some(nominal)) => match (parse_rational(avg), parse_rational(nominal)) {
            (Some((an, ad)), Some((nn, nd))) => {
                let a = an as f64 / ad as f64;
                let n = nn as f64 / nd as f64;
                // More than ~0.1% apart is not a rounding difference.
                (a - n).abs() > n * 0.001
            }
            _ => true,
        },
        // A video stream with no usable rate information is treated as VFR:
        // it is safer to flag and let the user decide than to assume CFR.
        _ => true,
    }
}

/// Read the rotation tag, which ffprobe reports on the *display* matrix side.
///
/// It appears in two forms depending on the muxer: as an integer in
/// `disposition`, and as a string in `tags` ("rotate" = "-90"). Both are
/// checked, and the value is normalised into 0..360.
fn rotation_from(stream: &ProbeStream) -> u16 {
    let from_disposition = stream
        .disposition
        .get("rotate")
        .and_then(|v| match v {
            serde_json::Value::Number(n) => n.as_i64(),
            serde_json::Value::String(s) => s.trim().parse::<i64>().ok(),
            _ => None,
        });
    let from_tag = stream.tags.get("rotate").and_then(|v| v.trim().parse::<i64>().ok());
    from_disposition
        .or(from_tag)
        .map(|v| v.rem_euclid(360) as u16)
        .unwrap_or(0)
}

/// Parse ffprobe JSON into a stream summary.
pub fn parse_probe_output(json: &str) -> Result<StreamSummary, ProbeError> {
    let out: ProbeOutput =
        serde_json::from_str(json).map_err(|e| ProbeError::Json(e.to_string()))?;

    let mut summary = StreamSummary::default();

    for s in &out.streams {
        match s.codec_type.as_str() {
            "video" if summary.video.is_none() => {
                summary.video = Some(VideoStreamInfo {
                    codec: s.codec_name.clone(),
                    width: s.width,
                    height: s.height,
                    pix_fmt: s.pix_fmt.clone(),
                    bits_per_raw_sample: s.bits_per_raw_sample,
                    avg_frame_rate: s
                        .avg_frame_rate
                        .as_deref()
                        .and_then(parse_rational)
                        .or_else(|| s.r_frame_rate.as_deref().and_then(parse_rational)),
                    nb_frames: s.nb_frames,
                });
            }
            "audio" if summary.audio.is_none() => {
                summary.audio = Some(AudioStreamInfo {
                    codec: s.codec_name.clone(),
                    sample_rate: s
                        .sample_rate
                        .as_deref()
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(48_000),
                    channels: s.channels.unwrap_or(2),
                    channel_layout: s.channel_layout.clone(),
                });
            }
            "subtitle" => summary.has_subtitles = true,
            _ => {}
        }
    }

    if summary.video.is_none() {
        return Err(ProbeError::NoVideo);
    }
    Ok(summary)
}

/// Rotation for a probed video stream.
pub fn rotation(json: &str) -> Option<u16> {
    let out: ProbeOutput = serde_json::from_str(json).ok()?;
    out.streams
        .iter()
        .find(|s| s.codec_type == "video")
        .map(rotation_from)
}

/// Container duration in seconds.
///
/// Tries the format-level value first and falls back to the video stream,
/// because some containers (raw streams, fragmented MP4 written by a phone)
/// only carry a per-stream duration.
pub fn probe_duration(json: &str) -> Option<f64> {
    let out: ProbeOutput = serde_json::from_str(json).ok()?;
    out.format.duration.or_else(|| {
        out.streams
            .iter()
            .filter(|s| s.codec_type == "video" || s.codec_type == "audio")
            .filter_map(|s| s.duration)
            .fold(None, |acc: Option<f64>, d| Some(acc.map_or(d, |a| a.max(d))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_h264_mp4() {
        let json = r#"{
          "streams": [
            {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
             "pix_fmt":"yuv420p","avg_frame_rate":"30000/1001","r_frame_rate":"30000/1001",
             "nb_frames":900,"bits_per_raw_sample":"8"},
            {"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2,
             "channel_layout":"stereo"}
          ],
          "format": {"duration":"30.0"}
        }"#;
        let s = parse_probe_output(json).unwrap();
        let v = s.video.unwrap();
        assert_eq!(v.codec, "h264");
        assert_eq!((v.width, v.height), (1920, 1080));
        assert_eq!(v.avg_frame_rate, Some((30000, 1001)));
        let a = s.audio.unwrap();
        assert_eq!(a.sample_rate, 48_000);
        assert_eq!(a.channels, 2);
    }

    #[test]
    fn parses_ten_bit_and_4k_which_drive_the_proxy_decision() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"hevc","width":3840,
          "height":2160,"bits_per_raw_sample":"10","avg_frame_rate":"24000/1001",
          "r_frame_rate":"24000/1001"}]}"#;
        let s = parse_probe_output(json).unwrap();
        let v = s.video.unwrap();
        assert_eq!(v.bits_per_raw_sample, Some(10));
        assert_eq!((v.width, v.height), (3840, 2160));
    }

    #[test]
    fn an_unknown_frame_rate_is_none_not_guessed() {
        // "0/0" means unknown. Returning 30 here would silently convert a VFR
        // source into a CFR one.
        assert_eq!(parse_rational("0/0"), None);
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":640,
          "height":480,"avg_frame_rate":"0/0","r_frame_rate":"0/0"}]}"#;
        let s = parse_probe_output(json).unwrap();
        assert_eq!(s.video.unwrap().avg_frame_rate, None);
    }

    #[test]
    fn a_file_with_no_video_is_an_error() {
        let json = r#"{"streams":[{"codec_type":"audio","codec_name":"aac"}]}"#;
        assert_eq!(parse_probe_output(json), Err(ProbeError::NoVideo));
    }

    #[test]
    fn unknown_fields_do_not_break_the_parse() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":100,
          "height":100,"some_future_field":{"nested":true}}],
          "format":{"duration":"1.0","new_key":"x"}}"#;
        assert!(parse_probe_output(json).is_ok());
    }

    #[test]
    fn phone_rotation_is_read_from_the_disposition_side() {
        // Phone video stores orientation as a rotation in the display matrix.
        // Missing this makes portrait footage play landscape.
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":1920,
          "height":1080,"avg_frame_rate":"30000/1001","r_frame_rate":"30000/1001",
          "disposition":{"rotate":90}}]}"#;
        assert_eq!(rotation(json), Some(90));
    }

    #[test]
    fn rotation_normalises_into_range() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":1920,
          "height":1080,"tags":{"rotate":"-90"}}]}"#;
        assert_eq!(rotation(json), Some(270));
    }

    #[test]
    fn no_rotation_tag_means_zero() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":1920,
          "height":1080}]}"#;
        assert_eq!(rotation(json), Some(0));
    }
}