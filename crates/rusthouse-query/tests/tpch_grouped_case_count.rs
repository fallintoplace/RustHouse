use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};

#[test]
fn grouped_case_then_one_else_zero_counts_matching_rows() {
    let path = std::env::temp_dir().join(format!("rusthouse-tpch-case-count-{}", std::process::id()));
    let columns = [
        Column { name: "region".into(), data_type: "String".into() },
        Column { name: "priority".into(), data_type: "String".into() },
    ];
    let records = [
        vec![Some("east".into()), Some("urgent".into())],
        vec![Some("east".into()), Some("normal".into())],
        vec![Some("west".into()), Some("normal".into())],
    ];
    write_wide_part(&path, &columns, &records).unwrap();
    let query = parse_select("SELECT region, sum(CASE WHEN priority = 'urgent' THEN 1 ELSE 0 END) AS high_count FROM part GROUP BY region ORDER BY region").unwrap();
    let mut output = Vec::new();
    execute(&query, Some(&path), &mut output).unwrap();
    assert_eq!(String::from_utf8(output).unwrap(), "region\thigh_count\neast\t1\nwest\t0\n");
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn grouped_case_unit_count_requires_zero_else_branch() {
    for sql in [
        "SELECT region, sum(CASE WHEN priority = 'urgent' THEN 1 END) FROM part GROUP BY region",
        "SELECT region, sum(CASE WHEN priority = 'urgent' THEN 2 ELSE 0 END) FROM part GROUP BY region",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
