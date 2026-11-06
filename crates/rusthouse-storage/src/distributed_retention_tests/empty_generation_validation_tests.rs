use super::*;

#[test]
fn rejects_empty_generation_history_without_panicking() {
    assert!(validated_generations(&[], 1).is_err());
}
