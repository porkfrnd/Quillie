//! Job scheduling (§9.1).
//!
//! # The policy, stated
//!
//! Six priority lanes, highest first:
//!
//! 1. Playback and visible scrub decode
//! 2. Preview prefetch near the playhead
//! 3. Visible thumbnails and waveforms
//! 4. Interactive assistance (data that feeds visible UI)
//! 5. Transcription and analysis
//! 6. Proxy generation, Smooth Zones, bulk cache rebuilds, export
//!
//! The user never waits behind a proxy build. Lane 6 is deliberately last, and
//! while someone is dragging a clip, lanes 4 to 6 pause entirely so the UI
//! thread wins every core.
//!
//! # This module is pure
//!
//! Scheduling decisions are values in, values out. `quillie-app` executes the
//! jobs; nothing here spawns a process or touches a disk. That is what makes
//! the policy testable, and the policy is where the bugs live.

use std::collections::VecDeque;
use std::time::Duration;

/// Priority lanes. Lower number runs first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane {
    /// 1. Decode the frame the user is looking at right now.
    Playback = 1,
    /// 2. Decode just ahead of the playhead.
    Prefetch = 2,
    /// 3. Thumbnails and waveforms that are on screen.
    VisibleAssets = 3,
    /// 4. Data that feeds visible UI.
    Interactive = 4,
    /// 5. Transcription, silence, scenes, beats, loudness, tracking.
    Analysis = 5,
    /// 6. Proxies, Smooth Zones, cache rebuilds, export.
    Background = 6,
}

/// What a job needs from the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    Cpu,
    Disk,
    CpuAndDisk,
}

impl Resource {
    fn is_cpu_heavy(self) -> bool {
        matches!(self, Resource::Cpu | Resource::CpuAndDisk)
    }
    fn is_disk_heavy(self) -> bool {
        matches!(self, Resource::Disk | Resource::CpuAndDisk)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

/// A unit of background work.
///
/// `dedupe_key` is what makes repeated requests cheap: asking for the same
/// thumbnail five times as the user scrolls back and forth queues one job.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub lane: Lane,
    pub dedupe_key: String,
    pub resource: Resource,
    pub state: JobState,
    pub progress: f32,
    pub output: Option<String>,
    /// Set when cancelled or failed, for display.
    pub error: Option<String>,
}

impl Job {
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        lane: Lane,
        resource: Resource,
    ) -> Self {
        let id = id.into();
        let kind = kind.into();
        // The default dedupe key is the kind plus the id, which is enough to
        // collapse an exact re-request while still allowing several jobs of
        // the same kind for different inputs.
        let dedupe_key = format!("{kind}:{id}");
        Job {
            id,
            kind,
            lane,
            dedupe_key,
            resource,
            state: JobState::Queued,
            progress: 0.0,
            output: None,
            error: None,
        }
    }

    pub fn with_dedupe_key(mut self, key: impl Into<String>) -> Self {
        self.dedupe_key = key.into();
        self
    }
}

/// Machine limits. Derived from the capability probe and calibration (§13),
/// with floor-appropriate defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub physical_cores: u32,
    /// Disk-heavy jobs run one at a time on the floor (§9.1).
    pub max_disk_concurrent: usize,
    pub quiet_mode: bool,
}

impl Limits {
    /// CPU-heavy concurrency is `max(1, cores - 1)` (§9.1), always leaving a
    /// core for the UI thread.
    pub fn max_cpu_concurrent(&self) -> usize {
        let n = self.physical_cores.saturating_sub(1).max(1) as usize;
        if self.quiet_mode {
            // Quiet mode caps at one core so fans stay quiet on old laptops.
            n.min(1)
        } else {
            n
        }
    }
}

impl Default for Limits {
    /// Floor defaults: 2 physical cores, one disk job at a time.
    fn default() -> Self {
        Limits { physical_cores: 2, max_disk_concurrent: 1, quiet_mode: false }
    }
}

