//! Capability detection and tier derivation (§13).
//!
//! # The rule
//!
//! Hardware decode is **never assumed**. If `ffmpeg -hwaccels` lists a
//! hwaccel, Quillie runs a one-frame test decode against it. If that fails the
//! hwaccel is disabled and the result remembered, because a hwaccel that
//! appears in a list but fails on the first frame is common and produces a
//! confusing failure much later.
//!
//! Every optional feature has a documented fallback, and every tier and every
//! fallback is testable by injecting a fake [`Capabilities`]. Nothing in the
//! codebase should branch on "this machine has X" without going through here.

use crate::time::FrameRate;
use serde::{Deserialize, Serialize};

/// Machine tier (§2, §13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// 4 GB, 2 physical cores, no AVX2, integrated graphics, 5400 RPM.
    Floor,
    /// 8 GB.
    Mid,
    /// 16 GB or more.
    High,
}

impl Tier {
    /// Highest preview resolution scale this tier allows (§8.3 ladder).
    ///
    /// The floor still previews at full quality; it just starts from a lower
    /// rung under load. "Degrades, never disappears" is Feature Gate rule 5,
    /// and this is where that is encoded.
    pub fn max_preview_scale(&self) -> f32 {
        match self {
            Tier::Floor => 1.0,
            Tier::Mid => 1.0,
            Tier::High => 1.0,
        }
    }

    /// Proxy resolution ceiling. The floor tops out at 540 (§7.2).
    pub fn max_proxy_height(&self) -> u32 {
        match self {
            Tier::Floor => 540,
            Tier::Mid => 720,
            Tier::High => 720,
        }
    }

    /// Label for the diagnostics panel.
    pub fn label(&self) -> &'static str {
        match self {
            Tier::Floor => "floor",
            Tier::Mid => "mid",
            Tier::High => "high",
        }
    }
}

/// WebGL and WebCodecs facts, reported by the webview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphicsCapabilities {
    pub webgl_version: u32,
    pub max_texture_size: u32,
    pub renderer: String,
    /// True when the renderer is a software rasteriser (llvmpipe, SwiftShader).
    /// §14.4 requires a Canvas2D fallback in that case.
    pub is_software: bool,
    pub video_decoder_available: bool,
}

/// FFmpeg facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FfmpegCapabilities {
    pub version: String,
    /// hwaccel names from `-hwaccels`, minus the ones that failed their test.
    pub working_hwaccels: Vec<String>,
    /// Listed but rejected by the one-frame test.
    pub rejected_hwaccels: Vec<String>,
    pub has_encoder: Vec<String>,
    pub has_filter: Vec<String>,
}

/// Everything probed at startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub os: String,
    pub arch: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub total_ram_mb: u64,
    pub available_ram_mb: u64,
    pub free_disk_app_data: u64,
    pub free_disk_project: u64,
    pub battery_on_ac: Option<bool>,
    pub graphics: GraphicsCapabilities,
    pub ffmpeg: FfmpegCapabilities,
    pub whisper_present: bool,
}

impl Default for GraphicsCapabilities {
    fn default() -> Self {
        GraphicsCapabilities {
            webgl_version: 1,
            max_texture_size: 4096,
            renderer: "unknown".into(),
            is_software: true,
            video_decoder_available: false,
        }
    }
}

impl Default for FfmpegCapabilities {
    fn default() -> Self {
        FfmpegCapabilities {
            version: "none".into(),
            working_hwaccels: Vec::new(),
            rejected_hwaccels: Vec::new(),
            has_encoder: Vec::new(),
            has_filter: Vec::new(),
        }
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Capabilities {
            os: "unknown".into(),
            arch: "unknown".into(),
            physical_cores: 2,
            logical_cores: 2,
            total_ram_mb: 4096,
            available_ram_mb: 2048,
            free_disk_app_data: 0,
            free_disk_project: 0,
            battery_on_ac: None,
            graphics: GraphicsCapabilities::default(),
            ffmpeg: FfmpegCapabilities::default(),
            whisper_present: false,
        }
    }
}

impl Capabilities {
    /// Derive the tier from RAM and cores.
    ///
    /// Conservative at the floor boundary: a machine at or below the floor
    /// numbers is treated as floor even if it is probably slightly better,
    /// because the failure mode of being wrong in this direction is a
    /// degraded experience, while being wrong the other way produces an
    /// editor that does not run on the machines it was supposed to support.
    pub fn tier(&self) -> Tier {
        if self.total_ram_mb <= 4096 && self.physical_cores <= 2 {
            Tier::Floor
        } else if self.total_ram_mb <= 8192 {
            Tier::Mid
        } else {
            Tier::High
        }
    }

