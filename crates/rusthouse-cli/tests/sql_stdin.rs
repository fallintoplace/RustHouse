use std::io::Write;
use std::process::{Command, Output, Stdio};

fn stdin_sql(bytes: &[u8], args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Err(error) = child.stdin.take().unwrap().write_all(bytes) {
        // Invalid CLI arguments may make the child close stdin before we finish writing.
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "unexpected stdin write error: {error}"
        );
    }
    child.wait_with_output().unwrap()
}

#[test]
fn executes_stdin_sql_with_format_and_show_ast() {
    let result = stdin_sql(
        b"SELECT number FROM numbers(5) WHERE number >= 3",
        &["sql", "--stdin"],
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"number\n3\n4\n");

    let result = stdin_sql(
        b"SELECT count(*) FROM numbers(3)",
        &["sql", "--stdin", "--format", "csv"],
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"count\r\n3\r\n");

    let result = stdin_sql(b"SELECT 1", &["sql", "--stdin", "--show-ast"]);
    assert!(result.status.success(), "{result:?}");
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("Literal")
    );
}

#[test]
fn refuses_oversized_utf8_invalid_and_conflicting_sources() {
    for bytes in [&vec![b' '; 4097][..], &[0xff][..]] {
        let result = stdin_sql(bytes, &["sql", "--stdin"]);
        assert!(!result.status.success());
    }

    let result = stdin_sql(b"SELECT 1", &["sql", "--stdin", "SELECT 2"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("not both"));

    let result = stdin_sql(b"SELECT 1", &["sql", "--stdin", "--stdin"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("duplicate"));
}
