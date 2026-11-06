use crate::parse_select;

fn arm(key: usize, prefix: &str) -> String {
    let categories = (0..4)
        .map(|category| {
            format!(
                "category = '{prefix}{category}' AND (color = 'a' OR color = 'b') AND (units = 'x' OR units = 'y') AND (size = 'm' OR size = 'l')"
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    format!("(key = {key} AND ({categories}))")
}

#[test]
fn accepts_64_nested_branches_with_320_expanded_conditions() {
    let sql = format!(
        "SELECT count(*) FROM items WHERE {} OR {}",
        arm(1, "a"),
        arm(2, "b")
    );
    let select = parse_select(&sql).unwrap();
    assert_eq!(select.or_predicates.len() + 1, 64);
    assert_eq!(select.predicates.len(), 5);
    assert!(select.or_predicates.iter().all(|branch| branch.len() == 5));
}