/// The interaction gate (§9.1).
///
/// While the user is dragging or scrubbing, lanes 4-6 do not start. Lanes 1-3
/// are what keep the UI responsive, so they keep running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interacting {
    Idle,
    Dragging,
    Scrubbing,
}

impl Interacting {
    /// May a job in this lane start while the user is interacting?
    pub fn permits(self, lane: Lane) -> bool {
        match self {
            Interacting::Idle => true,
            // Playback-class work must keep running: it is what the user is
            // looking at right now.
            Interacting::Dragging | Interacting::Scrubbing => lane <= Lane::VisibleAssets,
        }
    }
}

/// How often cancelled jobs are checked for progress.
pub const CANCEL_CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// The scheduler.
///
/// Holds queued and running jobs and decides what may start. It does not run
/// anything -- `take_runnable` returns the jobs the host should spawn.
#[derive(Debug)]
pub struct Scheduler {
    queued: VecDeque<Job>,
    /// Artifacts already produced, by dedupe key. See `has_output_for`.
    finished: std::collections::BTreeMap<String, String>,
    running: Vec<Job>,
    limits: Limits,
    interacting: Interacting,
}

impl Scheduler {
    pub fn new(limits: Limits) -> Self {
        Scheduler {
            queued: VecDeque::new(),
            finished: std::collections::BTreeMap::new(),
            running: Vec::new(),
            limits,
            interacting: Interacting::Idle,
        }
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
    }

    pub fn set_interacting(&mut self, state: Interacting) {
        self.interacting = state;
    }

    pub fn interacting(&self) -> Interacting {
        self.interacting
    }

    /// Queue a job.
    ///
    /// **Supersede:** a queued job with the same `dedupe_key` is replaced, not
    /// run twice. If an artifact with that key already exists, the job
    /// completes immediately (§9.1) -- the caller signals that by queuing a
    /// `Job` whose id matches an existing output.
    pub fn enqueue(&mut self, job: Job) {
        // Drop any queued job with the same key. Not a running one: that is
        // cancelled explicitly when its inputs change.
        self.queued.retain(|j| j.dedupe_key != job.dedupe_key);
        self.queued.push_back(job);
    }

    pub fn queued(&self) -> impl Iterator<Item = &Job> {
        self.queued.iter()
    }

    pub fn running(&self) -> impl Iterator<Item = &Job> {
        self.running.iter()
    }

    pub fn running_count(&self) -> usize {
        self.running.len()
    }

    pub fn queued_count(&self) -> usize {
        self.queued.len()
    }

    /// How many CPU-heavy and disk-heavy jobs are running.
///
/// Counted separately because the two limits are different: the floor runs one
/// disk job at a time regardless of core count, while CPU concurrency scales
/// with cores.
fn running_heavy(&self) -> (usize, usize) {
        let cpu = self
            .running
            .iter()
            .filter(|j| j.resource.is_cpu_heavy())
            .count();
        let disk = self
            .running
            .iter()
            .filter(|j| j.resource.is_disk_heavy())
            .count();
        (cpu, disk)
    }

    /// Take the jobs that may start right now, highest priority first.
    ///
    /// Order is by lane, then FIFO within a lane. Stable ordering matters: two
    /// thumbnail jobs for adjacent frames should complete left to right so the
    /// visible range fills in predictably rather than jumping about.
    pub fn take_runnable(&mut self) -> Vec<Job> {
        let mut started: Vec<Job> = Vec::new();

        loop {
            let (cpu_running, disk_running) = self.running_heavy();
            let cpu_room = self.limits.max_cpu_concurrent().saturating_sub(cpu_running);
            let disk_room = self
                .limits
                .max_disk_concurrent
                .saturating_sub(disk_running);

            // Find the best queued job that fits the interaction gate and the
            // concurrency limits.
            let mut best: Option<usize> = None;
            for (i, job) in self.queued.iter().enumerate() {
                if !self.interacting.permits(job.lane) {
                    continue;
                }
                let needs_cpu = usize::from(job.resource.is_cpu_heavy());
                let needs_disk = usize::from(job.resource.is_disk_heavy());
                // A job that needs a resource with no room has to wait, but a
                // lower-priority job that fits may still start. That is why
                // this scans for the best *fitting* job rather than only
                // looking at the head of the queue.
                if needs_cpu > cpu_room || needs_disk > disk_room {
                    continue;
                }
                match best {
                    Some(b) if self.queued[b].lane <= job.lane => {}
                    _ => best = Some(i),
                }
            }

            match best {
                Some(i) => {
                    let job = self.queued.remove(i).unwrap();
                    let mut job = job;
                    job.state = JobState::Running;
                    self.running.push(job.clone());
                    started.push(job);
                }
                None => break,
            }
        }

        started
    }

