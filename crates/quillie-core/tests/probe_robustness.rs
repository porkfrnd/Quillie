//! ffprobe output is untrusted input.
//!
//! `parse_probe_output` is handed the stdout of an external process. That
//! output is shaped by whichever build of FFmpeg shipped in the sidecar, by
//! the container, and by the muxer that wrote the file -- and a corrupt or
//! hostile file can put almost anything in those fields. Nothing here should
//! ever panic, and nothing should be *silently* wrong: a stream that parses to
//! a nonsense rate has to come out as "unknown", not as a plausible-looking
//! frame rate that then drifts the entire timeline.
//!
//! These are unit tests against the pure parser, so they need no sidecar.

use quillie_core::media::probe::{
    is_vfr, parse_probe_output, parse_rational, rotation, ProbeError, ProbeStream,
};

/// A well-formed stream, so each test can spoil exactly one field.
fn good_stream() -> String {
    r#"{
      "codec_type": "video",
      "codec_name": "h264",
      "width": 1920,
      "height": 1080,
      "pix_fmt": "yuv420p",
      "avg_frame_rate": "30/1",
      "r_frame_rate": "30/1",
      "duration": "10.000000",
      "nb_frames": "300",
      "disposition": { "rotate": "0" },
      "tags": {}
    }"#
    .to_string()
}

fn wrap(streams: &[String]) -> String {
    let joined = streams
        .iter()
        .map(|s| s.trim())
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"streams\":[{joined}],\"format\":{{\"duration\":\"10.0\"}}}}")
}

#[test]
fn a_well_formed_probe_parses() {
    let s = parse_probe_output(&wrap(&[good_stream()])).expect("valid probe");
    let v = s.video.expect("video stream");
    assert_eq!(v.codec, "h264");
    assert_eq!(v.width, 1920);
    assert_eq!(v.avg_frame_rate, Some((30, 1)));
    assert!(!s.has_subtitles);
}

#[test]
fn malformed_json_is_an_error_not_a_panic() {
    for bad in [
        "",
        "{",
        "null",
        "[]",
        "{}",
        r#"{"streams": "not an array"}"#,
        r#"{"streams": [1, 2, 3]}"#,
        r#"{"streams": [null]}"#,
        r#"{"streams": [{"codec_type": 42}]}"#,
        "\u{0}\u{1}\u{2}",
        &"{\"streams\":[".repeat(500),
        &"}".repeat(500),
    ] {
        // The contract is Err, never panic.
        let r = parse_probe_output(bad);
        assert!(r.is_err(), "expected an error for {bad:?}");
    }
}

#[test]
fn a_container_with_no_video_stream_is_rejected() {
    let audio_only = r#"{"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2}"#;
    let err = parse_probe_output(&wrap(&[audio_only.to_string()]))
        .expect_err("audio-only must not import as a video");
    assert!(matches!(err, ProbeError::NoVideo));

    // An empty stream list is the same problem.
    assert!(parse_probe_output(&wrap(&[])).is_err());
}

#[test]
fn hostile_frame_rates_are_refused_rather_than_guessed_at() {
    // Each of these is something a real file has been observed to contain.
    for rate in [
        "0/0",     // undefined, extremely common in containers with no timing
        "0/1",     // zero numerator
        "1/0",     // zero denominator: would divide by zero
        "-30/1",   // negative
        "30/-1",   // negative denominator
        "abc/def", // not numbers
        "",        // empty
        "/",
        "30",    // no denominator
        "/1",    // no numerator
        " 30 / 1 ", // whitespace, which is fine and must still work
        "99999999999999999999/1", // overflows u32
        "30/99999999999999999999",
        "NaN/1",
        "Infinity/1",
    ] {
        let parsed = parse_rational(rate);
        if rate.trim() == "30 / 1" {
            assert_eq!(parsed, Some((30, 1)), "whitespace must be tolerated");
        } else {
            assert_eq!(parsed, None, "{rate:?} must not parse as a frame rate");
        }
    }
}

