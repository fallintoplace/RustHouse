//! ClickHouse -If aggregate combinators on stored numeric columns.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn conditional_aggregates_share_a_scan_and_ignore_null_conditions() {
    let path = std::env::temp_dir().join(format!("rusthouse-conditional-analytics-{}",std::process::id()));
    let columns = [
        Column {name:"amount".into(),data_type:"UInt64".into()},
        Column {name:"flag".into(),data_type:"Nullable(UInt64)".into()},
    ];
    let rows = [
        vec![Some("10".into()),Some("1".into())],
        vec![Some("20".into()),Some("0".into())],
        vec![Some("30".into()),Some("1".into())],
        vec![Some("40".into()),Some("0".into())],
        vec![Some("50".into()),None],
    ];
    write_wide_part(&path,&columns,&rows).unwrap();
    let run=|sql:&str|{
        let mut result=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&path),&mut result).unwrap();
        String::from_utf8(result).unwrap()
    };
    assert_eq!(
        run("SELECT sumIf(amount, flag), avgIf(amount, flag), minIf(amount, flag), maxIf(amount, flag), count(*) FROM part"),
        "sumIf(amount, flag)\tavgIf(amount, flag)\tminIf(amount, flag)\tmaxIf(amount, flag)\tcount(*)\n40\t20\t10\t30\t5\n"
    );
    assert_eq!(
        run("SELECT sumIf(amount, flag) FROM part WHERE amount > 10"),
        "sumIf(amount, flag)\n30\n"
    );
    assert_eq!(
        run("SELECT minIf(amount, flag) FROM part WHERE flag = 0"),
        "minIf(amount, flag)\nNULL\n"
    );
    assert_eq!(
        run("SELECT sumIf(amount, flag * 2) FROM part"),
        "sumIf(amount, flag * 2)\n40\n"
    );
    for sql in ["SELECT sumIf(amount) FROM part","SELECT avgIf(amount, flag, flag) FROM part",
        "SELECT sumIf(*, flag) FROM part"] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn conditional_aggregate_generated_number_semantics() {
    let run=|sql:&str|{
        let mut result=Vec::new();
        execute(&parse_select(sql).unwrap(),None,&mut result).unwrap();
        String::from_utf8(result).unwrap()
    };
    assert_eq!(
        run("SELECT sumIf(number, number), count(*) FROM numbers(5)"),
        "sumIf(number, number)\tcount(*)\n10\t5\n"
    );
    assert_eq!(
        run("SELECT sumIf(number, number) FROM numbers(1)"),
        "sumIf(number, number)\nNULL\n"
    );
}
