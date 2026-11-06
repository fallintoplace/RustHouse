use super::*;

#[test]
fn schema_parser_preserves_row_count_for_post_query_consistency_checks() {
    let first =
        parse_peer_schema(br#"{"rows":12,"columns":[{"name":"id","type":"UInt64"}]}"#).unwrap();
    let updated =
        parse_peer_schema(br#"{"rows":13,"columns":[{"name":"id","type":"UInt64"}]}"#).unwrap();
    assert_eq!(first.columns, updated.columns);
    assert_ne!(first.rows, updated.rows);
}

#[test]
fn schema_parser_rejects_unrepresentable_and_noncanonical_row_counts() {
    for bad in [
        br#"{"rows":18446744073709551616,"columns":[{"name":"id","type":"UInt64"}]}"#
            .as_slice(),
        br#"{"rows":01,"columns":[{"name":"id","type":"UInt64"}]}"#.as_slice(),
        br#"{"rows":-1,"columns":[{"name":"id","type":"UInt64"}]}"#.as_slice(),
    ] {
        assert!(parse_peer_schema(bad).is_err());
    }
}
