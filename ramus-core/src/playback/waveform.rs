//! Waveform level normalization for visualization.
//!
//! Converts dB amplitude levels from audio analysis into 0.0..1.0 values
//! suitable for canvas/UI rendering.

/// Normalize dB levels to 0.0..1.0 range for display.
///
/// Input: array of negative dB values (e.g., -35.0 quiet, -3.0 loud).
/// Output: array of 0.0..1.0 values suitable for waveform rendering.
pub fn normalize_db_levels(db_levels: &[f32]) -> Vec<f32> {
    if db_levels.is_empty() {
        return Vec::new();
    }

    // Convert dB to linear amplitude: linear = 10^(dB/20)
    let linear: Vec<f32> = db_levels.iter().map(|db| 10.0f32.powf(db / 20.0)).collect();

    let max_val = linear.iter().copied().fold(0.0f32, f32::max);

    if max_val <= 0.0 {
        return linear;
    }

    linear.iter().map(|v| v / max_val).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        // -3.0 dB is loudest, should normalize to ~1.0
        assert!((result[2] - 1.0).abs() < 0.001);
        // Ordering preserved: quieter values are smaller
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
        // 0 dB = linear 1.0, which should be the max
        assert!((result[1] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_relative_proportions() {
        // -6 dB ≈ 0.5 linear, 0 dB = 1.0 linear
        let result = normalize_db_levels(&[-6.0, 0.0]);
        assert!((result[1] - 1.0).abs() < 0.001);
        // -6 dB should be roughly 0.5 relative to 0 dB
        assert!((result[0] - 0.501).abs() < 0.01);
    }
}
