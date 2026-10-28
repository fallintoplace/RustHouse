use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn sql(statement: &str, database: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rusthouse"))
        .args(["sql", statement, "--database"])
        .arg(database)
        .output()
        .unwrap()
}

#[test]
fn dense_sixteen_bit_single_grouped_aggregates_preserve_signed_values() {
    let db =
        std::env::temp_dir().join(format!("rusthouse-dense-u16-single-{}", std::process::id()));
    for statement in [
        "CREATE TABLE values_by_key (category UInt16, signed_category Int16, amount UInt64)",
        "INSERT INTO values_by_key VALUES (65535, -32768, 6), (0, 32767, 3), (42, -1, 7)",
        "INSERT INTO values_by_key VALUES (65535, -32768, 1), (42, -1, 5), (0, 32767, 4)",
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
    }
    for (statement, expected) in [
        (
            "SELECT category, sum(amount) FROM values_by_key GROUP BY category ORDER BY category",
            "category\tsum(amount)\n0\t7\n42\t12\n65535\t7\n",
        ),
        (
            "SELECT signed_category, max(amount) FROM values_by_key GROUP BY signed_category ORDER BY signed_category",
            "signed_category\tmax(amount)\n-32768\t6\n-1\t7\n32767\t4\n",
        ),
    ] {
        let result = sql(statement, &db);
        assert!(result.status.success(), "{statement}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            expected,
            "{statement}"
        );
    }
    fs::remove_dir_all(db).unwrap();
}
