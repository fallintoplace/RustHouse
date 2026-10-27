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
fn preserves_signed_integer_math() {
    for (query, expected) in [
        (
            "SELECT MUL_DIV(9223372036854775807, 2, 2) AS exact",
            "exact\n9223372036854775807\n",
        ),
        (
            "SELECT FLOOR_DIV(-7, 3) AS floor_value, CEIL_DIV(-7, 3) AS ceil_value, ROUND_DIV(-7, 3) AS near_value",
            "floor_value\tceil_value\tnear_value\n-3\t-2\t-2\n",
        ),
        (
            "SELECT GCD(-252, 198) AS g, LCM(12, 18) AS l, IS_DIVISIBLE(252, 7) AS divides",
            "g\tl\tdivides\n18\t36\t1\n",
        ),
        (
            "SELECT MOD_POW(-2, 5, 13) AS power, WIDTH_BUCKET(0, -10, 10, 4) AS bucket, INTLOG(1024, 2) AS exponent",
            "power\tbucket\texponent\n7\t3\t10\n",
        ),
    ] {
        assert_eq!(run(query), expected.as_bytes(), "{query}");
    }
}
