use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DistributionSummary {
    pub sample_count: usize,
    pub min: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub median_absolute_deviation: f64,
    pub p50_bootstrap_ci95: [f64; 2],
    pub p95_bootstrap_ci95: [f64; 2],
    pub quantile_method: String,
    pub bootstrap_resamples: usize,
}

pub fn summarize_samples(
    samples: &[f64],
    bootstrap_resamples: usize,
    seed: u64,
) -> Result<DistributionSummary, String> {
    if samples.is_empty() {
        return Err("Cannot summarize an empty sample set".to_string());
    }
    if samples
        .iter()
        .any(|sample| !sample.is_finite() || *sample < 0.0)
    {
        return Err("Samples must be finite non-negative values".to_string());
    }
    if bootstrap_resamples == 0 {
        return Err("bootstrap_resamples must be greater than zero".to_string());
    }
    let mut ordered = samples.to_vec();
    ordered.sort_by(f64::total_cmp);
    let p50 = quantile_r7(&ordered, 0.50);
    let p95 = quantile_r7(&ordered, 0.95);
    let mut deviations = ordered
        .iter()
        .map(|sample| (sample - p50).abs())
        .collect::<Vec<_>>();
    deviations.sort_by(f64::total_cmp);

    let (p50_ci, p95_ci) =
        bootstrap_quantile_ci(&ordered, bootstrap_resamples, seed, &[0.50, 0.95]);
    Ok(DistributionSummary {
        sample_count: ordered.len(),
        min: ordered[0],
        p50,
        p95,
        max: ordered[ordered.len() - 1],
        median_absolute_deviation: quantile_r7(&deviations, 0.50),
        p50_bootstrap_ci95: p50_ci,
        p95_bootstrap_ci95: p95_ci,
        quantile_method: "R-7 linear interpolation".to_string(),
        bootstrap_resamples,
    })
}

fn quantile_r7(ordered: &[f64], probability: f64) -> f64 {
    if ordered.len() == 1 {
        return ordered[0];
    }
    let index = probability * (ordered.len() - 1) as f64;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    let fraction = index - lower as f64;
    ordered[lower] + (ordered[upper] - ordered[lower]) * fraction
}

fn bootstrap_quantile_ci(
    samples: &[f64],
    resamples: usize,
    seed: u64,
    probabilities: &[f64; 2],
) -> ([f64; 2], [f64; 2]) {
    let mut generator = DeterministicRng::new(seed);
    let mut first = Vec::with_capacity(resamples);
    let mut second = Vec::with_capacity(resamples);
    let mut resample = Vec::with_capacity(samples.len());
    for _ in 0..resamples {
        resample.clear();
        for _ in 0..samples.len() {
            resample.push(samples[generator.index(samples.len())]);
        }
        resample.sort_by(f64::total_cmp);
        first.push(quantile_r7(&resample, probabilities[0]));
        second.push(quantile_r7(&resample, probabilities[1]));
    }
    first.sort_by(f64::total_cmp);
    second.sort_by(f64::total_cmp);
    (
        [quantile_r7(&first, 0.025), quantile_r7(&first, 0.975)],
        [quantile_r7(&second, 0.025), quantile_r7(&second, 0.975)],
    )
}

pub(crate) struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.state = value;
        value.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub(crate) fn index(&mut self, upper: usize) -> usize {
        debug_assert!(upper > 0);
        (self.next_u64() % upper as u64) as usize
    }

    pub(crate) fn shuffle<T>(&mut self, values: &mut [T]) {
        for index in (1..values.len()).rev() {
            let swap_with = self.index(index + 1);
            values.swap(index, swap_with);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_reports_r7_percentiles_mad_and_deterministic_intervals() {
        let samples = (1..=100).map(|value| value as f64).collect::<Vec<_>>();
        let first = summarize_samples(&samples, 1_000, 42).unwrap();
        let second = summarize_samples(&samples, 1_000, 42).unwrap();
        assert_eq!(first.p50, 50.5);
        assert!((first.p95 - 95.05).abs() < 1e-9);
        assert_eq!(first.median_absolute_deviation, 25.0);
        assert_eq!(first.p50_bootstrap_ci95, second.p50_bootstrap_ci95);
        assert_eq!(first.p95_bootstrap_ci95, second.p95_bootstrap_ci95);
    }

    #[test]
    fn success_and_failure_samples_cannot_be_accidentally_combined() {
        assert!(summarize_samples(&[], 1_000, 1).is_err());
        assert!(summarize_samples(&[1.0, f64::NAN], 1_000, 1).is_err());
    }
}
