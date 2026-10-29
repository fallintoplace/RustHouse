use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn sorted_offsets_past_end_keep_the_schema_and_no_rows() {
    for rows in [1u64, 7, 1024, 1025, 4097] {
        for offset in [rows, rows + 1, rows + 1024] {
            let sql = format!(
                "SELECT number FROM numbers({rows}) ORDER BY number DESC LIMIT 3 OFFSET {offset}"
            );
            assert_eq!(run(&sql), "number\n", "rows={rows} offset={offset}");
        }
    }
}