    /// Does the timeline need the Canvas2D fallback (§14.4)?
    ///
    /// True for software renderers and for WebGL 1 without the extensions the
    /// timeline needs. A software rasteriser can run a viewport-sized timeline,
    /// but not a 4K one at 60 Hz.
    pub fn needs_canvas_fallback(&self) -> bool {
        self.graphics.is_software || self.graphics.webgl_version < 2
    }

    /// Can this machine use WebCodecs preview path C (§8.2)?
    pub fn can_use_webcodecs(&self) -> bool {
        self.graphics.video_decoder_available && !self.graphics.is_software
    }

    /// Is hardware decode available *and* verified working?
    pub fn hardware_decode_available(&self) -> bool {
        !self.ffmpeg.working_hwaccels.is_empty()
    }

    /// The lowest proxy profile this machine should use (§7.2).
    pub fn default_proxy_height(&self) -> u32 {
        match self.tier() {
            Tier::Floor => 360,
            Tier::Mid => 540,
            Tier::High => 720,
        }
    }

    /// Concurrency limits derived from capability (§9.1).
    pub fn job_limits(&self) -> crate::jobs::Limits {
        let disk = match self.tier() {
            // One disk job at a time unless we have an SSD, which we cannot
            // assume. Whether two is better on higher tiers is a Phase 2
            // measurement, not a guess.
            Tier::Floor => 1,
            Tier::Mid => 1,
            Tier::High => 2,
        };
        crate::jobs::Limits {
            physical_cores: self.physical_cores,
            max_disk_concurrent: disk,
            quiet_mode: false,
        }
    }

    /// Initial memory budgets (§14.2), scaled by tier.
    ///
    /// These are placeholders until calibration runs (§13). Every value here
    /// is a floor-sane default that a calibration is expected to refine, not a
    /// measured constant.
    pub fn memory_budgets_mb(&self) -> MemoryBudgets {
        let base = MemoryBudgets {
            decoded_frames_mb: 64,
            texture_pool_mb: 48,
            thumbnails_mb: 16,
            audio_chunks_mb: 24,
            peaks_mb: 8,
            undo_mb: 8,
        };
        let scale = match self.tier() {
            Tier::Floor => 1.0,
            Tier::Mid => 2.0,
            Tier::High => 4.0,
        };
        MemoryBudgets {
            decoded_frames_mb: (base.decoded_frames_mb as f32 * scale) as u64,
            texture_pool_mb: (base.texture_pool_mb as f32 * scale) as u64,
            thumbnails_mb: (base.thumbnails_mb as f32 * scale) as u64,
            audio_chunks_mb: (base.audio_chunks_mb as f32 * scale) as u64,
            peaks_mb: (base.peaks_mb as f32 * scale) as u64,
            undo_mb: 8,
        }
    }

    /// Whether to apply Quiet mode by default (§14.7).
    pub fn quiet_mode_by_default(&self) -> bool {
        // On battery, unless the user says otherwise. Unknown battery state
        // does not trigger it -- assuming a laptop is on battery would quiet
        // every desktop.
        self.battery_on_ac == Some(false)
    }
}

/// Per-class memory budgets (§14.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBudgets {
    pub decoded_frames_mb: u64,
    pub texture_pool_mb: u64,
    pub thumbnails_mb: u64,
    pub audio_chunks_mb: u64,
    pub peaks_mb: u64,
    pub undo_mb: u64,
}

impl MemoryBudgets {
    pub fn total_mb(&self) -> u64 {
        self.decoded_frames_mb
            + self.texture_pool_mb
            + self.thumbnails_mb
            + self.audio_chunks_mb
            + self.peaks_mb
            + self.undo_mb
    }
}

/// Default project frame rate when nothing else determines it.
pub fn default_frame_rate() -> FrameRate {
    FrameRate::NTSC_30
}

