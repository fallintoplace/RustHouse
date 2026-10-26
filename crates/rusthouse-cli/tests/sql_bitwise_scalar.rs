use std::process::Command;

#[test]
fn executes_integer_bitwise_functions_in_scalar_sql() {
    let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", "SELECT bitXor(bitAnd(7, 3), bitOr(4, 2)) AS bits"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"bits\n5\n");
}
