use std::process::Command;

fn run(query: &str, format: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--format", format])
        .output()
        .unwrap();
    assert!(output.status.success(), "{query}: {output:?}");
    output.stdout
}

#[test]
fn escapes_scalar_sql_results_in_multiple_formats() {
    for (query, format, expected) in [
        (
            "SELECT 'A,B' AS label, 3 AS amount",
            "csv",
            "label,amount\r\n\"A,B\",3\r\n",
        ),
        (
            "SELECT 'a|b' AS label",
            "markdown",
            "| label |\n| --- |\n| a\\|b |\n",
        ),
        (
            "SELECT '<x&y>' AS label",
            "html",
            "<table>\n<thead><tr><th>label</th></tr></thead>\n<tbody>\n<tr><td>&lt;x&amp;y&gt;</td></tr>\n</tbody>\n</table>\n",
        ),
        (
            "SELECT 'O''Brien' AS name, NULL AS note",
            "jsonl",
            "{\"name\":\"O'Brien\",\"note\":null}\n",
        ),
    ] {
        assert_eq!(
            run(query, format),
            expected.as_bytes(),
            "{query} ({format})"
        );
    }
}
