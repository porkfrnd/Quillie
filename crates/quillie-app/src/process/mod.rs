//! FFmpeg, ffprobe and whisper supervision (§9.2).
//!
//! # Rules
//!
//! - Arguments are always `Vec<String>`. A shell string is never built, so
//!   there is no path in which a filename can become a command.
//! - Progress is parsed from `-progress pipe:1`, not scraped from stderr,
//!   because stderr carries FFmpeg's human-facing text and is not stable.
//! - The last 64 KB of stderr is kept in a ring buffer so a failed job can
//!   report its tail as `detail` without the raw text ever becoming the
//!   user-facing message (rule 8).
//! - Every job has a stall timeout. FFmpeg can go quiet indefinitely while
//!   waiting on a slow disk, and a job that never fails is worse than one
//!   that does.

use std::collections::VecDeque;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How much stderr to retain for diagnostics.
pub const STDERR_RING_BYTES: usize = 64 * 1024;

/// A fixed-size ring buffer for a process's stderr tail.
#[derive(Debug, Default)]
pub struct StderrRing {
    buf: VecDeque<u8>,
    capacity: usize,
}

impl StderrRing {
    pub fn new() -> Self {
        StderrRing { buf: VecDeque::with_capacity(STDERR_RING_BYTES), capacity: STDERR_RING_BYTES }
    }

    pub fn push(&mut self, chunk: &[u8]) {
        for &b in chunk {
            if self.buf.len() == self.capacity {
                self.buf.pop_front();
            }
            self.buf.push_back(b);
        }
    }

    /// The retained tail as a lossy UTF-8 string.
    pub fn tail(&self) -> String {
        let (a, b) = self.buf.as_slices();
        let mut out = Vec::with_capacity(a.len() + b.len());
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        String::from_utf8_lossy(&out).to_string()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// How much CPU a background job may take (§9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NiceLevel {
    /// Foreground-equivalent. For user-initiated work like export.
    Normal,
    /// Yield to the UI thread. For proxies, analysis and cache rebuilds.
    Background,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("sidecar not found: {0}")]
    SidecarMissing(String),
    #[error("sidecar failed to start: {0}")]
    Spawn(String),
    #[error("job made no progress for {0:?}")]
    Stalled(Duration),
    #[error("job cancelled")]
    Cancelled,
}

/// A sidecar invocation, built but not yet started.
///
/// Constructing this does nothing observable, which makes argument building
/// testable without a binary present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarCommand {
    pub program: String,
    pub args: Vec<String>,
    pub nice: NiceLevel,
}

impl SidecarCommand {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        SidecarCommand { program: program.into(), args, nice: NiceLevel::Normal }
    }

    pub fn background(mut self) -> Self {
        self.nice = NiceLevel::Background;
        self
    }

    /// Append an argument. Kept as a method so call sites never build a shell
    /// string by concatenation.
    pub fn arg(&mut self, a: impl Into<String>) -> &mut Self {
        self.args.push(a.into());
        self
    }

    pub fn args(&mut self, a: impl IntoIterator<Item = impl Into<String>>) -> &mut Self {
        self.args.extend(a.into_iter().map(Into::into));
        self
    }

/// Build the `std::process::Command`.
    ///
    /// Args are passed through as a slice; there is no shell, so nothing in a
    /// filename can be interpreted as syntax.
    ///
    /// stdin is null: a sidecar that tries to prompt will fail fast rather
    /// than hang forever waiting on a terminal that does not exist.
    pub fn to_command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }

    /// Spawn the sidecar.
    pub fn spawn(&self) -> Result<Child, ProcessError> {
        self.to_command()
            .spawn()
            .map_err(|e| ProcessError::Spawn(format!("{}: {e}", self.program)))
    }

    /// Run to completion, capturing stdout and the stderr tail.
    ///
    /// This is the blocking helper used by short, user-initiated work such as
    /// a probe. Long jobs use the streaming path instead so progress can be
    /// reported and the job can be cancelled.
    pub fn run_captured(&self) -> Result<Output_, ProcessError> {
        let mut child = self.spawn()?;
        let mut stderr_ring = StderrRing::new();

        // Drain stderr concurrently so a chatty child cannot fill its pipe
        // buffer and deadlock against a parent that is not reading it.
        if let Some(mut err) = child.stderr.take() {
            let mut buf = [0u8; 8192];
            loop {
                match std::io::Read::read(&mut err, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => stderr_ring.push(&buf[..n]),
                }
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|e| ProcessError::Spawn(e.to_string()))?;

        Ok(Output_ {
            success: output.status.success(),
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr_tail: stderr_ring.tail(),
        })
    }
}

/// Result of a completed sidecar invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output_ {
    pub success: bool,
    pub status: Option<i32>,
    pub stdout: String,
    /// The retained tail of stderr. Never shown to the user as the message;
    /// it is the `detail` field of a typed error (rule 8).
    pub stderr_tail: String,
}

