//! Exact distinct numeric aggregates share a bounded one-pass input scan.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn sum_avg_min_and_max_distinct_handle_duplicates_and_nulls() {
    let dir=std::env::temp_dir().join(format!("rusthouse-distinct-metrics-{}",std::process::id()));
    let columns=[Column{name:"value".into(),data_type:"Nullable(UInt64)".into()}];
    let records=[Some("1"),Some("1"),Some("2"),Some("4"),None]
        .into_iter().map(|value|vec![value.map(str::to_string)]).collect::<Vec<_>>();
    write_wide_part(&dir,&columns,&records).unwrap();
    let run=|sql:&str|{
        let mut out=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&dir),&mut out).unwrap();
        String::from_utf8(out).unwrap()
    };
    let value=run("SELECT sum(DISTINCT value), avg(DISTINCT value), min(DISTINCT value), max(DISTINCT value), count(*) FROM part");
    let row=value.lines().nth(1).unwrap().split('\t').collect::<Vec<_>>();
    assert_eq!(row[0],"7");
    assert!((row[1].parse::<f64>().unwrap() - 7.0/3.0).abs() < 1e-12);
    assert_eq!(&row[2..],["1","4","5"]);
    assert_eq!(run("SELECT sum(DISTINCT value) FROM part"),"sum(DISTINCT value)\n7\n");
    assert_eq!(run("SELECT sum(DISTINCT value), count(*) FROM part WHERE value IS NULL"),
        "sum(DISTINCT value)\tcount(*)\nNULL\t1\n");
    assert!(parse_select("SELECT median(DISTINCT value), count(*) FROM part").is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn distinct_metrics_are_bounded_and_generated_source_works() {
    let mut out=Vec::new();
    execute(&parse_select("SELECT sum(DISTINCT number), avg(DISTINCT number) FROM numbers(5)").unwrap(),
        None,&mut out).unwrap();
    let result=String::from_utf8(out).unwrap();
    assert_eq!(result,"sum(DISTINCT number)\tavg(DISTINCT number)\n10\t2\n");
}