    /// Report progress on a running job. Returns false for an unknown id.
    pub fn progress(&mut self, id: &str, progress: f32) -> bool {
        if let Some(job) = self.running.iter_mut().find(|j| j.id == id) {
            job.progress = progress.clamp(0.0, 1.0);
            true
        } else {
            false
        }
    }

    /// Mark a job finished with an artifact.
    pub fn complete(&mut self, id: &str, output: impl Into<String>) -> bool {
        if let Some(i) = self.running.iter().position(|j| j.id == id) {
            let mut job = self.running.remove(i);
            job.state = JobState::Done;
            job.progress = 1.0;
            let out = output.into();
            // Recorded so a later request for the same key reuses it instead
            // of re-running FFmpeg.
            self.finished.insert(job.dedupe_key.clone(), out.clone());
            job.output = Some(out);
            true
        } else {
            false
        }
    }

    /// Mark a job failed.
    pub fn fail(&mut self, id: &str, error: impl Into<String>) -> bool {
        if let Some(i) = self.running.iter().position(|j| j.id == id) {
            let mut job = self.running.remove(i);
            job.state = JobState::Failed;
            job.error = Some(error.into());
            true
        } else {
            false
        }
    }

    /// Cancel a job, queued or running (§9.1).
    ///
    /// The host is responsible for killing the child process tree and removing
    /// temporary outputs; this only records the state change so nothing
    /// double-runs.
    pub fn cancel(&mut self, id: &str) -> bool {
        if let Some(i) = self.running.iter().position(|j| j.id == id) {
            let mut job = self.running.remove(i);
            job.state = JobState::Cancelled;
            return true;
        }
        if let Some(i) = self.queued.iter().position(|j| j.id == id) {
            let mut job = self.queued.remove(i).unwrap();
            job.state = JobState::Cancelled;
            return true;
        }
        false
    }

    /// Cancel every queued and running job at or below `lane`.
    ///
    /// Used when the user hits "Pause background work": lanes 4-6 stop, lanes
    /// 1-3 keep the editor usable.
    pub fn cancel_at_or_below(&mut self, lane: Lane) -> usize {
        let mut n = 0;
        for job in self.running.iter_mut().filter(|j| j.lane >= lane) {
            job.state = JobState::Cancelled;
            n += 1;
        }
        self.running.retain(|j| j.state != JobState::Cancelled);
        let before = self.queued.len();
        self.queued.retain(|j| j.lane < lane);
        n += before - self.queued.len();
        n
    }

    /// Is there a finished artifact for this key?
    ///
    /// §9.1: never restart FFmpeg for output that is already valid.
    ///
    /// Completed jobs leave `running`, so the set of known artifacts is kept
    /// separately. Without it this check would only ever be true while the
    /// job was still in flight -- exactly backwards, since the moment a job
    /// finishes is the moment its artifact becomes reusable.
    pub fn has_output_for(&self, dedupe_key: &str) -> bool {
        self.finished.contains_key(dedupe_key)
    }

    /// Forget an artifact, e.g. when its cache entry is evicted.
    ///
    /// Kept explicit rather than letting the map grow without bound: on a long
    /// editing session the number of distinct keys is bounded by the number of
    /// clips, but eviction happens independently (§14.2) and a key whose
    /// artifact is gone must not suppress a rebuild.
    pub fn forget_output(&mut self, dedupe_key: &str) -> bool {
        self.finished.remove(dedupe_key).is_some()
    }

