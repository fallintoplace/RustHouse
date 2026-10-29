use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn monotonic_count_matches_independent_boundary_math() {
    for rows in [1u64, 7, 1024, 4097, 1_000_000_000] {
        for cutoff in [0u64, 1, 7, 1023, 1024, 4096, 1_000_000_000] {
            let count = rows.saturating_sub(cutoff);
            let sql = format!(
                "SELECT count(*) FROM numbers({rows}) WHERE number >= {cutoff}"
            );
            assert_eq!(run(&sql), format!("count\n{count}\n"), "rows={rows} cutoff={cutoff}");
        }
    }
}
