use super::*;

fn partial(value: &str) -> (usize, Vec<u8>) {
    (10, format!("sum(amount)\n{value}\n").into_bytes())
}

#[test]
fn allows_signed_cancellation_even_when_peer_order_would_overflow() {
    let max = i128::MAX.to_string();
    let min = i128::MIN.to_string();
    let positive = [partial(&max), partial("1"), partial("-1")];
    assert_eq!(
        merge_integer_sums(&positive, "Int128").unwrap(),
        format!("sum(amount)\n{max}\n").into_bytes()
    );
    let negative = [partial(&min), partial("-1"), partial("1")];
    assert_eq!(
        merge_integer_sums(&negative, "Int128").unwrap(),
        format!("sum(amount)\n{min}\n").into_bytes()
    );
}

#[test]
fn still_rejects_truly_overflowing_signed_and_unsigned_totals() {
    assert_eq!(
        merge_integer_sums(&[partial(&i128::MAX.to_string()), partial("1")], "Int128"),
        Err(MergeError::SumOverflow),
    );
    let max = u128::MAX.to_string();
    assert_eq!(
        merge_integer_sums(&[partial(&max), partial("1")], "UInt128"),
        Err(MergeError::SumOverflow),
    );
}
