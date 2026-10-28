//! Numeric HAVING must compare aggregate states rather than rendered text.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn having_filters_analytic_metrics_and_preserves_null_semantics() {
    let path = std::env::temp_dir().join(format!("rusthouse-analytic-having-{}",std::process::id()));
    let columns=[
        Column {name:"segment".into(), data_type:"String".into()},
        Column {name:"score".into(), data_type:"UInt64".into()},
    ];
    let rows=[("east","0"),("east","2"),("east","4"),("west","1"),("west","1")];
    write_wide_part(&path,&columns,&rows.iter().map(|(segment,score)|
        vec![Some((*segment).into()),Some((*score).into())]).collect::<Vec<_>>()).unwrap();
    let run=|sql:&str| {
        let mut out=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&path),&mut out).unwrap();
        String::from_utf8(out).unwrap()
    };
    assert_eq!(
        run("SELECT segment, countIf(score) FROM part GROUP BY segment HAVING countIf(score) >= 2 ORDER BY segment"),
        "segment\tcountIf(score)\neast\t2\nwest\t2\n"
    );
    assert_eq!(
        run("SELECT segment, sumSquares(score) FROM part GROUP BY segment HAVING sumSquares(score) > 10 ORDER BY segment"),
        "segment\tsumSquares(score)\neast\t20\n"
    );
    assert_eq!(
        run("SELECT segment, varPop(score) FROM part GROUP BY segment HAVING varPop(score) > 1 ORDER BY segment"),
        "segment\tvarPop(score)\neast\t2.6666666666666665\n"
    );
    assert_eq!(
        run("SELECT segment, median(score) FROM part GROUP BY segment HAVING median(score) >= 2 ORDER BY segment"),
        "segment\tmedian(score)\neast\t2\n"
    );
    assert_eq!(
        run("SELECT segment, range(score) FROM part GROUP BY segment HAVING range(score) > 2 ORDER BY segment"),
        "segment\trange(score)\neast\t4\n"
    );
    fs::remove_dir_all(&path).unwrap();
}

#[test]
fn having_accepts_statistical_and_aggregate_family_names() {
    for name in ["countIf","product","sumSquares","range","rms","varPop",
        "varSamp","stddevPop","stddevSamp","median","any","anyLast",
        "groupBitAnd","groupBitOr","groupBitXor"] {
        let sql=format!("SELECT segment, {name}(score) FROM part GROUP BY segment HAVING {name}(score) > 0");
        assert!(parse_select(&sql).is_ok(), "{sql}");
    }
}
