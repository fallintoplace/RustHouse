use super::*;

#[test]
fn rewrites_validated_ordered_global_pagination() {
    let sql = "SELECT id FROM events WHERE id > 4 ORDER BY id DESC LIMIT 3 OFFSET 2";
    let parsed = parse_select(sql).unwrap();
    assert!(is_safe_sorted_scan(&parsed));
    assert_eq!(
        sorted_scan_sql(sql, &parsed).as_deref(),
        Some("SELECT id FROM events WHERE id > 4 ORDER BY id DESC LIMIT 5")
    );
    assert!(!is_safe_sorted_scan(
        &parse_select("SELECT id, name FROM events ORDER BY id LIMIT 3").unwrap()
    ));
}
