//! Capability probing at startup (§13).
//!
//! Runs after the window is visible (§14.7), never before. Nothing here is
//! allowed to block the first paint, and every failure degrades to a
//! conservative default rather than propagating.

use quillie_core::capability::{
    is_software_renderer, Capabilities, FfmpegCapabilities, GraphicsCapabilities,
};
use std::path::{Path, PathBuf};

use crate::process::SidecarCommand;

fn sidecar(name: &str) -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("sidecars");
    let wanted = format!("{name}-");
    std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name()
            .map(|f| f.to_string_lossy().starts_with(&wanted))
            .unwrap_or(false)
    })
}

/// Probe the host machine.
///
/// Every step is optional. A machine that cannot answer a question gets the
/// conservative default for it, because the cost of assuming a capability that
/// is absent is a broken feature and the cost of assuming the absence of one
/// that exists is merely slower.
pub fn probe() -> Capabilities {
    Capabilities {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        physical_cores: physical_cores(),
        logical_cores: std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(2),
        total_ram_mb: total_ram_mb(),
        available_ram_mb: available_ram_mb(),
        free_disk_app_data: free_disk(&app_data_dir()),
        free_disk_project: 0,
        battery_on_ac: battery_state(),
        // Graphics comes from the webview via IPC; the host's default is the
        // conservative software-renderer one.
        graphics: GraphicsCapabilities::default(),
        ffmpeg: probe_ffmpeg(),
        whisper_present: false,
    }
}

/// Physical cores, not logical.
///
/// Logical count is twice the physical count on an SMT machine, and using it
/// would hand the scheduler twice the concurrency the machine actually has.
/// The UI thread then competes with four decode jobs on two cores.
pub fn physical_cores() -> u32 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/cpuinfo") {
            // The "core id"/"physical id" pair identifies a physical package
            // core. Falls through to the logical count when unavailable.
            let mut ids = std::collections::HashSet::new();
            let mut physical_id = String::new();
            for line in s.lines() {
                if let Some(v) = line.strip_prefix("physical id") {
                    physical_id = v.trim_start_matches(':').trim().to_string();
                } else if let Some(v) = line.strip_prefix("core id") {
                    let core_id = v.trim_start_matches(':').trim().to_string();
                    if !physical_id.is_empty() {
                        ids.insert(format!("{physical_id}:{core_id}"));
                    }
                }
            }
            if !ids.is_empty() {
                return ids.len() as u32;
            }
        }
    }
    std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(2)
}

pub fn total_ram_mb() -> u64 {
    total_ram_bytes() / (1024 * 1024)
}

#[cfg(target_os = "linux")]
fn total_ram_bytes() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()))
        })
        .map(|kb| kb * 1024)
        .unwrap_or(4 * 1024 * 1024 * 1024)
}

#[cfg(not(target_os = "linux"))]
fn total_ram_bytes() -> u64 {
    4 * 1024 * 1024 * 1024
}

pub fn available_ram_mb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("MemAvailable:"))
                    .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok()))
            })
            .map(|kb| kb / 1024)
            .unwrap_or(2048)
    }
    #[cfg(not(target_os = "linux"))]
    {
        2048
    }
}

/// Free bytes on the filesystem holding `path`.
///
/// Uses the platform's own query rather than walking the directory tree, which
/// would be far too slow to run at startup. Returns 0 when the answer is
/// unavailable, and the disk guard treats 0 as "unknown, ask again" rather
/// than "full".
pub fn free_disk(path: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let Ok(c) = CString::new(path.as_os_str().as_bytes()) else {
            return 0;
        };
        let mut stat: libc_statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the
        // call, and `stat` is a correctly-sized, zeroed statvfs.
        let rc = unsafe { statvfs(c.as_ptr(), &mut stat) };
        if rc != 0 {
            return 0;
        }
        // f_bavail (not f_bfree) is what an unprivileged process can use.
        (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0
    }
}

#[cfg(unix)]
#[repr(C)]
#[derive(Default)]
struct libc_statvfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    __spare: [i32; 6],
}

#[cfg(unix)]
extern "C" {
    fn statvfs(path: *const std::os::raw::c_char, buf: *mut libc_statvfs) -> std::os::raw::c_int;
}

fn app_data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
}

