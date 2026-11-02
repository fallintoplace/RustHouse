use rusthouse_query::parse_select;
#[test]
fn order_using_maps_relations_to_sort_direction() {
    let asc = parse_select("SELECT number FROM numbers(3) ORDER BY number USING <").unwrap();
    assert!(!asc.order_by.unwrap().descending);
    let desc = parse_select("SELECT number FROM numbers(3) ORDER BY number USING >").unwrap();
    assert!(desc.order_by.unwrap().descending);
    assert!(parse_select("SELECT number FROM numbers(3) ORDER BY number USING =").is_err());
}
