use super::*;

#[test]
fn pushes_only_verified_prefixes_and_preserves_quoted_keywords() {
    for (sql, expected) in [
        ("SELECT id FROM events LIMIT 5", "SELECT id FROM events LIMIT 5"),
        ("SELECT id FROM events LIMIT 5 OFFSET 7", "SELECT id FROM events LIMIT 12"),
        ("SELECT name FROM events WHERE name = 'LIMIT 9' LIMIT 2 OFFSET 3",
         "SELECT name FROM events WHERE name = 'LIMIT 9' LIMIT 5"),
    ] {
        let select = parse_select(sql).unwrap();
        assert_eq!(bounded_scan_sql(sql, &select).as_deref(), Some(expected));
    }
    let unbounded = "SELECT id FROM events OFFSET 5";
    let select = parse_select(unbounded).unwrap();
    assert_eq!(bounded_scan_sql(unbounded, &select).as_deref(),
        Some("SELECT id FROM events"));
}
