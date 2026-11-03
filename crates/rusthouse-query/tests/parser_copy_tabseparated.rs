use rusthouse_query::parse_copy_from;

#[test]
fn tabseparated_copy_format_uses_tsv_parser() {
    let tsv = parse_copy_from("COPY events FROM 'rows.tsv' WITH (FORMAT TSV)")
        .unwrap();
    for name in ["TabSeparated", "TABSEPARATED", "tabseparated"] {
        let sql = format!("COPY events FROM 'rows.tsv' WITH (FORMAT {name})");
        assert_eq!(parse_copy_from(&sql).unwrap(), tsv);
    }
}
