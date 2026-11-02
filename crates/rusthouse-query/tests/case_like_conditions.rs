use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn case_when_like_uses_sql_pattern_semantics() {
    let root = std::env::temp_dir().join(format!(
        "rusthouse-case-like-{}-{}", std::process::id(), line!()
    ));
    let cols = [Column { name: "kind".into(), data_type: "Nullable(String)".into() }];
    let rows = [
        vec![Some("PROMO BIG".into())],
        vec![Some("promo small".into())],
        vec![Some("RETAIL".into())],
        vec![None],
    ];
    write_wide_part(&root, &cols, &rows).unwrap();
    let mut out = Vec::new();
    execute(
        &parse_select("SELECT sum(CASE WHEN kind LIKE 'PROMO%' THEN 1 ELSE 0 END), sum(CASE WHEN kind ILIKE 'promo%' THEN 1 ELSE 0 END), sum(CASE WHEN kind NOT LIKE 'PROMO%' THEN 1 ELSE 0 END) FROM part").unwrap(),
        Some(&root), &mut out
    ).unwrap();
    assert_eq!(String::from_utf8(out).unwrap().lines().last(), Some("1\t2\t2"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn like_conditions_reject_nonliteral_patterns() {
    assert!(parse_select("SELECT sum(CASE WHEN kind LIKE other THEN 1 ELSE 0 END) FROM part").is_err());
    assert!(parse_select("SELECT sum(CASE WHEN kind NOT LIKE THEN 1 ELSE 0 END) FROM part").is_err());
}
