use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> String {
    let statement = parse_select(sql).unwrap();
    let mut output = Vec::new();
    execute(&statement, None, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn sample_variance_differs_from_population_variance() {
    assert_eq!(
        query("SELECT varSamp(number) FROM numbers(5)"),
        "varSamp(number)\n2.5\n"
    );
    assert_eq!(
        query("SELECT varPop(number) FROM numbers(5)"),
        "varPop(number)\n2\n"
    );
    assert_eq!(
        query("SELECT varSamp(number) FROM numbers(5) WHERE number >= 3"),
        "varSamp(number)\n0.5\n"
    );
}

#[test]
fn sample_standard_deviation_uses_n_minus_one() {
    let output = query("SELECT stddevSamp(number) FROM numbers(5)");
    let deviation: f64 = output.lines().nth(1).unwrap().parse().unwrap();
    assert!((deviation - 2.5_f64.sqrt()).abs() < 1e-12);
}

#[test]
fn sample_variance_needs_two_non_null_values() {
    for source in ["numbers(0)", "numbers(1)"] {
        assert_eq!(
            query(&format!("SELECT varSamp(number) FROM {source}")),
            "varSamp(number)\nNULL\n"
        );
        assert_eq!(
            query(&format!("SELECT stddevSamp(number) FROM {source}")),
            "stddevSamp(number)\nNULL\n"
        );
    }
}

#[test]
fn grouped_sample_variance_supports_generated_sources() {
    assert_eq!(
        query("SELECT number, varSamp(number) FROM numbers(2) GROUP BY number"),
        "number\tvarSamp(number)\n0\tNULL\n1\tNULL\n"
    );
}
