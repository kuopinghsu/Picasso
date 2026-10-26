//! Frame times for checking that scrolling and browsing stay smooth. Start
//! Picasso with `PICASSO_FRAME_STATS=1` and every five seconds of drawing
//! prints a line like this to the terminal:
//!
//! `frames 300 · cpu per frame: median 2.1 ms, 95% 4.0 ms, slowest 9.8 ms · over 16.7 ms: 0`
//!
//! A frame over 16.7 ms misses a 60 Hz display refresh and shows as a stutter.

use std::time::{Duration, Instant};

const REPORT_EVERY: Duration = Duration::from_secs(5);
/// One refresh of a 60 Hz display.
const BUDGET_MS: f32 = 1000.0 / 60.0;

pub(super) struct FrameStats {
    started: Instant,
    cpu_ms: Vec<f32>,
}

impl FrameStats {
    pub(super) fn from_env() -> Option<Self> {
        std::env::var("PICASSO_FRAME_STATS")
            .is_ok_and(|value| value == "1")
            .then(|| FrameStats { started: Instant::now(), cpu_ms: Vec::new() })
    }

    /// Takes the time the last frame spent drawing, in seconds.
    pub(super) fn record(&mut self, cpu_seconds: Option<f32>) {
        if let Some(seconds) = cpu_seconds {
            self.cpu_ms.push(seconds * 1000.0);
        }
        if self.started.elapsed() >= REPORT_EVERY {
            if let Some(line) = summary(&mut self.cpu_ms) {
                eprintln!("{line}");
            }
            self.cpu_ms.clear();
            self.started = Instant::now();
        }
    }
}

fn summary(cpu_ms: &mut [f32]) -> Option<String> {
    if cpu_ms.is_empty() {
        return None;
    }
    cpu_ms.sort_by(f32::total_cmp);
    let at = |share: f32| cpu_ms[((cpu_ms.len() - 1) as f32 * share).round() as usize];
    let over = cpu_ms.iter().filter(|&&ms| ms > BUDGET_MS).count();
    Some(format!(
        "frames {} · cpu per frame: median {:.1} ms, 95% {:.1} ms, slowest {:.1} ms · over 16.7 ms: {over}",
        cpu_ms.len(),
        at(0.5),
        at(0.95),
        cpu_ms[cpu_ms.len() - 1],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_names_the_median_the_tail_and_the_slow_frames() {
        let mut frames: Vec<f32> = (1..=100).map(|ms| ms as f32 / 5.0).collect();
        let line = summary(&mut frames).unwrap();
        assert_eq!(line, "frames 100 · cpu per frame: median 10.2 ms, 95% 19.0 ms, slowest 20.0 ms · over 16.7 ms: 17");
        assert_eq!(summary(&mut []), None);
    }
}
