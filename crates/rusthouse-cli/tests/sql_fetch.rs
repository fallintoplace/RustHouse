use std::process::Command;

fn query(sql: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", sql])
        .output()
        .unwrap()
}

#[test]
fn fetch_first_pages_generated_numbers() {
    let result = query(
        "SELECT number FROM numbers(10) ORDER BY number DESC OFFSET 2 ROWS FETCH NEXT 3 ROWS ONLY",
    );
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        "number\n7\n6\n5\n"
    );
}

#[test]
fn fetch_zero_rows_keeps_the_header() {
    let result = query("SELECT number FROM numbers(10) FETCH FIRST 0 ROWS ONLY");
    assert!(result.status.success(), "{result:?}");
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "number\n");
}
