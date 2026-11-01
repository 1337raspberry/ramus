//! Convert dB amplitude levels into 0.0..1.0 values for waveform rendering.

/// Normalize dB levels to 0.0..1.0 for display.
///
/// Input: negative dB values (e.g., -35.0 quiet, -3.0 loud).
pub fn normalize_db_levels(db_levels: &[f32]) -> Vec<f32> {
    if db_levels.is_empty() {
        return Vec::new();
    }

    // linear = 10^(dB/20)
    let linear: Vec<f32> = db_levels.iter().map(|db| 10.0f32.powf(db / 20.0)).collect();

    let max_val = linear.iter().copied().fold(0.0f32, f32::max);

    if max_val <= 0.0 {
        return linear;
    }

    linear.iter().map(|v| v / max_val).collect()
}

/// Shrink a waveform to `points` values for a small display: the mean of
/// each bucket, clamped to 0..=1 and rounded to three decimals so the result
/// stays compact as JSON. Input already no longer than `points` is returned
/// as it is.
pub fn downsample_levels(levels: &[f32], points: usize) -> Vec<f32> {
    if levels.len() <= points || points == 0 {
        return levels.to_vec();
    }
    let n = levels.len();
    (0..points)
        .map(|j| {
            let bucket = &levels[j * n / points..(j + 1) * n / points];
            let mean = bucket.iter().sum::<f32>() / bucket.len() as f32;
            (mean.clamp(0.0, 1.0) * 1000.0).round() / 1000.0
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        assert!(normalize_db_levels(&[]).is_empty());
    }

    #[test]
    fn test_single_value_normalizes_to_one() {
        let result = normalize_db_levels(&[-6.0]);
        assert_eq!(result.len(), 1);
        assert!((result[0] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_loudest_normalizes_to_one() {
        let result = normalize_db_levels(&[-20.0, -6.0, -3.0]);
        assert_eq!(result.len(), 3);
        assert!((result[2] - 1.0).abs() < 0.001);
        assert!(result[0] < result[1]);
        assert!(result[1] < result[2]);
    }

    #[test]
    fn test_all_same_values_normalize_to_one() {
        let result = normalize_db_levels(&[-10.0, -10.0, -10.0]);
        for v in &result {
            assert!((v - 1.0).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn test_output_range_zero_to_one() {
        let input: Vec<f32> = (-40..0).map(|x| x as f32).collect();
        let result = normalize_db_levels(&input);
        for v in &result {
            assert!(*v >= 0.0 && *v <= 1.0, "value {v} out of 0..1 range");
        }
    }

    #[test]
    fn test_zero_db_is_loudest() {
        let result = normalize_db_levels(&[-20.0, 0.0, -10.0]);
        assert!((result[1] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_relative_proportions() {
        // -6 dB ≈ 0.5 linear, 0 dB = 1.0 linear
        let result = normalize_db_levels(&[-6.0, 0.0]);
        assert!((result[1] - 1.0).abs() < 0.001);
        assert!((result[0] - 0.501).abs() < 0.01);
    }

    #[test]
    fn downsample_600_to_96_is_mean_per_bucket() {
        let levels: Vec<f32> = (0..600).map(|i| (i % 2) as f32).collect();
        let out = downsample_levels(&levels, 96);
        assert_eq!(out.len(), 96);
        // 600 / 96 is 6.25, so buckets hold 6 or 7 alternating 0/1 values.
        for v in &out {
            assert!((0.42..=0.58).contains(v), "bucket mean {v}");
        }
        let ramp: Vec<f32> = (0..192).map(|i| i as f32 / 191.0).collect();
        let r = downsample_levels(&ramp, 96);
        assert_eq!(
            r[0],
            ((0.0 + 1.0 / 191.0) / 2.0 * 1000.0f32).round() / 1000.0
        );
    }

    #[test]
    fn downsample_short_input_unchanged() {
        assert_eq!(downsample_levels(&[0.25, 0.5], 96), vec![0.25, 0.5]);
        assert!(downsample_levels(&[], 96).is_empty());
    }

    #[test]
    fn downsample_clamps_and_rounds() {
        let out = downsample_levels(&[1.5, 1.5, -0.2, -0.2, 0.12345, 0.12345], 3);
        assert_eq!(out, vec![1.0, 0.0, 0.123]);
    }
}
