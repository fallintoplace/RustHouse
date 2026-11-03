use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};

#[test]
fn tpch_q12_priority_branches_count_correctly() {
    let path = std::env::temp_dir().join(format!("rusthouse-tpch-q12-case-{}", std::process::id()));
    let columns = [
        Column {
            name: "region".into(),
            data_type: "String".into(),
        },
        Column {
            name: "priority".into(),
            data_type: "String".into(),
        },
    ];
    let records = [
        vec![Some("east".into()), Some("1-URGENT".into())],
        vec![Some("east".into()), Some("2-HIGH".into())],
        vec![Some("east".into()), Some("normal".into())],
        vec![Some("west".into()), Some("normal".into())],
    ];
    write_wide_part(&path, &columns, &records).unwrap();
    let sql = "SELECT region, sum(CASE WHEN priority = '1-URGENT' OR priority = '2-HIGH' THEN 1 ELSE 0 END) AS high_count, sum(CASE WHEN priority <> '1-URGENT' AND priority <> '2-HIGH' THEN 1 ELSE 0 END) AS low_count FROM part GROUP BY region ORDER BY region";
    let mut result = Vec::new();
    execute(&parse_select(sql).unwrap(), Some(&path), &mut result).unwrap();
    assert_eq!(
        String::from_utf8(result).unwrap(),
        "region\thigh_count\tlow_count\neast\t2\t1\nwest\t0\t1\n"
    );
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn grouped_case_rejects_mixed_connectives_and_different_columns() {
    for sql in [
        "SELECT region, sum(CASE WHEN priority = 'hi' OR priority = 'lo' AND priority = 'mid' THEN 1 ELSE 0 END) FROM part GROUP BY region",
        "SELECT region, sum(CASE WHEN priority = 'hi' OR region = 'west' THEN 1 ELSE 0 END) FROM part GROUP BY region",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}
