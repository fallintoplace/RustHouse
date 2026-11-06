use super::parse_peer_schema;

#[test]
fn schema_parser_only_accepts_rfc_json_whitespace() {
    let canonical = br#"{"rows":1,"columns":[{"name":"id","type":"UInt64"}]}"#;
    for &space in b" \t\r\n" {
        let mut encoded = canonical.to_vec();
        encoded.insert(1, space);
        assert!(
            parse_peer_schema(&encoded).is_ok(),
            "valid JSON whitespace: {space}"
        );
    }
    for &invalid in b"\x0b\x0c" {
        let mut encoded = canonical.to_vec();
        encoded.insert(1, invalid);
        assert!(
            parse_peer_schema(&encoded).is_err(),
            "invalid JSON whitespace: {invalid}"
        );
    }
}
