use crate::{Distribution, MetricsError, Stage};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
struct Histogram {
    bins: [u64; 128],
    count: u64,
    maximum: f64,
    sum: f64,
    squared: f64,
}
impl Default for Histogram {
    fn default() -> Self {
        Self {
            bins: [0; 128],
            count: 0,
            maximum: 0.0,
            sum: 0.0,
            squared: 0.0,
        }
    }
}
impl Histogram {
    fn push(&mut self, value: f64) -> Result<(), MetricsError> {
        if !value.is_finite() || !(0.0..=1e12).contains(&value) {
            return Err(MetricsError::Invalid("invalid window sample"));
        }
        let index = if value < 1.0 {
            0
        } else {
            let power = value.log2().floor();
            let base = 2_f64.powf(power);
            (1 + (power as usize) * 4 + ((value / base - 1.0) * 4.0).floor() as usize).min(127)
        };
        self.bins[index] += 1;
        self.count += 1;
        self.maximum = self.maximum.max(value);
        self.sum += value;
        self.squared += value * value;
        Ok(())
    }
    fn merge(&mut self, other: &Self) {
        for (a, b) in self.bins.iter_mut().zip(&other.bins) {
            *a += *b;
        }
        self.count += other.count;
        self.maximum = self.maximum.max(other.maximum);
        self.sum += other.sum;
        self.squared += other.squared;
    }
    fn distribution(&self) -> Option<Distribution> {
        if self.count == 0 {
            return None;
        }
        let percentile = |percent: u64| {
            let rank = (self.count * percent).div_ceil(100);
            let mut count = 0;
            for (index, bin) in self.bins.iter().enumerate() {
                count += *bin;
                if count >= rank {
                    let upper = if index == 0 {
                        1.0
                    } else if index == 127 {
                        self.maximum
                    } else {
                        let i = index - 1;
                        2_f64.powi((i / 4) as i32) * (1.0 + (i % 4 + 1) as f64 / 4.0)
                    };
                    return upper.min(self.maximum);
                }
            }
            self.maximum
        };
        Some(Distribution {
            count: self.count.min(usize::MAX as u64) as usize,
            total_samples: self.count,
            p50: percentile(50),
            p95: percentile(95),
            p99: percentile(99),
            maximum: self.maximum,
            mean: self.sum / self.count as f64,
            rms: (self.squared / self.count as f64).sqrt(),
        })
    }
}
struct Bucket {
    second: u64,
    stages: [Histogram; 8],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowDistribution {
    pub seconds: Option<u64>,
    pub latency: Vec<Option<Distribution>>,
    pub quantiles: String,
}
/// One-second histogram buckets for the last hour. Fixed maximum storage ~30 MiB.
/// Quarter-octave quantile upper bounds; exact max, count, mean and RMS.
pub struct TimeWindows {
    buckets: VecDeque<Bucket>,
    session: [Histogram; 8],
    latest_second: u64,
}
impl Default for TimeWindows {
    fn default() -> Self {
        Self {
            buckets: VecDeque::with_capacity(3601),
            session: std::array::from_fn(|_| Histogram::default()),
            latest_second: 0,
        }
    }
}
impl TimeWindows {
    pub fn record(
        &mut self,
        timestamp: fly_core::FrameTimestamp,
        stage: Stage,
        value_us: f64,
    ) -> Result<(), MetricsError> {
        let second = timestamp.0 / 1_000_000;
        if self
            .buckets
            .back()
            .is_some_and(|bucket| second < bucket.second)
        {
            return Err(MetricsError::Invalid("window timestamp moved backwards"));
        }
        self.latest_second = second;
        while self
            .buckets
            .front()
            .is_some_and(|bucket| bucket.second.saturating_add(3600) < second)
        {
            self.buckets.pop_front();
        }
        if self
            .buckets
            .back()
            .is_none_or(|bucket| bucket.second != second)
        {
            if self.buckets.len() == 3601 {
                self.buckets.pop_front();
            }
            self.buckets.push_back(Bucket {
                second,
                stages: std::array::from_fn(|_| Histogram::default()),
            });
        }
        self.session[stage.index()].push(value_us)?;
        if let Some(bucket) = self.buckets.back_mut() {
            bucket.stages[stage.index()].push(value_us)?;
        }
        Ok(())
    }
    pub fn distributions(&self) -> Vec<WindowDistribution> {
        [Some(60),Some(300),Some(900),Some(3600),None].into_iter().map(|seconds|{
            let mut combined:[Histogram;8]=std::array::from_fn(|_|Histogram::default());
            if let Some(seconds)=seconds{let start=self.latest_second.saturating_sub(seconds-1);for bucket in self.buckets.iter().filter(|bucket|bucket.second>=start){for (a,b) in combined.iter_mut().zip(&bucket.stages){a.merge(b);}}}else{combined.clone_from(&self.session);}
            WindowDistribution{seconds,latency:combined.iter().map(Histogram::distribution).collect(),quantiles:"quarter-octave upper bounds; source-clock windows rounded to seconds; count/max/mean/RMS exact".into()}
        }).collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_expire_but_session_retains_samples() -> Result<(), MetricsError> {
        let mut windows = TimeWindows::default();
        windows.record(fly_core::FrameTimestamp(0), Stage::Capture, 10.0)?;
        windows.record(fly_core::FrameTimestamp(120_000_000), Stage::Capture, 20.0)?;
        let stats = windows.distributions();
        assert_eq!(stats[0].latency[0].as_ref().map(|d| d.count), Some(1));
        assert_eq!(stats[1].latency[0].as_ref().map(|d| d.count), Some(2));
        assert_eq!(stats[4].latency[0].as_ref().map(|d| d.maximum), Some(20.0));
        assert!(
            windows
                .record(fly_core::FrameTimestamp(1), Stage::Capture, 1.0)
                .is_err()
        );
        Ok(())
    }
}