/// Battery state, where the platform exposes it.
///
/// `None` means "cannot tell", which is different from "on battery" and is
/// treated as such: assuming battery on a desktop would put every machine into
/// Quiet mode.
pub fn battery_state() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        let p = Path::new("/sys/class/power_supply");
        let entries = std::fs::read_dir(p).ok()?;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("AC") {
                continue;
            }
            if let Ok(s) = std::fs::read_to_string(e.path().join("online")) {
                if let Ok(v) = s.trim().parse::<u8>() {
                    return Some(v == 1);
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Probe FFmpeg: version, encoders, filters, and *verified* hwaccels.
pub fn probe_ffmpeg() -> FfmpegCapabilities {
    let Some(ffmpeg) = sidecar("ffmpeg") else {
        return FfmpegCapabilities::default();
    };
    let prog = ffmpeg.display().to_string();

    let version = SidecarCommand::new(prog.clone(), vec!["-version".into()])
        .run_captured()
        .ok()
        .filter(|o| o.success)
        .and_then(|o| o.stdout.lines().next().map(|l| l.to_string()))
        .unwrap_or_else(|| "unknown".into());

    let listed = SidecarCommand::new(prog.clone(), vec!["-hide_banner".into(), "-hwaccels".into()])
        .run_captured()
        .ok()
        .filter(|o| o.success)
        .map(|o| {
            o.stdout
                .lines()
                .skip(1)
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    // A listed hwaccel is not a working hwaccel. Each one gets a one-frame
    // test decode; failures are recorded so the same failure is not paid for
    // twice on the next launch.
    let mut working = Vec::new();
    let mut rejected = Vec::new();
    let Some(_ffprobe) = sidecar("ffprobe") else {
        return FfmpegCapabilities {
            version,
            working_hwaccels: Vec::new(),
            rejected_hwaccels: listed,
            has_encoder: Vec::new(),
            has_filter: Vec::new(),
        };
    };

    let dev_null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    for hw in listed {
        let test = SidecarCommand::new(
            prog.clone(),
            vec![
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-hwaccel".into(),
                hw.clone(),
                // One frame, thrown away. The point is whether it decodes.
                "-i".into(),
                ffprobe_placeholder(),
                "-frames:v".into(),
                "1".into(),
                "-f".into(),
                "null".into(),
                dev_null.into(),
            ],
        )
        .run_captured();
        match test {
            Ok(o) if o.success => working.push(hw),
            // No media argument available yet, so an inconclusive result is
            // recorded as rejected rather than assumed working.
            _ => rejected.push(hw),
        }
    }

    let has_encoder = list_items(&prog, "-encoders");
    let has_filter = list_items(&prog, "-filters");

    FfmpegCapabilities {
        version,
        working_hwaccels: working,
        rejected_hwaccels: rejected,
        has_encoder,
        has_filter,
    }
}

/// Placeholder input for the hwaccel probe.
///
/// Phase 2 supplies a real fixture. Until then no hwaccel can pass the test,
/// which is the safe direction: an unverified hwaccel is exactly the thing
/// that fails later, in the middle of a user's export.
fn ffprobe_placeholder() -> String {
    std::env::var("QUILLIE_HWACCEL_PROBE_INPUT").unwrap_or_else(|_| {
        "/nonexistent/quillie-hwaccel-probe.mp4".to_string()
    })
}

/// Parse the names out of `ffmpeg -encoders` / `-filters` output.
fn list_items(prog: &str, flag: &str) -> Vec<String> {
    SidecarCommand::new(prog.to_string(), vec!["-hide_banner".into(), flag.into()])
        .run_captured()
        .ok()
        .filter(|o| o.success)
        .map(|o| {
            o.stdout
                .lines()
                .filter(|l| l.starts_with(char::is_whitespace) && !l.starts_with(" ---"))
                .filter_map(|l| l.split_whitespace().last())
                .filter(|n| !n.is_empty() && !n.contains('='))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Merge the webview's graphics report into the host's probe.
pub fn with_graphics(mut caps: Capabilities, graphics: GraphicsCapabilities) -> Capabilities {
    // Recompute is_software from the actual renderer string rather than
    // trusting the boolean the webview reported, since the string is the
    // evidence and the boolean is the guess.
    caps.graphics.is_software = graphics.is_software || is_software_renderer(&graphics.renderer);
    caps.graphics = GraphicsCapabilities {
        is_software: caps.graphics.is_software,
        ..graphics
    };
    caps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_cores_is_never_more_than_logical() {
        let p = physical_cores();
        let l = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1);
        assert!(p >= 1, "must detect at least one core");
        assert!(p <= l, "physical ({p}) cannot exceed logical ({l})");
    }

    #[test]
    fn the_probe_never_panics_and_returns_a_usable_capability_set() {
        let c = probe();
        assert!(c.physical_cores >= 1);
        assert!(c.total_ram_mb > 0);
        // Whatever the probe found, the tier must be derivable.
        let _ = c.tier();
        let _ = c.job_limits();
        let _ = c.memory_budgets_mb();
    }

    #[test]
    fn a_missing_ffmpeg_yields_the_conservative_default() {
        // list_items on a nonexistent binary must not panic.
        assert!(list_items("/nonexistent/ffmpeg", "-encoders").is_empty());
    }

    #[test]
    fn free_disk_reports_a_plausible_number_not_zero() {
        // The disk guard refuses proxy builds below 2 GB. If this returned 0
        // for a real filesystem the guard would either never fire or always
        // fire, and neither is discoverable until a user's export fails.
        let free = free_disk(Path::new("/"));
        assert!(free > 0, "free space on / should be non-zero");
        assert!(free < 100 * 1024 * 1024 * 1024 * 1024, "implausible value: {free}");
    }

    #[test]
    fn free_disk_of_a_nonexistent_path_is_zero_not_a_panic() {
        assert_eq!(free_disk(Path::new("/definitely/not/a/real/path")), 0);
    }

    #[test]
    fn graphics_merge_derives_software_status_from_the_renderer_string() {
        let c = Capabilities::default();
        let g = GraphicsCapabilities {
            webgl_version: 2,
            max_texture_size: 8192,
            renderer: "llvmpipe (LLVM 15)".into(),
            // The webview claims it is not software; the string says otherwise.
            is_software: false,
            video_decoder_available: true,
        };
        let merged = with_graphics(c.clone(), g);
        assert!(merged.graphics.is_software, "the string is the evidence");
        assert!(merged.needs_canvas_fallback());
    }
}