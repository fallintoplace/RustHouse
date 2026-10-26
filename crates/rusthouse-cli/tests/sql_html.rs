use std::process::Command;

fn sql(query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", query, "--format", "html"])
        .output()
        .unwrap()
}

#[test]
fn exports_query_results_as_html_tables() {
    let result = sql("SELECT number FROM numbers(2)");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "<table>\n<thead><tr><th>number</th></tr></thead>\n<tbody>\n<tr><td>0</td></tr>\n<tr><td>1</td></tr>\n</tbody>\n</table>\n"
    );

    let empty = sql("SELECT number FROM numbers(0)");
    assert!(empty.status.success(), "{empty:?}");
    assert_eq!(
        String::from_utf8(empty.stdout).unwrap(),
        "<table>\n<thead><tr><th>number</th></tr></thead>\n<tbody>\n</tbody>\n</table>\n"
    );
}
