//! Product analytics reports need stable names for derived metric columns.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn grouped_revenue_and_counts_emit_selected_aliases() {
    let path=std::env::temp_dir().join(format!("rusthouse-metric-aliases-{}",std::process::id()));
    let schema=[
        Column{name:"region".into(),data_type:"String".into()},
        Column{name:"category".into(),data_type:"String".into()},
        Column{name:"amount".into(),data_type:"UInt64".into()},
    ];
    let rows=[("west","books","3"),("west","tools","5"),("east","books","7")]
        .map(|(a,b,c)|vec![Some(a.into()),Some(b.into()),Some(c.into())]);
    write_wide_part(&path,&schema,&rows).unwrap();
    let run=|sql:&str|{
        let mut out=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&path),&mut out).unwrap();
        String::from_utf8(out).unwrap()
    };
    assert_eq!(
        run("SELECT region, SUM(amount) AS revenue, COUNT(amount) AS purchases FROM part GROUP BY region ORDER BY region"),
        "region\trevenue\tpurchases\neast\t7\t1\nwest\t8\t2\n"
    );
    assert_eq!(
        run("SELECT region, category, SUM(amount) AS revenue, MAX(amount) AS peak FROM part GROUP BY region, category ORDER BY region, category"),
        "region\tcategory\trevenue\tpeak\neast\tbooks\t7\t7\nwest\tbooks\t3\t3\nwest\ttools\t5\t5\n"
    );
    assert_eq!(
        run("SELECT region, SUM(amount) AS revenue FROM part GROUP BY region ORDER BY region"),
        "region\trevenue\neast\t7\nwest\t8\n"
    );
    for sql in [
        "SELECT region, SUM(amount) AS region FROM part GROUP BY region",
        "SELECT region, SUM(amount) AS profit, AVG(amount) AS profit FROM part GROUP BY region",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
    fs::remove_dir_all(path).unwrap();
}
