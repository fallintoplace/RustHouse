use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn sparse_in_extrema_preserve_exact_boundary_values() {
    for rows in [1u64, 7, 18, 1025, 4097] {
        let expected_max = [0u64, 3, 17, 1024]
            .into_iter()
            .filter(|value| *value < rows)
            .max()
            .unwrap();
        let sql = format!(
            "SELECT min(number), max(number) FROM numbers({rows}) WHERE number IN (0, 3, 17, 1024)"
        );
        assert_eq!(
            run(&sql),
            format!("min(number)\tmax(number)\n0\t{expected_max}\n"),
            "rows={rows}"
        );
    }
}