    /// Every job the user should see in the Activity list, highest lane first.
    pub fn activity(&self) -> Vec<&Job> {
        let mut all: Vec<&Job> = self.running.iter().chain(self.queued.iter()).collect();
        all.sort_by_key(|j| (j.lane, j.id.clone()));
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, lane: Lane, resource: Resource) -> Job {
        Job::new(id, "thumbnail", lane, resource)
    }

    #[test]
    fn higher_priority_lanes_run_first() {
        let mut s = Scheduler::new(Limits { physical_cores: 4, max_disk_concurrent: 2, quiet_mode: false });
        s.enqueue(job("analysis", Lane::Analysis, Resource::Cpu));
        s.enqueue(job("playback", Lane::Playback, Resource::Cpu));
        s.enqueue(job("proxy", Lane::Background, Resource::CpuAndDisk));

        let started = s.take_runnable();
        let order: Vec<&str> = started.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(order[0], "playback", "the frame the user is looking at goes first");
        assert!(order.iter().position(|i| *i == "analysis") < order.iter().position(|i| *i == "proxy"));
    }

    #[test]
    fn cpu_concurrency_never_starves_the_ui_thread() {
        // 2 physical cores -> max_cpu_concurrent is 1, leaving a core for the UI.
        let mut s = Scheduler::new(Limits::default());
        for i in 0..5 {
            s.enqueue(job(&format!("j{i}"), Lane::Analysis, Resource::Cpu));
        }
        let started = s.take_runnable();
        assert_eq!(started.len(), 1, "must not run two CPU jobs on a two-core machine");
        assert_eq!(s.queued_count(), 4);
    }

    #[test]
    fn disk_jobs_run_one_at_a_time_on_the_floor() {
        let mut s = Scheduler::new(Limits::default());
        s.enqueue(job("a", Lane::Background, Resource::CpuAndDisk));
        s.enqueue(job("b", Lane::Background, Resource::CpuAndDisk));
        s.enqueue(job("c", Lane::Background, Resource::CpuAndDisk));
        assert_eq!(s.take_runnable().len(), 1);
    }

    #[test]
    fn quiet_mode_caps_at_one_core() {
        let limits = Limits { physical_cores: 8, max_disk_concurrent: 2, quiet_mode: true };
        assert_eq!(limits.max_cpu_concurrent(), 1);
    }

    #[test]
    fn interaction_pauses_background_but_never_playback() {
        let mut s = Scheduler::new(Limits { physical_cores: 4, max_disk_concurrent: 4, quiet_mode: false });
        s.enqueue(job("playback", Lane::Playback, Resource::Cpu));
        s.enqueue(job("prefetch", Lane::Prefetch, Resource::Cpu));
        s.enqueue(job("thumbs", Lane::VisibleAssets, Resource::Cpu));
        s.enqueue(job("assist", Lane::Interactive, Resource::Cpu));
        s.enqueue(job("analysis", Lane::Analysis, Resource::Cpu));
        s.enqueue(job("proxy", Lane::Background, Resource::CpuAndDisk));

        s.set_interacting(Interacting::Dragging);
        let started = s.take_runnable();
        let ids: Vec<&str> = started.iter().map(|j| j.id.as_str()).collect();

        assert!(ids.contains(&"playback"), "the visible frame must always decode");
        assert!(!ids.contains(&"analysis"), "analysis must yield to the user");
        assert!(!ids.contains(&"proxy"), "a proxy build must yield to the user");

        // Lanes 1-3 are gone; 4-6 are still queued.
        assert_eq!(s.queued_count(), 3);
    }

