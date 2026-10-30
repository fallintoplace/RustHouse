use std::process::{Command, Output};

fn sql(statement: &str, flags: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .arg("sql")
        .arg(statement)
        .args(flags)
        .output()
        .unwrap()
}

#[test]
fn clickhouse_format_clause_selects_existing_output_encodings() {
    let tsv = sql("SELECT number FROM numbers(3) FORMAT TabSeparated", &[]);
    assert!(tsv.status.success(), "{tsv:?}");
    assert_eq!(tsv.stdout, b"0\n1\n2\n");

    let with_names = sql("SELECT number FROM numbers(2) FORMAT TabSeparatedWithNames", &[]);
    assert!(with_names.status.success(), "{with_names:?}");
    assert_eq!(with_names.stdout, b"number\n0\n1\n");

    let strings = sql("SELECT number FROM numbers(2) FORMAT JSONStringsEachRow", &[]);
    assert!(strings.status.success(), "{strings:?}");
    assert_eq!(strings.stdout, b"{\"number\":\"0\"}\n{\"number\":\"1\"}\n");
}

#[test]
fn rejects_conflicting_or_unimplemented_formats() {
    for output in [
        sql("SELECT 1 FORMAT CSV", &["--format", "csv"]),
        sql("SELECT 1 FORMAT JSONEachRow", &[]),
    ] {
        assert!(!output.status.success(), "{output:?}");
    }
}
