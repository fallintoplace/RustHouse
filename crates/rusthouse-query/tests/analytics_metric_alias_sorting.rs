//! Rank analytics dashboards by output metric aliases.
use rusthouse_query::{execute,parse_select};
use rusthouse_storage::{Column,write_wide_part};
use std::fs;
#[test]
fn aliases_are_valid_order_by_metrics_for_one_and_two_dimensions() {
    let dir=std::env::temp_dir().join(format!("rusthouse-analytic-metric-sort-alias-{}",std::process::id()));
    let schema=[
        Column{name:"region".into(),data_type:"String".into()},
        Column{name:"category".into(),data_type:"String".into()},
        Column{name:"amount".into(),data_type:"UInt64".into()},
    ];
    let records=[("east","a","10"),("east","b","20"),("west","a","100"),
        ("west","b","1"),("north","a","30")]
        .map(|(region,category,amount)|vec![Some(region.into()),Some(category.into()),Some(amount.into())]);
    write_wide_part(&dir,&schema,&records).unwrap();
    let run=|sql:&str|{
        let mut output=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&dir),&mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    assert_eq!(
        run("SELECT region, sum(amount) AS revenue, count(amount) AS events FROM part GROUP BY region ORDER BY revenue DESC LIMIT 2"),
        "region\trevenue\tevents\nwest\t101\t2\neast\t30\t2\n"
    );
    assert_eq!(
        run("SELECT region, category, sum(amount) AS revenue, max(amount) AS peak FROM part GROUP BY region, category ORDER BY peak DESC, region ASC LIMIT 2"),
        "region\tcategory\trevenue\tpeak\nwest\ta\t100\t100\nnorth\ta\t30\t30\n"
    );
    assert!(parse_select("SELECT region, sum(amount) AS revenue FROM part GROUP BY region ORDER BY unknown DESC").is_err());
    fs::remove_dir_all(dir).unwrap();
}
