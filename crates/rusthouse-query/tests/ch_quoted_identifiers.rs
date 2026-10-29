use rusthouse_query::{
    Ddl, Insert, Projection, Source, execute, parse_ddl, parse_insert, parse_join, parse_select,
};

#[test]
fn clickhouse_quoted_identifiers_keep_reserved_names_as_columns() {
    let ddl = parse_ddl("CREATE TABLE \`order\` (\`select\` UInt64, \"amount\" Nullable(UInt64))")
        .unwrap()
        .unwrap();
    let Ddl::CreateTable { name, columns, .. } = ddl else {
        panic!("expected CREATE TABLE");
    };
    assert_eq!(name, "order");
    assert_eq!(columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
               ["select", "amount"]);

    let inserted = parse_insert("INSERT INTO \`order\` (\`select\`, \"amount\") VALUES (7, NULL)")
        .unwrap()
        .unwrap();
    assert_eq!(inserted.table, "order");
    assert_eq!(inserted.columns.as_deref().unwrap(), ["select", "amount"]);

    let select = parse_select("SELECT \`select\` FROM \`order\` WHERE \"amount\" IS NULL")
        .unwrap();
    assert!(matches!(select.projection, Projection::Columns(ref columns) if columns == &["select"]));
    assert!(matches!(select.source, Some(Source::Table(ref name)) if name == "order"));
}

#[test]
fn quoted_columns_work_in_generated_where_sort_and_aliases() {
    let query = parse_select(
        "SELECT \"number\" AS \`select\` FROM numbers(4) WHERE \`number\` >= 2 ORDER BY \"number\" DESC"
    ).unwrap();
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap();
    assert_eq!(output, b"select\n3\n2\n");
}

#[test]
fn quoted_join_identifiers_preserve_qualification_and_aliases() {
    // Verify the same quoted relation/column names can be parsed as a JOIN,
    // even though the executor needs persistent table fixtures.
    let join = parse_join(
        "SELECT \`l\`.\`id\`, \"r\".\"id\" FROM \`events\` AS \`l\` INNER JOIN \"accounts\" AS \"r\" ON \`l\`.\`id\` = \"r\".\"id\""
    ).unwrap().expect("quoted JOIN");
    assert_eq!(join.left, "events");
    assert_eq!(join.right, "accounts");
    assert_eq!(join.left_alias, "l");
    assert_eq!(join.right_alias, "r");
}

#[test]
fn quoted_keyword_does_not_turn_into_sql_syntax() {
    assert!(parse_select("\`SELECT\` 1").is_err());
    assert!(parse_select("SELECT \"FROM\" FROM numbers(2)").is_ok());
    assert!(parse_select("SELECT number FROM numbers(2) WHERE \"AND\" = 1").is_ok());
}