/// Recognise a software renderer from its string.
///
/// llvmpipe, SwiftShader, Mesa's softpipe and Apple's software renderer all
/// report themselves in the renderer string, and all of them need the Canvas2D
/// timeline fallback (§14.4).
pub fn is_software_renderer(renderer: &str) -> bool {
    let r = renderer.to_ascii_lowercase();
    ["llvmpipe", "swiftshader", "softpipe", "software", "microsoft basic render"]
        .iter()
        .any(|needle| r.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(ram_mb: u64, cores: u32) -> Capabilities {
        Capabilities {
            total_ram_mb: ram_mb,
            physical_cores: cores,
            logical_cores: cores * 2,
            ..Default::default()
        }
    }

    #[test]
    fn the_floor_is_detected_at_its_exact_numbers() {
        assert_eq!(caps(4096, 2).tier(), Tier::Floor);
    }

    #[test]
    fn the_tier_boundary_is_conservative() {
        // Just under floor RAM but many cores: mid, because the floor needs
        // both. Just over floor RAM: mid.
        assert_eq!(caps(4000, 8).tier(), Tier::Mid);
        assert_eq!(caps(4097, 2).tier(), Tier::Mid);
        assert_eq!(caps(8192, 4).tier(), Tier::Mid);
        assert_eq!(caps(16384, 8).tier(), Tier::High);
    }

    #[test]
    fn a_software_renderer_forces_the_canvas_fallback() {
        let mut c = caps(8192, 4);
        c.graphics.renderer = "llvmpipe (LLVM 15.0.0, 256 bits)".into();
        assert!(is_software_renderer(&c.graphics.renderer));
        assert!(c.needs_canvas_fallback());

        c.graphics.renderer = "ANGLE (Intel(R) HD Graphics 520)".into();
        assert!(!is_software_renderer(&c.graphics.renderer));
    }

    #[test]
    fn webgl_1_alone_forces_the_canvas_fallback() {
        let mut c = caps(8192, 4);
        c.graphics.is_software = false;
        c.graphics.webgl_version = 1;
        assert!(c.needs_canvas_fallback(), "WebGL 1 is the floor minimum, not enough for the Pixi timeline");
        c.graphics.webgl_version = 2;
        assert!(!c.needs_canvas_fallback());
    }

    #[test]
    fn hardware_decode_is_false_until_a_hwaccel_is_verified() {
        let c = caps(8192, 4);
        assert!(!c.hardware_decode_available(), "an empty list means no hwaccel, not an assumed one");
    }

    #[test]
    fn a_rejected_hwaccel_does_not_count() {
        let mut c = caps(8192, 4);
        c.ffmpeg.rejected_hwaccels = vec!["vaapi".into()];
        assert!(!c.hardware_decode_available());
        c.ffmpeg.working_hwaccels = vec!["cuda".into()];
        assert!(c.hardware_decode_available());
    }

    #[test]
    fn the_floor_uses_a_360p_proxy_and_one_disk_job() {
        let c = caps(4096, 2);
        assert_eq!(c.default_proxy_height(), 360);
        assert_eq!(c.job_limits().max_disk_concurrent, 1);
    }

    #[test]
    fn every_tier_previews_at_full_quality() {
        // Feature Gate rule 5: degrade, never disappear. A tier must never
        // cap preview scale below 1.0.
        for t in [Tier::Floor, Tier::Mid, Tier::High] {
            assert_eq!(t.max_preview_scale(), 1.0, "{} must still preview at full quality", t.label());
        }
    }

    #[test]
    fn budgets_scale_up_with_tier_and_stay_bounded() {
        let floor = caps(4096, 2).memory_budgets_mb();
        let high = caps(16384, 8).memory_budgets_mb();
        assert!(high.total_mb() > floor.total_mb());
        // Undo does not scale: 1000 ops or 8 MB, whichever comes first (§6.3).
        assert_eq!(floor.undo_mb, high.undo_mb);
        // The floor's total must leave room inside 4 GB for everything else.
        assert!(floor.total_mb() <= 256, "floor budgets total {}", floor.total_mb());
    }

    #[test]
    fn battery_power_enables_quiet_mode_but_unknown_does_not() {
        let mut c = caps(8192, 4);
        c.battery_on_ac = Some(false);
        assert!(c.quiet_mode_by_default());
        c.battery_on_ac = Some(true);
        assert!(!c.quiet_mode_by_default());
        c.battery_on_ac = None;
        assert!(!c.quiet_mode_by_default(), "assuming battery on a desktop would quiet everything");
    }

    #[test]
    fn webcodecs_is_unavailable_on_a_software_renderer() {
        let mut c = caps(8192, 4);
        c.graphics.video_decoder_available = true;
        c.graphics.is_software = false;
        assert!(c.can_use_webcodecs());
        c.graphics.is_software = true;
        assert!(!c.can_use_webcodecs(), "a software VideoDecoder is too slow to be usable");
    }

    #[test]
    fn a_default_capabilities_value_is_the_floor() {
        // The default must be the conservative one, so a probe that fails
        // degrades rather than assuming a capable machine.
        let c = Capabilities::default();
        assert_eq!(c.tier(), Tier::Floor);
        assert!(c.needs_canvas_fallback());
        assert!(!c.hardware_decode_available());
        assert_eq!(c.ffmpeg.version, "none");
    }
}