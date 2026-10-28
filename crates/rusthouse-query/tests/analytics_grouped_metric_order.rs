//! Sort analytical group results by selected aggregate output, not just dimensions.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn top_groups_can_order_by_numeric_metrics_and_ordinal_positions() {
    let path = std::env::temp_dir().join(format!("rusthouse-metric-ordering-{}",std::process::id()));
    let columns = [
        Column {name:"region".into(),data_type:"String".into()},
        Column {name:"category".into(),data_type:"String".into()},
        Column {name:"amount".into(),data_type:"UInt64".into()},
    ];
    let rows = [
        ("east","books","10"),("east","books","20"),("east","tools","30"),
        ("west","books","5"),("west","books","5"),("north","tools","50")
    ].map(|(a,b,c)| vec![Some(a.into()),Some(b.into()),Some(c.into())]);
    write_wide_part(&path,&columns,&rows).unwrap();
    let run=|sql: &str| {
        let mut output=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&path),&mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    assert_eq!(
        run("SELECT region, sum(amount), count(amount), avg(amount) FROM part GROUP BY region ORDER BY 2 DESC, region ASC LIMIT 2"),
        "region\tsum(amount)\tcount(amount)\tavg(amount)\neast\t60\t3\t20\nnorth\t50\t1\t50\n"
    );
    assert_eq!(
        run("SELECT region, sum(amount), avg(amount) FROM part GROUP BY region ORDER BY avg(amount) DESC LIMIT 1 OFFSET 1"),
        "region\tsum(amount)\tavg(amount)\neast\t60\t20\n"
    );
    assert_eq!(
        run("SELECT region, category, sum(amount), count(amount) FROM part GROUP BY region, category ORDER BY 3 DESC, region, category LIMIT 3"),
        "region\tcategory\tsum(amount)\tcount(amount)\nnorth\ttools\t50\t1\neast\tbooks\t30\t2\neast\ttools\t30\t1\n"
    );
    assert_eq!(
        run("SELECT region, category, sum(amount), count(amount) FROM part GROUP BY region, category ORDER BY count(amount) DESC, 3 DESC LIMIT 2"),
        "region\tcategory\tsum(amount)\tcount(amount)\neast\tbooks\t30\t2\nwest\tbooks\t10\t2\n"
    );
    assert!(parse_select("SELECT region, sum(amount), avg(amount) FROM part GROUP BY region ORDER BY 4").is_err());
    fs::remove_dir_all(&path).unwrap();
}