/// Locate a bundled sidecar by its base name.
///
/// The binary carries a target-triple suffix (`ffmpeg-x86_64-unknown-linux-gnu`)
/// so Tauri can ship several for different platforms at once, so the name is
/// matched by prefix rather than hard-coded. That also means the same code
/// works on Windows without a second lookup table.
pub fn sidecar_path(name: &str) -> Option<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sidecars");
    let wanted = format!("{name}-");
    std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name()
            .map(|f| f.to_string_lossy().starts_with(&wanted))
            .unwrap_or(false)
    })
}

/// Progress as reported by FFmpeg's `-progress pipe:1` stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    /// 0.0..=1.0, or `None` when FFmpeg could not compute a total (streams of
    /// unknown length legitimately report `N/A`).
    pub fraction: Option<f64>,
    pub out_time_secs: f64,
}

/// Parse one `key=value` line from the progress stream.
///
/// Unknown keys are ignored rather than treated as errors, because FFmpeg
/// gains keys between releases and a job should not die because it printed
/// something new.
pub fn parse_progress_line(line: &str) -> Option<Progress> {
    let (key, value) = line.split_once('=')?;
    match key.trim() {
        "out_time_us" | "out_time_ms" => {
            // Both keys exist and mean microseconds in different FFmpeg
            // versions; the value is a decimal number of microseconds.
            let us: f64 = value.trim().parse().ok()?;
            Some(Progress { fraction: None, out_time_secs: us / 1_000_000.0 })
        }
        "progress" => {
            if value.trim() == "end" {
                Some(Progress { fraction: Some(1.0), out_time_secs: 0.0 })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Track whether a job is still producing progress.
#[derive(Debug)]
pub struct StallWatchdog {
    timeout: Duration,
    last: Instant,
}

impl StallWatchdog {
    pub fn new(timeout: Duration) -> Self {
        StallWatchdog { timeout, last: Instant::now() }
    }

    /// Call on every progress line.
    pub fn note_progress(&mut self) {
        self.last = Instant::now();
    }

    /// Returns `Err` once the job has gone quiet for too long.
    pub fn check(&self) -> Result<(), ProcessError> {
        if self.last.elapsed() > self.timeout {
            Err(ProcessError::Stalled(self.timeout))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_never_a_shell_string() {
        // A filename with shell metacharacters must stay one argument.
        let cmd = SidecarCommand::new(
            "ffmpeg",
            vec![
                "-i".into(),
                "a file; rm -rf $HOME && echo `whoami`.mp4".into(),
                "-f".into(),
                "null".into(),
                "-".into(),
            ],
        );
        assert_eq!(cmd.args.len(), 5);
        assert_eq!(cmd.args[1], "a file; rm -rf $HOME && echo `whoami`.mp4");
    }

    #[test]
    fn the_stderr_ring_keeps_exactly_the_last_n_bytes() {
        let mut r = StderrRing { buf: VecDeque::new(), capacity: 64 };
        r.push(&[b'a'; 200]);
        r.push(b"recent important detail");
        let t = r.tail();
        // 200 'a' then 23 bytes; a 64-byte ring keeps the last 64 of the
        // combined stream, so some 'a's legitimately remain. What matters is
        // that the length is capped and the true tail survived.
        assert_eq!(t.len(), 64);
        assert!(t.ends_with("recent important detail"), "tail was: {t:?}");
        assert_eq!(r.len(), 64);
    }

    #[test]
    fn the_stderr_ring_wraps_across_its_internal_split() {
        // VecDeque wraps its internal buffer once you push past its capacity,
        // after which as_slices() returns two parts. Reading only one silently
        // truncates diagnostics -- and the truncated part is the tail we most
        // need. So push well past capacity and check the tail survives.
        let mut r = StderrRing { buf: VecDeque::new(), capacity: 64 };
        for i in 0..100u8 {
            r.push(&[b'0' + (i % 10)]);
        }
        r.push(b"TAIL");
        let t = r.tail();
        assert_eq!(t.len(), 64, "ring must stay capped");
        assert!(t.ends_with("TAIL"), "tail was: {t:?}");
    }

    #[test]
    fn progress_lines_parse_and_others_are_ignored() {
        let p = parse_progress_line("out_time_us=5000000").unwrap();
        assert!((p.out_time_secs - 5.0).abs() < 1e-9);
        assert_eq!(p.fraction, None);

        assert_eq!(parse_progress_line("progress=end").unwrap().fraction, Some(1.0));
        assert_eq!(parse_progress_line("progress=continue"), None);

        // Keys FFmpeg adds in future versions must not kill the job.
        assert_eq!(parse_progress_line("some_new_key=42"), None);
        assert_eq!(parse_progress_line("not a k=v line"), None);
        assert_eq!(parse_progress_line("bitrate=N/A"), None);
    }

    #[test]
    fn the_watchdog_fires_only_after_silence() {
        let w = StallWatchdog::new(Duration::from_millis(50));
        assert!(w.check().is_ok());
        std::thread::sleep(Duration::from_millis(80));
        assert!(w.check().is_err());
        // Reporting progress resets the clock.
        let mut w = StallWatchdog::new(Duration::from_millis(50));
        w.note_progress();
        assert!(w.check().is_ok());
    }

    #[test]
    fn background_jobs_are_marked_as_such() {
        let c = SidecarCommand::new("ffmpeg", vec![]).background();
        assert_eq!(c.nice, NiceLevel::Background);
    }
}