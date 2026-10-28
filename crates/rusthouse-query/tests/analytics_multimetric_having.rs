//! Grouped analytical metrics may be filtered by any selected aggregate.
use rusthouse_query::{execute,parse_select};
use rusthouse_storage::{Column,write_wide_part};
use std::fs;
#[test]
fn having_on_multi_metric_reports() {
    let dir=std::env::temp_dir().join(format!("rusthouse-multimetric-having-{}",std::process::id()));
    let columns=[
        Column{name:"region".into(),data_type:"String".into()},
        Column{name:"channel".into(),data_type:"String".into()},
        Column{name:"amount".into(),data_type:"UInt64".into()},
    ];
    let rows=[("east","web","10"),("east","web","20"),("east","store","100"),
        ("west","web","5"),("west","store","1"),("west","store","3")]
        .map(|(region,channel,amount)|vec![Some(region.into()),Some(channel.into()),Some(amount.into())]);
    write_wide_part(&dir,&columns,&rows).unwrap();
    let run=|sql:&str| {
        let mut result=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&dir),&mut result).unwrap();
        String::from_utf8(result).unwrap()
    };
    assert_eq!(
        run("SELECT region, sum(amount), count(amount) FROM part GROUP BY region HAVING sum(amount) > 100 ORDER BY region"),
        "region\tsum(amount)\tcount(amount)\neast\t130\t3\n"
    );
    assert_eq!(
        run("SELECT region, channel, sum(amount), count(amount) FROM part GROUP BY region, channel HAVING count(amount) >= 2 ORDER BY region, channel"),
        "region\tchannel\tsum(amount)\tcount(amount)\neast\tweb\t30\t2\nwest\tstore\t4\t2\n"
    );
    assert_eq!(
        run("SELECT region, sum(amount), max(amount) FROM part GROUP BY region HAVING max(amount) > 50 ORDER BY region"),
        "region\tsum(amount)\tmax(amount)\neast\t130\t100\n"
    );
    assert!(parse_select("SELECT region, sum(amount), max(amount) FROM part GROUP BY region HAVING min(amount) > 1").is_err());
    fs::remove_dir_all(dir).unwrap();
}
