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
fn evaluates_composed_text_expressions() {
    for (query, expected) in [
        ("SELECT LENGTH('naïve') AS bytes, CHAR_LENGTH('naïve') AS chars, SUBSTRING('naïve', 3, 2) AS middle", "bytes\tchars\tmiddle\n6\t5\tïv\n"),
        ("SELECT CONCAT(LEFT('rust', 2), RIGHT('house', 3)) AS name", "name\nruuse\n"),
        ("SELECT REPLACE('bananana', 'na', 'X') AS result", "result\nbaXXX\n"),
        ("SELECT CONCAT(TRIM('  a  '), REPEAT('b', 3), SUBSTRING('12345', 4)) AS value", "value\nabbb45\n"),
    ] {
        assert_eq!(run(query), expected.as_bytes(), "{query}");
    }
}