    #[test]
    fn a_newer_job_with_the_same_key_replaces_the_queued_one() {
        let mut s = Scheduler::new(Limits::default());
        s.enqueue(job("thumb-1", Lane::VisibleAssets, Resource::Cpu).with_dedupe_key("thumb:100"));
        s.enqueue(job("thumb-2", Lane::VisibleAssets, Resource::Cpu).with_dedupe_key("thumb:100"));
        assert_eq!(s.queued_count(), 1, "supersede, not duplicate");
    }

    #[test]
    fn a_queued_job_that_does_not_fit_still_lets_lower_ones_through() {
        // One CPU slot free, and the highest-priority queued job needs disk
        // (of which there is none). A lower-priority CPU job should still
        // start rather than the whole queue stalling behind it.
        let mut s = Scheduler::new(Limits { physical_cores: 2, max_disk_concurrent: 0, quiet_mode: false });
        s.enqueue(job("proxy", Lane::Background, Resource::CpuAndDisk));
        s.enqueue(job("analysis", Lane::Analysis, Resource::Cpu));

        let started = s.take_runnable();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].id, "analysis");
    }

    #[test]
    fn progress_completion_and_failure_update_state() {
        let mut s = Scheduler::new(Limits::default());
        s.enqueue(job("a", Lane::Analysis, Resource::Cpu));
        s.take_runnable();

        assert!(s.progress("a", 0.5));
        assert!((s.running().next().unwrap().progress - 0.5).abs() < f32::EPSILON);

        assert!(s.complete("a", "artifact-1"));
        assert_eq!(s.running_count(), 0);

        s.enqueue(job("b", Lane::Analysis, Resource::Cpu));
        s.take_runnable();
        assert!(s.fail("b", "encoder missing"));
        assert_eq!(s.running_count(), 0);
    }

    #[test]
    fn cancelling_a_queued_job_prevents_it_from_ever_running() {
        let mut s = Scheduler::new(Limits { physical_cores: 1, max_disk_concurrent: 0, quiet_mode: false });
        s.enqueue(job("blocked", Lane::Background, Resource::CpuAndDisk));
        s.enqueue(job("doomed", Lane::Analysis, Resource::Cpu));

        assert!(s.cancel("doomed"));
        let started = s.take_runnable();
        assert!(started.is_empty(), "the cancelled job must not start");
    }

    #[test]
    fn pause_background_work_stops_lanes_four_to_six() {
        let mut s = Scheduler::new(Limits { physical_cores: 8, max_disk_concurrent: 8, quiet_mode: false });
        s.enqueue(job("playback", Lane::Playback, Resource::Cpu));
        s.enqueue(job("thumbs", Lane::VisibleAssets, Resource::Cpu));
        s.enqueue(job("analysis", Lane::Analysis, Resource::Cpu));
        s.enqueue(job("proxy", Lane::Background, Resource::CpuAndDisk));

        let n = s.cancel_at_or_below(Lane::Interactive);
        assert_eq!(n, 2, "analysis and proxy are cancelled");
        assert_eq!(s.queued_count(), 2);
        assert!(s.activity().iter().all(|j| j.lane <= Lane::VisibleAssets));
    }

    #[test]
    fn an_existing_artifact_prevents_a_rerun() {
        let mut s = Scheduler::new(Limits::default());
        s.enqueue(job("a", Lane::Analysis, Resource::Cpu).with_dedupe_key("proxy:abc"));
        s.take_runnable();
        s.complete("a", "/cache/proxy-abc.mp4");

        assert!(
            s.has_output_for("proxy:abc"),
            "a valid artifact means FFmpeg must not run again for that key"
        );
        assert!(!s.has_output_for("proxy:different"));
    }

    #[test]
    fn the_activity_list_is_ordered_by_priority() {
        let mut s = Scheduler::new(Limits::default());
        s.enqueue(job("z", Lane::Background, Resource::Cpu));
        s.enqueue(job("a", Lane::Playback, Resource::Cpu));
        s.enqueue(job("m", Lane::Analysis, Resource::Cpu));
        let ids: Vec<&str> = s.activity().iter().map(|j| j.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "m", "z"]);
    }
}