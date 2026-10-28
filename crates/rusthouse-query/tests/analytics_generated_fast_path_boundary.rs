//! Stored-table analytics must not access generated range variables.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
#[test]
fn fast_generated_aggregates_do_not_hijack_stored_table_scan() {
    let path=std::env::temp_dir().join(format!("rusthouse-fast-path-boundary-{}",std::process::id()));
    let columns=[Column{name:"amount".into(),data_type:"UInt64".into()}];
    let rows=[vec![Some("100".into())],vec![Some("300".into())]];
    write_wide_part(&path,&columns,&rows).unwrap();
    let mut out=Vec::new();
    execute(&parse_select("SELECT sum(amount), count(*) FROM part").unwrap(),Some(&path),&mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(),"sum(amount)\tcount(*)\n400\t2\n");
    fs::remove_dir_all(path).unwrap();
}
#[test]
fn generated_fast_path_keeps_correct_empty_and_large_semantics() {
    let mut out=Vec::new();
    execute(&parse_select("SELECT sum(number), count(*), max(number) FROM numbers(1000000000)").unwrap(),None,&mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(),"sum(number)\tcount(*)\tmax(number)\n499999999500000000\t1000000000\t999999999\n");
}
