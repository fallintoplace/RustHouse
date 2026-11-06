use super::*;

#[test]
fn bounds_parallel_query_workers_before_binding_server() {
    assert_eq!(parse_max_connections("1").unwrap(), 1);
    assert_eq!(parse_max_connections("16").unwrap(), 16);
    assert_eq!(parse_max_connections("64").unwrap(), 64);
    for invalid in ["", "0", "65", "1000000", "-1", "4.5", "many"] {
        assert!(parse_max_connections(invalid).is_err(), "{invalid}");
    }
    assert_eq!(MAX_CONNECTIONS, 16);
}