#[test]
fn a_nonsense_frame_rate_yields_unknown_not_a_wrong_answer() {
    let stream = good_stream().replace(r#""avg_frame_rate": "30/1""#, r#""avg_frame_rate": "0/0""#);
    let s = parse_probe_output(&wrap(&[stream])).expect("still parses");
    let v = s.video.unwrap();
    // Falls back to r_frame_rate if that is usable, otherwise nothing.
    assert!(
        v.avg_frame_rate.is_none() || v.avg_frame_rate == Some((30, 1)),
        "a 0/0 average must not become a fabricated rate, got {:?}",
        v.avg_frame_rate
    );
}

#[test]
fn no_frame_rate_at_all_yields_unknown() {
    let stream = good_stream()
        .replace(r#""avg_frame_rate": "30/1","#, "")
        .replace(r#""r_frame_rate": "30/1","#, "");
    let s = parse_probe_output(&wrap(&[stream])).expect("still parses");
    assert_eq!(s.video.unwrap().avg_frame_rate, None);
}

#[test]
fn vfr_detection_never_divides_by_zero() {
    // Built through the real deserializer rather than a struct literal: these
    // are exactly the field combinations a real container produces.
    let stream = |avg: Option<&str>, nom: Option<&str>| -> ProbeStream {
        let mut parts: Vec<String> = vec!["\"codec_type\":\"video\"".to_string()];
        if let Some(v) = avg {
            parts.push(format!("\"avg_frame_rate\":\"{v}\""));
        }
        if let Some(v) = nom {
            parts.push(format!("\"r_frame_rate\":\"{v}\""));
        }
        let json = format!("{{{}}}", parts.join(","));
        serde_json::from_str(&json).expect("stream shape is valid")
    };
    for avg in [None, Some("0/0"), Some("1/0"), Some("garbage"), Some("30/1")] {
        for nom in [None, Some("0/0"), Some("30/1"), Some("1/0"), Some("x/y")] {
            // Must simply return a bool.
            let _ = is_vfr(&stream(avg, nom));
        }
    }

    // Missing rate information means VFR: flagging is the safe answer.
    assert!(is_vfr(&stream(None, None)));
    assert!(is_vfr(&stream(Some("30/1"), None)));
    // Matching rates mean CFR.
    assert!(!is_vfr(&stream(Some("30/1"), Some("30/1"))));
    // Genuinely different rates mean VFR.
    assert!(is_vfr(&stream(Some("30/1"), Some("24000/1001"))));
}

#[test]
fn rotation_survives_every_shape_ffprobe_uses() {
    let cases: &[(&str, u16)] = &[
        (r#""disposition": {"rotate": "90"}"#, 90),
        (r#""disposition": {"rotate": 90}"#, 90),
        (r#""disposition": {"rotate": "-90"}"#, 270),
        (r#""disposition": {"rotate": "450"}"#, 90),
        (r#""disposition": {"rotate": "-450"}"#, 270),
        (r#""disposition": {"rotate": "360"}"#, 0),
        (r#""disposition": {"rotate": 0}"#, 0),
        (r#""tags": {"rotate": "270"}"#, 270),
        (r#""tags": {"rotate": " -90 "}"#, 270),
    ];
    for (tail, want) in cases {
        let json = video_only(tail);
        assert_eq!(rotation(&json), Some(*want), "for {tail} in {json}");
    }

    // Values that cannot be a rotation must not become one.
    for tail in [
        r#""disposition": {"rotate": "abc"}"#,
        r#""disposition": {"rotate": null}"#,
        r#""disposition": {"rotate": {}}"#,
        r#""disposition": {}"#,
        r#""tags": {"rotate": "1e400"}"#,
        r#""tags": {"rotate": ""}"#,
    ] {
        assert_eq!(rotation(&video_only(tail)), Some(0), "{tail} must normalise to 0");
    }

    // Absurd magnitudes must still normalise into 0..360 rather than wrapping
    // via `as u16`.
    let json = video_only(r#""disposition": {"rotate": "99999999999999999999"}"#);
    assert_eq!(rotation(&json), Some(0));
}

/// One video stream carrying only `tail`, inside an otherwise empty document.
///
/// Raw string, so the quotes need no escaping -- but `format!` still requires
/// doubled braces, and getting that wrong emits invalid JSON that makes every
/// assertion below pass for the wrong reason.
fn video_only(tail: &str) -> String {
    format!(r#"{{"streams":[{{"codec_type":"video",{tail}}}],"format":{{}}}}"#)
}

#[test]
fn an_audio_stream_with_absurd_values_falls_back_instead_of_breaking_audio() {
    let bad = r#"{"codec_type":"audio","codec_name":"aac","sample_rate":"not a number","channels":2}"#;
    let json = wrap(&[good_stream(), bad.to_string()]);
    let s = parse_probe_output(&json).expect("video is still there");
    let a = s.audio.expect("audio stream");
    assert_eq!(a.sample_rate, 48_000, "an unparseable rate falls back to 48k");
    assert_eq!(a.channels, 2);
}

#[test]
fn an_audio_stream_reporting_more_channels_than_u16_can_hold_is_survivable() {
    // A damaged or hand-crafted header can report a channel count outside the
    // field's range. The parser must fall back rather than reject the whole
    // file -- losing the video over a nonsense audio field helps nobody.
    let bad = r#"{"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":999999}"#;
    let json = wrap(&[good_stream(), bad.to_string()]);
    let s = parse_probe_output(&json).expect("the video is still usable");
    assert!(s.video.is_some());
    let a = s.audio.expect("audio stream");
    assert!(
        (1..=64).contains(&a.channels),
        "an implausible channel count must fall back to something sane, got {}",
        a.channels
    );
}

#[test]
fn only_the_first_video_and_audio_stream_are_taken() {
    let second = good_stream().replace(r#""codec_name": "h264""#, r#""codec_name": "hevc""#);
    let json = wrap(&[good_stream(), second]);
    let s = parse_probe_output(&json).unwrap();
    assert_eq!(s.video.unwrap().codec, "h264", "the first video stream wins");
}

#[test]
fn subtitle_streams_are_detected_without_a_video_stream_being_required_to_fail() {
    let sub = r#"{"codec_type":"subtitle","codec_name":"subrip"}"#;
    let s = parse_probe_output(&wrap(&[good_stream(), sub.to_string()])).unwrap();
    assert!(s.has_subtitles);
}

#[test]
fn deeply_nested_json_does_not_blow_the_stack() {
    // A hand-crafted file can nest far deeper than any real container.
    let deep = format!(
        "{{\"streams\":[{}{}{}],\"format\":{{}}}}",
        "{\"codec_type\":\"video\",\"tags\":{".repeat(200),
        good_stream(),
        "}}".repeat(200),
    );
    // Must return, whatever it decides.
    let _ = parse_probe_output(&deep);
}

#[test]
fn unknown_codec_types_are_ignored_not_rejected() {
    let data = r#"{"codec_type":"data","codec_name":"bin_data"}"#;
    let s = parse_probe_output(&wrap(&[good_stream(), data.to_string()]))
        .expect("an attached data stream must not break the import");
    assert!(s.video.is_some());
}

#[test]
fn a_stream_with_no_fields_at_all_is_still_a_valid_shape() {
    // Every field is `#[serde(default)]`, so a nearly-empty object must
    // deserialise and then be judged VFR rather than rejected outright.
    let s: ProbeStream = serde_json::from_str("{}").expect("defaults exist");
    assert!(is_vfr(&s));
    // Fields are private by design; reaching them from a test would mean
    // making them public for no reason. The VFR judgement above is the
    // observable contract.
}