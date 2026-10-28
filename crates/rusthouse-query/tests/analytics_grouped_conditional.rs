//! Conditional revenue and conversion metrics per dimension.
use rusthouse_query::{execute,parse_select};
use rusthouse_storage::{Column,write_wide_part};
use std::fs;
#[test]
fn conditional_grouped_kpis_share_one_scan() {
    let path=std::env::temp_dir().join(format!("rusthouse-grouped-if-{}",std::process::id()));
    let columns=[
        Column{name:"region".into(),data_type:"String".into()},
        Column{name:"category".into(),data_type:"String".into()},
        Column{name:"amount".into(),data_type:"UInt64".into()},
        Column{name:"paid".into(),data_type:"Nullable(UInt8)".into()},
    ];
    let records=[
        ("east","book","10",Some("1")),("east","book","30",Some("1")),
        ("east","tool","20",Some("0")),("west","book","5",Some("0")),
        ("west","book","7",Some("1")),("west","tool","90",None),
    ].map(|(region,category,amount,paid)| vec![
        Some(region.into()),Some(category.into()),Some(amount.into()),paid.map(str::to_owned)]);
    write_wide_part(&path,&columns,&records).unwrap();
    let run=|sql:&str|{
        let mut output=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&path),&mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    assert_eq!(
        run("SELECT region, sumIf(amount, paid) AS revenue, avgIf(amount, paid), sum(amount), count(amount) FROM part GROUP BY region ORDER BY region"),
        "region\trevenue\tavgIf(amount, paid)\tsum(amount)\tcount(amount)\neast\t40\t20\t60\t3\nwest\t7\t7\t102\t3\n"
    );
    assert_eq!(
        run("SELECT region, category, sumIf(amount, paid) AS revenue, minIf(amount, paid), maxIf(amount, paid) FROM part GROUP BY region, category ORDER BY region, category"),
        "region\tcategory\trevenue\tminIf(amount, paid)\tmaxIf(amount, paid)\neast\tbook\t40\t10\t30\neast\ttool\tNULL\tNULL\tNULL\nwest\tbook\t7\t7\t7\nwest\ttool\tNULL\tNULL\tNULL\n"
    );
    assert!(parse_select("SELECT region, sumIf(amount) FROM part GROUP BY region").is_err());
    fs::remove_dir_all(path).unwrap();
}
