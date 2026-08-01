//! Latency and loss injection.
//!
//! Sampling goes through a trait so tests are deterministic. The production
//! implementation draws from the thread-local OS-seeded generator.

use std::time::Duration;

use doppel_core::config::{LatencyConfig, LossConfig};

/// Draws a value in `[0.0, 1.0)`.
pub trait Sampler: Send + Sync {
    fn sample(&self) -> f64;
}

/// The production sampler.
pub struct OsSampler;

impl Sampler for OsSampler {
    fn sample(&self) -> f64 {
        rand::random::<f64>()
    }
}

/// A sampler that returns a fixed sequence, then panics. Panicking on
/// exhaustion is deliberate: it turns "sampled more times than expected" into a
/// test failure instead of a silent pass.
pub struct SequenceSampler {
    values: std::sync::Mutex<std::collections::VecDeque<f64>>,
}

impl SequenceSampler {
    #[must_use]
    pub fn new(values: Vec<f64>) -> Self {
        Self {
            values: std::sync::Mutex::new(values.into()),
        }
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.values.lock().expect("sampler mutex").len()
    }
}

impl Sampler for SequenceSampler {
    fn sample(&self) -> f64 {
        self.values
            .lock()
            .expect("sampler mutex")
            .pop_front()
            .expect("SequenceSampler exhausted: more draws than the test expected")
    }
}

/// What faults apply to one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FaultDecision {
    /// When set, respond with this status and do not contact the upstream.
    pub loss_status: Option<u16>,
    /// When set, sleep this long before proceeding.
    pub latency: Option<Duration>,
}

/// Decide the faults for one request. Loss is evaluated first and short
/// circuits: a dropped request is not delayed first.
pub fn decide(
    loss: Option<&LossConfig>,
    latency: Option<&LatencyConfig>,
    sampler: &dyn Sampler,
) -> FaultDecision {
    if let Some(loss) = loss
        && fires(loss.percentage, sampler)
    {
        return FaultDecision {
            loss_status: Some(loss.status),
            latency: None,
        };
    }

    let delay = latency.and_then(|cfg| {
        if fires(cfg.percentage, sampler) {
            let span = cfg.max - cfg.min;
            let seconds = cfg.min + span * sampler.sample();
            Some(Duration::from_secs_f64(seconds))
        } else {
            None
        }
    });

    FaultDecision {
        loss_status: None,
        latency: delay,
    }
}

/// `percentage` of 0.0 never fires and 1.0 always does, because the sampler's
/// range is half-open.
fn fires(percentage: f64, sampler: &dyn Sampler) -> bool {
    percentage > 0.0 && sampler.sample() < percentage
}

#[cfg(test)]
mod tests {
    use super::*;
    use doppel_core::config::{LatencyConfig, LossConfig};

    fn loss(percentage: f64) -> LossConfig {
        LossConfig {
            percentage,
            status: 503,
        }
    }

    fn latency(percentage: f64) -> LatencyConfig {
        LatencyConfig {
            percentage,
            min: 0.1,
            max: 0.2,
        }
    }

    #[test]
    fn zero_percent_never_fires() {
        let s = SequenceSampler::new(vec![0.0, 0.0]);
        let d = decide(Some(&loss(0.0)), Some(&latency(0.0)), &s);
        assert_eq!(d.loss_status, None);
        assert_eq!(d.latency, None);
    }

    #[test]
    fn one_hundred_percent_always_fires() {
        let s = SequenceSampler::new(vec![0.999_999]);
        let d = decide(Some(&loss(1.0)), None, &s);
        assert_eq!(d.loss_status, Some(503));
    }

    #[test]
    fn loss_short_circuits_latency() {
        // Only one draw is available; if latency were sampled the sampler would
        // be exhausted and this test would panic.
        let s = SequenceSampler::new(vec![0.0]);
        let d = decide(Some(&loss(1.0)), Some(&latency(1.0)), &s);
        assert_eq!(d.loss_status, Some(503));
        assert_eq!(
            d.latency, None,
            "latency must not be sampled once loss fires"
        );
        assert_eq!(s.remaining(), 0);
    }

    #[test]
    fn latency_is_drawn_within_bounds() {
        let s = SequenceSampler::new(vec![0.0, 0.0, 0.5]);
        let d = decide(None, Some(&latency(1.0)), &s);
        let delay = d.latency.expect("latency should fire");
        assert!(
            delay >= Duration::from_millis(100) && delay <= Duration::from_millis(200),
            "{delay:?}"
        );
    }

    #[test]
    fn latency_draw_of_zero_gives_the_minimum_and_one_gives_the_maximum() {
        let low = decide(
            None,
            Some(&latency(1.0)),
            &SequenceSampler::new(vec![0.0, 0.0]),
        );
        assert_eq!(low.latency, Some(Duration::from_millis(100)));

        let high = decide(
            None,
            Some(&latency(1.0)),
            &SequenceSampler::new(vec![0.0, 1.0]),
        );
        assert_eq!(high.latency, Some(Duration::from_millis(200)));
    }

    #[test]
    fn absent_config_means_no_fault() {
        let s = SequenceSampler::new(vec![]);
        let d = decide(None, None, &s);
        assert_eq!(d.loss_status, None);
        assert_eq!(d.latency, None);
    }

    #[test]
    fn equal_min_and_max_yield_a_fixed_delay() {
        let cfg = LatencyConfig {
            percentage: 1.0,
            min: 0.25,
            max: 0.25,
        };
        let d = decide(None, Some(&cfg), &SequenceSampler::new(vec![0.0, 0.7]));
        assert_eq!(d.latency, Some(Duration::from_millis(250)));
    }

    #[test]
    fn os_sampler_stays_in_range() {
        let s = OsSampler;
        for _ in 0..1000 {
            let value = s.sample();
            assert!((0.0..1.0).contains(&value), "{value}");
        }
    }
}
