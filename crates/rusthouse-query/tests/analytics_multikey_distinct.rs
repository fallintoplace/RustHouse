//! Exact user cardinality by more than one dimension.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;

#[test]
fn grouped_distinct_across_two_dimensions_filters_nulls_and_duplicate_users() {
    let part=std::env::temp_dir().join(format!("rusthouse-multikey-distinct-{}",std::process::id()));
    let schema=[
        Column{name:"region".into(),data_type:"String".into()},
        Column{name:"category".into(),data_type:"String".into()},
        Column{name:"user_id".into(),data_type:"Nullable(UInt64)".into()},
    ];
    let rows=[
        ("east","books",Some("1")),("east","books",Some("1")),
        ("east","books",Some("2")),("west","books",Some("2")),
        ("east","tools",None),("east","tools",Some("3")),
        ("west","books",Some("2")),("west","tools",None),
    ].map(|(a,b,c)|vec![Some(a.into()),Some(b.into()),c.map(str::to_owned)]);
    write_wide_part(&part,&schema,&rows).unwrap();
    let run=|sql:&str| {
        let mut out=Vec::new();
        execute(&parse_select(sql).unwrap(),Some(&part),&mut out).unwrap();
        String::from_utf8(out).unwrap()
    };
    assert_eq!(
        run("SELECT region, category, COUNT(DISTINCT user_id) FROM part GROUP BY region, category ORDER BY region, category"),
        "region\tcategory\tcountDistinct(user_id)\neast\tbooks\t2\neast\ttools\t1\nwest\tbooks\t1\nwest\ttools\t0\n"
    );
    assert_eq!(
        run("SELECT region, category, COUNT(DISTINCT user_id) FROM part GROUP BY region, category HAVING COUNT(DISTINCT user_id) > 1 ORDER BY region, category"),
        "region\tcategory\tcountDistinct(user_id)\neast\tbooks\t2\n"
    );
    assert_eq!(
        run("SELECT region, category, COUNT(DISTINCT user_id) FROM part GROUP BY region, category ORDER BY 3 DESC, region, category LIMIT 2"),
        "region\tcategory\tcountDistinct(user_id)\neast\tbooks\t2\neast\ttools\t1\n"
    );
    fs::remove_dir_all(part).unwrap();
}

#[test]
fn invalid_grouping_keys_are_rejected() {
    assert!(parse_select(
        "SELECT region, category, COUNT(DISTINCT user_id) FROM part GROUP BY category, region"
    ).is_err());
}
