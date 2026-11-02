use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn ilike_parses_with_explicit_case_insensitivity() {
    assert_eq!(
        parse_ddl("SHOW TABLES ILIKE 'EV%'").unwrap(),
        Some(Ddl::ShowTablesILike { pattern: "EV%".into() })
    );
    assert_eq!(
        parse_ddl("SHOW TABLES NOT ILIKE 'e_%'").unwrap(),
        Some(Ddl::ShowTablesNotILike { pattern: "e_%".into() })
    );
    for sql in [
        "SHOW TABLES NOT",
        "SHOW TABLES NOT ILIKE",
        "SHOW TABLES ILIKE 1",
        "SHOW TABLES ILIKE 'x'; DROP TABLE t",
    ] {
        assert!(parse_ddl(sql).is_err(), "{sql}");
    }
}
