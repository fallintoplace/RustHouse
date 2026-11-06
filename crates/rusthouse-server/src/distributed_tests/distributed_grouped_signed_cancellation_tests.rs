use super::*;

fn partial(value: &str) -> (usize, Vec<u8>) {
    (
        10,
        format!("category\tsum(amount)\na\t{value}\n").into_bytes(),
    )
}

#[test]
fn permits_exact_signed_cancellation_across_shards() {
    let max = i128::MAX.to_string();
    let values = [partial(&max), partial("1"), partial("-1")];
    assert_eq!(
        merge_grouped_integer_sums(&values, 10, "Int128", "category", "amount").unwrap(),
        format!("category\tsum(amount)\na\t{max}\n").into_bytes(),
    );
    let min = i128::MIN.to_string();
    let values = [partial(&min), partial("-1"), partial("1")];
    assert_eq!(
        merge_grouped_integer_sums(&values, 10, "Int128", "category", "amount").unwrap(),
        format!("category\tsum(amount)\na\t{min}\n").into_bytes(),
    );
}

#[test]
fn rejects_true_global_signed_overflow_after_reducing_all_shards() {
    let values = [partial(&i128::MAX.to_string()), partial("1")];
    assert_eq!(
        merge_grouped_integer_sums(&values, 10, "Int128", "category", "amount"),
        Err(MergeError::SumOverflow),
    );
}
