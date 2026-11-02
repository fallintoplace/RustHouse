use rusthouse_query::parse_select;

#[test]
fn bitwise_aggregate_names_match_existing_functions() {
    for (name, original) in [
        ("BIT_AND", "groupBitAnd"),
        ("BIT_OR", "groupBitOr"),
        ("BIT_XOR", "groupBitXor"),
    ] {
        for (before, after) in [
            (format!("SELECT {name}(number) FROM numbers(4)"),
             format!("SELECT {original}(number) FROM numbers(4)")),
            (format!("SELECT number, {name}(number) FROM numbers(4) GROUP BY number"),
             format!("SELECT number, {original}(number) FROM numbers(4) GROUP BY number")),
            (format!("SELECT {name}(number), COUNT(*) FROM numbers(4)"),
             format!("SELECT {original}(number), COUNT(*) FROM numbers(4)")),
        ] {
            assert_eq!(parse_select(&before).unwrap(), parse_select(&after).unwrap());
        }
    }
}
