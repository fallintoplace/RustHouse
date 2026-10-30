use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> String {
    let query = parse_select(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    let mut output = Vec::new();
    execute(&query, None, &mut output).unwrap_or_else(|error| panic!("{sql}: {error}"));
    String::from_utf8(output).unwrap()
}

#[test]
fn stepped_numbers_match_inclusive_series_at_partial_final_steps() {
    for start in [0u64, 5, 100] {
        for length in [1u64, 2, 37, 64] {
            for step in [1u64, 2, 3, 5] {
                let end = start + ((length - 1) / step) * step;
                let numbers = format!(
                    "SELECT number FROM numbers({start}, {length}, {step}) ORDER BY number"
                );
                let series = format!(
                    "SELECT number FROM generate_series({start}, {end}, {step}) ORDER BY number"
                );
                assert_eq!(
                    run(&numbers), run(&series),
                    "start={start} length={length} step={step}"
                );
            }
        }
    }
}
