use std::process::Command;

fn sql(query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("sql")
        .arg(query)
        .output()
        .unwrap()
}

#[test]
fn calculates_median_for_generated_numeric_rows() {
    for (query, expected) in [
        ("SELECT median(number) FROM numbers(5)", "median(number)\n2\n"),
        ("SELECT median(number) FROM numbers(6)", "median(number)\n2.5\n"),
        (
            "SELECT median(number) FROM numbers(10) WHERE number >= 8",
            "median(number)\n8.5\n",
        ),
        ("SELECT median(number) FROM numbers(0)", "median(number)\nNULL\n"),
    ] {
        let output = sql(query);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
}

#[test]
fn rejects_medians_over_the_bounded_memory_limit() {
    let output = sql("SELECT median(number) FROM numbers(100001)");
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("at most 100000 matching rows")
    );
}
