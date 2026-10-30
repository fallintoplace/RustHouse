use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn shifted_numbers_and_inclusive_series_emit_the_same_rows() {
    for start in [0u64, 1, 17, 1024] {
        for count in [1u64, 2, 7, 128] {
            let end = start + count - 1;
            let numbers = format!(
                "SELECT number FROM numbers({start}, {count}) ORDER BY number DESC"
            );
            let series = format!(
                "SELECT number FROM generate_series({start}, {end}) ORDER BY number DESC"
            );
            assert_eq!(run(&numbers), run(&series), "start={start} count={count}");
        }
    }
}
