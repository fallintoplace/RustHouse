//! Rank grouped analytics by statistical and conditional metric values.
use rusthouse_query::{execute,parse_select};
use rusthouse_storage::{Column,write_wide_part};
use std::fs;
#[test]
fn metric_rankings_support_variance_median_and_conditional_counts() {
    let dir=std::env::temp_dir().join(format!("rusthouse-statistical-ranking-{}",std::process::id()));
    let columns=[
        Column{name:"region".into(),data_type:"String".into()},
        Column{name:"category".into(),data_type:"String".into()},
        Column{name:"value".into(),data_type:"UInt64".into()},
    ];
    let rows=[("east","a","2"),("east","a","8"),("west","b","10"),
        ("west","b","10"),("north","a","1")]
        .map(|(region,category,value)|vec![Some(region.into()),Some(category.into()),Some(value.into())]);
    write_wide_part(&dir,&columns,&rows).unwrap();
    let run=|sql:&str|{
        let mut out=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&dir),&mut out).unwrap();
        String::from_utf8(out).unwrap()
    };
    let median=run("SELECT region, median(value), count(value) FROM part GROUP BY region ORDER BY median(value) DESC");
    assert_eq!(median.lines().skip(1).map(|line|line.split('\t').next().unwrap()).collect::<Vec<_>>(),
        ["west","east","north"]);
    let variance=run("SELECT region, varPop(value), countIf(value) FROM part GROUP BY region ORDER BY varPop(value) DESC, region ASC");
    assert_eq!(variance.lines().skip(1).map(|line|line.split('\t').next().unwrap()).collect::<Vec<_>>(),
        ["east","north","west"]);
    let multi=run("SELECT region, category, median(value), rms(value) FROM part GROUP BY region, category ORDER BY 3 DESC LIMIT 2");
    let values=multi.lines().skip(1).map(|line|line.split('\t').take(2).collect::<Vec<_>>()).collect::<Vec<_>>();
    assert_eq!(values,[["west","b"],["east","a"]]);
    fs::remove_dir_all(dir).unwrap();
}
