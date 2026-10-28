//! Conditional counts and weighted revenue expressions.
use rusthouse_query::{execute,parse_select};
use rusthouse_storage::{Column,write_wide_part};
use std::fs;
#[test]
fn numeric_aggregate_comparisons_work_with_arithmetic_and_nulls() {
    let dir=std::env::temp_dir().join(format!("rusthouse-conditional-kpi-{}",std::process::id()));
    let schema=[
        Column{name:"amount".into(),data_type:"Nullable(UInt64)".into()},
        Column{name:"target".into(),data_type:"UInt64".into()},
    ];
    let rows=[
        vec![Some("5".into()),Some("10".into())],
        vec![Some("10".into()),Some("10".into())],
        vec![Some("20".into()),Some("10".into())],
        vec![None,Some("10".into())],
    ];
    write_wide_part(&dir,&schema,&rows).unwrap();
    let run=|sql:&str| {
        let mut result=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&dir),&mut result).unwrap();
        String::from_utf8(result).unwrap()
    };
    assert_eq!(
        run("SELECT countIf(amount > target), countIf(amount >= 10), sum(amount) FROM part"),
        "countIf(amount > target)\tcountIf(amount >= 10)\tsum(amount)\n1\t2\t35\n"
    );
    assert_eq!(
        run("SELECT sum(amount * (amount > 9)), count(*) FROM part"),
        "sum(amount * (amount > 9))\tcount(*)\n30\t4\n"
    );
    assert_eq!(
        run("SELECT countIf(amount + 1 <= target * 2), count(*) FROM part"),
        "countIf(amount + 1 <= target * 2)\tcount(*)\n2\t4\n"
    );
    fs::remove_dir_all(dir).unwrap();
}
