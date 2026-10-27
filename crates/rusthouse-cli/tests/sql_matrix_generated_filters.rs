use std::process::Command;

fn run(query: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query])
        .output()
        .unwrap();
    assert!(output.status.success(), "{query}: {output:?}");
    output.stdout
}

#[test]
fn combines_generated_source_filters() {
    for (query, expected) in [
        (
            "SELECT number FROM generate_series(3, 18, 3) WHERE number BETWEEN 6 AND 15 ORDER BY number DESC LIMIT 2",
            "number\n15\n12\n",
        ),
        (
            "SELECT sum(number) AS odd_sum FROM numbers(10) WHERE number % 2 = 1",
            "odd_sum\n25\n",
        ),
        (
            "SELECT count(*) AS remaining FROM numbers(20) WHERE number NOT IN (2, 5, 7, 11, 13, 17, 19) AND number >= 10",
            "remaining\n6\n",
        ),
        (
            "SELECT number FROM numbers(20) WHERE number = 1 OR number BETWEEN 15 AND 18 ORDER BY number DESC LIMIT 3 OFFSET 1",
            "number\n17\n16\n15\n",
        ),
    ] {
        assert_eq!(run(query), expected.as_bytes(), "{query}");
    }
}
