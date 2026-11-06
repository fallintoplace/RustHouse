use super::*;

fn mock_schema(rows: &str) -> remote::PeerResponse {
    remote::PeerResponse {
        status: 200,
        content_type: "application/json; charset=utf-8".into(),
        body: format!(
            "{{\"rows\":{rows},\"columns\":[{{\"name\":\"id\",\"type\":\"UInt64\"}}]}}\n"
        )
        .into_bytes(),
    }
}

#[test]
fn requires_a_canonical_unsigned_64_bit_row_count() {
    assert!(schema_signature(&mock_schema("0")).is_ok());
    assert!(schema_signature(&mock_schema(&u64::MAX.to_string())).is_ok());
    for count in ["", "01", "-1", "1.0", "18446744073709551616"] {
        assert!(schema_signature(&mock_schema(count)).is_err(), "{count:?}");
    }
}
