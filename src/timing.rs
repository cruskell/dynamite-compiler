//! Optional wall-clock stage measurements for build profiling.

use std::time::Instant;

/// Emit a tab-separated `dynamite-timing`, PID, stage, seconds record on drop
/// when `DYNAMITE_TIMINGS` is set. Nested stages must be interpreted separately.
pub struct Timer {
    stage: &'static str,
    start: Option<Instant>,
}

impl Timer {
    pub fn new(stage: &'static str) -> Self {
        Self {
            stage,
            start: std::env::var_os("DYNAMITE_TIMINGS").map(|_| Instant::now()),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            eprintln!(
                "dynamite-timing\t{}\t{}\t{:.9}",
                std::process::id(),
                self.stage,
                start.elapsed().as_secs_f64()
            );
        }
    }
}
