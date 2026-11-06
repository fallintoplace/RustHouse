use crate::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn prefix_not_comparisons_share_the_parenthesized_inversion() {
    assert_eq!(
        run("SELECT number FROM numbers(5) WHERE NOT number > 3 ORDER BY number"),
        "number\n0\n1\n2\n3\n"
    );
}

#[test]
fn nested_boolean_groups_respect_and_precedence() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(8) WHERE (number = 1 OR number = 2) AND number >= 2 ORDER BY number"
        ),
        "number\n2\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(8) WHERE number = 1 OR number = 2 AND number > 2 ORDER BY number"
        ),
        "number\n1\n"
    );
}

#[test]
fn integer_arithmetic_is_folded_in_in_lists() {
    assert_eq!(
        run("SELECT number FROM numbers(4) WHERE number IN (1, 1 + 1) ORDER BY number"),
        "number\n1\n2\n"
    );
}

#[test]
fn integer_arithmetic_without_spaces_is_folded_in_in_lists() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(2002) WHERE number IN (1999,1999+1,1999+2) ORDER BY number"
        ),
        "number\n1999\n2000\n2001\n"
    );
}

#[test]
fn integer_bound_stops_before_a_set_operator() {
    let mut parser = crate::Parser {
        tokens: crate::tokenize("1200 + 11 INTERSECT SELECT").unwrap(),
        index: 0,
    };
    assert_eq!(super::numeric_or_literal(&mut parser).unwrap(), "1211");
    assert!(matches!(
        parser.tokens.get(parser.index),
        Some(crate::Token::Word(word)) if word.eq_ignore_ascii_case("INTERSECT")
    ));
}

#[test]
fn integer_arithmetic_is_folded_in_between_bounds() {
    assert_eq!(
        run("SELECT number FROM numbers(5) WHERE number BETWEEN 1 AND 1 + 2 ORDER BY number"),
        "number\n1\n2\n3\n"
    );
}

#[test]
fn distributes_and_over_or_without_changing_between() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(8) WHERE number BETWEEN 1 AND 5 AND (number = 2 OR number = 4) ORDER BY number"
        ),
        "number\n2\n4\n"
    );
}

#[test]
fn not_group_distributes_over_or_and_between_predicates() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(7) WHERE NOT (number = 1 OR number BETWEEN 3 AND 5) ORDER BY number"
        ),
        "number\n0\n2\n6\n",
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(7) WHERE NOT (number = 1 AND number < 2) ORDER BY number"
        ),
        "number\n0\n2\n3\n4\n5\n6\n",
    );
}

#[test]
fn not_group_handles_nested_groups_and_modulo() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(8) WHERE NOT (number % 2 = 0 OR (number > 5 AND number < 7)) ORDER BY number"
        ),
        "number\n1\n3\n5\n7\n",
    );
    assert!(parse_select("SELECT number FROM numbers(8) WHERE NOT (number = 1").is_err());
}

#[test]
fn repeated_eight_atom_filters_simplify_before_branch_budget() {
    // Four independent binary disjunctions produce 16 DNF branches
    // from just eight source predicates. Repeating the group uses the
    // 16-condition parser budget but creates 256 temporary pairs.
    let clause = "(number > 0 OR number = 0) AND (number < 5 OR number > 8) AND (number % 2 = 0 OR number = 3) AND (number < 4 OR number > 1)";
    let base = format!("SELECT number FROM numbers(11) WHERE {clause} ORDER BY number");
    let repeated = format!(
        "SELECT number FROM numbers(11) WHERE ({clause}) AND ({clause}) ORDER BY number"
    );
    let query = crate::parse_select(&repeated).unwrap();
    assert_eq!(query.or_predicates.len(), 15);
    assert_eq!(run(&base), "number\n0\n2\n3\n4\n10\n");
    assert_eq!(run(&repeated), run(&base));
}

#[test]
fn rejects_bad_brackets_and_exponential_expansion() {
    for sql in [
        "SELECT number FROM numbers(8) WHERE (number = 1",
        "SELECT number FROM numbers(8) WHERE number = 1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
    // Independent branches still trigger the bounded DNF guard.
    let groups = (0..7)
        .map(|index| format!("(number > {index} OR number < {index})"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!("SELECT number FROM numbers(8) WHERE {groups}");
    assert!(parse_select(&sql).is_err());
}
