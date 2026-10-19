//! Phase boundaries, not a sampler or a scheduler. RSS observations at the
//! endpoints are not the peak RSS during a phase; external sampling owns peaks.
use std::time::Instant;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseMeasurement {
    pub phase: String,
    pub wall_seconds: f64,
    pub rss_before_bytes: Option<u64>,
    pub rss_after_bytes: Option<u64>,
}

pub struct PhaseTimer {
    phase: &'static str,
    start: Instant,
    rss: Option<u64>,
    finished: bool,
}
impl PhaseTimer {
    pub fn start(phase: &'static str) -> Self {
        let rss = resident_bytes();
        tracing::info!(phase, rss_bytes = rss, "pipeline phase started");
        Self {
            phase,
            start: Instant::now(),
            rss,
            finished: false,
        }
    }
    pub fn finish(mut self) -> PhaseMeasurement {
        self.finished = true;
        let measurement = PhaseMeasurement {
            phase: self.phase.into(),
            wall_seconds: self.start.elapsed().as_secs_f64(),
            rss_before_bytes: self.rss,
            rss_after_bytes: resident_bytes(),
        };
        tracing::info!(
            phase = self.phase,
            wall_seconds = measurement.wall_seconds,
            rss_before_bytes = measurement.rss_before_bytes,
            rss_after_bytes = measurement.rss_after_bytes,
            "pipeline phase completed"
        );
        measurement
    }
}
impl Drop for PhaseTimer {
    fn drop(&mut self) {
        if !self.finished {
            tracing::warn!(
                phase = self.phase,
                wall_seconds = self.start.elapsed().as_secs_f64(),
                rss_bytes = resident_bytes(),
                "pipeline phase did not complete"
            );
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)] // Windows exposes process counters through a pointer-taking OS API.
fn resident_bytes() -> Option<u64> {
    use windows::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
    counters.cb = size;
    // SAFETY: the current-process pseudo handle is valid for this read. The
    // writable counters buffer has exactly the size supplied to the API.
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, size) }.ok()?;
    Some(counters.WorkingSetSize as u64)
}

#[cfg(target_os = "linux")]
fn resident_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let rss = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?;
    rss.split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn resident_bytes() -> Option<u64> {
    None
}
