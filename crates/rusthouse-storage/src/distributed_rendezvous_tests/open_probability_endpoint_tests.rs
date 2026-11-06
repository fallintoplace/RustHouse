use super::*;

#[test]
fn unit_interval_excludes_zero_and_one_for_all_hash_boundaries() {
    for hash in [0, 1, 2047, 2048, 4095, u64::MAX - 2048, u64::MAX] {
        let probability = hash_to_open_unit_interval(hash);
        assert!(
            probability > 0.0 && probability < 1.0,
            "{hash}: {probability}"
        );
        assert!((-probability.ln()).is_finite());
    }
    assert_eq!(hash_to_open_unit_interval(0), 1.0 / ((1u64 << 53) as f64));
    assert_eq!(
        hash_to_open_unit_interval(u64::MAX),
        ((1u64 << 53) - 1) as f64 / ((1u64 << 53) as f64),
    );
}
