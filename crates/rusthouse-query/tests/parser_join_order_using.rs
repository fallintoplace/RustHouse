use rusthouse_query::parse_join;

#[test]
fn join_sort_direction_accepts_using_comparison() {
    let base = "SELECT a.id FROM a JOIN b ON a.id = b.id ORDER BY a.id";
    let asc = parse_join(&format!("{base} USING <")).unwrap().unwrap();
    let desc = parse_join(&format!("{base} USING >")).unwrap().unwrap();
    assert!(!asc.order_by[0].1);
    assert!(desc.order_by[0].1);
    assert!(parse_join(&format!("{base} USING =")).is_err());
    assert!(parse_join(&format!("{base} DESC")).unwrap().is_some());
}
