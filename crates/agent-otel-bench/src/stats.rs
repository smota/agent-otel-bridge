/*
 * Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0
 */

#[derive(Debug, Clone)]
pub struct Stats {
    samples: Vec<u64>,
}

impl Stats {
    pub fn new(mut samples: Vec<u64>) -> Self {
        samples.sort_unstable();
        Self { samples }
    }

    pub fn count(&self) -> usize {
        self.samples.len()
    }

    pub fn min(&self) -> u64 {
        self.samples.first().copied().unwrap_or(0)
    }

    pub fn max(&self) -> u64 {
        self.samples.last().copied().unwrap_or(0)
    }

    pub fn mean(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.samples.iter().sum();
        sum as f64 / self.samples.len() as f64
    }

    pub fn percentile(&self, p: f64) -> u64 {
        if self.samples.is_empty() {
            return 0;
        }
        let rank = (p / 100.0 * (self.samples.len() - 1) as f64).round() as usize;
        self.samples[rank.min(self.samples.len() - 1)]
    }

    pub fn p50(&self) -> u64 {
        self.percentile(50.0)
    }

    pub fn p90(&self) -> u64 {
        self.percentile(90.0)
    }

    pub fn p95(&self) -> u64 {
        self.percentile(95.0)
    }

    pub fn p99(&self) -> u64 {
        self.percentile(99.0)
    }

    pub fn p999(&self) -> u64 {
        self.percentile(99.9)
    }

    pub fn summary(&self) -> StatsSummary {
        StatsSummary {
            count: self.count(),
            min_us: self.min(),
            mean_us: (self.mean() * 10.0).round() / 10.0,
            p50_us: self.p50(),
            p90_us: self.p90(),
            p95_us: self.p95(),
            p99_us: self.p99(),
            p999_us: self.p999(),
            max_us: self.max(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StatsSummary {
    pub count: usize,
    pub min_us: u64,
    pub mean_us: f64,
    pub p50_us: u64,
    pub p90_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub p999_us: u64,
    pub max_us: u64,
}
